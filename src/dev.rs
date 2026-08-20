//! Repository development, release, marketplace, and benchmark workflows.
//!
//! This module keeps checkout-specific process supervision separate from the
//! runtime adapters while exposing every command through the public `windie`
//! CLI. It never creates a second executable or runtime path.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use flate2::{Compression, write::GzEncoder};
use sha2::{Digest, Sha256};
use tar::Builder;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::process::{Child, Command};
use tokio::time::{sleep, timeout};

use crate::cli::{BenchmarkCommand, DevCommand, DevComponent, MarketplaceCommand, ReleaseCommand};
use crate::config;
use crate::conversation::ConversationId;
use crate::llm::gateway::GatewayUrl;
use crate::llm::{BaseUrl, ModelName};
use crate::operation;
use crate::output::TerminalOutput;
use crate::perf::{self, BenchmarkMode, BenchmarkOptions};
use crate::plugin::{
    InstalledPlugin, MarketplaceIndex, MarketplacePlugin, MarketplacePresentation,
    MarketplaceVersion, PluginComponentKind,
};

const DEV_GATEWAY_START_TIMEOUT: Duration = Duration::from_secs(180);
const LOCAL_MARKETPLACE_PORT: u16 = 8788;

/// Runs the selected development workflow through the public CLI.
pub async fn run_dev(command: DevCommand) -> Result<()> {
    match command {
        DevCommand::Up => dev_up().await,
        DevCommand::Run { component } => dev_run(component).await,
        DevCommand::Status => dev_status().await,
        DevCommand::Down => dev_down().await,
    }
}

/// Runs the selected release workflow through the public CLI.
pub async fn run_release(command: ReleaseCommand) -> Result<()> {
    match command {
        ReleaseCommand::Build => release_script("package-release").await,
        ReleaseCommand::Install => release_script("test-local-installer").await,
        ReleaseCommand::Verify => release_verify().await,
    }
}

/// Runs the selected local marketplace workflow through the public CLI.
pub async fn run_marketplace(command: MarketplaceCommand) -> Result<()> {
    match command {
        MarketplaceCommand::Build => build_local_marketplace().map(|_| ()),
        MarketplaceCommand::Serve => serve_local_marketplace().await,
    }
}

/// Runs the selected deterministic benchmark workflow through the public CLI.
pub async fn run_benchmark(command: BenchmarkCommand) -> Result<()> {
    match command {
        BenchmarkCommand::Run {
            conversation_id,
            options,
        } => benchmark(conversation_id, options).await,
        BenchmarkCommand::CompareBaseline { options } => compare_baseline(options).await,
        BenchmarkCommand::UpdateBaseline { options } => update_baseline(options).await,
    }
}

/// Builds and runs gateway, API, and the HMR Inspector together.
async fn dev_up() -> Result<()> {
    println!("windie: starting development gateway");
    let mut gateway = spawn_gateway().await?;
    if let Err(error) = wait_for_gateway(&mut gateway).await {
        stop_child(&mut gateway).await;
        return Err(error);
    }

    let mut api = match spawn_component("api").await {
        Ok(child) => child,
        Err(error) => {
            stop_child(&mut gateway).await;
            return Err(error);
        }
    };
    let mut inspector = match spawn_component("inspector").await {
        Ok(child) => child,
        Err(error) => {
            stop_child(&mut api).await;
            stop_child(&mut gateway).await;
            return Err(error);
        }
    };
    println!("windie: development API and Inspector are running; press Ctrl-C to stop");

    let result = supervise_children(&mut gateway, &mut api, &mut inspector).await;
    stop_child(&mut api).await;
    stop_child(&mut inspector).await;
    stop_child(&mut gateway).await;
    result
}

/// Runs one development component in the foreground.
async fn dev_run(component: DevComponent) -> Result<()> {
    match component {
        DevComponent::Gateway => {
            let mut gateway = spawn_gateway().await?;
            if let Err(error) = wait_for_gateway(&mut gateway).await {
                stop_child(&mut gateway).await;
                return Err(error);
            }
            println!("windie: development gateway is running; press Ctrl-C to stop");
            let result = supervise_one(&mut gateway).await;
            stop_child(&mut gateway).await;
            result
        }
        DevComponent::Api | DevComponent::Inspector => {
            let component = match component {
                DevComponent::Api => "api",
                DevComponent::Inspector => "inspector",
                DevComponent::Gateway => unreachable!("gateway is handled above"),
            };
            let mut child = spawn_component(component).await?;
            println!("windie: development {component} is running; press Ctrl-C to stop");
            let result = supervise_one(&mut child).await;
            stop_child(&mut child).await;
            result
        }
    }
}

/// Reports health for all three local runtime endpoints.
async fn dev_status() -> Result<()> {
    println!("windie dev status");
    println!(
        "gateway: {}",
        health(&format!("{}/health", config::gateway_url())).await
    );
    println!(
        "api: {}",
        health(&format!("{}/api/health", config::api_url())).await
    );
    println!(
        "inspector: {}",
        health(&format!("http://{}/", config::inspector_address())).await
    );
    Ok(())
}

/// Stops all detached runtime components owned by the current environment.
async fn dev_down() -> Result<()> {
    for args in [
        ["api", "stop"].as_slice(),
        ["inspector", "stop"].as_slice(),
        ["gateway", "stop"].as_slice(),
    ] {
        run_windie(args).await?;
    }
    Ok(())
}

/// Builds the local marketplace artifact and index used for end-to-end tests.
fn build_local_marketplace() -> Result<PathBuf> {
    let root = repository_root()?;
    let package_root = root.join("packages/parallel-search");
    let package = InstalledPlugin::load(&package_root)?;
    let output_root = root.join("target/local-marketplace");
    if output_root.exists() {
        fs::remove_dir_all(&output_root).with_context(|| {
            format!(
                "failed to replace local marketplace {}",
                output_root.display()
            )
        })?;
    }

    let release_root = output_root.join("plugins/parallel-search/1.0.0");
    fs::create_dir_all(&release_root)?;
    fs::copy(
        package_root.join("plugin.json"),
        release_root.join("plugin.json"),
    )?;

    let archive_path = release_root.join("parallel-search-1.0.0.tar.gz");
    let archive = package_archive(&package_root)?;
    fs::write(&archive_path, &archive)?;
    let digest = format!("sha256:{:x}", Sha256::digest(&archive));

    let capabilities: Vec<String> = package
        .manifest
        .components
        .iter()
        .filter(|component| component.kind == PluginComponentKind::Mcp)
        .flat_map(|component| component.windie.capabilities.iter().cloned())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();

    let fixture_root = root.join("packages/local-mcp-fixture");
    let fixture = InstalledPlugin::load(&fixture_root)?;
    let fixture_release_root = output_root.join("plugins/local-mcp-fixture/1.0.0");
    fs::create_dir_all(&fixture_release_root)?;
    fs::copy(
        fixture_root.join("plugin.json"),
        fixture_release_root.join("plugin.json"),
    )?;
    let fixture_archive_path = fixture_release_root.join("local-mcp-fixture-1.0.0.tar.gz");
    let fixture_archive = package_archive(&fixture_root)?;
    fs::write(&fixture_archive_path, &fixture_archive)?;
    let fixture_digest = format!("sha256:{:x}", Sha256::digest(&fixture_archive));
    let fixture_capabilities: Vec<String> = fixture
        .manifest
        .components
        .iter()
        .filter(|component| component.kind == PluginComponentKind::Mcp)
        .flat_map(|component| component.windie.capabilities.iter().cloned())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();

    let desktop_root = root.join("packages/desktop-commander");
    let desktop = InstalledPlugin::load(&desktop_root)?;
    let desktop_release_root = output_root.join("plugins/desktop-commander/0.2.47");
    fs::create_dir_all(&desktop_release_root)?;
    fs::copy(
        desktop_root.join("plugin.json"),
        desktop_release_root.join("plugin.json"),
    )?;
    let desktop_archive_path = desktop_release_root.join("desktop-commander-0.2.47.tar.gz");
    let desktop_archive = package_archive(&desktop_root)?;
    fs::write(&desktop_archive_path, &desktop_archive)?;
    let desktop_digest = format!("sha256:{:x}", Sha256::digest(&desktop_archive));
    let desktop_capabilities: Vec<String> = desktop
        .manifest
        .components
        .iter()
        .filter(|component| component.kind == PluginComponentKind::Mcp)
        .flat_map(|component| component.windie.capabilities.iter().cloned())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();

    let basic_memory_root = root.join("packages/basic-memory");
    let basic_memory = InstalledPlugin::load(&basic_memory_root)?;
    let basic_memory_release_root = output_root.join("plugins/basic-memory/0.22.1");
    fs::create_dir_all(&basic_memory_release_root)?;
    fs::copy(
        basic_memory_root.join("plugin.json"),
        basic_memory_release_root.join("plugin.json"),
    )?;
    let basic_memory_archive_path = basic_memory_release_root.join("basic-memory-0.22.1.tar.gz");
    let basic_memory_archive = package_archive(&basic_memory_root)?;
    fs::write(&basic_memory_archive_path, &basic_memory_archive)?;
    let basic_memory_digest = format!("sha256:{:x}", Sha256::digest(&basic_memory_archive));
    let basic_memory_capabilities: Vec<String> = basic_memory
        .manifest
        .components
        .iter()
        .filter(|component| component.kind == PluginComponentKind::Mcp)
        .flat_map(|component| component.windie.capabilities.iter().cloned())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();

    let blender_root = root.join("packages/blender-mcp");
    let blender = InstalledPlugin::load(&blender_root)?;
    let blender_release_root = output_root.join("plugins/blender-mcp/1.8.3");
    fs::create_dir_all(&blender_release_root)?;
    fs::copy(
        blender_root.join("plugin.json"),
        blender_release_root.join("plugin.json"),
    )?;
    let blender_archive_path = blender_release_root.join("blender-mcp-1.8.3.tar.gz");
    let blender_archive = package_archive(&blender_root)?;
    fs::write(&blender_archive_path, &blender_archive)?;
    let blender_digest = format!("sha256:{:x}", Sha256::digest(&blender_archive));
    let blender_capabilities: Vec<String> = blender
        .manifest
        .components
        .iter()
        .filter(|component| component.kind == PluginComponentKind::Mcp)
        .flat_map(|component| component.windie.capabilities.iter().cloned())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();

    let chrome_root = root.join("packages/chrome-devtools");
    let chrome = InstalledPlugin::load(&chrome_root)?;
    let chrome_release_root = output_root.join("plugins/chrome-devtools/1.6.0");
    fs::create_dir_all(&chrome_release_root)?;
    fs::copy(
        chrome_root.join("plugin.json"),
        chrome_release_root.join("plugin.json"),
    )?;
    let chrome_archive_path = chrome_release_root.join("chrome-devtools-1.6.0.tar.gz");
    let chrome_archive = package_archive(&chrome_root)?;
    fs::write(&chrome_archive_path, &chrome_archive)?;
    let chrome_digest = format!("sha256:{:x}", Sha256::digest(&chrome_archive));
    let chrome_capabilities: Vec<String> = chrome
        .manifest
        .components
        .iter()
        .filter(|component| component.kind == PluginComponentKind::Mcp)
        .flat_map(|component| component.windie.capabilities.iter().cloned())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();

    let brightdata_root = root.join("packages/brightdata");
    let brightdata = InstalledPlugin::load(&brightdata_root)?;
    let brightdata_release_root = output_root.join("plugins/brightdata/2.11.1");
    fs::create_dir_all(&brightdata_release_root)?;
    fs::copy(
        brightdata_root.join("plugin.json"),
        brightdata_release_root.join("plugin.json"),
    )?;
    let brightdata_archive_path = brightdata_release_root.join("brightdata-2.11.1.tar.gz");
    let brightdata_archive = package_archive(&brightdata_root)?;
    fs::write(&brightdata_archive_path, &brightdata_archive)?;
    let brightdata_digest = format!("sha256:{:x}", Sha256::digest(&brightdata_archive));
    let brightdata_capabilities: Vec<String> = brightdata
        .manifest
        .components
        .iter()
        .filter(|component| component.kind == PluginComponentKind::Mcp)
        .flat_map(|component| component.windie.capabilities.iter().cloned())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();

    let cua_root = root.join("packages/cua-driver");
    let cua = InstalledPlugin::load(&cua_root)?;
    let cua_release_root = output_root.join("plugins/cua-driver/0.12.6");
    fs::create_dir_all(&cua_release_root)?;
    fs::copy(
        cua_root.join("plugin.json"),
        cua_release_root.join("plugin.json"),
    )?;
    let cua_archive_path = cua_release_root.join("cua-driver-0.12.6.tar.gz");
    let cua_archive = package_archive(&cua_root)?;
    fs::write(&cua_archive_path, &cua_archive)?;
    let cua_digest = format!("sha256:{:x}", Sha256::digest(&cua_archive));
    let cua_capabilities: Vec<String> = cua
        .manifest
        .components
        .iter()
        .filter(|component| component.kind == PluginComponentKind::Mcp)
        .flat_map(|component| component.windie.capabilities.iter().cloned())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();

    for (package_root, release_root, package) in [
        (&package_root, &release_root, &package),
        (&fixture_root, &fixture_release_root, &fixture),
        (&desktop_root, &desktop_release_root, &desktop),
        (
            &basic_memory_root,
            &basic_memory_release_root,
            &basic_memory,
        ),
        (&blender_root, &blender_release_root, &blender),
        (&chrome_root, &chrome_release_root, &chrome),
        (&brightdata_root, &brightdata_release_root, &brightdata),
        (&cua_root, &cua_release_root, &cua),
    ] {
        copy_presentation_assets(package_root, release_root, package)?;
    }

    let index = MarketplaceIndex {
        index_version: 1,
        plugins: vec![
            MarketplacePlugin {
                id: package.manifest.plugin.id.clone(),
                versions: vec![MarketplaceVersion {
                    version: package.manifest.plugin.version.clone(),
                    components: package
                        .manifest
                        .components
                        .iter()
                        .map(|component| component.kind.to_string())
                        .collect(),
                    capabilities: capabilities.into_iter().collect(),
                    presentation: Some(marketplace_presentation(
                        &package,
                        "plugins/parallel-search/1.0.0",
                    )),
                    manifest_url: "plugins/parallel-search/1.0.0/plugin.json".to_string(),
                    artifact_url: "plugins/parallel-search/1.0.0/parallel-search-1.0.0.tar.gz"
                        .to_string(),
                    digest,
                    publisher: package.manifest.plugin.publisher.clone(),
                    status: "verified".to_string(),
                }],
            },
            MarketplacePlugin {
                id: fixture.manifest.plugin.id.clone(),
                versions: vec![MarketplaceVersion {
                    version: fixture.manifest.plugin.version.clone(),
                    components: fixture
                        .manifest
                        .components
                        .iter()
                        .map(|component| component.kind.to_string())
                        .collect(),
                    capabilities: fixture_capabilities,
                    presentation: Some(marketplace_presentation(
                        &fixture,
                        "plugins/local-mcp-fixture/1.0.0",
                    )),
                    manifest_url: "plugins/local-mcp-fixture/1.0.0/plugin.json".to_string(),
                    artifact_url: "plugins/local-mcp-fixture/1.0.0/local-mcp-fixture-1.0.0.tar.gz"
                        .to_string(),
                    digest: fixture_digest,
                    publisher: fixture.manifest.plugin.publisher.clone(),
                    status: "verified".to_string(),
                }],
            },
            MarketplacePlugin {
                id: desktop.manifest.plugin.id.clone(),
                versions: vec![MarketplaceVersion {
                    version: desktop.manifest.plugin.version.clone(),
                    components: desktop
                        .manifest
                        .components
                        .iter()
                        .map(|component| component.kind.to_string())
                        .collect(),
                    capabilities: desktop_capabilities,
                    presentation: Some(marketplace_presentation(
                        &desktop,
                        "plugins/desktop-commander/0.2.47",
                    )),
                    manifest_url: "plugins/desktop-commander/0.2.47/plugin.json".to_string(),
                    artifact_url:
                        "plugins/desktop-commander/0.2.47/desktop-commander-0.2.47.tar.gz"
                            .to_string(),
                    digest: desktop_digest,
                    publisher: desktop.manifest.plugin.publisher.clone(),
                    status: "verified".to_string(),
                }],
            },
            MarketplacePlugin {
                id: basic_memory.manifest.plugin.id.clone(),
                versions: vec![MarketplaceVersion {
                    version: basic_memory.manifest.plugin.version.clone(),
                    components: basic_memory
                        .manifest
                        .components
                        .iter()
                        .map(|component| component.kind.to_string())
                        .collect(),
                    capabilities: basic_memory_capabilities,
                    presentation: Some(marketplace_presentation(
                        &basic_memory,
                        "plugins/basic-memory/0.22.1",
                    )),
                    manifest_url: "plugins/basic-memory/0.22.1/plugin.json".to_string(),
                    artifact_url: "plugins/basic-memory/0.22.1/basic-memory-0.22.1.tar.gz"
                        .to_string(),
                    digest: basic_memory_digest,
                    publisher: basic_memory.manifest.plugin.publisher.clone(),
                    status: "verified".to_string(),
                }],
            },
            MarketplacePlugin {
                id: blender.manifest.plugin.id.clone(),
                versions: vec![MarketplaceVersion {
                    version: blender.manifest.plugin.version.clone(),
                    components: blender
                        .manifest
                        .components
                        .iter()
                        .map(|component| component.kind.to_string())
                        .collect(),
                    capabilities: blender_capabilities,
                    presentation: Some(marketplace_presentation(
                        &blender,
                        "plugins/blender-mcp/1.8.3",
                    )),
                    manifest_url: "plugins/blender-mcp/1.8.3/plugin.json".to_string(),
                    artifact_url: "plugins/blender-mcp/1.8.3/blender-mcp-1.8.3.tar.gz".to_string(),
                    digest: blender_digest,
                    publisher: blender.manifest.plugin.publisher.clone(),
                    status: "verified".to_string(),
                }],
            },
            MarketplacePlugin {
                id: chrome.manifest.plugin.id.clone(),
                versions: vec![MarketplaceVersion {
                    version: chrome.manifest.plugin.version.clone(),
                    components: chrome
                        .manifest
                        .components
                        .iter()
                        .map(|component| component.kind.to_string())
                        .collect(),
                    capabilities: chrome_capabilities,
                    presentation: Some(marketplace_presentation(
                        &chrome,
                        "plugins/chrome-devtools/1.6.0",
                    )),
                    manifest_url: "plugins/chrome-devtools/1.6.0/plugin.json".to_string(),
                    artifact_url: "plugins/chrome-devtools/1.6.0/chrome-devtools-1.6.0.tar.gz"
                        .to_string(),
                    digest: chrome_digest,
                    publisher: chrome.manifest.plugin.publisher.clone(),
                    status: "verified".to_string(),
                }],
            },
            MarketplacePlugin {
                id: brightdata.manifest.plugin.id.clone(),
                versions: vec![MarketplaceVersion {
                    version: brightdata.manifest.plugin.version.clone(),
                    components: brightdata
                        .manifest
                        .components
                        .iter()
                        .map(|component| component.kind.to_string())
                        .collect(),
                    capabilities: brightdata_capabilities,
                    presentation: Some(marketplace_presentation(
                        &brightdata,
                        "plugins/brightdata/2.11.1",
                    )),
                    manifest_url: "plugins/brightdata/2.11.1/plugin.json".to_string(),
                    artifact_url: "plugins/brightdata/2.11.1/brightdata-2.11.1.tar.gz".to_string(),
                    digest: brightdata_digest,
                    publisher: brightdata.manifest.plugin.publisher.clone(),
                    status: "verified".to_string(),
                }],
            },
            MarketplacePlugin {
                id: cua.manifest.plugin.id.clone(),
                versions: vec![MarketplaceVersion {
                    version: cua.manifest.plugin.version.clone(),
                    components: cua
                        .manifest
                        .components
                        .iter()
                        .map(|component| component.kind.to_string())
                        .collect(),
                    capabilities: cua_capabilities,
                    presentation: Some(marketplace_presentation(&cua, "plugins/cua-driver/0.12.6")),
                    manifest_url: "plugins/cua-driver/0.12.6/plugin.json".to_string(),
                    artifact_url: "plugins/cua-driver/0.12.6/cua-driver-0.12.6.tar.gz".to_string(),
                    digest: cua_digest,
                    publisher: cua.manifest.plugin.publisher.clone(),
                    status: "verified".to_string(),
                }],
            },
        ],
    };
    index.validate()?;
    fs::create_dir_all(&output_root)?;
    fs::write(
        output_root.join("index.json"),
        serde_json::to_vec_pretty(&index)?,
    )?;

    println!("local marketplace built at {}", output_root.display());
    println!("index: {}", output_root.join("index.json").display());
    println!("artifact: {}", archive_path.display());
    println!("fixture artifact: {}", fixture_archive_path.display());
    println!(
        "desktop commander artifact: {}",
        desktop_archive_path.display()
    );
    println!(
        "basic memory artifact: {}",
        basic_memory_archive_path.display()
    );
    println!("blender MCP artifact: {}", blender_archive_path.display());
    println!(
        "Chrome DevTools artifact: {}",
        chrome_archive_path.display()
    );
    println!("CUA Driver artifact: {}", cua_archive_path.display());
    Ok(output_root)
}

/// Serves the generated local marketplace over localhost HTTP.
async fn serve_local_marketplace() -> Result<()> {
    let root = build_local_marketplace()?;
    let listener = TcpListener::bind(("127.0.0.1", LOCAL_MARKETPLACE_PORT)).await?;
    println!(
        "local marketplace serving {} at http://127.0.0.1:{LOCAL_MARKETPLACE_PORT}",
        root.display()
    );
    println!(
        "set WINDIE_MARKETPLACE_INDEX_URL=http://127.0.0.1:{LOCAL_MARKETPLACE_PORT}/index.json"
    );

    let shutdown = tokio::signal::ctrl_c();
    tokio::pin!(shutdown);
    loop {
        tokio::select! {
            result = listener.accept() => {
                let (stream, _) = result?;
                let root = root.clone();
                tokio::spawn(async move {
                    if let Err(error) = serve_marketplace_connection(stream, root).await {
                        eprintln!("local marketplace request failed: {error}");
                    }
                });
            }
            result = &mut shutdown => {
                result.context("local marketplace shutdown signal failed")?;
                return Ok(());
            }
        }
    }
}

/// Serves one safe GET request from the generated marketplace directory.
async fn serve_marketplace_connection(
    mut stream: tokio::net::TcpStream,
    root: PathBuf,
) -> Result<()> {
    let mut request = [0_u8; 8192];
    let count = stream.read(&mut request).await?;
    if count == 0 {
        return Ok(());
    }

    let request = String::from_utf8_lossy(&request[..count]);
    let mut parts = request
        .lines()
        .next()
        .unwrap_or_default()
        .split_whitespace();
    let method = parts.next().unwrap_or_default();
    let request_path = parts.next().unwrap_or_default();
    if method != "GET" && method != "HEAD" {
        write_marketplace_response(&mut stream, "405 Method Not Allowed", b"method not allowed")
            .await?;
        return Ok(());
    }

    let relative = request_path
        .split('?')
        .next()
        .unwrap_or_default()
        .strip_prefix('/')
        .unwrap_or_default();
    if relative.is_empty() || relative.contains("..") {
        write_marketplace_response(&mut stream, "404 Not Found", b"not found").await?;
        return Ok(());
    }

    let file = root.join(relative);
    if !file.is_file() {
        write_marketplace_response(&mut stream, "404 Not Found", b"not found").await?;
        return Ok(());
    }

    let body = fs::read(&file)?;
    let content_type = match file.extension().and_then(|extension| extension.to_str()) {
        Some("json") => "application/json",
        Some("gz") => "application/gzip",
        _ => "application/octet-stream",
    };
    let header = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: {content_type}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(header.as_bytes()).await?;
    if method == "GET" {
        stream.write_all(&body).await?;
    }
    Ok(())
}

async fn write_marketplace_response(
    stream: &mut tokio::net::TcpStream,
    status: &str,
    body: &[u8],
) -> Result<()> {
    let header = format!(
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(header.as_bytes()).await?;
    stream.write_all(body).await?;
    Ok(())
}

fn package_archive(package_root: &Path) -> Result<Vec<u8>> {
    let encoder = GzEncoder::new(Vec::new(), Compression::default());
    let mut archive = Builder::new(encoder);
    append_package_files(&mut archive, package_root, package_root)?;
    let encoder = archive.into_inner()?;
    Ok(encoder.finish()?)
}

/// Copies the public presentation assets beside one generated release entry.
fn copy_presentation_assets(
    package_root: &Path,
    release_root: &Path,
    package: &InstalledPlugin,
) -> Result<()> {
    for relative in [
        &package.manifest.presentation.readme,
        &package.manifest.presentation.icon,
    ] {
        let source = package_root.join(relative);
        let destination = release_root.join(relative);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(&source, &destination).with_context(|| {
            format!(
                "failed to copy marketplace presentation asset {}",
                source.display()
            )
        })?;
    }
    Ok(())
}

/// Builds discovery presentation URLs from one plugin manifest.
fn marketplace_presentation(
    package: &InstalledPlugin,
    release_root: &str,
) -> MarketplacePresentation {
    MarketplacePresentation {
        name: package.manifest.presentation.name.clone(),
        description: package.manifest.presentation.description.clone(),
        readme_url: Some(format!(
            "{release_root}/{}",
            package.manifest.presentation.readme
        )),
        icon_url: Some(format!(
            "{release_root}/{}",
            package.manifest.presentation.icon
        )),
    }
}

fn append_package_files(
    archive: &mut Builder<GzEncoder<Vec<u8>>>,
    package_root: &Path,
    current: &Path,
) -> Result<()> {
    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            append_package_files(archive, package_root, &path)?;
        } else if entry.file_type()?.is_file() {
            let relative = path
                .strip_prefix(package_root)
                .expect("package file should be below package root");
            archive.append_path_with_name(&path, relative)?;
        } else {
            bail!(
                "cannot package unsupported plugin entry: {}",
                path.display()
            );
        }
    }
    Ok(())
}

/// Builds and starts the current Bifrost source as one foreground process.
async fn spawn_gateway() -> Result<Child> {
    let root = repository_root()?;
    let bifrost_root = root.join("vendor/bifrost");
    let transport_root = bifrost_root.join("transports/bifrost-http");
    if !transport_root.join("main.go").is_file() {
        bail!("Bifrost source is missing at {}", transport_root.display());
    }
    prepare_bifrost_workspace(&bifrost_root).await?;

    let app_dir = crate::local::windie_home_dir()?.join("bifrost/data");
    let port = gateway_url().port();
    let executable = root
        .join("target/development")
        .join(executable_name("bifrost-http"));
    fs::create_dir_all(
        executable
            .parent()
            .expect("Bifrost binary has a parent directory"),
    )
    .with_context(|| format!("failed to create {}", executable.display()))?;
    let build_status = Command::new("go")
        .args(["build", "-tags", "dev", "-o"])
        .arg(&executable)
        .arg("./transports/bifrost-http")
        .current_dir(&bifrost_root)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .await
        .context("failed to build Bifrost")?;
    if !build_status.success() {
        bail!("Bifrost build exited with {build_status}");
    }

    let mut command = Command::new(&executable);
    command
        .args(["-host", "127.0.0.1", "-port"])
        .arg(port)
        .arg("-app-dir")
        .arg(app_dir)
        .current_dir(&bifrost_root)
        .env("BIFROST_UI_DEV", "true")
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    command
        .spawn()
        .context("failed to start the Bifrost process")
}

/// Waits for the directly launched Bifrost process to become healthy.
async fn wait_for_gateway(child: &mut Child) -> Result<()> {
    let health_url = format!("{}/health", config::gateway_url());
    for _ in 0..(DEV_GATEWAY_START_TIMEOUT.as_millis() / 200) {
        if health(&health_url).await == "running" {
            return Ok(());
        }
        if let Some(status) = child.try_wait().context("failed to poll Bifrost process")? {
            bail!("Bifrost development process exited with {status}");
        }
        sleep(Duration::from_millis(200)).await;
    }
    bail!(
        "Bifrost did not become healthy within {} seconds",
        DEV_GATEWAY_START_TIMEOUT.as_secs()
    )
}

/// Creates the local Bifrost Go workspace used by the development build.
async fn prepare_bifrost_workspace(bifrost_root: &Path) -> Result<()> {
    if !bifrost_root.join("go.work").is_file() {
        run_go(
            bifrost_root,
            &[
                "work",
                "init",
                "./cli",
                "./core",
                "./framework",
                "./transports",
            ],
        )
        .await?;
    }

    let mut modules = vec![
        "./cli".to_string(),
        "./core".to_string(),
        "./framework".to_string(),
        "./transports".to_string(),
    ];
    let plugins = bifrost_root.join("plugins");
    if plugins.is_dir() {
        for entry in fs::read_dir(plugins).context("failed to read Bifrost plugins")? {
            let path = entry?.path();
            if path.join("go.mod").is_file() {
                let name = path
                    .file_name()
                    .ok_or_else(|| anyhow!("invalid Bifrost plugin path {}", path.display()))?
                    .to_string_lossy();
                modules.push(format!("./plugins/{name}"));
            }
        }
    }
    for module in modules {
        run_go(bifrost_root, &["work", "use", module.as_str()]).await?;
    }
    run_go(bifrost_root, &["work", "sync"]).await
}

/// Runs one local Go workspace command and preserves its terminal output.
async fn run_go(directory: &Path, args: &[&str]) -> Result<()> {
    let status = Command::new("go")
        .args(args)
        .current_dir(directory)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .await
        .context("failed to run Go workspace command")?;
    if status.success() {
        Ok(())
    } else {
        bail!("Go workspace command exited with {status}")
    }
}

/// Builds and starts one foreground development component.
async fn spawn_component(component: &str) -> Result<Child> {
    let root = repository_root()?;
    let mut command = if component == "inspector" {
        let mut command = Command::new(npm_command());
        command
            .arg("start")
            .arg("--prefix")
            .arg(root.join("vendor/windie-inspector/frontend"));
        command.env("BROWSER", "none");
        command
    } else if component == "api" {
        let executable = build_windie_binary(&root).await?;
        let mut command = Command::new(executable);
        command.args(["api", "run"]);
        command
    } else {
        bail!("unknown development component {component}");
    };

    command
        .current_dir(root)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    command
        .spawn()
        .with_context(|| format!("failed to start development {component}"))
}

/// Builds the current Windie API executable and returns its debug path.
async fn build_windie_binary(root: &Path) -> Result<PathBuf> {
    let target_directory = cargo_target_directory(root).await?;
    let status = Command::new("cargo")
        .args(["build", "--bin", "windie"])
        .current_dir(root)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .await
        .context("failed to build the Windie API")?;
    if !status.success() {
        bail!("Windie API build exited with {status}");
    }

    let executable = target_directory
        .join("debug")
        .join(executable_name("windie"));
    if !executable.is_file() {
        bail!(
            "Windie API executable was not produced at {}",
            executable.display()
        );
    }
    Ok(executable)
}

/// Finds Cargo's effective target directory for the current workspace.
async fn cargo_target_directory(root: &Path) -> Result<PathBuf> {
    let output = Command::new("cargo")
        .args(["metadata", "--no-deps", "--format-version", "1"])
        .current_dir(root)
        .output()
        .await
        .context("failed to inspect the Cargo target directory")?;
    if !output.status.success() {
        bail!(
            "Cargo metadata failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let metadata: serde_json::Value =
        serde_json::from_slice(&output.stdout).context("Cargo metadata returned invalid JSON")?;
    let target_directory = metadata
        .get("target_directory")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| anyhow!("Cargo metadata did not report a target directory"))?;
    Ok(PathBuf::from(target_directory))
}

/// Adds the platform executable suffix used by local development binaries.
fn executable_name(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

/// Waits until one foreground child exits or Ctrl-C is pressed.
async fn supervise_children(
    gateway: &mut Child,
    api: &mut Child,
    inspector: &mut Child,
) -> Result<()> {
    loop {
        if let Some(status) = gateway
            .try_wait()
            .context("failed to poll Bifrost process")?
        {
            return Err(anyhow!("Bifrost development process exited with {status}"));
        }
        if let Some(status) = api.try_wait().context("failed to poll API process")? {
            return Err(anyhow!("API development process exited with {status}"));
        }
        if let Some(status) = inspector
            .try_wait()
            .context("failed to poll Inspector process")?
        {
            return Err(anyhow!(
                "Inspector development process exited with {status}"
            ));
        }
        if ctrl_c_or_tick().await {
            return Ok(());
        }
    }
}

/// Waits until one foreground child exits or Ctrl-C is pressed.
async fn supervise_one(child: &mut Child) -> Result<()> {
    loop {
        if let Some(status) = child.try_wait().context("failed to poll process")? {
            return Err(anyhow!("development process exited with {status}"));
        }
        if ctrl_c_or_tick().await {
            return Ok(());
        }
    }
}

/// Returns true for Ctrl-C and false for the normal polling tick.
async fn ctrl_c_or_tick() -> bool {
    tokio::select! {
        result = tokio::signal::ctrl_c() => result.is_ok(),
        _ = sleep(Duration::from_millis(250)) => false,
    }
}

/// Terminates one child and tolerates a process that already exited.
async fn stop_child(child: &mut Child) {
    if child.try_wait().ok().flatten().is_none() {
        let _ = child.kill().await;
        let _ = timeout(Duration::from_secs(3), child.wait()).await;
    }
}

/// Runs the public Windie executable through Cargo so the dev command uses the
/// current checkout rather than a stale installed binary.
async fn run_windie(args: &[&str]) -> Result<()> {
    let root = repository_root()?;
    let status = Command::new("cargo")
        .args(["run", "--bin", "windie", "--"])
        .args(args)
        .current_dir(root)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .await
        .context("failed to run windie lifecycle command")?;
    if status.success() {
        Ok(())
    } else {
        bail!("windie lifecycle command exited with {status}")
    }
}

/// Executes one platform-native release helper from the repository root.
async fn release_script(script: &str) -> Result<()> {
    let root = repository_root()?;
    let package_args = if script == "package-release" {
        Some((
            release_target()?,
            release_asset_label()?,
            root.join("target/release-dist"),
        ))
    } else {
        None
    };
    #[cfg(windows)]
    let mut command = {
        let mut command = Command::new("powershell");
        command
            .args(["-ExecutionPolicy", "Bypass", "-File"])
            .arg(root.join(format!("scripts/{script}.ps1")));
        command
    };
    #[cfg(not(windows))]
    let mut command = {
        let mut command = Command::new("bash");
        command.arg(root.join(format!("scripts/{script}.sh")));
        command
    };
    if let Some((target, label, dist)) = package_args {
        command.args([target, label]).arg(dist);
    }
    let status = command.current_dir(root).status().await?;

    if status.success() {
        Ok(())
    } else {
        bail!("release command {script} exited with {status}")
    }
}

/// Returns the native Rust target used by the checked-in release scripts.
fn release_target() -> Result<String> {
    match (env::consts::OS, env::consts::ARCH) {
        ("macos", "aarch64") => Ok("aarch64-apple-darwin".to_string()),
        ("macos", "x86_64") => Ok("x86_64-apple-darwin".to_string()),
        ("linux", "aarch64") => Ok("aarch64-unknown-linux-gnu".to_string()),
        ("linux", "x86_64") => Ok("x86_64-unknown-linux-gnu".to_string()),
        ("windows", "x86_64") => Ok("x86_64-pc-windows-msvc".to_string()),
        _ => bail!("unsupported host for native release packaging"),
    }
}

/// Returns the asset label used by the public installer for the current host.
fn release_asset_label() -> Result<String> {
    match (env::consts::OS, env::consts::ARCH) {
        ("macos", "aarch64") => Ok("macos-aarch64".to_string()),
        ("macos", "x86_64") => Ok("macos-x86_64".to_string()),
        ("linux", "aarch64") => Ok("linux-aarch64".to_string()),
        ("linux", "x86_64") => Ok("linux-x86_64".to_string()),
        ("windows", "x86_64") => Ok("windows-x86_64".to_string()),
        _ => bail!("unsupported host for release asset packaging"),
    }
}

/// Verifies that the local installer produced an executable that responds.
async fn release_verify() -> Result<()> {
    let install_dir = local_install_dir()?;
    let executable = install_dir.join(if cfg!(windows) {
        "windie.exe"
    } else {
        "windie"
    });
    if !executable.is_file() {
        bail!(
            "local Windie executable was not found at {}",
            executable.display()
        );
    }
    let version = Command::new(&executable)
        .arg("--version")
        .output()
        .await
        .with_context(|| format!("failed to run {}", executable.display()))?;
    if !version.status.success() {
        bail!("local Windie version check failed");
    }
    print!("{}", String::from_utf8_lossy(&version.stdout));
    println!("verified: {}", executable.display());
    Ok(())
}

/// Finds the isolated local-installer binary for the current host.
fn local_install_dir() -> Result<PathBuf> {
    if let Ok(path) = env::var("WINDIE_INSTALL_DIR") {
        return Ok(PathBuf::from(path));
    }
    let root = repository_root()?;
    let label = match (env::consts::OS, env::consts::ARCH) {
        ("macos", "aarch64") => "macos-aarch64",
        ("macos", "x86_64") => "macos-x86_64",
        ("linux", "aarch64") => "linux-aarch64",
        ("linux", "x86_64") => "linux-x86_64",
        ("windows", "aarch64") => "windows-aarch64",
        ("windows", "x86_64") => "windows-x86_64",
        _ => bail!("unsupported host for local release verification"),
    };
    Ok(env::var("WINDIE_LOCAL_TEST_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| root.join("target/local-installer").join(label))
        .join("bin"))
}

/// Runs one provider-free benchmark command through the shared perf module.
async fn benchmark(
    conversation_id: Option<ConversationId>,
    options: BenchmarkOptions,
) -> Result<()> {
    let mode = if conversation_id.is_some() {
        BenchmarkMode::Conversation
    } else {
        BenchmarkMode::Local
    };
    let model = benchmark_model().await?;
    let output = TerminalOutput;
    if options.runs == 1 && !options.json {
        let baseline = perf::run(
            mode,
            conversation_id,
            gateway_url(),
            base_url(),
            model.clone(),
            &options.categories,
        )
        .await?;
        output.performance_baseline(&baseline);
    } else {
        let report = perf::run_report(
            mode,
            conversation_id,
            gateway_url(),
            base_url(),
            model,
            &options,
        )
        .await?;
        if options.json {
            output.performance_report_json(&report)?;
        } else {
            output.performance_report(&report);
        }
    }
    Ok(())
}

/// Compares the current local benchmark run with the checked-in baseline.
async fn compare_baseline(options: BenchmarkOptions) -> Result<()> {
    let model = benchmark_model().await?;
    let baseline_path = perf::default_baseline_path()?;
    let baseline = perf::read_report(&baseline_path)?;
    let current = perf::run_report(
        BenchmarkMode::Local,
        None,
        gateway_url(),
        base_url(),
        model,
        &options,
    )
    .await?;
    TerminalOutput.performance_comparison(&perf::compare_reports(&baseline, &current));
    Ok(())
}

/// Replaces the checked-in benchmark baseline with a current local run.
async fn update_baseline(options: BenchmarkOptions) -> Result<()> {
    let model = benchmark_model().await?;
    let baseline_path = perf::default_baseline_path()?;
    let report = perf::run_report(
        BenchmarkMode::Local,
        None,
        gateway_url(),
        base_url(),
        model,
        &options,
    )
    .await?;
    perf::write_report(&baseline_path, &report)?;
    TerminalOutput.updated_baseline(&baseline_path);
    Ok(())
}

/// Selects the first model currently exposed by Bifrost for development
/// benchmarks when the caller has not supplied a model override.
async fn benchmark_model() -> Result<ModelName> {
    operation::list_models(gateway_url(), base_url())
        .await?
        .into_iter()
        .next()
        .map(|model| ModelName::new(model.id))
        .ok_or_else(|| anyhow!("no models are available; configure a provider key first"))
}

fn gateway_url() -> GatewayUrl {
    GatewayUrl::new(config::gateway_url())
}

fn base_url() -> BaseUrl {
    BaseUrl::new(
        env::var("WINDIE_BASE_URL").unwrap_or_else(|_| format!("{}/v1", config::gateway_url())),
    )
}

async fn health(url: &str) -> &'static str {
    match reqwest::Client::new().get(url).send().await {
        Ok(response) if response.status().is_success() => "running",
        _ => "stopped",
    }
}

fn repository_root() -> Result<PathBuf> {
    if let Ok(root) = env::var("WINDIE_REPOSITORY_ROOT") {
        let root = PathBuf::from(root);
        if root.join("Cargo.toml").is_file() {
            return Ok(root);
        }
    }
    let current = env::current_dir().context("failed to determine repository root")?;
    if current.join("Cargo.toml").is_file() {
        return Ok(current);
    }
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    Ok(manifest.to_path_buf())
}

fn npm_command() -> &'static str {
    if cfg!(windows) { "npm.cmd" } else { "npm" }
}

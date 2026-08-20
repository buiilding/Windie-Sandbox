//! Repository development, release, marketplace, and benchmark workflows.
//!
//! This module keeps checkout-specific process supervision separate from the
//! runtime adapters while exposing every command through the public `windie`
//! CLI. It never creates a second executable or runtime path.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

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
        MarketplaceCommand::Publish => publish_marketplace().await,
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

/// One generated marketplace output, separated into a catalog site and archive assets.
struct MarketplaceBuild {
    site_root: PathBuf,
    archives: Vec<PathBuf>,
}

/// Selects whether generated archives remain local or are served by GitHub Releases.
enum MarketplaceArchiveSource {
    Local,
    GitHubRelease {
        repository: String,
        release_tag: String,
    },
}

/// Builds the local marketplace catalog and archives used for end-to-end tests.
fn build_local_marketplace() -> Result<PathBuf> {
    let root = repository_root()?;
    let output_root = root.join("target/local-marketplace");
    build_marketplace(&root, &output_root, &MarketplaceArchiveSource::Local)?;
    println!("local marketplace built at {}", output_root.display());
    println!("index: {}", output_root.join("index.json").display());
    Ok(output_root)
}

/// Builds one static marketplace from every package that explicitly opts in through
/// `plugin.json`. The generated index is the only catalog source; it is never
/// edited by hand because it must match the archive digests exactly.
fn build_marketplace(
    root: &Path,
    output_root: &Path,
    archive_source: &MarketplaceArchiveSource,
) -> Result<MarketplaceBuild> {
    if output_root.exists() {
        fs::remove_dir_all(output_root).with_context(|| {
            format!(
                "failed to replace marketplace output {}",
                output_root.display()
            )
        })?;
    }

    let (site_root, archive_root) = match archive_source {
        MarketplaceArchiveSource::Local => (output_root.to_path_buf(), output_root.to_path_buf()),
        MarketplaceArchiveSource::GitHubRelease { .. } => {
            (output_root.join("site"), output_root.join("archives"))
        }
    };
    fs::create_dir_all(&site_root)?;
    fs::create_dir_all(&archive_root)?;

    let packages = discover_marketplace_packages(root)?;
    let mut archives = Vec::with_capacity(packages.len());
    let mut plugins = Vec::with_capacity(packages.len());
    for package in packages {
        let id = &package.manifest.plugin.id;
        let version = &package.manifest.plugin.version;
        let release_path = format!("plugins/{id}/{version}");
        let release_root = site_root.join(&release_path);
        fs::create_dir_all(&release_root)?;
        fs::copy(
            package.root.join("plugin.json"),
            release_root.join("plugin.json"),
        )?;
        copy_presentation_assets(&package.root, &release_root, &package)?;

        let archive_name = format!("{id}-{version}.tar.gz");
        let archive = package_archive(&package.root)?;
        let archive_path = match archive_source {
            MarketplaceArchiveSource::Local => release_root.join(&archive_name),
            MarketplaceArchiveSource::GitHubRelease { .. } => archive_root.join(&archive_name),
        };
        fs::write(&archive_path, &archive)?;
        let digest = format!("sha256:{:x}", Sha256::digest(&archive));
        let artifact_url = match archive_source {
            MarketplaceArchiveSource::Local => format!("{release_path}/{archive_name}"),
            MarketplaceArchiveSource::GitHubRelease {
                repository,
                release_tag,
            } => format!(
                "https://github.com/{repository}/releases/download/{release_tag}/{archive_name}"
            ),
        };
        let capabilities = package
            .manifest
            .components
            .iter()
            .filter(|component| component.kind == PluginComponentKind::Mcp)
            .flat_map(|component| component.windie.capabilities.iter().cloned())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();

        plugins.push(MarketplacePlugin {
            id: id.clone(),
            versions: vec![MarketplaceVersion {
                version: version.clone(),
                components: package
                    .manifest
                    .components
                    .iter()
                    .map(|component| component.kind.to_string())
                    .collect(),
                capabilities,
                presentation: Some(marketplace_presentation(&package, &release_path)),
                manifest_url: format!("{release_path}/plugin.json"),
                artifact_url,
                digest,
                publisher: package.manifest.plugin.publisher.clone(),
                status: "verified".to_string(),
            }],
        });
        archives.push(archive_path);
    }

    let index = MarketplaceIndex {
        index_version: 1,
        plugins,
    };
    index.validate()?;
    fs::write(
        site_root.join("index.json"),
        serde_json::to_vec_pretty(&index)?,
    )?;

    Ok(MarketplaceBuild {
        site_root,
        archives,
    })
}

/// Loads only package directories that explicitly opt into marketplace publishing.
/// Sorting by plugin ID keeps generated indexes deterministic across filesystems.
fn discover_marketplace_packages(root: &Path) -> Result<Vec<InstalledPlugin>> {
    let packages_root = root.join("packages");
    let mut packages = Vec::new();
    let mut ids = std::collections::HashSet::new();
    for entry in fs::read_dir(&packages_root).with_context(|| {
        format!(
            "failed to read package directory {}",
            packages_root.display()
        )
    })? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let package_root = entry.path();
        if !package_root.join("plugin.json").is_file() {
            continue;
        }
        let package = InstalledPlugin::load(&package_root)?;
        if !package.manifest.marketplace.publish {
            continue;
        }
        if !ids.insert(package.manifest.plugin.id.clone()) {
            bail!(
                "marketplace contains duplicate plugin id: {}",
                package.manifest.plugin.id
            );
        }
        packages.push(package);
    }
    packages.sort_by(|left, right| left.manifest.plugin.id.cmp(&right.manifest.plugin.id));
    if packages.is_empty() {
        bail!("no packages opted into marketplace publishing")
    }
    Ok(packages)
}

/// Publishes one immutable archive set to GitHub Releases, then deploys the
/// small catalog site to the existing `windie-marketplace` Vercel project.
async fn publish_marketplace() -> Result<()> {
    let root = repository_root()?;
    let repository = github_repository(&root).await?;
    let release_tag = marketplace_release_tag()?;
    let output_root = root.join("target/marketplace-publish");
    let build = build_marketplace(
        &root,
        &output_root,
        &MarketplaceArchiveSource::GitHubRelease {
            repository: repository.clone(),
            release_tag: release_tag.clone(),
        },
    )?;

    let title = format!("Windie marketplace {release_tag}");
    let status = Command::new("gh")
        .args(["release", "create", &release_tag, "--repo", &repository])
        .args([
            "--title",
            &title,
            "--notes",
            "Generated marketplace plugin archives.",
        ])
        .args(&build.archives)
        .current_dir(&root)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .await
        .context("failed to start GitHub Release publishing")?;
    if !status.success() {
        bail!("GitHub Release publishing exited with {status}");
    }

    let status = Command::new("vercel")
        .args(["link", "--yes", "--project", "windie-marketplace"])
        .current_dir(&build.site_root)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .await
        .context("failed to link the Vercel marketplace project")?;
    if !status.success() {
        bail!("Vercel marketplace project link exited with {status}");
    }

    let status = Command::new("vercel")
        .args(["deploy", "--prod", "--yes"])
        .current_dir(&build.site_root)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .await
        .context("failed to start Vercel marketplace deployment")?;
    if !status.success() {
        bail!("Vercel marketplace deployment exited with {status}");
    }

    println!("published marketplace release {release_tag}");
    println!("catalog: https://marketplace.windieos.com/index.json");
    Ok(())
}

/// Produces a unique, sortable release tag without requiring publishing users
/// to choose one. GitHub Release asset URLs remain immutable once published.
fn marketplace_release_tag() -> Result<String> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before the Unix epoch")?
        .as_secs();
    Ok(format!("marketplace-{timestamp}"))
}

/// Reads the GitHub owner/repository from this checkout's `origin` remote.
async fn github_repository(root: &Path) -> Result<String> {
    let output = Command::new("git")
        .args(["remote", "get-url", "origin"])
        .current_dir(root)
        .output()
        .await
        .context("failed to read the origin Git remote")?;
    if !output.status.success() {
        bail!("failed to read the origin Git remote: {}", output.status);
    }
    let remote = String::from_utf8(output.stdout).context("origin Git remote is not UTF-8")?;
    let remote = remote.trim().trim_end_matches(".git");
    let repository = remote
        .strip_prefix("https://github.com/")
        .or_else(|| remote.strip_prefix("git@github.com:"))
        .ok_or_else(|| anyhow!("origin must be a GitHub remote, found: {remote}"))?;
    if repository.split('/').count() != 2 || repository.split('/').any(str::is_empty) {
        bail!("origin must identify one GitHub owner/repository, found: {remote}");
    }
    Ok(repository.to_string())
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

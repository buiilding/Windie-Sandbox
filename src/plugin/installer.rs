//! Marketplace artifact acquisition.
//!
//! This module owns the network boundary between a marketplace index and the
//! Windie plugin store. It downloads an immutable artifact, verifies its
//! declared digest, and delegates safe extraction and publication to the
//! plugin store.

use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use reqwest::blocking::Client;
use sha2::{Digest, Sha256};

use super::{InstalledPlugin, MarketplaceIndex, PluginStore};

const INDEX_TIMEOUT: Duration = Duration::from_secs(30);
const ARTIFACT_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const MAX_INDEX_BYTES: u64 = 4 * 1024 * 1024;
const MAX_ARTIFACT_BYTES: u64 = 512 * 1024 * 1024;

/// Downloads and installs plugins from a marketplace index.
#[derive(Debug, Clone)]
pub struct MarketplaceInstaller {
    client: Client,
}

impl Default for MarketplaceInstaller {
    fn default() -> Self {
        Self::new().expect("marketplace HTTP client should be constructible")
    }
}

impl MarketplaceInstaller {
    /// Builds a marketplace client with bounded network requests.
    pub fn new() -> Result<Self> {
        Ok(Self {
            client: Client::builder()
                .connect_timeout(Duration::from_secs(10))
                .timeout(ARTIFACT_TIMEOUT)
                .build()
                .context("failed to build marketplace HTTP client")?,
        })
    }

    /// Downloads and validates one marketplace index.
    pub fn fetch_index(&self, index_url: &str) -> Result<MarketplaceIndex> {
        let index_url = secure_marketplace_url(index_url, "marketplace index")?;
        let response = self
            .client
            .get(index_url.as_str())
            .timeout(INDEX_TIMEOUT)
            .send()
            .with_context(|| format!("failed to download marketplace index {index_url}"))?
            .error_for_status()
            .with_context(|| format!("marketplace index request failed: {index_url}"))?;
        ensure_content_length(&response, MAX_INDEX_BYTES, "marketplace index")?;
        let document = response
            .text()
            .context("failed to read marketplace index response")?;
        MarketplaceIndex::parse(&document)
    }

    /// Fetches the index and installs its newest listed release for a plugin.
    pub fn install(
        &self,
        store: &PluginStore,
        index_url: &str,
        plugin_id: &str,
    ) -> Result<InstalledPlugin> {
        let index = self.fetch_index(index_url)?;
        self.install_from_index(store, index_url, &index, plugin_id)
    }

    /// Installs a release from an already validated index.
    pub fn install_from_index(
        &self,
        store: &PluginStore,
        index_url: &str,
        index: &MarketplaceIndex,
        plugin_id: &str,
    ) -> Result<InstalledPlugin> {
        let listing = index.plugin(plugin_id)?;
        let release = listing
            .versions
            .first()
            .ok_or_else(|| anyhow!("plugin has no published versions: {plugin_id}"))?;
        if release.digest == "bundled" {
            bail!("marketplace release uses the development-only bundled digest");
        }

        let artifact_url = resolve_url(index_url, &release.artifact_url)?;
        secure_marketplace_url(artifact_url.as_str(), "plugin artifact")?;
        let response = self
            .client
            .get(artifact_url.clone())
            .timeout(ARTIFACT_TIMEOUT)
            .send()
            .with_context(|| format!("failed to download plugin artifact {artifact_url}"))?
            .error_for_status()
            .with_context(|| format!("plugin artifact request failed: {artifact_url}"))?;
        ensure_content_length(&response, MAX_ARTIFACT_BYTES, "plugin artifact")?;
        let artifact = response
            .bytes()
            .with_context(|| format!("failed to read plugin artifact {artifact_url}"))?;
        if artifact.len() as u64 > MAX_ARTIFACT_BYTES {
            bail!(
                "plugin artifact exceeds the {} byte limit",
                MAX_ARTIFACT_BYTES
            );
        }
        verify_sha256(&release.digest, &artifact)?;

        let plugin = store.install_from_archive_checked(
            &artifact,
            artifact_url.path(),
            &listing.id,
            &release.version,
            &release.publisher,
        )?;
        Ok(plugin)
    }
}

fn resolve_url(index_url: &str, reference: &str) -> Result<reqwest::Url> {
    let base = reqwest::Url::parse(index_url).context("invalid marketplace index URL")?;
    base.join(reference)
        .with_context(|| format!("invalid marketplace artifact reference: {reference}"))
}

fn secure_marketplace_url(value: &str, description: &str) -> Result<reqwest::Url> {
    let url = reqwest::Url::parse(value).with_context(|| format!("invalid {description} URL"))?;
    if url.scheme() == "https" {
        return Ok(url);
    }
    if url.scheme() == "http"
        && url
            .host_str()
            .is_some_and(|host| matches!(host, "localhost" | "127.0.0.1" | "[::1]"))
    {
        return Ok(url);
    }
    bail!("{description} must use HTTPS unless it targets localhost")
}

fn ensure_content_length(
    response: &reqwest::blocking::Response,
    limit: u64,
    description: &str,
) -> Result<()> {
    if response
        .content_length()
        .is_some_and(|length| length > limit)
    {
        bail!("{description} exceeds the {limit} byte limit");
    }
    Ok(())
}

fn verify_sha256(expected: &str, bytes: &[u8]) -> Result<()> {
    let Some((algorithm, expected_hex)) = expected.split_once(':') else {
        bail!("unsupported plugin artifact digest format: {expected}");
    };
    if algorithm != "sha256" || expected_hex.len() != 64 {
        bail!("plugin artifact digest must be sha256:<64 hex characters>");
    }
    let actual_hex = format!("{:x}", Sha256::digest(bytes));
    if !actual_hex.eq_ignore_ascii_case(expected_hex) {
        bail!("plugin artifact digest verification failed");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::path::Path;
    use std::thread;

    use flate2::{Compression, write::GzEncoder};
    use sha2::{Digest, Sha256};
    use tar::Builder;

    use super::*;
    use crate::plugin::PluginStore;

    #[test]
    fn downloads_verifies_and_installs_plugin_artifact() {
        let archive = parallel_archive();
        let digest = format!("sha256:{:x}", Sha256::digest(&archive));
        let index = serde_json::json!({
            "index_version": 1,
            "plugins": [{
                "id": "parallel-search",
                "versions": [{
                    "version": "1.0.0",
                    "components": ["mcp"],
                    "capabilities": ["web_search"],
                    "manifest_url": "plugins/parallel-search/1.0.0/plugin.json",
                    "artifact_url": "plugins/parallel-search/1.0.0/parallel-search-1.0.0.tar.gz",
                    "digest": digest,
                    "publisher": "parallel",
                    "status": "verified"
                }]
            }]
        })
        .to_string();

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().unwrap();
                let request = read_request(&mut stream);
                let body = if request.contains("GET /index.json") {
                    index.as_bytes()
                } else {
                    &archive
                };
                write_response(&mut stream, body);
            }
        });

        let root = std::env::temp_dir().join(format!(
            "windie-marketplace-test-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let store = PluginStore::new(&root);
        let index_url = format!("http://{address}/index.json");
        let installed = MarketplaceInstaller::default()
            .install(&store, &index_url, "parallel-search")
            .unwrap();

        assert_eq!(installed.manifest.plugin.id, "parallel-search");
        assert_eq!(installed.manifest.plugin.version, "1.0.0");
        assert!(installed.root.join("plugin.json").is_file());
        assert!(installed.root.join("mcp/server.json").is_file());

        server.join().unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_digest_mismatch() {
        assert!(verify_sha256(&format!("sha256:{}", "0".repeat(64)), b"artifact").is_err());
    }

    fn parallel_archive() -> Vec<u8> {
        let package_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("packages/parallel-search");
        let encoder = GzEncoder::new(Vec::new(), Compression::default());
        let mut builder = Builder::new(encoder);
        for relative in [
            "plugin.json",
            "mcp/server.json",
            "README.md",
            "assets/icon.svg",
        ] {
            builder
                .append_path_with_name(package_root.join(relative), relative)
                .unwrap();
        }
        builder.into_inner().unwrap().finish().unwrap()
    }

    fn read_request(stream: &mut TcpStream) -> String {
        let mut request = Vec::new();
        let mut buffer = [0; 1024];
        loop {
            let count = stream.read(&mut buffer).unwrap();
            if count == 0 {
                break;
            }
            request.extend_from_slice(&buffer[..count]);
            if request.windows(4).any(|window| window == b"\r\n\r\n") {
                break;
            }
        }
        String::from_utf8(request).unwrap()
    }

    fn write_response(stream: &mut TcpStream, body: &[u8]) {
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .unwrap();
        stream.write_all(body).unwrap();
    }
}

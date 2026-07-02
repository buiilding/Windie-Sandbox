use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use reqwest::Client;
use tokio::time::sleep;

const BIFROST_BINARY: &str = "../bifrost/tmp/bifrost-http";
const BIFROST_APP_DIR: &str = "../bifrost/data";
const BIFROST_PORT: &str = "8080";
const START_TIMEOUT: Duration = Duration::from_secs(10);
const HEALTH_CHECK_INTERVAL: Duration = Duration::from_millis(200);

pub struct BifrostGateway {
    http: Client,
    url: String,
    binary: PathBuf,
    app_dir: PathBuf,
}

impl BifrostGateway {
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            http: Client::new(),
            url: url.into().trim_end_matches('/').to_string(),
            binary: PathBuf::from(BIFROST_BINARY),
            app_dir: PathBuf::from(BIFROST_APP_DIR),
        }
    }

    pub async fn ensure_running(&self) -> Result<()> {
        if self.is_running().await {
            return Ok(());
        }

        self.start()?;
        self.wait_until_running().await
    }

    async fn is_running(&self) -> bool {
        self.health_check().await.is_ok()
    }

    async fn health_check(&self) -> Result<()> {
        let health_url = format!("{}/health", self.url);
        let response = self
            .http
            .get(&health_url)
            .send()
            .await
            .context("failed to reach Bifrost health endpoint")?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(anyhow!("Bifrost health check failed with {status}: {body}"));
        }

        Ok(())
    }

    fn start(&self) -> Result<()> {
        if !self.binary.exists() {
            return Err(anyhow!(
                "Bifrost binary was not found. Build Bifrost before running Windie."
            ));
        }

        Command::new(&self.binary)
            .arg("-app-dir")
            .arg(&self.app_dir)
            .arg("-port")
            .arg(BIFROST_PORT)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .context("failed to start Bifrost")?;

        Ok(())
    }

    async fn wait_until_running(&self) -> Result<()> {
        let mut waited = Duration::ZERO;

        while waited < START_TIMEOUT {
            if self.is_running().await {
                return Ok(());
            }

            sleep(HEALTH_CHECK_INTERVAL).await;
            waited += HEALTH_CHECK_INTERVAL;
        }

        Err(anyhow!(
            "Bifrost did not become healthy within {} seconds",
            START_TIMEOUT.as_secs()
        ))
    }
}

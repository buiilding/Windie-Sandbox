//! Local Inspector launch and packaged-asset discovery.
//!
//! The CLI asks the running loopback API for a one-time launch code, places it
//! in the Inspector URL fragment, and delegates opening to the operating
//! system. The long-lived local component credential never enters the URL or
//! browser. Release assets live beside the `windie` executable so the API can
//! serve the same static frontend without a Node.js process.

use std::env;
use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context, Result, anyhow, bail};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct LocalAccessLaunchResponse {
    code: String,
}

/// Returns the installed Inspector build directory when one is available.
///
/// `WINDIE_INSPECTOR_DIR` supports release verification and nonstandard
/// packaging. Normal installations discover the release-owned `inspector`
/// directory beside the running executable.
pub fn installed_assets_directory() -> Option<PathBuf> {
    let candidate = env::var_os("WINDIE_INSPECTOR_DIR")
        .map(PathBuf::from)
        .or_else(|| {
            env::current_exe()
                .ok()?
                .parent()
                .map(|parent| parent.join("inspector"))
        })?;

    candidate.join("index.html").is_file().then_some(candidate)
}

/// Opens the packaged Inspector served by the running local API.
pub async fn open() -> Result<()> {
    if installed_assets_directory().is_none() {
        bail!(
            "the packaged local Inspector is not installed beside Windie; use `windie dev run inspector` from a source checkout"
        );
    }
    open_at(&crate::config::api_url()).await
}

/// Opens one local Inspector origin after asking the API for a one-time code.
/// Development uses the React server origin; installed releases use the API's
/// own static asset origin.
pub async fn open_at(inspector_origin: &str) -> Result<()> {
    let code = request_launch_code().await?;
    let url = format!(
        "{}/#windie-local-code={code}",
        inspector_origin.trim_end_matches('/')
    );
    open_browser(&url)?;
    println!("windie: opened local Inspector at {inspector_origin}");
    Ok(())
}

async fn request_launch_code() -> Result<String> {
    let token = crate::local::api_component_token()?;
    let endpoint = format!(
        "{}/api/runtime/local-access/launch",
        crate::config::api_url().trim_end_matches('/')
    );
    let response = reqwest::Client::new()
        .post(&endpoint)
        .header(crate::config::LOCAL_COMPONENT_TOKEN_HEADER, token)
        .send()
        .await
        .with_context(
            || "could not reach the local Windie API; start it with `windie api start`",
        )?;
    if !response.status().is_success() {
        let status = response.status();
        let detail = response.text().await.unwrap_or_default();
        bail!("local Inspector launch failed with {status}: {detail}");
    }
    let response = response
        .json::<LocalAccessLaunchResponse>()
        .await
        .context("local Windie API returned an invalid Inspector launch response")?;
    if response.code.trim().is_empty() {
        return Err(anyhow!(
            "local Windie API returned an empty Inspector launch code"
        ));
    }
    Ok(response.code)
}

fn open_browser(url: &str) -> Result<()> {
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = Command::new("open");
        command.arg(url);
        command
    };

    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = Command::new("cmd");
        command.args(["/C", "start", "", url]);
        command
    };

    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = {
        let mut command = Command::new("xdg-open");
        command.arg(url);
        command
    };

    #[cfg(not(any(target_os = "macos", target_os = "windows", unix)))]
    return Err(anyhow!(
        "opening the Inspector is unsupported on this platform"
    ));

    let status = command
        .status()
        .context("failed to ask the operating system to open the Inspector")?;
    if !status.success() {
        bail!("the operating system could not open the Inspector");
    }
    Ok(())
}

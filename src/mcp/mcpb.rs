//! MCPB package validation and extraction.
//!
//! This module handles the local package boundary for MCP components. It only
//! extracts a verified MCPB and turns its declarative `mcp_config` into an
//! owned process command. Windie still owns process lifecycle and removal.

use std::fs;
use std::io::Cursor;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use zip::ZipArchive;

use crate::mcp::McpOwnedCommand;

use crate::plugin::manifest::{
    McpAuthentication, McpDelivery, WindieMcpSetup, WindieSetupEnvironmentValue,
    resolve_package_path, validate_relative_path,
};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
/// The subset of MCPB `manifest.json` required to launch a local server.
pub(crate) struct McpbManifest {
    pub manifest_version: String,
    pub name: String,
    pub version: String,
    pub server: McpbServer,
    #[serde(default)]
    pub compatibility: Option<McpbCompatibility>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
/// Platform declarations from an MCPB manifest.
pub(crate) struct McpbCompatibility {
    #[serde(default)]
    pub platforms: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
/// MCPB server runtime and launch configuration.
pub(crate) struct McpbServer {
    #[serde(rename = "type")]
    pub kind: String,
    pub entry_point: String,
    pub mcp_config: McpbConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
/// Declarative command configuration from an MCPB manifest.
pub(crate) struct McpbConfig {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Clone)]
/// An extracted MCPB runtime owned by one installed plugin version.
pub(crate) struct McpbRuntime {
    root: PathBuf,
    manifest: McpbManifest,
}

impl McpbRuntime {
    /// Verifies and extracts an MCPB into a Windie-owned runtime directory.
    pub(crate) fn install(
        artifact: &Path,
        destination: &Path,
        expected_sha256: &str,
    ) -> Result<Self> {
        let bytes = fs::read(artifact)
            .with_context(|| format!("failed to read MCPB artifact {}", artifact.display()))?;
        verify_sha256(expected_sha256, &bytes)?;

        if !destination.is_dir() {
            let staging = destination.with_extension(format!("staging-{}", uuid::Uuid::new_v4()));
            fs::create_dir_all(&staging)?;
            let result = (|| {
                extract_zip(&bytes, &staging)?;
                let manifest = load_manifest(&staging)?;
                validate_manifest(&manifest, &staging)?;
                fs::rename(&staging, destination).with_context(|| {
                    format!("failed to publish MCPB runtime {}", destination.display())
                })?;
                Ok::<_, anyhow::Error>(manifest)
            })();
            if result.is_err() {
                let _ = fs::remove_dir_all(&staging);
            }
            let manifest = result?;
            return Ok(Self {
                root: destination.to_path_buf(),
                manifest,
            });
        }

        let manifest = load_manifest(destination)?;
        validate_manifest(&manifest, destination)?;
        Ok(Self {
            root: destination.to_path_buf(),
            manifest,
        })
    }

    /// Converts the MCPB launch declaration into a shell-free owned command.
    pub(crate) fn command(&self) -> Result<McpOwnedCommand> {
        let root = self.root.to_string_lossy();
        let entry_point = resolve_template(&self.manifest.server.entry_point, &root)?;
        let entry_path = Path::new(&entry_point);
        let entry_point = if entry_path.is_absolute() {
            entry_point
        } else {
            self.root.join(entry_path).to_string_lossy().into_owned()
        };
        let args = self
            .manifest
            .server
            .mcp_config
            .args
            .iter()
            .map(|value| resolve_template(value, &root))
            .collect::<Result<Vec<_>>>()?;
        let env = self
            .manifest
            .server
            .mcp_config
            .env
            .iter()
            .map(|(key, value)| Ok((key.clone(), resolve_template(value, &root)?)))
            .collect::<Result<Vec<_>>>()?;

        let mut args = args;
        let configured_entry = Path::new(&self.manifest.server.entry_point);
        let has_entry_point = args
            .iter()
            .any(|argument| Path::new(argument).ends_with(configured_entry));
        if !has_entry_point {
            let command = self.manifest.server.mcp_config.command.as_str();
            if matches!(command, "node" | "python" | "python3") {
                args.insert(0, entry_point);
            }
        }

        Ok(McpOwnedCommand {
            program: resolve_template(&self.manifest.server.mcp_config.command, &root)?,
            args,
            env,
            secret_env: Vec::new(),
        })
    }

    /// Applies Windie's declarative setup and returns the launch command with
    /// the component's isolated environment attached.
    pub(crate) fn prepare(
        &self,
        setup: &WindieMcpSetup,
        authentication: &McpAuthentication,
    ) -> Result<McpOwnedCommand> {
        let home = self.root.join("home");
        let windie_data_dir = crate::local::windie_home_dir()?;
        if setup.isolated_home {
            fs::create_dir_all(&home).with_context(|| {
                format!("failed to create isolated MCP HOME {}", home.display())
            })?;
        }

        for file in &setup.files {
            let path = resolve_package_path(&home, &file.path)?;
            let parent = path
                .parent()
                .ok_or_else(|| anyhow!("MCP setup file has no parent: {}", file.path))?;
            fs::create_dir_all(parent)?;
            let content = resolve_json_templates(&file.content, &self.root, &windie_data_dir)?;
            let contents = serde_json::to_vec_pretty(&content)
                .context("failed to serialize MCP setup file")?;
            fs::write(&path, contents)
                .with_context(|| format!("failed to write MCP setup file {}", path.display()))?;
        }

        let mut command = self.command()?;
        self.apply_setup_environment(&mut command, setup, &windie_data_dir)?;
        self.apply_authentication(&mut command, authentication)?;
        if setup.isolated_home {
            let home = home.to_string_lossy().into_owned();
            command.env.push(("HOME".to_string(), home.clone()));
            #[cfg(windows)]
            command.env.push(("USERPROFILE".to_string(), home));
        }
        Ok(command)
    }

    /// Builds the package-manager command used to prepare a package before
    /// the MCP protocol starts. uv packages install their declared Python
    /// dependencies during this phase rather than during MCP initialization.
    pub(crate) fn package_command(
        &self,
        setup: &WindieMcpSetup,
    ) -> Result<Option<McpOwnedCommand>> {
        if self.manifest.server.kind != "uv" {
            return Ok(None);
        }

        let windie_data_dir = crate::local::windie_home_dir()?;
        let mut command = McpOwnedCommand {
            program: self.manifest.server.mcp_config.command.clone(),
            args: vec![
                "sync".to_string(),
                "--project".to_string(),
                self.root.to_string_lossy().into_owned(),
                "--no-dev".to_string(),
            ],
            env: Vec::new(),
            secret_env: Vec::new(),
        };
        self.apply_setup_environment(&mut command, setup, &windie_data_dir)?;
        Ok(Some(command))
    }

    fn apply_setup_environment(
        &self,
        command: &mut McpOwnedCommand,
        setup: &WindieMcpSetup,
        windie_data_dir: &Path,
    ) -> Result<()> {
        for environment in &setup.environment {
            let value = match &environment.value {
                WindieSetupEnvironmentValue::Literal { value } => value.clone(),
                WindieSetupEnvironmentValue::ComponentPath { path } => {
                    resolve_package_path(&self.root, path)?
                        .to_string_lossy()
                        .into_owned()
                }
                WindieSetupEnvironmentValue::WindieDataDir { path } => {
                    resolve_package_path(windie_data_dir, path)?
                        .to_string_lossy()
                        .into_owned()
                }
            };
            command.env.push((environment.name.clone(), value));
        }
        Ok(())
    }

    /// Delivers a package-declared API key without persisting the secret in
    /// the installed package or resolving it during package installation.
    fn apply_authentication(
        &self,
        command: &mut McpOwnedCommand,
        authentication: &McpAuthentication,
    ) -> Result<()> {
        if let McpAuthentication::ApiKey {
            required,
            secret_id,
            delivery: McpDelivery::Environment { name },
            ..
        } = authentication
        {
            command
                .secret_env
                .push((name.clone(), secret_id.clone(), *required));
        }
        Ok(())
    }
}

fn resolve_json_templates(
    value: &serde_json::Value,
    component_root: &Path,
    windie_data_dir: &Path,
) -> Result<serde_json::Value> {
    match value {
        serde_json::Value::String(value) => Ok(serde_json::Value::String(
            value
                .replace("${windie_data_dir}", &windie_data_dir.to_string_lossy())
                .replace("${__dirname}", &component_root.to_string_lossy()),
        )),
        serde_json::Value::Array(values) => Ok(serde_json::Value::Array(
            values
                .iter()
                .map(|value| resolve_json_templates(value, component_root, windie_data_dir))
                .collect::<Result<Vec<_>>>()?,
        )),
        serde_json::Value::Object(values) => Ok(serde_json::Value::Object(
            values
                .iter()
                .map(|(key, value)| {
                    Ok((
                        key.clone(),
                        resolve_json_templates(value, component_root, windie_data_dir)?,
                    ))
                })
                .collect::<Result<serde_json::Map<_, _>>>()?,
        )),
        other => Ok(other.clone()),
    }
}

fn load_manifest(root: &Path) -> Result<McpbManifest> {
    let path = root.join("manifest.json");
    let document = fs::read_to_string(&path)
        .with_context(|| format!("failed to read MCPB manifest {}", path.display()))?;
    serde_json::from_str(&document).context("failed to parse MCPB manifest.json")
}

fn validate_manifest(manifest: &McpbManifest, root: &Path) -> Result<()> {
    if manifest.manifest_version != "0.3" && manifest.manifest_version != "0.4" {
        bail!(
            "unsupported MCPB manifest version {}",
            manifest.manifest_version
        );
    }
    if manifest.name.trim().is_empty() || manifest.version.trim().is_empty() {
        bail!("MCPB manifest name and version are required");
    }
    if let Some(compatibility) = &manifest.compatibility
        && !compatibility.platforms.is_empty()
        && !compatibility
            .platforms
            .iter()
            .any(|platform| platform == current_platform())
    {
        bail!(
            "MCPB package {} does not support the current platform ({})",
            manifest.name,
            current_platform()
        );
    }
    if !matches!(
        manifest.server.kind.as_str(),
        "node" | "python" | "binary" | "uv"
    ) {
        bail!("unsupported MCPB server type: {}", manifest.server.kind);
    }
    validate_relative_path("MCPB entry point", &manifest.server.entry_point)?;
    if !resolve_package_path(root, &manifest.server.entry_point)?.is_file() {
        bail!(
            "MCPB entry point does not exist: {}",
            manifest.server.entry_point
        );
    }
    if manifest.server.mcp_config.command.trim().is_empty() {
        bail!("MCPB server command cannot be empty");
    }
    for key in manifest.server.mcp_config.env.keys() {
        if key.is_empty() || key.contains('=') || key.contains('\0') {
            bail!("invalid MCPB environment variable name: {key}");
        }
    }
    Ok(())
}

fn current_platform() -> &'static str {
    if cfg!(target_os = "macos") {
        "darwin"
    } else if cfg!(target_os = "windows") {
        "win32"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else {
        "unknown"
    }
}

fn resolve_template(value: &str, root: &str) -> Result<String> {
    let value = value.replace("${__dirname}", root);
    if value.contains("${") {
        bail!("unsupported MCPB variable in value: {value}");
    }
    Ok(value)
}

fn verify_sha256(expected: &str, bytes: &[u8]) -> Result<()> {
    // MCP Registry `fileSha256` is a bare hexadecimal digest. Accept the
    // prefixed form too because Windie's marketplace index uses that form and
    // older development fixtures may still contain it.
    let expected_hex = expected.strip_prefix("sha256:").unwrap_or(expected);
    if expected_hex.len() != 64 || !expected_hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("MCPB artifact digest must be a SHA-256 digest");
    }
    let actual = format!("{:x}", Sha256::digest(bytes));
    if !actual.eq_ignore_ascii_case(expected_hex) {
        bail!("MCPB artifact digest verification failed");
    }
    Ok(())
}

fn extract_zip(bytes: &[u8], destination: &Path) -> Result<()> {
    let mut archive = ZipArchive::new(Cursor::new(bytes)).context("failed to read MCPB archive")?;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .context("failed to read MCPB entry")?;
        let name = entry.name().replace('\\', "/");
        let relative = Path::new(&name);
        if relative.is_absolute()
            || relative
                .components()
                .any(|component| matches!(component, Component::ParentDir))
        {
            bail!("MCPB archive contains an unsafe path: {name}");
        }
        if entry
            .unix_mode()
            .is_some_and(|mode| mode & 0o170000 == 0o120000)
        {
            bail!("MCPB archive contains a symbolic link: {name}");
        }
        let path = destination.join(relative);
        if entry.is_dir() {
            fs::create_dir_all(path)?;
            continue;
        }
        let parent = path
            .parent()
            .ok_or_else(|| anyhow!("MCPB file has no parent: {name}"))?;
        fs::create_dir_all(parent)?;
        let unix_mode = entry.unix_mode();
        let mut output = fs::File::create(&path)?;
        std::io::copy(&mut entry, &mut output)?;
        apply_unix_permissions(&path, unix_mode)?;
    }
    Ok(())
}

/// Restores executable bits recorded by MCPB archives on Unix-like systems.
///
/// Native MCPB servers may be shipped as executable files rather than scripts.
/// The archive is the source of truth for those mode bits; without restoring
/// them, a valid package would install successfully but fail with `Permission
/// denied` when Windie starts it. Other platforms keep their filesystem's
/// normal permission behavior.
fn apply_unix_permissions(path: &Path, mode: Option<u32>) -> Result<()> {
    #[cfg(unix)]
    if let Some(mode) = mode {
        use std::os::unix::fs::PermissionsExt;

        fs::set_permissions(path, fs::Permissions::from_mode(mode & 0o7777))?;
    }

    let _ = (path, mode);
    Ok(())
}

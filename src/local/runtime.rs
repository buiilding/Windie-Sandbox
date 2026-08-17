//! User-local runtime provisioning and executable resolution.
//!
//! Approved providers must not depend on a user's shell, global Node.js/uv
//! installation, or Windows command shims. This module provisions the shared
//! runtimes Windie needs with Rust's HTTP and archive libraries, then resolves
//! executable paths immediately before a child process starts.

use std::env;
use std::ffi::OsString;
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use reqwest::blocking::Client;
use sha2::{Digest, Sha256};

use super::windie_home_dir;
use crate::tool_provider::ProviderRuntime;

const NODE_VERSION: &str = "22.14.0";
const UV_VERSION: &str = "latest";

/// The archive format used by one official runtime release asset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
enum ArchiveFormat {
    Zip,
    TarGz,
    TarXz,
}

/// Ensures one declared runtime family is available.
pub(crate) fn ensure_runtime(runtime: ProviderRuntime) -> Result<bool> {
    match runtime {
        ProviderRuntime::Native => Ok(false),
        ProviderRuntime::Node => ensure_node_runtime(),
        ProviderRuntime::Uv => ensure_uv_runtime(),
    }
}

/// Removes a Windie-managed runtime family after its last provider is gone.
///
/// Runtime directories are never removed while another installed provider
/// still depends on the same runtime. The caller performs that ownership
/// check; this function only validates and removes the exact managed path.
pub(crate) fn remove_managed_runtime(runtime: ProviderRuntime) -> Result<()> {
    let name = match runtime {
        ProviderRuntime::Native => return Ok(()),
        ProviderRuntime::Node => "node",
        ProviderRuntime::Uv => "uv",
    };
    let root = windie_home_dir()?.join("runtimes").join(name);
    let metadata = match fs::symlink_metadata(&root) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("failed to inspect managed runtime: {}", root.display()));
        }
    };

    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(anyhow!(
            "refusing to remove managed runtime that is not a directory: {}",
            root.display()
        ));
    }

    fs::remove_dir_all(&root)
        .with_context(|| format!("failed to remove managed runtime: {}", root.display()))
}

/// Resolves an approved provider command without relying on shell lookup.
pub(crate) fn resolve_command(program: &str) -> Result<PathBuf> {
    if managed_runtime_program(program) {
        return local_runtime_command(program)?.ok_or_else(|| {
            anyhow!(
                "Windie-managed runtime command is not installed: {program}; set up the provider first"
            )
        });
    }

    path_command(program).ok_or_else(|| anyhow!("required command is not available: {program}"))
}

/// Prepends an executable's directory to the inherited PATH.
pub(crate) fn path_with_command_parent(executable: &Path) -> Option<OsString> {
    let parent = executable.parent()?;
    let mut paths = vec![parent.to_path_buf()];
    if let Some(existing) = env::var_os("PATH") {
        paths.extend(env::split_paths(&existing));
    }
    env::join_paths(paths).ok()
}

fn ensure_node_runtime() -> Result<bool> {
    let version = env::var("WINDIE_NODE_VERSION").unwrap_or_else(|_| NODE_VERSION.to_string());
    let runtime_dir = runtime_directory(ProviderRuntime::Node, &version)?;
    if runtime_contains(&runtime_dir, &["node", "npx"]) {
        return Ok(false);
    }

    let (url, checksum_url, format) = node_asset(&version)?;
    install_runtime_archive(
        &url,
        &checksum_url,
        format,
        &runtime_dir,
        "Node.js",
        &["node", "npx"],
    )?;
    Ok(true)
}

fn ensure_uv_runtime() -> Result<bool> {
    let version = env::var("WINDIE_UV_VERSION").unwrap_or_else(|_| UV_VERSION.to_string());
    let runtime_dir = runtime_directory(ProviderRuntime::Uv, &version)?;
    if runtime_contains(&runtime_dir, &["uv", "uvx"]) {
        return Ok(false);
    }

    let (url, checksum_url, format) = uv_asset(&version)?;
    install_runtime_archive(
        &url,
        &checksum_url,
        format,
        &runtime_dir,
        "uv",
        &["uv", "uvx"],
    )?;
    Ok(true)
}

fn node_asset(version: &str) -> Result<(String, String, ArchiveFormat)> {
    let architecture = match env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        other => return Err(anyhow!("Node.js runtime is not supported on {other}")),
    };

    #[cfg(target_os = "windows")]
    {
        let base = format!("https://nodejs.org/dist/v{version}");
        return Ok((
            format!("{base}/node-v{version}-win-{architecture}.zip"),
            format!("{base}/SHASUMS256.txt"),
            ArchiveFormat::Zip,
        ));
    }

    #[cfg(target_os = "macos")]
    {
        let base = format!("https://nodejs.org/dist/v{version}");
        return Ok((
            format!("{base}/node-v{version}-darwin-{architecture}.tar.gz"),
            format!("{base}/SHASUMS256.txt"),
            ArchiveFormat::TarGz,
        ));
    }

    #[cfg(target_os = "linux")]
    {
        let base = format!("https://nodejs.org/dist/v{version}");
        return Ok((
            format!("{base}/node-v{version}-linux-{architecture}.tar.xz"),
            format!("{base}/SHASUMS256.txt"),
            ArchiveFormat::TarXz,
        ));
    }

    #[allow(unreachable_code)]
    Err(anyhow!(
        "Node.js runtime is not supported on this operating system"
    ))
}

fn uv_asset(version: &str) -> Result<(String, String, ArchiveFormat)> {
    let architecture = match env::consts::ARCH {
        "x86_64" => "x86_64",
        "aarch64" => "aarch64",
        other => return Err(anyhow!("uv runtime is not supported on {other}")),
    };
    let target = if cfg!(target_os = "windows") {
        format!("{architecture}-pc-windows-msvc")
    } else if cfg!(target_os = "macos") {
        format!("{architecture}-apple-darwin")
    } else if cfg!(target_os = "linux") {
        format!("{architecture}-unknown-linux-gnu")
    } else {
        return Err(anyhow!(
            "uv runtime is not supported on this operating system"
        ));
    };

    let base = if version == "latest" {
        "https://github.com/astral-sh/uv/releases/latest/download".to_string()
    } else {
        format!("https://github.com/astral-sh/uv/releases/download/{version}")
    };
    let format = if cfg!(target_os = "windows") {
        ArchiveFormat::Zip
    } else {
        ArchiveFormat::TarGz
    };

    let archive_url = format!("{base}/uv-{target}.{}", archive_suffix(format));
    Ok((archive_url.clone(), format!("{archive_url}.sha256"), format))
}

fn install_runtime_archive(
    url: &str,
    checksum_url: &str,
    format: ArchiveFormat,
    runtime_dir: &Path,
    runtime_name: &str,
    expected: &[&str],
) -> Result<()> {
    let parent = runtime_dir
        .parent()
        .ok_or_else(|| anyhow!("runtime path has no parent: {}", runtime_dir.display()))?;
    fs::create_dir_all(parent)
        .with_context(|| format!("failed to create runtime directory: {}", parent.display()))?;

    let temporary_dir = parent.join(format!(".install-{}", std::process::id()));
    if temporary_dir.exists() {
        fs::remove_dir_all(&temporary_dir).with_context(|| {
            format!(
                "failed to remove interrupted runtime install: {}",
                temporary_dir.display()
            )
        })?;
    }
    fs::create_dir_all(&temporary_dir)?;

    let archive_path = temporary_dir.join(format!("runtime.{}", archive_suffix(format)));
    let extracted_dir = temporary_dir.join("extracted");
    fs::create_dir_all(&extracted_dir)?;

    let bytes = download_archive(url, runtime_name)?;
    let checksum_text = download_checksum(checksum_url, runtime_name)?;
    verify_archive_checksum(url, &bytes, &checksum_text)
        .with_context(|| format!("failed to verify downloaded {runtime_name} archive"))?;
    let fingerprint = archive_fingerprint(&bytes);
    fs::write(&archive_path, &bytes)
        .with_context(|| format!("failed to save downloaded {runtime_name} archive"))?;
    extract_archive(&bytes, format, &extracted_dir)
        .with_context(|| format!("failed to extract {runtime_name} archive ({fingerprint})"))?;

    for executable in expected {
        let file_name = runtime_file_name(executable);
        if find_file(&extracted_dir, file_name)?.is_none() {
            return Err(anyhow!(
                "downloaded {runtime_name} archive did not contain {file_name}"
            ));
        }
    }

    if runtime_dir.exists() {
        fs::remove_dir_all(runtime_dir).with_context(|| {
            format!(
                "failed to replace runtime directory: {}",
                runtime_dir.display()
            )
        })?;
    }
    fs::rename(&extracted_dir, runtime_dir)
        .with_context(|| format!("failed to install runtime into {}", runtime_dir.display()))?;
    let _ = fs::remove_dir_all(&temporary_dir);

    Ok(())
}

fn download_checksum(url: &str, runtime_name: &str) -> Result<String> {
    let client = Client::builder()
        .user_agent(format!("windie/{runtime_name}"))
        .build()
        .context("failed to build runtime checksum client")?;
    client
        .get(url)
        .send()
        .with_context(|| format!("failed to download {runtime_name} checksum from {url}"))?
        .error_for_status()
        .with_context(|| format!("runtime checksum download failed for {runtime_name}: {url}"))?
        .text()
        .context("failed to read runtime checksum")
}

fn verify_archive_checksum(url: &str, bytes: &[u8], checksum_text: &str) -> Result<()> {
    let archive_name = url
        .rsplit('/')
        .next()
        .filter(|name| !name.is_empty())
        .ok_or_else(|| anyhow!("runtime archive URL has no filename: {url}"))?;
    let expected = checksum_text.lines().find_map(|line| {
        let mut fields = line.split_whitespace();
        let checksum = fields.next()?;
        let name = fields.next().map(|name| name.trim_start_matches('*'));
        if name.is_none() || name == Some(archive_name) {
            Some(checksum)
        } else {
            None
        }
    });
    let expected = expected
        .ok_or_else(|| anyhow!("runtime checksum did not contain an entry for {archive_name}"))?;
    if expected.len() != 64 || !expected.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(anyhow!("runtime checksum is not a SHA-256 digest"));
    }

    let actual = archive_fingerprint(bytes);
    if !actual.eq_ignore_ascii_case(expected) {
        return Err(anyhow!(
            "runtime checksum mismatch for {archive_name}: expected {expected}, got {actual}"
        ));
    }
    Ok(())
}

fn download_archive(url: &str, runtime_name: &str) -> Result<Vec<u8>> {
    let client = Client::builder()
        .user_agent(format!("windie/{runtime_name}"))
        .build()
        .context("failed to build runtime download client")?;
    let response = client
        .get(url)
        .send()
        .with_context(|| format!("failed to download {runtime_name} runtime from {url}"))?
        .error_for_status()
        .with_context(|| format!("runtime download failed for {runtime_name}: {url}"))?;
    let bytes = response
        .bytes()
        .context("failed to read runtime download")?
        .to_vec();
    if bytes.is_empty() {
        return Err(anyhow!("runtime download returned an empty archive: {url}"));
    }
    Ok(bytes)
}

fn extract_archive(bytes: &[u8], format: ArchiveFormat, destination: &Path) -> Result<()> {
    match format {
        ArchiveFormat::Zip => {
            let mut archive =
                zip::ZipArchive::new(Cursor::new(bytes)).context("invalid zip archive")?;
            for index in 0..archive.len() {
                let mut entry = archive
                    .by_index(index)
                    .context("failed to read zip entry")?;
                let Some(relative) = entry.enclosed_name() else {
                    return Err(anyhow!("runtime archive contains an unsafe path"));
                };
                let output = destination.join(relative);
                if entry.is_dir() {
                    fs::create_dir_all(&output)?;
                } else {
                    if let Some(parent) = output.parent() {
                        fs::create_dir_all(parent)?;
                    }
                    let mut file = fs::File::create(&output)?;
                    std::io::copy(&mut entry, &mut file)?;
                }
            }
        }
        ArchiveFormat::TarGz => {
            let decoder = flate2::read::GzDecoder::new(Cursor::new(bytes));
            tar::Archive::new(decoder).unpack(destination)?;
        }
        ArchiveFormat::TarXz => {
            let decoder = xz2::read::XzDecoder::new(Cursor::new(bytes));
            tar::Archive::new(decoder).unpack(destination)?;
        }
    }
    Ok(())
}

fn archive_suffix(format: ArchiveFormat) -> &'static str {
    match format {
        ArchiveFormat::Zip => "zip",
        ArchiveFormat::TarGz => "tar.gz",
        ArchiveFormat::TarXz => "tar.xz",
    }
}

fn runtime_file_name(executable: &str) -> &str {
    #[cfg(target_os = "windows")]
    return match executable {
        "npx" => "npx.cmd",
        "node" => "node.exe",
        "uv" => "uv.exe",
        "uvx" => "uvx.exe",
        other => other,
    };

    #[cfg(not(target_os = "windows"))]
    executable
}

fn runtime_contains(root: &Path, executables: &[&str]) -> bool {
    find_runtime_directory(root, executables)
        .ok()
        .flatten()
        .is_some()
}

/// Returns the versioned directory owned by Windie for one runtime family.
fn runtime_directory(runtime: ProviderRuntime, version: &str) -> Result<PathBuf> {
    runtime_directory_under(&windie_home_dir()?, runtime, version)
}

/// Builds a versioned runtime path beneath a supplied Windie data directory.
fn runtime_directory_under(
    home: &Path,
    runtime: ProviderRuntime,
    version: &str,
) -> Result<PathBuf> {
    let name = match runtime {
        ProviderRuntime::Node => "node",
        ProviderRuntime::Uv => "uv",
        ProviderRuntime::Native => {
            return Err(anyhow!("native providers do not have a managed runtime"));
        }
    };

    Ok(home.join("runtimes").join(name).join(version))
}

/// Returns whether a command must be resolved from Windie's managed runtime.
fn managed_runtime_program(program: &str) -> bool {
    matches!(program, "node" | "npx" | "uv" | "uvx")
}

/// Returns the configured Windie runtime version for one runtime family.
fn configured_runtime_version(runtime: ProviderRuntime) -> Result<String> {
    match runtime {
        ProviderRuntime::Node => {
            Ok(env::var("WINDIE_NODE_VERSION").unwrap_or_else(|_| NODE_VERSION.to_string()))
        }
        ProviderRuntime::Uv => {
            Ok(env::var("WINDIE_UV_VERSION").unwrap_or_else(|_| UV_VERSION.to_string()))
        }
        ProviderRuntime::Native => Err(anyhow!("native providers do not have a managed runtime")),
    }
}

#[cfg(target_os = "windows")]
fn local_runtime_command(program: &str) -> Result<Option<PathBuf>> {
    let (runtime, anchor): (ProviderRuntime, &str) = match program {
        "npx" | "node" => (ProviderRuntime::Node, "node"),
        "uvx" | "uv" => (ProviderRuntime::Uv, "uv"),
        _ => return Ok(None),
    };
    let version = configured_runtime_version(runtime)?;
    let root = runtime_directory(runtime, &version)?;
    Ok(find_runtime_command(&root, anchor, program)?)
}

#[cfg(not(target_os = "windows"))]
fn local_runtime_command(program: &str) -> Result<Option<PathBuf>> {
    let (runtime, anchor): (ProviderRuntime, &str) = match program {
        "npx" | "node" => (ProviderRuntime::Node, "node"),
        "uvx" | "uv" => (ProviderRuntime::Uv, "uv"),
        _ => return Ok(None),
    };
    let version = configured_runtime_version(runtime)?;
    let root = runtime_directory(runtime, &version)?;
    find_runtime_command(&root, anchor, program)
}

/// Finds one runtime command beside its runtime's anchor executable.
///
/// Runtime archives may contain nested command shims, especially Node's
/// Corepack files. A command is valid only when it is in the same directory
/// as the anchor executable (`node` or `uv`), which identifies the archive's
/// actual runtime bin directory.
fn find_runtime_command(root: &Path, anchor: &str, command: &str) -> Result<Option<PathBuf>> {
    let Some(directory) = find_runtime_directory(root, &[anchor, command])? else {
        return Ok(None);
    };

    Ok(Some(directory.join(runtime_file_name(command))))
}

/// Finds a runtime directory containing all required executable siblings.
fn find_runtime_directory(root: &Path, executables: &[&str]) -> Result<Option<PathBuf>> {
    if !root.is_dir() {
        return Ok(None);
    }

    if executables
        .iter()
        .all(|executable| root.join(runtime_file_name(executable)).is_file())
    {
        return Ok(Some(root.to_path_buf()));
    }

    for entry in fs::read_dir(root).with_context(|| format!("failed to read {}", root.display()))? {
        let path = entry?.path();
        if path.is_dir()
            && let Some(found) = find_runtime_directory(&path, executables)?
        {
            return Ok(Some(found));
        }
    }

    Ok(None)
}

fn find_file(root: &Path, file_name: &str) -> Result<Option<PathBuf>> {
    if !root.exists() {
        return Ok(None);
    }
    for entry in fs::read_dir(root).with_context(|| format!("failed to read {}", root.display()))? {
        let path = entry?.path();
        if path.is_file() && path.file_name().is_some_and(|name| name == file_name) {
            return Ok(Some(path));
        }
        if path.is_dir()
            && let Some(found) = find_file(&path, file_name)?
        {
            return Ok(Some(found));
        }
    }
    Ok(None)
}

fn path_command(program: &str) -> Option<PathBuf> {
    let candidate = Path::new(program);
    if candidate.is_absolute() && candidate.is_file() {
        return Some(candidate.to_path_buf());
    }

    let paths = env::var_os("PATH")?;
    for directory in env::split_paths(&paths) {
        let direct = directory.join(program);
        if direct.is_file() {
            #[cfg(not(target_os = "windows"))]
            return Some(direct);

            #[cfg(target_os = "windows")]
            if direct.extension().is_some_and(|extension| {
                extension.eq_ignore_ascii_case("exe")
                    || extension.eq_ignore_ascii_case("cmd")
                    || extension.eq_ignore_ascii_case("bat")
            }) {
                return Some(direct);
            }
        }
        #[cfg(target_os = "windows")]
        for suffix in [".exe", ".cmd", ".bat"] {
            let with_suffix = directory.join(format!("{program}{suffix}"));
            if with_suffix.is_file() {
                return Some(with_suffix);
            }
        }
    }
    None
}

/// Returns a stable fingerprint for a downloaded archive for diagnostics.
pub(crate) fn archive_fingerprint(bytes: &[u8]) -> String {
    let mut digest = Sha256::new();
    digest.update(bytes);
    format!("{:x}", digest.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::local::ENVIRONMENT_LOCK;

    #[test]
    fn archive_suffix_matches_format() {
        assert_eq!(archive_suffix(ArchiveFormat::Zip), "zip");
        assert_eq!(archive_suffix(ArchiveFormat::TarGz), "tar.gz");
        assert_eq!(archive_suffix(ArchiveFormat::TarXz), "tar.xz");
    }

    #[test]
    fn archive_fingerprint_is_stable() {
        assert_eq!(
            archive_fingerprint(b"windie"),
            "2d6945726283047b18baa8e618ba50c7c532079da77b2439d54e30716fb5bdd3"
        );
    }

    #[test]
    fn runtime_directories_are_version_scoped() {
        let home = Path::new("/tmp/windie-runtime-test");

        assert_eq!(
            runtime_directory_under(home, ProviderRuntime::Node, "22.14.0").unwrap(),
            home.join("runtimes/node/22.14.0")
        );
    }

    #[test]
    fn removes_one_managed_runtime_family() {
        let _lock = ENVIRONMENT_LOCK.lock().unwrap();
        let root = env::temp_dir().join(format!(
            "windie-runtime-cleanup-test-{}",
            std::process::id()
        ));
        let runtime = root.join("runtimes/node");
        fs::create_dir_all(&runtime).unwrap();
        fs::write(runtime.join("node"), b"owned").unwrap();

        let previous_home = env::var_os("WINDIE_HOME");
        unsafe {
            env::set_var("WINDIE_HOME", &root);
        }
        let result = remove_managed_runtime(ProviderRuntime::Node);
        unsafe {
            match previous_home {
                Some(value) => env::set_var("WINDIE_HOME", value),
                None => env::remove_var("WINDIE_HOME"),
            }
        }

        result.unwrap();
        assert!(!root.join("runtimes/node").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn runtime_resolution_ignores_nested_corepack_shims() {
        let root = env::temp_dir().join(format!("windie-runtime-siblings-{}", std::process::id()));
        let runtime_bin = root.join("node-v22.14.0-win-x64");
        let corepack_shims = runtime_bin.join("node_modules/corepack/shims/nodewin");
        fs::create_dir_all(&corepack_shims).unwrap();
        fs::write(runtime_bin.join(runtime_file_name("node")), b"node").unwrap();
        fs::write(runtime_bin.join(runtime_file_name("npx")), b"real npx").unwrap();
        fs::write(
            corepack_shims.join(runtime_file_name("npx")),
            b"corepack npx",
        )
        .unwrap();

        let directory = find_runtime_directory(&root, &["node", "npx"])
            .unwrap()
            .unwrap();
        let command = find_runtime_command(&root, "node", "npx").unwrap().unwrap();

        assert_eq!(directory, runtime_bin);
        assert_eq!(command, runtime_bin.join(runtime_file_name("npx")));
        assert!(runtime_contains(&root, &["node", "npx"]));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn global_runtime_commands_do_not_fall_back_to_path() {
        let _lock = ENVIRONMENT_LOCK.lock().unwrap();
        let root =
            env::temp_dir().join(format!("windie-runtime-resolution-{}", std::process::id()));
        let global_bin = root.join("global-bin");
        let windie_home = root.join("windie-home");
        fs::create_dir_all(&global_bin).unwrap();
        fs::write(global_bin.join(runtime_file_name("npx")), b"global runtime").unwrap();

        let previous_home = env::var_os("WINDIE_HOME");
        let previous_path = env::var_os("PATH");
        unsafe {
            env::set_var("WINDIE_HOME", &windie_home);
            env::set_var("PATH", &global_bin);
        }
        let result = resolve_command("npx");
        unsafe {
            match previous_home {
                Some(value) => env::set_var("WINDIE_HOME", value),
                None => env::remove_var("WINDIE_HOME"),
            }
            match previous_path {
                Some(value) => env::set_var("PATH", value),
                None => env::remove_var("PATH"),
            }
        }
        let _ = fs::remove_dir_all(&root);

        let error = result.unwrap_err().to_string();
        assert!(error.contains("Windie-managed runtime command is not installed"));
    }

    #[test]
    fn verifies_node_style_checksum_manifest_entry() {
        let bytes = b"windie";
        let checksum = format!(
            "{}  node-v22.14.0-darwin-arm64.tar.gz\n",
            archive_fingerprint(bytes)
        );

        verify_archive_checksum(
            "https://nodejs.org/dist/v22.14.0/node-v22.14.0-darwin-arm64.tar.gz",
            bytes,
            &checksum,
        )
        .unwrap();
    }

    #[test]
    fn verifies_binary_checksum_manifest_entry() {
        let bytes = b"windie";
        let checksum = format!(
            "{} *uv-x86_64-pc-windows-msvc.zip\n",
            archive_fingerprint(bytes)
        );

        verify_archive_checksum(
            "https://example.test/uv-x86_64-pc-windows-msvc.zip",
            bytes,
            &checksum,
        )
        .unwrap();
    }

    #[test]
    fn rejects_runtime_checksum_mismatch() {
        let error = verify_archive_checksum(
            "https://example.test/runtime.tar.gz",
            b"windie",
            &format!("{}  runtime.tar.gz", "0".repeat(64)),
        )
        .unwrap_err();

        assert!(error.to_string().contains("checksum mismatch"));
    }
}

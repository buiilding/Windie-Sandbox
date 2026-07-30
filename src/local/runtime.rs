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

/// Ensures the runtime needed by one approved provider is available.
pub(super) fn ensure_provider_runtime(target: &str) -> Result<()> {
    match target {
        "desktop-commander" | "brightdata" => ensure_node_runtime(),
        "blender-mcp" | "basic-memory" => ensure_uv_runtime(),
        "cua-driver" | "bifrost" => Ok(()),
        _ => Err(anyhow!("unknown runtime target: {target}")),
    }
}

/// Resolves an approved provider command without relying on shell lookup.
pub(crate) fn resolve_command(program: &str) -> Result<PathBuf> {
    if let Some(path) = local_runtime_command(program)? {
        return Ok(path);
    }

    #[cfg(target_os = "windows")]
    if program == "cua-driver"
        && let Some(path) = cua_driver_command()?
    {
        return Ok(path);
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

fn ensure_node_runtime() -> Result<()> {
    let version = env::var("WINDIE_NODE_VERSION").unwrap_or_else(|_| NODE_VERSION.to_string());
    let runtime_dir = windie_home_dir()?
        .join("runtimes")
        .join("node")
        .join(&version);
    if runtime_contains(&runtime_dir, &["node", "npx"]) || node_on_path() {
        return Ok(());
    }

    let (url, checksum_url, format) = node_asset(&version)?;
    install_runtime_archive(
        &url,
        &checksum_url,
        format,
        &runtime_dir,
        "Node.js",
        &["node", "npx"],
    )
}

fn ensure_uv_runtime() -> Result<()> {
    let version = env::var("WINDIE_UV_VERSION").unwrap_or_else(|_| UV_VERSION.to_string());
    let runtime_dir = windie_home_dir()?
        .join("runtimes")
        .join("uv")
        .join(&version);
    if runtime_contains(&runtime_dir, &["uv", "uvx"]) || uv_on_path() {
        return Ok(());
    }

    let (url, checksum_url, format) = uv_asset(&version)?;
    install_runtime_archive(
        &url,
        &checksum_url,
        format,
        &runtime_dir,
        "uv",
        &["uv", "uvx"],
    )
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
    executables.iter().all(|executable| {
        find_file(root, runtime_file_name(executable))
            .ok()
            .flatten()
            .is_some()
    })
}

fn node_on_path() -> bool {
    path_command("npx").is_some()
}

fn uv_on_path() -> bool {
    path_command("uvx").is_some()
}

#[cfg(target_os = "windows")]
fn local_runtime_command(program: &str) -> Result<Option<PathBuf>> {
    let (runtime, names): (&str, &[&str]) = match program {
        "npx" => ("node", &["npx.cmd", "npx.exe"]),
        "node" => ("node", &["node.exe"]),
        "uvx" => ("uv", &["uvx.exe"]),
        "uv" => ("uv", &["uv.exe"]),
        _ => return Ok(None),
    };
    let root = windie_home_dir()?.join("runtimes").join(runtime);
    for name in names {
        if let Some(path) = find_file(&root, name)? {
            return Ok(Some(path));
        }
    }
    Ok(None)
}

#[cfg(not(target_os = "windows"))]
fn local_runtime_command(program: &str) -> Result<Option<PathBuf>> {
    let runtime = match program {
        "npx" | "node" => "node",
        "uvx" | "uv" => "uv",
        _ => return Ok(None),
    };
    let root = windie_home_dir()?.join("runtimes").join(runtime);
    find_file(&root, program)
}

#[cfg(target_os = "windows")]
fn cua_driver_command() -> Result<Option<PathBuf>> {
    let candidates = [
        env::var_os("LOCALAPPDATA").map(|path| {
            PathBuf::from(path)
                .join("Programs")
                .join("Cua")
                .join("cua-driver")
                .join("bin")
                .join("cua-driver.exe")
        }),
        env::var_os("USERPROFILE").map(|path| {
            PathBuf::from(path)
                .join(".cua-driver")
                .join("packages")
                .join("current")
                .join("cua-driver.exe")
        }),
    ];
    Ok(candidates.into_iter().flatten().find(|path| path.is_file()))
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

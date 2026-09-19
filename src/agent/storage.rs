//! Owner-only, atomic credential files and OS-released single-agent locking.
//! Credentials are scoped to an API origin and never stored in the conversation DB.

use crate::device::*;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Credentials {
    pub server: String,
    pub enrollment_secret: String,
    pub device_secret: String,
    pub request: EnrollmentRequest,
    pub started: Option<EnrollmentStarted>,
    pub device_id: Option<DeviceId>,
}
/// Pending and active state share one atomic record; device_id marks successful activation.
#[derive(Clone)]
pub(crate) struct Storage {
    root: PathBuf,
}
impl Storage {
    pub fn open() -> Result<Self> {
        let home = std::env::var_os("HOME").context("HOME is not configured")?;
        Self::at(Path::new(&home).join(".windie").join("agent"))
    }
    pub(crate) fn at(root: PathBuf) -> Result<Self> {
        #[cfg(not(unix))]
        anyhow::bail!(
            "Agent credential storage is currently supported only on Unix; Windows requires an ACL/vault adapter"
        );
        #[cfg(unix)]
        {
            let parent = root.parent().context("invalid agent directory")?;
            create_directory(parent)?;
            check_directory(parent, false)?;
            create_directory(&root)?;
            check_directory(&root, true)?;
            Ok(Self { root })
        }
    }
    pub fn lock(&self) -> Result<File> {
        let file = secure_open(&self.root.join("lock"), true)?;
        fs2::FileExt::try_lock_exclusive(&file)
            .context("Another agent connect/run process is active")?;
        Ok(file)
    }
    pub fn load(&self) -> Result<Option<Credentials>> {
        let path = self.root.join("credentials.json");
        if !path.try_exists()? {
            // try_exists follows symlinks, including a dangling one: reject it explicitly.
            if fs::symlink_metadata(&path).is_ok() {
                anyhow::bail!("Unsafe agent credential path");
            }
            return Ok(None);
        }
        let file = secure_open(&path, false)?;
        let mut text = String::new();
        file.take(16385).read_to_string(&mut text)?;
        ensure!(text.len() <= 16384, "Agent credential file is too large");
        let value = serde_json::from_str(&text).map_err(|_| {
            anyhow::anyhow!("Invalid agent credential file; preserve it for recovery")
        })?;
        Ok(Some(value))
    }
    pub fn save(&self, value: &Credentials) -> Result<()> {
        self.save_before_rename(value, || Ok(()))
    }

    /// Loads one fixed, owner-only JSON record used by another agent concern.
    /// Callers supply compile-time file names; this is not a general file API.
    pub(crate) fn load_record<T: DeserializeOwned>(&self, name: &'static str) -> Result<Option<T>> {
        ensure!(
            valid_record_name(name),
            "Invalid protected agent record name"
        );
        let path = self.root.join(name);
        if !path.try_exists()? {
            if fs::symlink_metadata(&path).is_ok() {
                anyhow::bail!("Unsafe protected agent record path");
            }
            return Ok(None);
        }
        let file = secure_open(&path, false)?;
        let mut text = String::new();
        file.take(524_289).read_to_string(&mut text)?;
        ensure!(text.len() <= 524_288, "Protected agent record is too large");
        Ok(Some(serde_json::from_str(&text).map_err(|_| {
            anyhow::anyhow!("Invalid protected agent record; preserve it for recovery")
        })?))
    }

    /// Atomically replaces one fixed owner-only JSON record after flushing it.
    pub(crate) fn save_record<T: Serialize>(&self, name: &'static str, value: &T) -> Result<()> {
        ensure!(
            valid_record_name(name),
            "Invalid protected agent record name"
        );
        check_directory(&self.root, true)?;
        let target = self.root.join(name);
        if fs::symlink_metadata(&target).is_ok() {
            secure_open(&target, false)?;
        }
        let temporary = self.root.join(format!("pending-{}", uuid::Uuid::new_v4()));
        let result = (|| -> Result<()> {
            let mut file = secure_open(&temporary, true)?;
            file.write_all(&serde_json::to_vec(value)?)?;
            file.sync_all()?;
            fs::rename(&temporary, &target)?;
            File::open(&self.root)?.sync_all()?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }

    /// Injection point proves failures before the atomic rename preserve old state.
    fn save_before_rename(
        &self,
        value: &Credentials,
        before_rename: impl FnOnce() -> Result<()>,
    ) -> Result<()> {
        check_directory(&self.root, true)?;
        let target = self.root.join("credentials.json");
        if fs::symlink_metadata(&target).is_ok() {
            secure_open(&target, false)?;
        }
        let temp = self.root.join(format!("pending-{}", uuid::Uuid::new_v4()));
        let result = (|| -> Result<()> {
            let mut file = secure_open(&temp, true)?;
            file.write_all(&serde_json::to_vec(value)?)?;
            file.sync_all()?;
            before_rename()?;
            fs::rename(&temp, &target)?;
            File::open(&self.root)?.sync_all()?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temp);
        }
        result
    }
    /// Only invoked after explicit user confirmation; preserve rejected credentials for recovery.
    pub fn archive(&self) -> Result<()> {
        let path = self.root.join("credentials.json");
        secure_open(&path, false)?;
        fs::rename(
            path,
            self.root
                .join(format!("previous-{}.json", uuid::Uuid::new_v4())),
        )?;
        File::open(&self.root)?.sync_all()?;
        Ok(())
    }
}
fn valid_record_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 80
        && name.ends_with(".json")
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'.' || byte == b'-')
}
fn create_directory(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        match fs::DirBuilder::new().mode(0o700).create(path) {
            Ok(()) => (),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (),
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}
fn check_directory(path: &Path, private: bool) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let m = fs::symlink_metadata(path)?;
        ensure!(
            m.is_dir()
                && !m.file_type().is_symlink()
                && m.uid() == unsafe { libc::geteuid() }
                && m.mode() & if private { 0o077 } else { 0o022 } == 0,
            "Unsafe agent directory ownership or permissions"
        );
    }
    Ok(())
}
fn secure_open(path: &Path, create: bool) -> Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).write(create).create(create);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let file = options
        .open(path)
        .context("Could not open protected agent state")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let m = file.metadata()?;
        ensure!(
            m.is_file()
                && m.uid() == unsafe { libc::geteuid() }
                && m.mode() & 0o077 == 0
                && m.nlink() == 1,
            "Unsafe agent file ownership or permissions"
        );
    }
    Ok(file)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    #[test]
    fn atomic_roundtrip_and_failed_write_preserve_pairing() {
        use std::os::unix::fs::PermissionsExt;
        let base = std::env::temp_dir().join(format!("windie-agent-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&base).unwrap();
        let s = Storage::at(base.join("agent")).unwrap();
        let c = super::super::fresh_credentials(PRODUCTION_API.into()).unwrap();
        s.save(&c).unwrap();
        assert_eq!(s.load().unwrap().unwrap().device_secret, c.device_secret);
        assert_eq!(
            fs::metadata(s.root.join("credentials.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        let mut changed = super::super::fresh_credentials(PRODUCTION_API.into()).unwrap();
        changed.device_id = Some(DeviceId::new());
        // Deterministic fault after writing the temporary file, before promotion.
        assert!(
            s.save_before_rename(&changed, || Err(anyhow::anyhow!(
                "injected storage failure"
            )))
            .is_err()
        );
        assert_eq!(s.load().unwrap().unwrap().device_secret, c.device_secret);
        s.archive().unwrap();
        assert!(s.load().unwrap().is_none());
        assert_eq!(fs::read_dir(&s.root).unwrap().count(), 1);
        fs::remove_dir_all(base).unwrap();
    }
    #[test]
    fn rejects_symlinks_permissions_and_duplicate_locks() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let base = std::env::temp_dir().join(format!("windie-agent-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&base).unwrap();
        let storage = Storage::at(base.join("agent")).unwrap();
        let lock = storage.lock().unwrap();
        assert!(storage.lock().is_err());
        drop(lock);
        assert!(storage.lock().is_ok());
        symlink(base.join("absent"), storage.root.join("credentials.json")).unwrap();
        assert!(storage.load().is_err());
        fs::remove_file(storage.root.join("credentials.json")).unwrap();
        fs::set_permissions(&storage.root, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(Storage::at(storage.root.clone()).is_err());
        fs::remove_dir_all(base).unwrap();
    }
}

//! Named secrets of the HaryX account on this machine: the session, and the sync key.
//!
//! macOS keeps them in the Keychain (service `haryx-account`, account = the name) through the same
//! `security` binary every other haryx secret uses. Elsewhere each is `<store>/account/<name>`,
//! sealed as slot backups are ([`crate::credentials::sealed`]: a 0600 file, DPAPI on Windows).

use std::io::Write as _;
use std::path::{Path, PathBuf};

use super::AccountError;
use crate::credentials::sealed;
use crate::keychain::Security;

pub const KEYCHAIN_SERVICE: &str = "haryx-account";

#[derive(Debug, Clone)]
enum Kind {
    Keychain(Security),
    File,
}

#[derive(Debug, Clone)]
pub struct Vault {
    dir: PathBuf,
    kind: Kind,
}

impl Vault {
    /// The vault for this OS, under the haryx store root.
    pub fn for_store(store_root: &Path) -> Self {
        let dir = store_root.join("account");
        if cfg!(target_os = "macos") {
            Vault::keychain(dir, Security::system())
        } else {
            Vault::file(dir)
        }
    }

    pub fn file(dir: PathBuf) -> Self {
        Vault {
            dir,
            kind: Kind::File,
        }
    }

    pub fn keychain(dir: PathBuf, security: Security) -> Self {
        Vault {
            dir,
            kind: Kind::Keychain(security),
        }
    }

    /// `<store>/account`: lock files live here on every OS, and the secrets too off macOS.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Whether values go through `security -i`, whose line limit caps their size.
    pub fn is_keychain(&self) -> bool {
        matches!(self.kind, Kind::Keychain(_))
    }

    pub fn read(&self, name: &str) -> Result<Option<String>, AccountError> {
        match &self.kind {
            Kind::Keychain(security) => security
                .get_password(KEYCHAIN_SERVICE, name, None)
                .map_err(storage),
            Kind::File => match std::fs::read(self.dir.join(name)) {
                Ok(bytes) => Ok(Some(sealed::unseal(&bytes).map_err(AccountError::Storage)?)),
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(err) => Err(storage(err)),
            },
        }
    }

    pub fn write(&self, name: &str, text: &str) -> Result<(), AccountError> {
        match &self.kind {
            Kind::Keychain(security) => security
                .set_password(KEYCHAIN_SERVICE, name, text)
                .map_err(storage),
            Kind::File => write_private(&self.dir, name, &sealed::seal(text).map_err(storage)?),
        }
    }

    /// Remove `name`. Already gone counts as done.
    pub fn remove(&self, name: &str) -> Result<(), AccountError> {
        match &self.kind {
            Kind::Keychain(security) => security
                .delete_password(KEYCHAIN_SERVICE, name)
                .map_err(storage),
            Kind::File => match std::fs::remove_file(self.dir.join(name)) {
                Err(err) if err.kind() != std::io::ErrorKind::NotFound => Err(storage(err)),
                _ => Ok(()),
            },
        }
    }
}

pub(crate) fn storage(err: impl std::fmt::Display) -> AccountError {
    AccountError::Storage(err.to_string())
}

/// Write `name` in `dir` (made 0700) readable by this user only, replacing any old copy in one
/// step.
pub fn write_private(dir: &Path, name: &str, text: impl AsRef<[u8]>) -> Result<(), AccountError> {
    std::fs::create_dir_all(dir).map_err(storage)?;
    let tmp = dir.join(format!(".{name}.tmp"));
    // A leftover from a failed write keeps its old mode if reopened; start from nothing.
    let _ = std::fs::remove_file(&tmp);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        options.mode(0o600);
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).map_err(storage)?;
    }
    let written = options.open(&tmp).and_then(|mut file| {
        file.write_all(text.as_ref())?;
        file.sync_all()
    });
    let result = written.and_then(|()| std::fs::rename(&tmp, dir.join(name)));
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result.map_err(storage)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_vault_round_trips_sealed_and_private() {
        let dir = tempfile::tempdir().unwrap();
        let vault = Vault::file(dir.path().join("account"));
        assert_eq!(vault.read("x").unwrap(), None);
        vault.write("x", "secret value").unwrap();
        assert_eq!(vault.read("x").unwrap().as_deref(), Some("secret value"));
        let raw = std::fs::read_to_string(vault.dir().join("x")).unwrap();
        assert!(!raw.contains("secret"), "sealed, not plain");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(&vault.dir().join("x")), 0o600);
            assert_eq!(mode(vault.dir()), 0o700);
        }
        vault.remove("x").unwrap();
        vault.remove("x").unwrap();
        assert_eq!(vault.read("x").unwrap(), None);
        assert!(!vault.is_keychain());
    }
}

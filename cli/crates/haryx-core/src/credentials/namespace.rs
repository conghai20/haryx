//! Where slot backups live: haryx's own namespace, and cswap's for `import --from-cswap`.
//!
//! The LIVE credential is shared with Claude Code (see `claude_keychain`). Slot backups are not: haryx keeps
//! them under its own store root and its own Keychain service, so the two tools never overwrite each
//! other's backups.

use std::path::{Path, PathBuf};

use crate::paths::{self, PathEnv};

/// Keychain service of haryx's per-slot backup credentials (upstream `SECURITY_SERVICE`, renamed).
pub const HARYX_BACKUP_SERVICE: &str = "haryx";
/// cswap's per-slot backup service (upstream `SECURITY_SERVICE`).
pub const CSWAP_BACKUP_SERVICE: &str = "claude-swap";
/// The service the legacy `keyring` library used for cswap's backups before `security` (upstream
/// `switcher.KEYRING_SERVICE`). cswap 1.2.0b1 migrates these away; listed for completeness of an import.
pub const CSWAP_LEGACY_KEYRING_SERVICE: &str = "claude-code";
/// Subdirectory of a store root holding the `.enc` backup files and the unclaimed stash.
pub const CREDENTIALS_DIRNAME: &str = "credentials";
/// The account registry inside a store root.
pub const SEQUENCE_FILENAME: &str = "sequence.json";

/// Keychain account of a slot's backup item: `account-{num}-{email}`.
pub fn backup_keychain_account(account_num: &str, email: &str) -> String {
    format!("account-{account_num}-{email}")
}

/// Keychain account of a slot's retained previous generation: `account-{num}-{email}.prev`.
pub fn prev_backup_keychain_account(account_num: &str, email: &str) -> String {
    format!("{}.prev", backup_keychain_account(account_num, email))
}

/// `.creds-{num}-{email}.enc`, the file form of a slot backup.
pub fn backup_enc_filename(account_num: &str, email: &str) -> String {
    format!(".creds-{account_num}-{email}.enc")
}

/// `.creds-{num}-{email}.enc.prev`, the file form of a retained previous generation.
pub fn prev_backup_enc_filename(account_num: &str, email: &str) -> String {
    format!("{}.prev", backup_enc_filename(account_num, email))
}

/// One backup namespace: a Keychain service plus the directory of `.enc` files.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BackupNamespace {
    pub keychain_service: &'static str,
    pub store_root: PathBuf,
    pub credentials_dir: PathBuf,
    pub sequence_file: PathBuf,
}

impl BackupNamespace {
    fn at(keychain_service: &'static str, store_root: &Path) -> Self {
        BackupNamespace {
            keychain_service,
            store_root: store_root.to_path_buf(),
            credentials_dir: store_root.join(CREDENTIALS_DIRNAME),
            sequence_file: store_root.join(SEQUENCE_FILENAME),
        }
    }

    /// haryx's own namespace under `store_root`.
    pub fn haryx(store_root: &Path) -> Self {
        Self::at(HARYX_BACKUP_SERVICE, store_root)
    }

    /// cswap's namespace under one of its store roots.
    pub fn cswap(store_root: &Path) -> Self {
        Self::at(CSWAP_BACKUP_SERVICE, store_root)
    }
}

/// haryx's namespace for this environment ([`paths::store_root_in`]).
pub fn haryx_backup_namespace(env: &PathEnv) -> crate::Result<BackupNamespace> {
    Ok(BackupNamespace::haryx(&paths::store_root_in(env)?))
}

/// Every place cswap may keep its backups, most likely first ([`paths::cswap_store_candidates_in`]), all under
/// the `claude-swap` Keychain service.
pub fn cswap_backup_namespaces(env: &PathEnv) -> Vec<BackupNamespace> {
    paths::cswap_store_candidates_in(env)
        .iter()
        .map(|root| BackupNamespace::cswap(root))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::HostOs;
    use std::ffi::OsString;

    #[test]
    fn haryx_and_cswap_namespaces_differ() {
        let mut env = PathEnv::with_home("/home/u");
        env.os = HostOs::Linux;
        env.haryx_home = Some(OsString::from("/data/haryx"));
        let ours = haryx_backup_namespace(&env).unwrap();
        assert_eq!(ours.keychain_service, "haryx");
        assert_eq!(
            ours.credentials_dir,
            PathBuf::from("/data/haryx/credentials")
        );
        let theirs = cswap_backup_namespaces(&env);
        assert_eq!(theirs.len(), 2);
        assert_eq!(theirs[0].keychain_service, "claude-swap");
        assert_eq!(
            theirs[0].credentials_dir,
            PathBuf::from("/home/u/.local/share/claude-swap/credentials")
        );
        assert_eq!(
            theirs[1].credentials_dir,
            PathBuf::from("/home/u/.claude-swap-backup/credentials")
        );
    }

    #[test]
    fn item_names_match_upstream() {
        assert_eq!(backup_keychain_account("2", "a@b.c"), "account-2-a@b.c");
        assert_eq!(
            prev_backup_keychain_account("2", "a@b.c"),
            "account-2-a@b.c.prev"
        );
        assert_eq!(backup_enc_filename("2", "a@b.c"), ".creds-2-a@b.c.enc");
        assert_eq!(
            prev_backup_enc_filename("2", "a@b.c"),
            ".creds-2-a@b.c.enc.prev"
        );
    }
}

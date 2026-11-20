//! Credential and config backup access (upstream `switcher.py` ~710-1066 and the public wrappers
//! ~1859-1880): thin delegation to [`crate::credentials::CredentialStore`] plus the slot-level rules the
//! switcher adds (session invalidation after a backup write, the capture read, the config backups).

use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use super::{JsonObject, Switcher, record, text};
use crate::credentials::{
    ActiveCredentials, CLAUDE_CODE_KEYCHAIN_SERVICE, merge_shared_credential_fields,
    shared_credential_fields,
};
use crate::error::{Error, Result};
use crate::fsutil::{self, WriteOptions};
use crate::mappings::MappingStore;
use crate::paths;
use crate::printer::dimmed;
use crate::session;

impl Switcher {
    // --- the live credential -----------------------------------------------------------------------

    /// Claude Code's active credential (`Some("")` when none, `None` on a file read error).
    pub fn read_credentials(&self) -> Option<String> {
        self.store.read_credentials()
    }

    pub fn read_active_credentials(&self) -> ActiveCredentials {
        self.store.read_active_credentials()
    }

    pub fn write_credentials(&self, credentials: &str) -> Result<()> {
        self.store.write_credentials(credentials)
    }

    /// Refuse to CAPTURE bytes a degraded read produced, returning the value THIS read produced (a second
    /// read could pass the check and then capture a stale fallback).
    pub fn refuse_degraded_capture(&self) -> Result<Option<String>> {
        let active = self.read_active_credentials();
        if active.degraded {
            return Err(Error::CredentialRead(
                "The macOS Keychain is unreadable right now (locked or no GUI session), so the only \
                 readable credential is a plaintext fallback that may be a superseded generation — \
                 capturing it would file a spent refresh token against this slot. Retry from a GUI \
                 terminal."
                    .into(),
            ));
        }
        Ok(active.value)
    }

    /// Read the credential of the profile the environment points at, the way claude resolves it, so a
    /// slot's email and token come from one profile (upstream `_read_capture_credentials`).
    ///
    /// A DEFINED `CLAUDE_SECURESTORAGE_CONFIG_DIR` names the only store claude reads (defined-but-empty:
    /// the default profile's unsuffixed item); a miss there is "logged out". Without it, an unset
    /// `CLAUDE_CONFIG_DIR` reads the active store; a set one reads that profile, falling back to the active
    /// store only when it IS the default profile, then to the profile's own `primaryApiKey`. Strict on the
    /// Keychain: unreadable (not absent) fails instead of capturing a possibly stale plaintext seed.
    pub fn read_capture_credentials(&self) -> Result<Option<String>> {
        let kc = self.profile_keychain();
        if let Some(secure) = &self.env.secure_storage_dir {
            let default_home = paths::default_claude_config_home_in(&self.env);
            let (dir, service) = if secure.is_empty() {
                (
                    default_home.into_os_string(),
                    Some(CLAUDE_CODE_KEYCHAIN_SERVICE),
                )
            } else {
                (secure.clone(), None)
            };
            let creds = session::read_config_dir_credentials(&kc, &dir, true, service)?;
            return Ok(Some(creds.filter(|c| !c.is_empty()).unwrap_or_default()));
        }
        let Some(config_dir) = self.env.claude_config_dir.clone().filter(|d| !d.is_empty()) else {
            return self.refuse_degraded_capture();
        };
        let creds = session::read_config_dir_credentials(&kc, &config_dir, true, None)?;
        if let Some(creds) = creds.filter(|c| !c.is_empty()) {
            return Ok(Some(creds));
        }
        if same_directory(
            Path::new(&config_dir),
            &paths::default_claude_config_home_in(&self.env),
        ) {
            return self.refuse_degraded_capture();
        }
        let config = self.read_json(&self.claude_config_path(), false)?;
        let key = config
            .as_ref()
            .and_then(|c| c.get("primaryApiKey"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        Ok(Some(key.to_owned()))
    }

    /// Compose the credential to activate: the machine-shared OAuth integrations come from the live
    /// credential (current generation, absence included), everything else travels with the slot.
    pub fn prepare_credentials_for_activation(
        &self,
        target_credentials: &str,
        live_credentials: Option<&str>,
    ) -> String {
        match shared_credential_fields(live_credentials) {
            None => target_credentials.to_owned(),
            Some(live_shared) => merge_shared_credential_fields(target_credentials, &live_shared),
        }
    }

    // --- slot backups ------------------------------------------------------------------------------

    pub fn uses_file_backup_backend(&self) -> bool {
        self.store.uses_file_backup_backend()
    }

    pub fn backup_enc_path(&self, account_num: &str, email: &str) -> PathBuf {
        self.store.backup_enc_path(account_num, email)
    }

    /// A slot's backup, `""` when missing (session bootstrap reads through this too). `cached` serves
    /// display reads from the store's short-lived Keychain cache.
    pub fn read_account_credentials(&self, account_num: &str, email: &str, cached: bool) -> String {
        self.store
            .read_account_credentials(account_num, email, cached)
    }

    /// A slot's backup with the unreadable-vs-absent verdict: `(value, unreadable)`.
    pub fn read_account_credentials_ex(
        &self,
        account_num: &str,
        email: &str,
        cached: bool,
    ) -> (String, bool) {
        self.store
            .read_account_credentials_ex(account_num, email, cached)
    }

    /// Write a slot's backup, then invalidate the slot's session profile.
    ///
    /// Takes NO lock: callers hold the account lock when they need one. PAST THE STORE WRITE NOTHING MAY
    /// FAIL: the write advanced the slot, and callers read an error as "the persist failed". So a failed
    /// invalidation leaves the stale marker instead (which forces the re-bootstrap anyway), and only a
    /// marker that could not land either is reported, at ERROR.
    pub fn write_account_credentials(
        &self,
        account_num: &str,
        email: &str,
        credentials: &str,
    ) -> Result<()> {
        #[cfg(test)]
        self.faults.check_credential_write()?;
        self.store
            .write_account_credentials(account_num, email, credentials)?;
        if let Err(err) = self.post_backup_write(account_num, email) {
            if session::mark_session_stale(&self.session_dir(account_num, email)) {
                log::warn!(
                    "Stored account {account_num}'s credential but could not invalidate its session \
                     profile; marked it stale so the next run re-bootstraps. ({err})"
                );
            } else {
                log::error!(
                    "Stored account {account_num}'s credential but could NOT invalidate its session \
                     profile OR mark it stale; the profile may keep serving the superseded generation \
                     until its token expires. ({err})"
                );
            }
        }
        Ok(())
    }

    /// Invalidate the slot's session profile after its backup changed: a live session gets the stale
    /// marker (claude owns its copy while it runs), a quiescent one loses its credential material.
    fn post_backup_write(&self, account_num: &str, email: &str) -> std::io::Result<()> {
        if self.live_session_pids(account_num, email).is_empty() {
            return self.invalidate_session_credentials(account_num, email);
        }
        if !session::mark_session_stale(&self.session_dir(account_num, email)) {
            log::error!(
                "Account {account_num}'s backup credentials changed but its live session profile could \
                 not be marked stale; it may keep serving the superseded generation once it exits."
            );
        }
        Ok(())
    }

    /// Persist rotated credentials to an INACTIVE slot's backup, under the account lock. The caller must
    /// not hold the lock (it is non-reentrant).
    pub fn persist_backup_credentials(
        &self,
        account_num: &str,
        email: &str,
        credentials: &str,
    ) -> Result<()> {
        let _lock = self.lock()?;
        self.write_account_credentials(account_num, email, credentials)
    }

    /// Delete a slot's backup from both backends, best effort.
    pub fn delete_account_credentials(&self, account_num: &str, email: &str) {
        self.store.delete_account_credentials(account_num, email);
    }

    /// Pre-commit clear that fails unless the key is assured empty.
    pub fn delete_account_credentials_strict(&self, account_num: &str, email: &str) -> Result<()> {
        self.store
            .delete_account_credentials_strict(account_num, email)
    }

    /// Delete everything a slot key holds (credential, config, session profile): the single chokepoint
    /// of every path that removes or displaces a slot. Refuses while a session-mode claude is live on it.
    pub fn delete_account_files(&self, account_num: &str, email: &str) -> Result<()> {
        self.ensure_no_live_session(account_num, email, "the operation")?;
        self.delete_account_credentials(account_num, email);
        let config_file = self.config_backup_path(account_num, email);
        if config_file.exists() {
            std::fs::remove_file(&config_file)?;
        }
        self.delete_session_profile(account_num, email);
        Ok(())
    }

    /// Drop directory mappings for an identity that no longer has a slot.
    pub fn prune_mappings(&self, email: &str, org_uuid: &str) -> Result<()> {
        let pruned = MappingStore::new(&self.backup_dir).prune_account(email, org_uuid)?;
        if pruned > 0 {
            println!(
                "{}",
                dimmed(&format!(
                    "Removed {pruned} directory mapping(s) for this account"
                ))
            );
        }
        Ok(())
    }

    // --- config backups ----------------------------------------------------------------------------

    pub fn config_backup_path(&self, account_num: &str, email: &str) -> PathBuf {
        self.configs_dir
            .join(format!(".claude-config-{account_num}-{email}.json"))
    }

    /// A slot's config backup, `""` when missing.
    pub fn read_account_config(&self, account_num: &str, email: &str) -> Result<String> {
        match std::fs::read_to_string(self.config_backup_path(account_num, email)) {
            Ok(text) => Ok(text),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
            Err(err) => Err(err.into()),
        }
    }

    /// Write a slot's config backup atomically (a torn backup is a broken future restore).
    pub fn write_account_config(&self, account_num: &str, email: &str, config: &str) -> Result<()> {
        fsutil::atomic_write_text(
            &self.config_backup_path(account_num, email),
            config,
            WriteOptions::default(),
        )?;
        Ok(())
    }

    /// Delete one slot key's config backup. Unconditional unlink (an `exists` check fails open on an
    /// inaccessible directory); missing is fine, anything else is an error.
    pub fn delete_config_backup(&self, account_num: &str, email: &str) -> Result<()> {
        match std::fs::remove_file(self.config_backup_path(account_num, email)) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(err.into()),
        }
    }

    /// Whether a slot has both a stored credential and a config backup (activation eligibility; served
    /// from the display read cache, since every activation re-reads fresh).
    pub fn account_is_switchable(&self, account_num: &str) -> Result<bool> {
        let data = self.get_sequence_data()?.unwrap_or_default();
        Ok(self.switchable_in(&data, account_num))
    }

    pub(crate) fn switchable_in(&self, data: &JsonObject, account_num: &str) -> bool {
        let Some(rec) = record(data, account_num) else {
            return false;
        };
        let email = text(rec, "email");
        !self
            .read_account_credentials(account_num, email, true)
            .is_empty()
            && self
                .read_account_config(account_num, email)
                .is_ok_and(|c| !c.is_empty())
    }

    // --- the unclaimed stash -----------------------------------------------------------------------

    /// Safety copies preserved at switch time (diagnostics only; never consumed automatically).
    pub fn list_unclaimed_credentials(&self) -> Map<String, Value> {
        self.store.list_unclaimed_credentials()
    }

    pub fn remove_unclaimed_credential(&self, entry_id: &str) -> Result<()> {
        self.store.remove_unclaimed_credential(entry_id)
    }
}

/// Whether two paths name the same directory, symlinks and `..` included (resolved leniently, like
/// Python's `Path.resolve()`).
pub(crate) fn same_directory(left: &Path, right: &Path) -> bool {
    paths::resolve_lenient(left) == paths::resolve_lenient(right)
}

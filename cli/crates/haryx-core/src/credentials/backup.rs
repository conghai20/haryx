//! Per-slot BACKUP credentials, in haryx's own namespace.
//!
//! Two backends: base64 `.enc` files under `credentials_dir` and the macOS Keychain (the host's
//! `backup_service`). On macOS reads are `.enc`-wins: a fallback `.enc` (written while the Keychain was
//! unusable) is authoritative over a possibly-stale Keychain copy, so a Keychain that recovers cannot shadow
//! a newer file. A successful Keychain write therefore reconciles the `.enc` away (correctness-critical).
//!
//! One retained previous generation per slot (`.prev`), routed like the backup itself, so retention never
//! weakens the storage posture (a Keychain-backed Mac does not grow a plaintext copy). Best-effort: it is
//! defense in depth behind the switch-time provenance check and the unclaimed stash.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;

use super::namespace::{
    backup_enc_filename, backup_keychain_account, prev_backup_enc_filename,
    prev_backup_keychain_account,
};
use super::{CredentialStore, lock};
use crate::fsutil::{self, WriteOptions};
use crate::keychain::KeychainError;
use crate::{Error, Result};

/// `sequence.json`'s (inode, mtime_ns, size).
type RegistryGeneration = (u64, i128, u64);

/// The display read cache: `(num, email) -> (value or "", read_at)`, voided by an epoch bump on every own
/// write/delete and whenever the registry generation changes.
#[derive(Debug, Default)]
pub(super) struct BackupReadCache {
    entries: HashMap<(String, String), (String, Instant)>,
    epoch: u64,
    generation: Option<RegistryGeneration>,
}

/// Why a Keychain backup read produced no value.
#[derive(Debug)]
pub(super) enum KcReadError {
    /// Skipped inside the failure cooldown without spawning `security`: still "unreadable", but not a new
    /// failure to warn about (upstream `KeychainBackedOff`).
    BackedOff,
    Failed(KeychainError),
}

fn decode_b64_text(encoded: &[u8]) -> std::result::Result<String, String> {
    // STANDARD is strict like Python's `validate=True`: non-alphabet junk is rejected, not discarded, so a
    // corrupt `.enc` cannot shadow a valid Keychain copy.
    let bytes = STANDARD.decode(encoded).map_err(|e| e.to_string())?;
    String::from_utf8(bytes).map_err(|e| e.to_string())
}

impl CredentialStore {
    pub fn backup_enc_path(&self, account_num: &str, email: &str) -> PathBuf {
        self.host
            .credentials_dir
            .join(backup_enc_filename(account_num, email))
    }

    pub fn prev_backup_path(&self, account_num: &str, email: &str) -> PathBuf {
        self.host
            .credentials_dir
            .join(prev_backup_enc_filename(account_num, email))
    }

    fn registry_generation(&self) -> Option<RegistryGeneration> {
        let meta = std::fs::metadata(&self.host.sequence_file).ok()?;
        Some(metadata_generation(&meta))
    }

    /// Forget every cached backup read. The epoch bump also voids a fill that was mid-`security` when the
    /// drop landed.
    pub fn drop_backup_read_cache(&self) {
        let mut cache = lock(&self.cache);
        cache.entries.clear();
        cache.epoch += 1;
    }

    /// Read a backup from the Keychain only (no file fallback). `""` when absent.
    ///
    /// Honours the failure cooldown: inside it, fails with [`KcReadError::BackedOff`] WITHOUT spawning
    /// `security` (upstream #496). A pinned file mode has no deadline, so it never blocks backup reads.
    ///
    /// `cached` serves the value from the per-slot cache for up to the TTL. For DISPLAY and ELIGIBILITY
    /// reads only; anything whose bytes are POSTed, written, activated or compared to decide a write reads
    /// fresh. Only successes are cached.
    pub(super) fn kc_read_backup(
        &self,
        account_num: &str,
        email: &str,
        cached: bool,
    ) -> std::result::Result<String, KcReadError> {
        let key = (account_num.to_string(), email.to_string());
        let mut fill_epoch = None;
        let now = Instant::now();
        if cached {
            let generation = self.registry_generation();
            let mut cache = lock(&self.cache);
            if generation != cache.generation {
                // The registry changed: any entry may describe a slot that moved or was re-added.
                cache.entries.clear();
                cache.epoch += 1;
                cache.generation = generation;
            }
            if let Some((value, read_at)) = cache.entries.get(&key) {
                if now.duration_since(*read_at) < self.tunables.backup_read_cache_ttl {
                    return Ok(value.clone());
                }
            }
            fill_epoch = Some(cache.epoch);
        }
        self.check_backoff()?;
        let account = backup_keychain_account(account_num, email);
        let value = self
            .kc_call(|kc| kc.get_password(&self.host.backup_service, &account))
            .map_err(KcReadError::Failed)?
            .unwrap_or_default();
        if let Some(epoch) = fill_epoch {
            let mut cache = lock(&self.cache);
            // Store only if nothing was dropped while `security` ran.
            if epoch == cache.epoch {
                cache.entries.insert(key, (value.clone(), now));
            }
        }
        Ok(value)
    }

    fn check_backoff(&self) -> std::result::Result<(), KcReadError> {
        let mut state = lock(&self.state);
        let Some(deadline) = state.disabled_until else {
            return Ok(());
        };
        let now = Instant::now();
        if now >= deadline {
            return Ok(());
        }
        if state.backoff_announced_until != Some(deadline) {
            state.backoff_announced_until = Some(deadline);
            log::info!(
                "Keychain backed off after a failure: backup reads report unreadable for up to {:.0} s \
                 instead of probing",
                (deadline - now).as_secs_f64()
            );
        }
        Err(KcReadError::BackedOff)
    }

    /// Write a backup to the Keychain only. The cache is dropped AFTER the op, success or not: dropping first
    /// would let a racing miss cache the old item before the write lands.
    pub(super) fn kc_write_backup(
        &self,
        account_num: &str,
        email: &str,
        credentials: &str,
    ) -> std::result::Result<(), KeychainError> {
        let account = backup_keychain_account(account_num, email);
        let result =
            self.kc_call(|kc| kc.set_password(&self.host.backup_service, &account, credentials));
        self.drop_backup_read_cache();
        result
    }

    fn kc_delete_backup(
        &self,
        account_num: &str,
        email: &str,
    ) -> std::result::Result<(), KeychainError> {
        let account = backup_keychain_account(account_num, email);
        let result = self.kc_call(|kc| kc.delete_password(&self.host.backup_service, &account));
        self.drop_backup_read_cache();
        result
    }

    fn delete_backup_keychain_quiet(&self, account_num: &str, email: &str) {
        if let Err(err) = self.kc_delete_backup(account_num, email) {
            log::warn!("Failed to delete credentials from Keychain: {err}");
        }
    }

    /// Atomically write a base64-encoded credential file (0600), creating `credentials_dir`.
    fn atomic_b64_write(&self, target: &Path, credentials: &str) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.host.credentials_dir)?;
        let encoded = STANDARD.encode(credentials.as_bytes());
        fsutil::atomic_write_text(target, &encoded, WriteOptions::default())
    }

    /// Stop a leftover `.enc` from shadowing a just-written Keychain backup: delete it; if that fails,
    /// rewrite it with the same fresh credential; if that fails too, return the error.
    fn reconcile_enc_after_keychain_write(
        &self,
        account_num: &str,
        email: &str,
        credentials: &str,
    ) -> std::io::Result<()> {
        let enc_file = self.backup_enc_path(account_num, email);
        match std::fs::remove_file(&enc_file) {
            Ok(()) => return Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(err) => log::warn!(
                "Could not delete .enc after Keychain backup write ({err}); rewriting it with the fresh \
                 credentials to keep both consistent"
            ),
        }
        self.atomic_b64_write(&enc_file, credentials)
    }

    /// Read a slot's backup, `""` when missing. See [`Self::read_account_credentials_ex`] for the verdict.
    pub fn read_account_credentials(&self, account_num: &str, email: &str, cached: bool) -> String {
        self.read_backup_inner(account_num, email, cached).0
    }

    /// `(value, failed)`: `failed` when a read actually FAILED (not when it found nothing).
    fn read_backup_inner(&self, account_num: &str, email: &str, cached: bool) -> (String, bool) {
        let mut failed = false;
        let enc_file = self.backup_enc_path(account_num, email);
        // `read`, not `exists`: an unsearchable directory must be a read failure, not an absent backup.
        match std::fs::read(&enc_file) {
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => {
                failed = true;
                log::warn!("Failed to read credentials file: {err}");
            }
            Ok(bytes) => match decode_b64_text(bytes.trim_ascii()) {
                // Corrupt content falls through to the Keychain copy: content-level, not a read failure.
                Err(err) => log::warn!("Failed to read credentials file: {err}"),
                Ok(decoded) if !decoded.is_empty() => return (decoded, false),
                Ok(_) => {} // empty is not a real backup -> try the Keychain
            },
        }
        if self.is_macos() {
            match self.kc_read_backup(account_num, email, cached) {
                Ok(value) => return (value, failed),
                // An expected skip inside the cooldown, announced once when it began.
                Err(KcReadError::BackedOff) => failed = true,
                Err(KcReadError::Failed(err)) => {
                    failed = true;
                    log::warn!("Failed to read credentials from Keychain: {err}");
                }
            }
        }
        (String::new(), failed)
    }

    /// Backup read with an unreadable-vs-absent verdict: `(value, unreadable)`.
    ///
    /// `unreadable` is true when the read that produced `""` FAILED: the `.enc` exists but could not be read
    /// (every platform), or the macOS Keychain read raised or was backed off. A genuinely absent backup reads
    /// `("", false)`. Observed at the read itself, never from process-wide flags: a pinned file mode for the
    /// ACTIVE credential says nothing about a backup, and a concurrent read of another slot must not decide
    /// this one's verdict.
    pub fn read_account_credentials_ex(
        &self,
        account_num: &str,
        email: &str,
        cached: bool,
    ) -> (String, bool) {
        let (value, failed) = self.read_backup_inner(account_num, email, cached);
        if !value.is_empty() {
            return (value, false);
        }
        (String::new(), failed)
    }

    /// Write a slot's backup (pure I/O).
    ///
    /// macOS writes the Keychain when usable, then reconciles the `.enc` away. Otherwise it writes the `.enc`
    /// atomically, then (macOS) best-effort deletes any stale Keychain copy. The current generation is
    /// retained as `.prev` first. Fails before returning on a file-write failure.
    pub fn write_account_credentials(
        &self,
        account_num: &str,
        email: &str,
        credentials: &str,
    ) -> Result<()> {
        self.retain_previous_backup(account_num, email, credentials);
        if self.use_keychain() {
            match self.kc_write_backup(account_num, email, credentials) {
                Ok(()) => {
                    return self
                        .reconcile_enc_after_keychain_write(account_num, email, credentials)
                        .map_err(Error::from);
                }
                Err(err) => {
                    log::warn!("Keychain backup write failed, falling back to file: {err}")
                }
            }
        }
        if let Err(err) =
            self.atomic_b64_write(&self.backup_enc_path(account_num, email), credentials)
        {
            log::warn!("Failed to write credentials file: {err}");
            return Err(err.into());
        }
        if self.is_macos() {
            self.delete_backup_keychain_quiet(account_num, email);
        }
        Ok(())
    }

    /// Delete a slot's backup from both backends, best-effort, including the legacy `account-None-{email}`
    /// alias and the `.prev` generations. A lingering Keychain item is harmless cruft (purge sweeps it).
    pub fn delete_account_credentials(&self, account_num: &str, email: &str) {
        let mut nums = vec![account_num];
        if account_num != "None" {
            nums.push("None");
        }
        for num in nums {
            let enc_file = self.backup_enc_path(num, email);
            if let Err(err) = std::fs::remove_file(&enc_file) {
                if err.kind() != std::io::ErrorKind::NotFound {
                    log::warn!("Failed to delete credentials file: {err}");
                }
            }
            if self.is_macos() {
                self.delete_backup_keychain_quiet(num, email);
            }
            self.delete_previous_backup(num, email);
        }
    }

    /// Clear a slot key, failing closed: error unless emptiness is assured.
    ///
    /// For transactional pre-commit clears (swap/move and their rollback): a destination that must be empty
    /// but may still serve material is the wrong-credential state the transaction exists to prevent. The
    /// served backends are deleted with errors propagating (absence is success), the Keychain one even when
    /// routing says file mode; then an unreadable-or-present read-back aborts too.
    pub fn delete_account_credentials_strict(&self, account_num: &str, email: &str) -> Result<()> {
        self.delete_account_credentials(account_num, email);
        let abort = |detail: String| {
            Error::Credential(format!(
                "Could not clear stored credentials for slot {account_num} ({email}) — aborting before \
                 commit{detail}"
            ))
        };
        match std::fs::remove_file(self.backup_enc_path(account_num, email)) {
            Ok(()) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(abort(format!(": {err}"))),
        }
        if self.is_macos() {
            self.kc_delete_backup(account_num, email)
                .map_err(|err| abort(format!(": {err}")))?;
        }
        let (value, unreadable) = self.read_account_credentials_ex(account_num, email, false);
        if !value.is_empty() || unreadable {
            return Err(abort(String::new()));
        }
        Ok(())
    }

    /// Drop a slot key's retained `.prev` generation (both backends), best-effort. Also called alone when a
    /// key's history stops belonging to its account (a renumber), so recovery never resurrects a displaced
    /// generation onto the key's new owner.
    pub fn delete_previous_backup(&self, account_num: &str, email: &str) {
        if let Err(err) = std::fs::remove_file(self.prev_backup_path(account_num, email)) {
            if err.kind() != std::io::ErrorKind::NotFound {
                log::warn!("Failed to delete .prev file: {err}");
            }
        }
        if self.is_macos() {
            let account = prev_backup_keychain_account(account_num, email);
            if let Err(err) =
                self.kc_call(|kc| kc.delete_password(&self.host.backup_service, &account))
            {
                log::warn!("Failed to delete .prev from Keychain: {err}");
            }
        }
    }

    /// Retain the slot's current backup as `.prev` before it is replaced.
    ///
    /// When the current generation cannot be read, nothing is retained and the overwrite proceeds: a
    /// checkpoint of the incoming bytes could silently outrank a real `.prev` (a plaintext `.enc.prev` wins
    /// over the Keychain one), which is worse than no cushion.
    fn retain_previous_backup(&self, account_num: &str, email: &str, new_credentials: &str) {
        let (current, unreadable) = self.read_account_credentials_ex(account_num, email, false);
        if unreadable {
            log::info!(
                "Could not retain previous credential generation for account {account_num}: the current \
                 backup exists but could not be read (not absent) — no .prev recovery copy will exist for \
                 this write"
            );
            return;
        }
        if current.is_empty() || current == new_credentials {
            return;
        }
        let result = if self.use_keychain() {
            let account = prev_backup_keychain_account(account_num, email);
            self.kc_call(|kc| kc.set_password(&self.host.backup_service, &account, &current))
                .map_err(|e| e.to_string())
        } else {
            self.atomic_b64_write(&self.prev_backup_path(account_num, email), &current)
                .map_err(|e| e.to_string())
        };
        if let Err(err) = result {
            log::warn!(
                "Failed to retain previous credential generation for account {account_num}: {err}"
            );
        }
    }

    /// Read the retained previous generation, `""` when absent or corrupt. `.enc.prev`-wins like the main
    /// backup read.
    pub fn read_previous_backup(&self, account_num: &str, email: &str) -> String {
        let prev_file = self.prev_backup_path(account_num, email);
        match std::fs::read(&prev_file) {
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => log::warn!("Failed to read .prev file: {err}"),
            Ok(bytes) => match decode_b64_text(bytes.trim_ascii()) {
                Ok(decoded) if !decoded.is_empty() => return decoded,
                Ok(_) => {}
                Err(err) => log::warn!("Failed to read .prev file: {err}"),
            },
        }
        if self.is_macos() {
            let account = prev_backup_keychain_account(account_num, email);
            match self.kc_call(|kc| kc.get_password(&self.host.backup_service, &account)) {
                Ok(value) => return value.unwrap_or_default(),
                Err(err) => log::warn!("Failed to read .prev from Keychain: {err}"),
            }
        }
        String::new()
    }
}

#[cfg(unix)]
fn metadata_generation(meta: &std::fs::Metadata) -> RegistryGeneration {
    use std::os::unix::fs::MetadataExt;
    let mtime_ns = i128::from(meta.mtime()) * 1_000_000_000 + i128::from(meta.mtime_nsec());
    (meta.ino(), mtime_ns, meta.size())
}

#[cfg(not(unix))]
fn metadata_generation(meta: &std::fs::Metadata) -> RegistryGeneration {
    // TODO(windows): no inode; mtime and size only.
    let mtime_ns = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos() as i128);
    (0, mtime_ns, meta.len())
}

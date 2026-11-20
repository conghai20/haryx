//! Renumbering accounts: `swap` (two accounts trade slots) and `move` (an account goes to a slot number,
//! swapping when it is occupied). Upstream `switcher.py` ~1172-1811.
//!
//! Everything keyed by the slot number moves with the account: the records (aliases included), the
//! credential and config backups, membership in `sequence` (kept sorted), `activeAccountNumber` and the
//! session profile. The whole resolve-validate-mutate span runs under the account lock, and the
//! `sequence.json` write is the commit point: a failure before it rolls back, after it only best-effort
//! cleanup of old keys remains.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::policy::sequence_numbers;
use super::sequence::is_digits;
use super::{JsonObject, Switcher, accounts, record, text};
use crate::error::{Error, Result};
use crate::models::get_timestamp;
use crate::printer;

/// One slot's backup material read before a relocation: `(credentials, config)`.
type Material = (String, String);

/// Highest legal `move` target unless the table already grew past it (`add` numbers from the max, so a
/// stray huge target would inflate every future account number).
const MOVE_TARGET_CAP: i64 = 99;

impl Switcher {
    /// Exchange two accounts' slot numbers. Returns the two resolved slot numbers `(first, second)`.
    ///
    /// Directory mappings key on (email, org) and are unaffected; usage rows and auto-switch quarantine
    /// entries key on the slot but carry the identity, so they self-heal on the next pass.
    pub fn swap_accounts(&self, first: &str, second: &str) -> Result<(String, String)> {
        if !self.sequence_file.exists() {
            return Err(Error::Config("No accounts are managed yet".into()));
        }
        self.refuse_session_shell()?;
        let _lock = self.lock()?;
        self.swap_accounts_locked(first, second)
    }

    /// Backup read for the pre-mutation snapshot; an unreadable (not absent) backup aborts before
    /// anything moved, instead of silently dropping the slot's live refresh token.
    fn read_backup_or_abort(&self, account_num: &str, email: &str) -> Result<String> {
        let (creds, unreadable) = self.read_account_credentials_ex(account_num, email, false);
        if unreadable {
            return Err(Error::Config(format!(
                "Account-{account_num}'s stored credential could not be read (keychain unavailable?); \
                 nothing was changed. Retry once it is readable again."
            )));
        }
        Ok(creds)
    }

    /// Body of [`Self::swap_accounts`]; the caller holds the account lock (the lock is non-reentrant, so
    /// `move` resolves and dispatches inside one acquisition).
    fn swap_accounts_locked(&self, first: &str, second: &str) -> Result<(String, String)> {
        self.get_sequence_data_migrated()?;
        let num_a = self.resolve_account_identifier(first)?.ok_or_else(|| {
            Error::AccountNotFound(format!("No account found with identifier: {first}"))
        })?;
        let num_b = self.resolve_account_identifier(second)?.ok_or_else(|| {
            Error::AccountNotFound(format!("No account found with identifier: {second}"))
        })?;
        if num_a == num_b {
            return Err(Error::Validation(
                "Cannot swap an account with itself".into(),
            ));
        }
        let mut data = self.sequence_or_empty()?;
        let record_a = record(&data, &num_a)
            .cloned()
            .ok_or_else(|| Error::AccountNotFound(format!("Account-{num_a} does not exist")))?;
        let record_b = record(&data, &num_b)
            .cloned()
            .ok_or_else(|| Error::AccountNotFound(format!("Account-{num_b} does not exist")))?;
        let email_a = text(&record_a, "email").to_owned();
        let email_b = text(&record_b, "email").to_owned();

        self.ensure_no_live_session(&num_a, &email_a, "--swap-accounts")?;
        self.ensure_no_live_session(&num_b, &email_b, "--swap-accounts")?;
        let org_a = text(&record_a, "organizationUuid").to_owned();
        let org_b = text(&record_b, "organizationUuid").to_owned();
        self.adopt_before_relocation(&num_a, &email_a, &org_a, "--swap-accounts")?;
        self.adopt_before_relocation(&num_b, &email_b, &org_b, "--swap-accounts")?;

        let mat_a = (
            self.read_backup_or_abort(&num_a, &email_a)?,
            self.read_account_config(&num_a, &email_a)?,
        );
        let mat_b = (
            self.read_backup_or_abort(&num_b, &email_b)?,
            self.read_account_config(&num_b, &email_b)?,
        );

        let slots = SwapSlots {
            num_a: &num_a,
            email_a: &email_a,
            num_b: &num_b,
            email_b: &email_b,
        };
        let mut staging = BTreeMap::new();
        let result = (|| -> Result<()> {
            if email_a == email_b {
                // Fully overlapping backup keys: park durable copies before any write overwrites the
                // other account's material.
                staging = self.stage_overlap_material(&[(&num_a, &mat_a), (&num_b, &mat_b)])?;
            }
            self.swap_session_dirs(&num_a, &email_a, &num_b, &email_b);
            self.write_or_clear(&num_b, &email_a, &mat_a)?;
            self.write_or_clear(&num_a, &email_b, &mat_b)?;

            if let Some(Value::Object(accts)) = data.get_mut("accounts") {
                accts.insert(num_a.clone(), Value::Object(record_b.clone()));
                accts.insert(num_b.clone(), Value::Object(record_a.clone()));
            }
            let (int_a, int_b) = (slot_int(&num_a)?, slot_int(&num_b)?);
            renumber(&mut data, |n| {
                if n == int_a {
                    int_b
                } else if n == int_b {
                    int_a
                } else {
                    n
                }
            });
            data.insert("lastUpdated".into(), Value::String(get_timestamp()));
            // The commit point.
            self.write_json(&self.sequence_file, &data)
        })();
        if let Err(err) = result {
            self.rollback_swap(&slots, &mat_a, &mat_b, &staging);
            return Err(err);
        }

        // Post-commit cleanup, best effort: the records already reference the new keys only.
        if email_a != email_b {
            for (num, email) in [(&num_a, &email_a), (&num_b, &email_b)] {
                if let Err(err) = self.delete_account_files(num, email) {
                    log::error!("Stale backup left under old key {num} ({email}): {err}");
                }
            }
        }
        // The .prev generations retained while writing the destinations hold displaced material.
        if !mat_a.0.is_empty() {
            self.store.delete_previous_backup(&num_b, &email_a);
        }
        if !mat_b.0.is_empty() {
            self.store.delete_previous_backup(&num_a, &email_b);
        }
        self.discard_staging(&staging);
        log::info!("Swapped slots: {num_a} ({email_a}) <-> {num_b} ({email_b})");
        Ok((num_a, num_b))
    }

    /// Set a destination key to its owner's exact state: write material that exists, actively clear
    /// what does not (an empty source must never leave the destination serving leftovers).
    fn write_or_clear(&self, num: &str, email: &str, material: &Material) -> Result<()> {
        let (creds, config) = material;
        if creds.is_empty() {
            self.delete_account_credentials_strict(num, email)?;
        } else {
            self.write_account_credentials(num, email, creds)?;
        }
        if config.is_empty() {
            self.delete_config_backup(num, email)
        } else {
            self.write_account_config(num, email, config)
        }
    }

    /// Remove staged pre-swap copies; a survivor (it holds plaintext credentials) is reported loudly.
    fn discard_staging(&self, staging: &BTreeMap<String, PathBuf>) {
        for path in staging.values() {
            match std::fs::remove_file(path) {
                Ok(()) => {}
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(err) => {
                    log::error!("Could not remove swap staging copy: {err}");
                    printer::warning(&format!(
                        "Could not remove swap staging file {} — it holds pre-swap credentials; please \
                         delete it manually.",
                        path.display()
                    ));
                }
            }
        }
    }

    /// Park slots' backup material in `credentials/.swap-staging-<kind>-<num>.json` before overlapping
    /// writes: 0600, created exclusively, never over an existing file (a leftover from an interrupted
    /// swap may be the only surviving copy, so the swap refuses and points at it). A failure here aborts
    /// before anything was overwritten, removing only what this call created.
    fn stage_overlap_material(
        &self,
        material: &[(&String, &Material)],
    ) -> Result<BTreeMap<String, PathBuf>> {
        let mut staged = BTreeMap::new();
        for (num, (creds, config)) in material {
            for (kind, content) in [("creds", creds), ("config", config)] {
                if content.is_empty() {
                    continue;
                }
                let path = self
                    .credentials_dir
                    .join(format!(".swap-staging-{kind}-{num}.json"));
                match create_private(&path, content) {
                    Ok(()) => {
                        staged.insert(format!("{kind}-{num}"), path);
                    }
                    Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                        self.discard_staging(&staged);
                        return Err(Error::Config(format!(
                            "Found leftover staging from an interrupted swap: {}. It holds that slot's \
                             pre-swap credentials and may be the only surviving copy. Verify both \
                             accounts still work (`{} list`), then delete the file and retry.",
                            path.display(),
                            crate::PROG
                        )));
                    }
                    Err(err) => {
                        // A partially written file is ours: drop it with the rest.
                        staged.insert(format!("{kind}-{num}"), path);
                        self.discard_staging(&staged);
                        return Err(Error::Config(format!(
                            "Could not stage swap material, nothing was changed: {err}"
                        )));
                    }
                }
            }
        }
        Ok(staged)
    }

    /// Exchange two slots' session profile directories, best effort (a skipped move costs at most that
    /// slot's history: the old keys are pruned and a missing profile re-bootstraps from the backup).
    fn swap_session_dirs(&self, num_a: &str, email_a: &str, num_b: &str, email_b: &str) {
        let dir_a = self.session_dir(num_a, email_a);
        let dir_b = self.session_dir(num_b, email_b);
        let new_a = self.session_dir(num_b, email_a);
        let new_b = self.session_dir(num_a, email_b);
        let mut staging: Option<PathBuf> = None;
        let result = (|| -> std::io::Result<()> {
            if dir_a.exists() {
                let tmp = with_suffix(&dir_a, ".swapping");
                std::fs::rename(&dir_a, &tmp)?;
                staging = Some(tmp);
            }
            if dir_b.exists() && !new_b.exists() {
                std::fs::rename(&dir_b, &new_b)?;
            }
            if let Some(tmp) = &staging {
                if !new_a.exists() {
                    std::fs::rename(tmp, &new_a)?;
                    staging = None;
                }
            }
            Ok(())
        })();
        if let Err(err) = result {
            log::warn!("Session profile move skipped during swap: {err}");
        }
        // Never strand a profile under the staging name.
        if let Some(tmp) = staging {
            if !dir_a.exists() {
                let _ = std::fs::rename(&tmp, &dir_a);
            }
        }
    }

    /// Best-effort restore of both slots after a failed swap (before the commit, so the OLD keys go
    /// back). A key whose original was empty goes back to empty when the keys overlap. If any step fails,
    /// the staged pre-swap copies are kept and reported instead of deleted.
    fn rollback_swap(
        &self,
        slots: &SwapSlots<'_>,
        mat_a: &Material,
        mat_b: &Material,
        staging: &BTreeMap<String, PathBuf>,
    ) {
        let SwapSlots {
            num_a,
            email_a,
            num_b,
            email_b,
        } = *slots;
        log::error!("Swap {num_a} <-> {num_b} failed mid-write; restoring both slots");
        let mut failures = 0;
        self.swap_session_dirs(num_b, email_a, num_a, email_b);
        let overlap = email_a == email_b;
        let steps = [
            (true, num_a, email_a, &mat_a.0),
            (false, num_a, email_a, &mat_a.1),
            (true, num_b, email_b, &mat_b.0),
            (false, num_b, email_b, &mat_b.1),
        ];
        for (is_creds, num, email, original) in steps {
            let outcome = match (original.is_empty(), is_creds) {
                (false, true) => self.write_account_credentials(num, email, original),
                (false, false) => self.write_account_config(num, email, original),
                (true, true) if overlap => self.delete_account_credentials_strict(num, email),
                (true, false) if overlap => self.delete_config_backup(num, email),
                (true, _) => Ok(()),
            };
            if let Err(err) = outcome {
                failures += 1;
                let kind = if is_creds { "creds" } else { "config" };
                log::error!("Rollback {kind} restore failed for slot {num}: {err}");
            }
        }
        if !overlap {
            for (num, email) in [(num_b, email_a), (num_a, email_b)] {
                self.delete_account_credentials(num, email);
                if let Err(err) = self.delete_config_backup(num, email) {
                    failures += 1;
                    log::error!("Rollback cleanup failed for slot {num}: {err}");
                }
            }
        }
        if failures == 0 {
            for (num, email, original) in [(num_a, email_a, &mat_a.0), (num_b, email_b, &mat_b.0)] {
                if !original.is_empty() {
                    self.store.delete_previous_backup(num, email);
                }
            }
        }
        if staging.is_empty() {
            return;
        }
        if failures == 0 {
            self.discard_staging(staging);
            return;
        }
        let kept: Vec<String> = staging.values().map(|p| p.display().to_string()).collect();
        let kept = kept.join(", ");
        log::error!(
            "Rollback incomplete — staged pre-swap copies kept for manual recovery: {kept}"
        );
        printer::warning(&format!(
            "Swap rollback was incomplete; your pre-swap credentials are preserved in: {kept}"
        ));
    }

    /// Assign `account` (NUM|EMAIL|ALIAS) to slot number `target`. Returns `(source, target, swapped)`:
    /// the same slot is a no-op, an empty slot relocates the account, an occupied one swaps the two.
    pub fn move_account(&self, account: &str, target: &str) -> Result<(String, String, bool)> {
        self.refuse_session_shell()?;
        if !self.sequence_file.exists() {
            return Err(Error::Config("No accounts are managed yet".into()));
        }
        let trimmed = target.trim();
        let parsed = is_digits(trimmed)
            .then(|| match trimmed.trim_start_matches('0') {
                "" => 0,
                digits => digits.parse::<i64>().unwrap_or(i64::MAX),
            })
            .filter(|n| *n >= 1);
        let Some(target_int) = parsed else {
            return Err(Error::Validation(format!(
                "Target slot must be a positive slot number, got: {} (use `swap` to trade two \
                 accounts by identifier)",
                crate::models::pyfmt::repr_str(trimmed)
            )));
        };
        let target = target_int.to_string();

        let _lock = self.lock()?;
        self.get_sequence_data_migrated()?;
        let num_src = self.resolve_account_identifier(account)?.ok_or_else(|| {
            Error::AccountNotFound(format!("No account found with identifier: {account}"))
        })?;
        let data = self.sequence_or_empty()?;
        if record(&data, &num_src).is_none() {
            return Err(Error::AccountNotFound(format!(
                "Account-{num_src} does not exist"
            )));
        }
        let max_slot = accounts(&data)
            .map(|a| {
                a.keys()
                    .filter(|k| is_digits(k))
                    .filter_map(|k| k.parse::<i64>().ok())
                    .max()
                    .unwrap_or(0)
            })
            .unwrap_or(0);
        let cap = MOVE_TARGET_CAP.max(max_slot);
        if target_int > cap {
            return Err(Error::Validation(format!(
                "Target slot {target} is out of range (1-{cap}): new accounts are numbered from the \
                 highest slot, so a large target would inflate future account numbers"
            )));
        }
        if num_src == target {
            return Ok((num_src, target, false));
        }
        if record(&data, &target).is_some() {
            self.swap_accounts_locked(&num_src, &target)?;
            return Ok((num_src, target, true));
        }
        self.relocate_locked(&num_src, &target)?;
        Ok((num_src, target, false))
    }

    /// Move one account from `num_src` to the EMPTY slot `target`; the caller holds the account lock.
    /// Before the commit the old keys are untouched (strays under the target are cleaned on failure);
    /// after it only best-effort cleanup of the old keys remains.
    fn relocate_locked(&self, num_src: &str, target: &str) -> Result<()> {
        let mut data = self.sequence_or_empty()?;
        let rec = record(&data, num_src)
            .cloned()
            .ok_or_else(|| Error::AccountNotFound(format!("Account-{num_src} does not exist")))?;
        if record(&data, target).is_some() {
            return Err(Error::Validation(format!(
                "Slot {target} is already occupied — retry the move"
            )));
        }
        let email = text(&rec, "email").to_owned();
        self.ensure_no_live_session(num_src, &email, "--move-account")?;
        let org_uuid = text(&rec, "organizationUuid").to_owned();
        self.adopt_before_relocation(num_src, &email, &org_uuid, "--move-account")?;
        let material = (
            self.read_backup_or_abort(num_src, &email)?,
            self.read_account_config(num_src, &email)?,
        );

        let src_dir = self.session_dir(num_src, &email);
        let dst_dir = self.session_dir(target, &email);
        let result = (|| -> Result<()> {
            if src_dir.exists() && !dst_dir.exists() {
                if let Err(err) = std::fs::rename(&src_dir, &dst_dir) {
                    log::warn!("Session profile move skipped during move: {err}");
                }
            }
            self.write_or_clear(target, &email, &material)?;
            if let Some(Value::Object(accts)) = data.get_mut("accounts") {
                accts.insert(target.to_owned(), Value::Object(rec.clone()));
                accts.shift_remove(num_src);
            }
            let (int_src, int_target) = (slot_int(num_src)?, slot_int(target)?);
            renumber(&mut data, |n| if n == int_src { int_target } else { n });
            data.insert("lastUpdated".into(), Value::String(get_timestamp()));
            self.write_json(&self.sequence_file, &data)
        })();
        if let Err(err) = result {
            self.delete_account_credentials(target, &email);
            let cleanup = self.delete_config_backup(target, &email).and_then(|()| {
                if dst_dir.exists() && !src_dir.exists() {
                    std::fs::rename(&dst_dir, &src_dir)?;
                }
                Ok(())
            });
            if let Err(cleanup_err) = cleanup {
                log::error!("Cleanup after failed move incomplete: {cleanup_err}");
            }
            return Err(err);
        }

        if let Err(err) = self.delete_account_files(num_src, &email) {
            log::error!("Stale backup left under old key {num_src} ({email}): {err}");
        }
        if !material.0.is_empty() {
            self.store.delete_previous_backup(target, &email);
        }
        log::info!("Moved slot: {num_src} ({email}) -> {target}");
        Ok(())
    }
}

/// The two sides of a swap.
#[derive(Clone, Copy)]
struct SwapSlots<'a> {
    num_a: &'a str,
    email_a: &'a str,
    num_b: &'a str,
    email_b: &'a str,
}

fn slot_int(num: &str) -> Result<i64> {
    num.parse()
        .map_err(|_| Error::General(format!("invalid literal for int() with base 10: '{num}'")))
}

/// Renumber `sequence` and `activeAccountNumber` through `map`, then sort `sequence` (kept sorted
/// everywhere, so rotation and list order follow the new numbers).
fn renumber(data: &mut JsonObject, map: impl Fn(i64) -> i64) {
    let mut seq: Vec<i64> = sequence_numbers(data)
        .iter()
        .filter_map(|n| n.parse::<i64>().ok())
        .map(&map)
        .collect();
    seq.sort_unstable();
    data.insert(
        "sequence".into(),
        Value::Array(seq.into_iter().map(Value::from).collect()),
    );
    if let Some(active) = data.get("activeAccountNumber").and_then(Value::as_i64) {
        data.insert("activeAccountNumber".into(), Value::from(map(active)));
    }
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(suffix);
    path.with_file_name(name)
}

/// Create `path` exclusively with mode 0600 and write `content`.
fn create_private(path: &Path, content: &str) -> std::io::Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(content.as_bytes())
}

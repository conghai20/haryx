//! Session-profile lifecycle as the switcher sees it (upstream `switcher.py` ~3216-3520): where a slot's
//! profile lives, whether a claude runs against it, invalidating it when the backup moves, adopting its
//! newer credential into the backup, and removing it with its slot.

use std::path::PathBuf;

use serde_json::{Map, Value};

use super::Switcher;
use crate::error::{Error, Result};
use crate::oauth;
use crate::session;

impl Switcher {
    /// The session profile directory of a slot key.
    pub fn session_dir(&self, account_num: &str, email: &str) -> PathBuf {
        session::session_dir_for(&self.backup_dir, account_num, email)
    }

    /// PIDs of Claude instances running against a slot's session profile. Scan-shaped: an unreadable
    /// record contributes no PID; destructive guards use [`Self::ensure_no_live_session`].
    pub fn live_session_pids(&self, account_num: &str, email: &str) -> Vec<i32> {
        let (sessions, _) = session::scan_live_sessions(&self.session_dir(account_num, email));
        sessions.into_iter().map(|s| s.pid).collect()
    }

    /// Public name of [`Self::live_session_pids`] (upstream `live_session_pids_for`).
    pub fn live_session_pids_for(&self, account_num: &str, email: &str) -> Vec<i32> {
        self.live_session_pids(account_num, email)
    }

    /// Refuse a destructive operation while a session-mode claude is live on the slot, and also when a
    /// session record could not be read (not knowing is not knowing nothing runs).
    pub fn ensure_no_live_session(
        &self,
        account_num: &str,
        email: &str,
        action: &str,
    ) -> Result<()> {
        let pids = self.live_session_pids(account_num, email);
        if !pids.is_empty() {
            let list: Vec<String> = pids.iter().map(ToString::to_string).collect();
            return Err(Error::Session(format!(
                "Account-{account_num} ({email}) has a live session-mode Claude instance (PID {}). Exit \
                 it first, then retry {action}.",
                list.join(", ")
            )));
        }
        let session_dir = self.session_dir(account_num, email);
        let (_, unreadable) = session::scan_live_sessions(&session_dir);
        if unreadable > 0 {
            return Err(Error::Session(format!(
                "Account-{account_num} ({email}) has {unreadable} session record(s) that could not be \
                 read, so whether a Claude instance is live cannot be determined. Inspect {} and remove \
                 or repair them, then retry {action}.",
                session_dir.join("sessions").display()
            )));
        }
        Ok(())
    }

    /// Drop a session profile's credential material, keeping its history: the next `run` re-bootstraps
    /// from the backup.
    pub fn invalidate_session_credentials(
        &self,
        account_num: &str,
        email: &str,
    ) -> std::io::Result<()> {
        let session_dir = self.session_dir(account_num, email);
        if !session_dir.exists() {
            return Ok(());
        }
        self.profile_keychain().delete_entry(&session_dir);
        match std::fs::remove_file(session_dir.join(".credentials.json")) {
            Ok(()) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(err),
        }
        session::clear_session_stale(&session_dir);
        log::info!("Invalidated session credentials for account {account_num}");
        Ok(())
    }

    /// The profile's credential when it is a NEWER generation of this slot's family than the backup.
    ///
    /// Claude rotates the family inside a profile and the backup never follows, so after a session that
    /// refreshed the backup holds a consumed generation. Generations differ by fingerprint and are ordered
    /// by `expiresAt` (every refresh and login issues a later one). `None` for a profile re-pointed at
    /// another account, a stale-marked profile, and anything unreadable.
    pub fn session_profile_ahead(
        &self,
        account_num: &str,
        email: &str,
        org_uuid: &str,
    ) -> Option<String> {
        let session_dir = self.session_dir(account_num, email);
        if session::is_session_stale(&session_dir) {
            return None;
        }
        let profile = session::read_session_credentials(&self.profile_keychain(), &session_dir)
            .filter(|p| !p.is_empty())?;
        if session::session_identity_drifted(&session_dir, email, org_uuid) {
            return None;
        }
        let (backup, unreadable) = self.read_account_credentials_ex(account_num, email, false);
        if unreadable {
            return None;
        }
        if oauth::credential_fingerprint(&profile) == oauth::credential_fingerprint(&backup) {
            return None;
        }
        let issued = expires_at(oauth::extract_oauth_data(&profile).as_ref())?;
        let stored = expires_at(oauth::extract_oauth_data(&backup).as_ref())?;
        (issued > stored).then_some(profile)
    }

    /// Capture a quiescent profile's newer credential into the slot backup, under the account lock.
    ///
    /// `Some(true)`: advanced. `Some(false)`: the profile is not ahead, the backup is the truth.
    /// `None`: undecidable, nothing written (a claude is live, a record is unreadable, or the profile's
    /// identity cannot be read; this is a WRITE, so an unreadable identity is not trusted).
    pub fn adopt_session_credential(
        &self,
        account_num: &str,
        email: &str,
        org_uuid: &str,
    ) -> Result<Option<bool>> {
        let _lock = self.lock()?;
        self.adopt_session_credential_locked(account_num, email, org_uuid)
    }

    /// Body of [`Self::adopt_session_credential`]; the caller holds the account lock. The store write is
    /// the pure one: invalidating would drop the very profile just captured.
    pub(crate) fn adopt_session_credential_locked(
        &self,
        account_num: &str,
        email: &str,
        org_uuid: &str,
    ) -> Result<Option<bool>> {
        let session_dir = self.session_dir(account_num, email);
        if !session::profile_is_quiescent(&session_dir) {
            return Ok(None);
        }
        let Some(profile) = self.session_profile_ahead(account_num, email, org_uuid) else {
            return Ok(Some(false));
        };
        if session::read_session_identity(&session_dir).is_none() {
            return Ok(None);
        }
        self.store
            .write_account_credentials(account_num, email, &profile)?;
        log::info!("Adopted account {account_num}'s session profile credential into its backup");
        Ok(Some(true))
    }

    /// Advance a slot's backup to its profile's generation before swap/move relocates the backup (the
    /// profile's hashed Keychain item cannot follow and is deleted with the old key). Refuses only when
    /// adoption is undecidable while the profile is provably ahead. The caller holds the account lock and
    /// has passed [`Self::ensure_no_live_session`].
    pub(crate) fn adopt_before_relocation(
        &self,
        account_num: &str,
        email: &str,
        org_uuid: &str,
        action: &str,
    ) -> Result<()> {
        let adopted = self.adopt_session_credential_locked(account_num, email, org_uuid)?;
        if adopted.is_none()
            && self
                .session_profile_ahead(account_num, email, org_uuid)
                .is_some()
        {
            return Err(Error::Session(format!(
                "Account-{account_num} ({email}) has a session profile whose credential has rotated past \
                 the stored backup, and it could not be adopted into the backup: the profile's identity \
                 file (.claude.json) could not be read, or a Claude instance started against it just \
                 now. Relocating the backup would carry a consumed generation and lose the fresh one. \
                 Re-login inside the profile ('{} run {account_num}', then /login) so its identity is \
                 rewritten, then retry {action}.",
                crate::PROG
            )));
        }
        Ok(())
    }

    /// Remove a slot's session profile, its Keychain entry and its stale marker.
    ///
    /// The Keychain entry first and unconditionally (its name hashes the PATH, which survives a relocated
    /// or missing dir); the marker is a sibling, so it is cleared even when the dir is already gone. A
    /// removal that did not fully happen is logged as such, never as success.
    pub fn delete_session_profile(&self, account_num: &str, email: &str) {
        let session_dir = self.session_dir(account_num, email);
        self.profile_keychain().delete_entry(&session_dir);
        if session_dir.exists() {
            let _ = std::fs::remove_dir_all(&session_dir);
        }
        let cleared = session::clear_session_stale(&session_dir);
        if session_dir.exists() || !cleared {
            log::warn!(
                "Could not fully remove account {account_num}'s session profile at {}; credentials are \
                 gone but the profile dir and/or its stale marker survive (check permissions on it and \
                 its parent).",
                session_dir.display()
            );
            return;
        }
        log::info!(
            "Removed session profile for account {account_num} at {}",
            session_dir.display()
        );
    }
}

/// `float(oauth.get("expiresAt") or 0)`: `None` where Python raises (a non-numeric string, a container).
fn expires_at(oauth_data: Option<&Map<String, Value>>) -> Option<f64> {
    match oauth_data.and_then(|o| o.get("expiresAt")) {
        None | Some(Value::Null) => Some(0.0),
        Some(Value::Bool(b)) => Some(if *b { 1.0 } else { 0.0 }),
        Some(Value::Number(n)) => n.as_f64(),
        Some(Value::String(s)) if s.is_empty() => Some(0.0),
        Some(Value::String(s)) => s.trim().parse().ok(),
        Some(Value::Array(a)) if a.is_empty() => Some(0.0),
        Some(Value::Object(o)) if o.is_empty() => Some(0.0),
        Some(_) => None,
    }
}

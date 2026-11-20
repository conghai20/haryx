//! The backup refresh gate: upstream `consume_backup_grant`, `_consume_backup_grant_locked`,
//! `_live_login_holds`, `_retire_stash_entry`, `_adopt_stashed_successor`.
//!
//! A refresh token is one-time-use, so the POST must consume the provably freshest copy of the slot's
//! grant, never a caller's snapshot. The sequence runs under a per-slot consume lock (re-read → POST →
//! CAS) so two consumers never POST the same grant; the slot lock itself never covers the network call.
//! A consumed generation is never discarded (a stash is adopted by the next pass), and the gate never
//! fails outward after the grant is consumed: callers run in the never-fails collect pass.

use log::{debug, error, info, warn};
use serde_json::{Map, Value, json};

use super::Switcher;
use crate::credentials::ManifestVerdict;
use crate::error::{Error, Result};
use crate::locking::{DEFAULT_LOCK_TIMEOUT, FileLock};
use crate::oauth::{self, REFRESH_TIMEOUT, RefreshOutcome};
use crate::session;

/// Stash reasons that demote a successful refresh to `transient`: the successor is parked, not
/// persisted, so the slot still holds the consumed generation.
const DEMOTING_STASH_REASONS: [&str; 4] = [
    "consume-gate-persist-failed",
    "consume-gate-persist-lock-failed",
    "consume-gate-unpersisted",
    "consume-gate-store-unreadable",
];

fn outcome(error: &str) -> RefreshOutcome {
    RefreshOutcome {
        error: Some(error.to_owned()),
        ..Default::default()
    }
}

/// What the locked pre-consume window decided.
enum Prepared {
    /// Stop with this outcome; nothing was consumed.
    Done(RefreshOutcome),
    /// POST these bytes, whose fingerprint is `consumed_fp`.
    Consume {
        refresh_input: String,
        consumed_fp: Option<String>,
    },
}

impl Switcher {
    /// The gate through which a backup refresh token is consumed. The caller must NOT hold the account
    /// lock (it is not reentrant).
    pub fn consume_backup_grant(
        &self,
        account_num: &str,
        email: &str,
        snapshot: &str,
    ) -> RefreshOutcome {
        // Claude Code honours CLAUDE_SECURESTORAGE_CONFIG_DIR; the consume path resolves the default store,
        // so consuming here would operate on a store Claude Code left behind. Refuse with its own kind.
        if self
            .env
            .secure_storage_dir
            .as_ref()
            .is_some_and(|dir| !dir.is_empty())
        {
            warn!(
                "CLAUDE_SECURESTORAGE_CONFIG_DIR is set; haryx mirrors it when capturing a credential \
                 but not when consuming one, so refusing to consume account {account_num}'s refresh \
                 token (unset the variable or run from a normal shell)."
            );
            return outcome("store-unmirrored");
        }
        let lock_path = self
            .credentials_dir
            .join(format!(".consume-{account_num}.lock"));
        let mut consume_lock = FileLock::new(lock_path, DEFAULT_LOCK_TIMEOUT);
        match consume_lock.acquire(None) {
            Ok(true) => {}
            Ok(false) | Err(_) => {
                info!(
                    "Another consume is in flight for account {account_num}; deferring to the next pass."
                );
                return outcome("consume-busy");
            }
        }
        self.consume_backup_grant_locked(account_num, email, snapshot)
    }

    fn consume_backup_grant_locked(
        &self,
        account_num: &str,
        email: &str,
        snapshot: &str,
    ) -> RefreshOutcome {
        let (refresh_input, consumed_fp) = match self.prepare_consume(account_num, email) {
            Ok(Prepared::Done(result)) => return result,
            Ok(Prepared::Consume {
                refresh_input,
                consumed_fp,
            }) => (refresh_input, consumed_fp),
            Err(Error::Lock(_)) => {
                info!(
                    "Slot lock held elsewhere; deferring account {account_num}'s backup refresh to the next pass."
                );
                return outcome("transient");
            }
            Err(err) => {
                // No POST was issued, so no grant of ours is outstanding: defer.
                warn!("Pre-consume window failed for account {account_num}; deferring: {err}");
                return outcome("transient");
            }
        };

        let input_oauth = oauth::extract_oauth_data(&refresh_input).unwrap_or_default();
        let input_at = input_oauth.get("accessToken").and_then(Value::as_str);
        let snap_at = oauth::extract_oauth_data(snapshot).and_then(|o| {
            o.get("accessToken")
                .and_then(Value::as_str)
                .map(str::to_owned)
        });
        if let (Some(input_at), Some(snap_at)) = (input_at.filter(|s| !s.is_empty()), snap_at)
            && !snap_at.is_empty()
            && input_at != snap_at
            && !oauth::is_oauth_token_expired(input_oauth.get("expiresAt"))
        {
            // The world already moved past the snapshot and the current generation is fresh: adopt it.
            return RefreshOutcome {
                credentials: Some(refresh_input),
                consumed_fp,
                ..Default::default()
            };
        }

        match self.live_login_holds(&refresh_input) {
            None => {
                info!(
                    "Live credential unreadable; deferring account {account_num}'s backup refresh to the next pass."
                );
                return outcome("transient");
            }
            Some(true) => {
                info!(
                    "Account {account_num}'s stored login is the live login; leaving its renewal to Claude Code."
                );
                return RefreshOutcome {
                    error: Some("live-login".into()),
                    consumed_fp,
                    ..Default::default()
                };
            }
            Some(false) => {}
        }

        let result = self
            .api
            .try_refresh_oauth_credentials(&refresh_input, REFRESH_TIMEOUT);
        let successor = match (&result.error, &result.credentials) {
            (None, Some(creds)) if !creds.is_empty() => creds.clone(),
            _ => {
                return RefreshOutcome {
                    consumed_fp,
                    ..result
                };
            }
        };
        let (outcome_creds, stashed_reason) =
            self.persist_successor(account_num, email, &successor, consumed_fp.as_deref());
        if DEMOTING_STASH_REASONS.contains(&stashed_reason.as_str()) {
            return RefreshOutcome {
                credentials: Some(outcome_creds),
                error: Some("transient".into()),
                token_account: result.token_account,
                consumed_fp,
                stashed: stashed_reason != "consume-gate-unpersisted",
            };
        }
        RefreshOutcome {
            credentials: Some(outcome_creds),
            error: None,
            token_account: result.token_account,
            consumed_fp,
            stashed: false,
        }
    }

    /// The locked pre-consume window: re-read the slot, adopt a stashed successor, prefer a newer session
    /// profile generation. Writes may happen here (adoption, resync) but nothing is POSTed.
    fn prepare_consume(&self, account_num: &str, email: &str) -> Result<Prepared> {
        let _slot = FileLock::lock(&self.lock_file, DEFAULT_LOCK_TIMEOUT)?;
        let (mut current, unreadable) = self.read_account_credentials_ex(account_num, email, false);
        if unreadable {
            info!("Backup for account {account_num} unreadable (keychain); deferring the refresh.");
            return Ok(Prepared::Done(outcome("transient")));
        }
        match self.adopt_stashed_successor(account_num, email, &current) {
            Ok(Some(adopted)) => current = adopted,
            Ok(None) => {}
            Err(Error::CredentialRead(reason)) => {
                info!(
                    "Account {account_num}'s stashed successor is unreadable; deferring the refresh: {reason}"
                );
                return Ok(Prepared::Done(outcome("stash-unreadable")));
            }
            Err(err) => return Err(err),
        }
        if current.is_empty() {
            // Absent (not unreadable): the slot was removed since the caller's read. Do not spend a grant
            // for a slot that no longer exists.
            info!(
                "Account {account_num}'s stored credential is gone; deferring the refresh rather than \
                 consuming a grant for a slot that no longer exists."
            );
            return Ok(Prepared::Done(outcome("transient")));
        }
        let refresh_input = self.prefer_session_generation(account_num, email, current)?;
        let consumed_fp = oauth::credential_fingerprint(&refresh_input);
        Ok(Prepared::Consume {
            refresh_input,
            consumed_fp,
        })
    }

    /// Session-profile precedence: when no live session owns the profile and its identity still matches
    /// the slot, a profile holding a newer generation means the backup's refresh token is already
    /// consumed. Resync the slot to the profile and consume THAT.
    fn prefer_session_generation(
        &self,
        account_num: &str,
        email: &str,
        current: String,
    ) -> Result<String> {
        let org_uuid = self
            .get_sequence_data()?
            .and_then(|data| {
                data.get("accounts")?
                    .get(account_num)?
                    .get("organizationUuid")?
                    .as_str()
                    .map(str::to_owned)
            })
            .unwrap_or_default();
        if !self.live_session_pids(account_num, email).is_empty() {
            return Ok(current);
        }
        let sdir = session::session_dir_for(&self.backup_dir, account_num, email);
        let Some(profile) = session::read_session_credentials(&self.profile_keychain(), &sdir)
        else {
            return Ok(current);
        };
        if profile.is_empty()
            || session::is_session_stale(&sdir)
            || session::session_identity_drifted(&sdir, email, &org_uuid)
        {
            return Ok(current);
        }
        let expires = |creds: &str| {
            oauth::extract_oauth_data(creds)
                .and_then(|o| o.get("expiresAt").and_then(Value::as_f64))
                .unwrap_or(0.0)
        };
        let Some(prof_oauth) = oauth::extract_oauth_data(&profile) else {
            return Ok(current);
        };
        let has = |key: &str| {
            prof_oauth
                .get(key)
                .and_then(Value::as_str)
                .is_some_and(|s| !s.is_empty())
        };
        if has("accessToken")
            && has("refreshToken")
            && oauth::credential_fingerprint(&profile) != oauth::credential_fingerprint(&current)
            && expires(&profile) > expires(&current)
        {
            self.write_account_credentials(account_num, email, &profile)?;
            return Ok(profile);
        }
        Ok(current)
    }

    /// Persist a consumed successor under the slot lock with a CAS on the consumed fingerprint, or stash
    /// it. Returns the slot's now-current credential and the stash reason ("" when persisted).
    pub(crate) fn persist_successor(
        &self,
        account_num: &str,
        email: &str,
        successor: &str,
        consumed_fp: Option<&str>,
    ) -> (String, String) {
        let stash = |reason: &str, note: &str| -> Result<String> {
            let mut meta = Map::new();
            meta.insert("reason".into(), reason.into());
            meta.insert("configSlot".into(), account_num.into());
            meta.insert("consumedFp".into(), json!(consumed_fp));
            meta.insert(
                "fingerprint".into(),
                json!(oauth::credential_fingerprint(successor)),
            );
            self.store.write_unclaimed_credential(successor, &meta)?;
            warn!("{}", note.replace("{}", account_num));
            Ok(reason.to_owned())
        };
        let attempt = || -> Result<(String, String)> {
            let Ok(_slot) = FileLock::lock(&self.lock_file, DEFAULT_LOCK_TIMEOUT) else {
                let reason = stash(
                    "consume-gate-persist-lock-failed",
                    "Slot lock unavailable after consuming account {}'s grant; successor stashed for the next pass.",
                )?;
                return Ok((successor.to_owned(), reason));
            };
            let (store_now, store_unreadable) =
                self.read_account_credentials_ex(account_num, email, false);
            if store_unreadable {
                let reason = stash(
                    "consume-gate-store-unreadable",
                    "Account {}'s stored credential was unreadable (keychain) after a refresh POST; successor stashed, nothing rewritten.",
                )?;
                return Ok((successor.to_owned(), reason));
            }
            if store_now.is_empty() {
                let reason = stash(
                    "consume-gate-slot-removed",
                    "Account {}'s stored credential disappeared during a refresh POST; successor stashed, nothing rewritten.",
                )?;
                return Ok((successor.to_owned(), reason));
            }
            if oauth::credential_fingerprint(&store_now).as_deref() != consumed_fp {
                let reason = stash(
                    "consume-gate-cas-conflict",
                    "Backup lineage for account {} moved during a refresh POST; successor stashed, adopting the newer store credential.",
                )?;
                return Ok((store_now, reason));
            }
            self.write_account_credentials(account_num, email, successor)?;
            Ok((successor.to_owned(), String::new()))
        };
        match attempt() {
            Ok(done) => done,
            Err(err) => {
                warn!(
                    "Persisting account {account_num}'s refreshed credential failed; stashing instead: {err}"
                );
                match stash(
                    "consume-gate-persist-failed",
                    "Persist failed after consuming account {}'s grant; successor stashed for the next pass.",
                ) {
                    Ok(reason) => (successor.to_owned(), reason),
                    Err(err) => {
                        error!(
                            "Account {account_num}'s consumed successor could not be persisted or stashed — it \
                             survives only for this pass. Fix the storage failure, then re-login and `haryx add` \
                             if the slot strikes: {err}"
                        );
                        (successor.to_owned(), "consume-gate-unpersisted".to_owned())
                    }
                }
            }
        }
    }

    /// Whether the live store holds the refresh token in `credentials`: `Some(true)` on an exact match,
    /// `Some(false)` when it holds another or none (or `credentials` carry no refresh token), `None` when
    /// the live store could not be read for certain.
    pub fn live_login_holds(&self, credentials: &str) -> Option<bool> {
        let has_rt = oauth::extract_oauth_data(credentials)
            .and_then(|o| {
                o.get("refreshToken")
                    .and_then(Value::as_str)
                    .map(|s| !s.is_empty())
            })
            .unwrap_or(false);
        if !has_rt {
            return Some(false);
        }
        let live = self.read_active_credentials();
        let value = live.value.as_deref()?;
        if live.keychain_unavailable || live.degraded {
            debug!("live credential read degraded; cannot tell who holds the grant");
            return None;
        }
        Some(
            !value.is_empty()
                && oauth::credential_fingerprint(value)
                    == oauth::credential_fingerprint(credentials),
        )
    }

    /// Drop one stash entry as housekeeping; never fatal (losing a retire leaves one stale row, losing an
    /// adoption would discard a live credential).
    fn retire_stash_entry(&self, entry_id: &str, account_num: &str) {
        if let Err(err) = self.store.remove_unclaimed_credential(entry_id) {
            warn!(
                "Could not retire account {account_num}'s stash entry {entry_id}; leaving it for the next \
                 pass (`haryx unclaimed --purge` drops it by hand): {err}"
            );
        }
    }

    /// Complete a prior gate's failed persist from the unclaimed stash: when the slot still stores exactly
    /// the generation an entry superseded (`consumedFp`), write the entry back and drop it. Caller holds the
    /// account lock. `Err(CredentialRead)` defers the refresh rather than discard a generation.
    pub(crate) fn adopt_stashed_successor(
        &self,
        account_num: &str,
        email: &str,
        current: &str,
    ) -> Result<Option<String>> {
        let Some(cur_fp) = oauth::credential_fingerprint(current) else {
            return Ok(None);
        };
        let (manifest, verdict) = self.store.read_stash_manifest_ex();
        let blocked = match verdict {
            ManifestVerdict::Unreadable => Some("unreadable"),
            ManifestVerdict::Corrupt if self.store.stash_entry_files_exist() => Some("corrupt"),
            _ => None,
        };
        if let Some(state) = blocked {
            return Err(Error::CredentialRead(format!(
                "the unclaimed manifest is {state} and stashed entry files exist; deferring account \
                 {account_num}'s adoption rather than POSTing a generation a stashed successor may already \
                 have superseded (`haryx unclaimed` lists them, `--purge` drops one)"
            )));
        }
        let mut deferred: Option<String> = None;
        for (entry_id, meta) in &manifest {
            let field = |key: &str| meta.get(key).and_then(Value::as_str);
            if field("configSlot") != Some(account_num) {
                continue;
            }
            if field("consumedFp") != Some(cur_fp.as_str()) {
                if field("reason") == Some("consume-gate-cas-conflict") {
                    self.retire_stash_entry(entry_id, account_num);
                    info!(
                        "Retired account {account_num}'s CAS-conflict stash entry: its generation was \
                         superseded by the writer that won the race, so no pass can ever adopt it."
                    );
                } else {
                    let (bytes, unreadable) = self.store.read_unclaimed_credential(entry_id);
                    if bytes.is_empty() && !unreadable {
                        self.retire_stash_entry(entry_id, account_num);
                        info!(
                            "Retired account {account_num}'s byte-less stash entry: its credential is gone \
                             and its generation has passed, so no pass could ever adopt it."
                        );
                    }
                }
                continue;
            }
            let (creds, unreadable) = self.store.read_unclaimed_credential(entry_id);
            if unreadable {
                deferred.get_or_insert_with(|| entry_id.clone());
                continue;
            }
            if creds.is_empty() {
                self.retire_stash_entry(entry_id, account_num);
                info!(
                    "Retired account {account_num}'s unreadable-bytes stash entry: its generation is gone, \
                     so no pass could ever adopt it."
                );
                continue;
            }
            self.write_account_credentials(account_num, email, &creds)?;
            self.retire_stash_entry(entry_id, account_num);
            info!(
                "Adopted account {account_num}'s stashed successor ({}): the stored generation was already \
                 consumed by the gate pass that stashed it.",
                field("reason").unwrap_or("unknown")
            );
            return Ok(Some(creds));
        }
        match deferred {
            Some(entry_id) => Err(Error::CredentialRead(format!(
                "stash entry {entry_id} for account {account_num} is unreadable; deferring adoption rather \
                 than discarding its generation"
            ))),
            None => Ok(None),
        }
    }
}

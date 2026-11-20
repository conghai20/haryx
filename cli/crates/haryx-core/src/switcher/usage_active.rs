//! The active account's usage fetch: upstream `_fetch_active_usage` and `_resync_rotated_backup`.
//!
//! The active account's expired token is refreshed here, inside Claude Code's own credential lock, so a
//! running Claude Code adopts the new token instead of colliding with it. This is the one deliberate
//! network call under a lock. A refresh token is never consumed unless the live credential is provably
//! this slot's (attributable to its backup, or verified by the profile oracle).

use std::time::Duration;

use log::{debug, info, warn};
use serde_json::{Map, Value};

use super::Switcher;
use crate::claude_locks::{claude_config_lock, claude_credentials_lock};
use crate::error::{Error, Result};
use crate::json_output::{
    USAGE_FOREIGN_CREDENTIAL, USAGE_KEYCHAIN_UNAVAILABLE, USAGE_NO_CREDENTIALS, USAGE_TOKEN_EXPIRED,
};
use crate::locking::{DEFAULT_LOCK_TIMEOUT, FileLock};
use crate::oauth;
use crate::usage_store::FetchRecord;

/// The active refresh's POST budget (shorter than the gate's: it runs under Claude Code's lock).
const ACTIVE_REFRESH_TIMEOUT: Duration = Duration::from_secs(6);

fn text<'a>(o: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    o.get(key).and_then(Value::as_str).filter(|s| !s.is_empty())
}

fn expires(o: Option<&Map<String, Value>>) -> f64 {
    o.and_then(|o| o.get("expiresAt"))
        .and_then(Value::as_f64)
        .unwrap_or(0.0)
}

/// What the locked recovery decided.
enum Recovery {
    /// Stop with this record.
    Done(FetchRecord),
    /// Fetch usage with this credential.
    Use(String),
}

impl Switcher {
    /// One usage fetch for the active account.
    pub(crate) fn fetch_active_usage(
        &self,
        num: &str,
        email: &str,
        creds: &str,
        org_uuid: &str,
        last_usage: Option<&Value>,
    ) -> FetchRecord {
        let Some(oauth_data) =
            oauth::extract_oauth_data(creds).filter(|o| text(o, "accessToken").is_some())
        else {
            return FetchRecord::sentinel(USAGE_NO_CREDENTIALS);
        };
        let defer = |record: Option<FetchRecord>| {
            record.unwrap_or_else(|| FetchRecord::sentinel(USAGE_TOKEN_EXPIRED))
        };

        let mut force_refresh: Option<FetchRecord> = None;
        if !oauth::is_oauth_token_expired(oauth_data.get("expiresAt")) {
            let req = self.usage_request(num, email, creds, true, last_usage);
            let outcome = self.api.try_fetch_usage_for_account(&req);
            if outcome.error.as_deref() != Some("http-401") {
                if outcome.usage.is_some() {
                    if !self.active_verdict().degraded {
                        self.resync_rotated_backup(num, email, org_uuid, creds);
                    }
                    let foreign = self
                        .lineage_of(num, email, creds)
                        .and_then(|key| self.probe_verdict(&key))
                        == Some(false);
                    if foreign {
                        return FetchRecord::sentinel(USAGE_FOREIGN_CREDENTIAL);
                    }
                }
                return FetchRecord {
                    struck_fp: None,
                    ..Self::fetch_record(outcome)
                };
            }
            force_refresh = Some(FetchRecord::failure(
                outcome.error.unwrap_or_default(),
                outcome.retry_after_s,
            ));
        }

        if self.active_verdict().degraded {
            return defer(
                force_refresh.or_else(|| Some(FetchRecord::sentinel(USAGE_KEYCHAIN_UNAVAILABLE))),
            );
        }
        if self
            .env
            .secure_storage_dir
            .as_ref()
            .is_some_and(|d| !d.is_empty())
        {
            warn!(
                "CLAUDE_SECURESTORAGE_CONFIG_DIR is set; haryx mirrors it when capturing a credential but \
                 not when refreshing one, so refusing to refresh account {num}'s active credential (unset \
                 the variable or run from a normal shell)."
            );
            return FetchRecord::failure("store-unmirrored", None);
        }

        let backup = self.read_account_credentials(num, email, false);
        let backup_fp = oauth::credential_fingerprint(&backup);
        let backup_oauth = oauth::extract_oauth_data(&backup);
        let backup_usable = backup_oauth
            .as_ref()
            .is_some_and(|o| text(o, "accessToken").is_some() && text(o, "refreshToken").is_some());
        let attributable = creds == backup || oauth::credential_fingerprint(creds) == backup_fp;
        if !attributable && !backup_usable {
            if self.warn_once(num, email, "unattributable") {
                warn!(
                    "Active credential does not match Account-{num}'s stored backup and the backup is \
                     unusable; cannot refresh (provenance unknown)."
                );
            }
            return defer(force_refresh);
        }
        self.clear_warned(num, email, "unattributable");

        let ctx = ActiveCtx {
            num,
            email,
            org_uuid,
            creds,
            backup: &backup,
            backup_fp: backup_fp.as_deref(),
            backup_oauth: backup_oauth.as_ref(),
            backup_usable,
            force_refresh: force_refresh.is_some(),
        };
        let working = match self.recover_active(&ctx) {
            Ok(Recovery::Use(working)) => working,
            Ok(Recovery::Done(record)) => return record,
            Err(Error::Lock(_) | Error::ClaudeCodeLockTimeout(_)) => {
                info!(
                    "Credential locks held elsewhere; deferring the active-token refresh for account {num} \
                     to the next pass."
                );
                return defer(force_refresh);
            }
            Err(err) => {
                warn!(
                    "Active-token refresh for account {num} failed unexpectedly; deferring: {err}"
                );
                return defer(force_refresh);
            }
        };
        if working.is_empty() {
            return defer(force_refresh);
        }
        let req = self.usage_request(num, email, &working, true, last_usage);
        FetchRecord {
            struck_fp: None,
            ..Self::fetch_record(self.api.try_fetch_usage_for_account(&req))
        }
    }

    /// The locked recovery: consume lock → account lock → Claude credential lock, then either adopt a
    /// rotated live credential, restore a fresh backup, or refresh, and write both stores.
    fn recover_active(&self, c: &ActiveCtx<'_>) -> Result<Recovery> {
        let mut consume = FileLock::new(
            self.credentials_dir
                .join(format!(".consume-{}.lock", c.num)),
            DEFAULT_LOCK_TIMEOUT,
        );
        if !consume.acquire(None)? {
            return Err(Error::Lock("consume lock busy".into()));
        }
        let _slot = FileLock::lock(&self.lock_file, DEFAULT_LOCK_TIMEOUT)?;
        let _creds_lock = claude_credentials_lock(&self.env, None)?;

        let Some(live) = self.read_credentials() else {
            return Ok(Recovery::Use(String::new()));
        };
        let live_oauth = if live.is_empty() {
            None
        } else {
            oauth::extract_oauth_data(&live)
        };
        if !self.live_identity_matches(c.email, c.org_uuid) {
            return Ok(Recovery::Use(String::new()));
        }
        let live_fresh = live_oauth.as_ref().is_some_and(|o| {
            text(o, "accessToken").is_some()
                && text(o, "refreshToken").is_some()
                && !oauth::is_oauth_token_expired(o.get("expiresAt"))
        });
        if live_fresh && live != c.creds {
            // Claude Code rotated the live credential since we read it: adopt it, never refresh.
            let verdict = self
                .lineage_of(c.num, c.email, &live)
                .and_then(|k| self.probe_verdict(&k));
            if verdict == Some(false) {
                return Ok(Recovery::Done(FetchRecord::sentinel(
                    USAGE_FOREIGN_CREDENTIAL,
                )));
            }
            if verdict == Some(true)
                || oauth::credential_fingerprint(&live).as_deref() == c.backup_fp
            {
                if self
                    .write_account_credentials(c.num, c.email, &live)
                    .is_err()
                {
                    warn!(
                        "Backup resync after adopting a rotated credential failed for account {}; the next \
                         expiry may refuse to refresh until a switch resyncs it.",
                        c.num
                    );
                }
            } else {
                debug!(
                    "Adopted a rotated live credential for account {} without a lineage verdict; backup \
                     resync deferred to the next fresh pass's oracle check.",
                    c.num
                );
            }
            return Ok(Recovery::Use(live));
        }

        let Some(refresh_input) = self.pick_refresh_input(c, &live, live_oauth.as_ref()) else {
            return Ok(Recovery::Use(String::new()));
        };
        let input_oauth = oauth::extract_oauth_data(&refresh_input);
        let restore = refresh_input == c.backup
            && c.backup_usable
            && !c.force_refresh
            && input_oauth
                .as_ref()
                .is_some_and(|o| !oauth::is_oauth_token_expired(o.get("expiresAt")));
        let working = if restore {
            c.backup.to_owned()
        } else {
            match self.refresh_active(c, &refresh_input) {
                Ok(working) => working,
                Err(record) => return Ok(Recovery::Done(*record)),
            }
        };
        let backup_ok = restore
            || self
                .write_account_credentials(c.num, c.email, &working)
                .is_ok();
        if !backup_ok {
            warn!(
                "Backup write failed after a consumed refresh for account {}; attempting the active store.",
                c.num
            );
        }
        let live_write = if refresh_input == live {
            working.clone()
        } else {
            self.prepare_credentials_for_activation(&working, Some(&live))
        };
        let live_ok = claude_config_lock(&self.env, None)
            .and_then(|_cfg| self.write_credentials(&live_write))
            .is_ok();
        if !live_ok {
            warn!(
                "Active-store write failed after a {} for account {}{}.",
                if restore {
                    "backup restore"
                } else {
                    "consumed refresh"
                },
                c.num,
                if backup_ok {
                    ""
                } else {
                    "; the rotated credential was NOT persisted anywhere — re-login may be required"
                }
            );
            return Ok(Recovery::Done(FetchRecord::sentinel(USAGE_TOKEN_EXPIRED)));
        }
        Ok(Recovery::Use(working))
    }

    /// Which bytes to refresh from: the live credential when it is the backup's lineage (or provably newer
    /// and verified), else the usable backup. `None` defers.
    fn pick_refresh_input(
        &self,
        c: &ActiveCtx<'_>,
        live: &str,
        live_oauth: Option<&Map<String, Value>>,
    ) -> Option<String> {
        let fallback = || {
            if c.backup_usable {
                c.backup.to_owned()
            } else {
                c.creds.to_owned()
            }
        };
        if live_oauth.is_some() && oauth::credential_fingerprint(live).as_deref() == c.backup_fp {
            let has_rt = live_oauth.is_some_and(|o| text(o, "refreshToken").is_some());
            return Some(if has_rt { live.to_owned() } else { fallback() });
        }
        if live.is_empty() {
            return Some(fallback());
        }
        if live != c.creds {
            return None;
        }
        let live_has_rt = live_oauth.is_some_and(|o| text(o, "refreshToken").is_some());
        if live_has_rt && expires(live_oauth) > expires(c.backup_oauth) {
            let verified = self
                .lineage_of(c.num, c.email, live)
                .and_then(|k| self.probe_verdict(&k))
                .unwrap_or(false);
            if verified {
                return Some(live.to_owned());
            }
            if self.warn_once(c.num, c.email, "expiry-unattributed") {
                warn!(
                    "Live credential is newer than Account-{}'s backup but its ownership is unverified; \
                     refresh deferred to Claude Code's next use.",
                    c.num
                );
            }
            return None;
        }
        Some(fallback())
    }

    /// POST the refresh. `Err` carries the record to return (a deterministic failure strikes the bytes
    /// that were POSTed, unless the source moved meanwhile).
    fn refresh_active(
        &self,
        c: &ActiveCtx<'_>,
        refresh_input: &str,
    ) -> Result<String, Box<FetchRecord>> {
        let result = self
            .api
            .try_refresh_oauth_credentials(refresh_input, ACTIVE_REFRESH_TIMEOUT);
        let empty_success =
            result.error.is_none() && result.credentials.as_deref().is_none_or(str::is_empty);
        let deterministic = matches!(
            result.error.as_deref(),
            Some("invalid_grant" | "no_refresh_token")
        );
        if deterministic || empty_success {
            let source_now = if refresh_input == c.backup {
                self.read_account_credentials(c.num, c.email, false)
            } else {
                self.read_credentials().unwrap_or_default()
            };
            let moved = !source_now.is_empty()
                && oauth::credential_fingerprint(&source_now)
                    != oauth::credential_fingerprint(refresh_input);
            if moved {
                return Err(Box::new(FetchRecord::failure("refresh-failed", None)));
            }
            return Err(Box::new(
                FetchRecord::failure(result.error.unwrap_or_else(|| "invalid_grant".into()), None)
                    .with_struck_fp(oauth::credential_fingerprint(refresh_input)),
            ));
        }
        if result.error.is_some() {
            return Err(Box::new(FetchRecord::failure("refresh-failed", None)));
        }
        let working = result.credentials.unwrap_or_default();
        if let Some(key) = self.lineage_of(c.num, c.email, &working) {
            self.set_probe_verdict(key, true);
        }
        self.clear_warned(c.num, c.email, "expiry-unattributed");
        Ok(working)
    }

    /// When the live credential rotated outside a collect pass, bring the slot's backup to it, after the
    /// profile oracle proves the credential is this slot's. Never fails outward.
    pub(crate) fn resync_rotated_backup(
        &self,
        num: &str,
        email: &str,
        org_uuid: &str,
        creds: &str,
    ) {
        if let Err(err) = self.try_resync_rotated_backup(num, email, org_uuid, creds)
            && !matches!(err, Error::Lock(_) | Error::ClaudeCodeLockTimeout(_))
        {
            warn!(
                "Backup resync for account {num} failed; the recovery branch's newer-generation check still \
                 guards the next expiry: {err}"
            );
        }
    }

    fn try_resync_rotated_backup(
        &self,
        num: &str,
        email: &str,
        org_uuid: &str,
        creds: &str,
    ) -> Result<()> {
        let complete = oauth::extract_oauth_data(creds).is_some_and(|o| {
            text(&o, "accessToken").is_some() && text(&o, "refreshToken").is_some()
        });
        if !complete {
            return Ok(());
        }
        let backup = self.read_account_credentials(num, email, false);
        if !backup.is_empty()
            && oauth::credential_fingerprint(creds) == oauth::credential_fingerprint(&backup)
        {
            return Ok(());
        }
        let Some(key) = self.lineage_of(num, email, creds) else {
            return Ok(());
        };
        match self.probe_verdict(&key) {
            Some(false) => return Ok(()),
            Some(true) => {}
            None => {
                let token = oauth::extract_access_token(creds).unwrap_or_default();
                let Some(resolved) = self.api.fetch_oauth_profile(&token) else {
                    debug!(
                        "Ownership probe for account {num}'s drifted live credential failed; resync skipped."
                    );
                    return Ok(());
                };
                let Some(matched) = self.resolved_matches_slot_identity(num, &resolved)? else {
                    debug!(
                        "Ownership of account {num}'s drifted live credential is unverifiable; resync skipped."
                    );
                    return Ok(());
                };
                self.set_probe_verdict(key.clone(), matched);
                if !matched {
                    if self.warn_once(num, email, "resync") {
                        warn!(
                            "Live credential resolves to a different account than Account-{num}'s identity; \
                             backup left untouched (foreign credential under a stale config)."
                        );
                    }
                    return Ok(());
                }
                self.clear_warned(num, email, "resync");
            }
        }
        let _slot = FileLock::lock(&self.lock_file, DEFAULT_LOCK_TIMEOUT)?;
        let _creds_lock = claude_credentials_lock(&self.env, None)?;
        if !self.live_identity_matches(email, org_uuid) || self.probe_verdict(&key) != Some(true) {
            return Ok(());
        }
        let Some(live) = self.read_credentials().filter(|l| !l.is_empty()) else {
            return Ok(());
        };
        let same = oauth::extract_oauth_data(&live).is_some_and(|o| {
            text(&o, "accessToken").is_some()
                && text(&o, "refreshToken").is_some()
                && oauth::credential_fingerprint(&live) == oauth::credential_fingerprint(creds)
        });
        if same {
            self.write_account_credentials(num, email, &live)?;
            info!(
                "Resynced account {num}'s backup to the rotated live credential (rotation completed outside a collect pass)."
            );
        }
        Ok(())
    }
}

/// Inputs of the locked recovery.
struct ActiveCtx<'a> {
    num: &'a str,
    email: &'a str,
    org_uuid: &'a str,
    creds: &'a str,
    backup: &'a str,
    backup_fp: Option<&'a str>,
    backup_oauth: Option<&'a Map<String, Value>>,
    backup_usable: bool,
    force_refresh: bool,
}

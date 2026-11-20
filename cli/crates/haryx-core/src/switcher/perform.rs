//! The switch itself: upstream `_live_matches_slot_backup`, `_self_switch_action`,
//! `_prefetch_live_identity`, `_classify_outgoing_credential`, `_attribute_live_credential`,
//! `_stash_live_credential`, `_rescue_foreign_successor`, `_read_target_credentials`, `_perform_switch`,
//! `_print_switch_followup`.
//!
//! Under three locks (haryx → Claude credential → Claude config) a switch: backs up the outgoing login
//! into its slot after deciding whether the live credential really belongs there; reads the target's
//! backup; writes it to the live store; replaces `oauthAccount` in `.claude.json`; records the new
//! active slot. A failure rolls the earlier steps back. The identity lookup step 1 needs is fetched
//! before the locks are taken.

use log::{debug, error, info, warn};
use serde_json::{Map, Value, json};

use super::{JsonObject, Switcher};
use crate::claude_locks::{claude_config_lock, claude_credentials_lock};
use crate::credentials::{CredentialBackend, looks_like_api_key};
use crate::error::{Error, Result};
use crate::fsutil::{WriteOptions, atomic_write_text};
use crate::json_output::account_ref;
use crate::locking::{DEFAULT_LOCK_TIMEOUT, FileLock};
use crate::models::{SwitchRollback, SwitchTransaction, get_timestamp};
use crate::oauth::{self, AccountIdentity};
use crate::printer::{accent, dimmed, warning};
use crate::{PROG, paths, session};

/// The live credential and what the profile oracle said about it, fetched before the locks.
#[derive(Clone, Debug, Default)]
pub struct Provenance {
    pub live: Option<String>,
    pub resolved: Option<AccountIdentity>,
}

/// Result of a performed switch: `from`/`to` refs and warnings collected for `--json`.
#[derive(Clone, Debug)]
pub struct SwitchOp {
    pub from: Value,
    pub to: Value,
    pub warnings: Vec<String>,
}

fn account_of<'a>(data: &'a JsonObject, num: &str) -> Option<&'a JsonObject> {
    data.get("accounts")?.get(num)?.as_object()
}

fn field(data: &JsonObject, num: &str, key: &str) -> String {
    account_of(data, num)
        .and_then(|a| a.get(key))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

fn same_lineage(a: &str, b: &str) -> bool {
    a == b || oauth::credential_fingerprint(a) == oauth::credential_fingerprint(b)
}

fn has_tokens(creds: &str) -> bool {
    oauth::extract_oauth_data(creds).is_some_and(|o| {
        ["accessToken", "refreshToken"].iter().any(|k| {
            o.get(*k)
                .and_then(Value::as_str)
                .is_some_and(|s| !s.is_empty())
        })
    })
}

impl SwitchRollback for Switcher {
    fn write_credentials(&self, credentials: &str) -> Result<()> {
        Switcher::write_credentials(self, credentials)
    }

    fn restore_active_account(&self, account_number: i64) -> Result<()> {
        let Some(mut data) = self.get_sequence_data()? else {
            return Ok(());
        };
        data.insert("activeAccountNumber".into(), json!(account_number));
        data.insert("lastUpdated".into(), json!(get_timestamp()));
        self.write_json(&self.sequence_file, &data)
    }
}

impl Switcher {
    fn emit(&self, emit_output: bool, warnings: &mut Vec<String>, msg: String) {
        if emit_output {
            warning(&msg);
        } else {
            warnings.push(msg);
        }
    }

    /// Whether the live credential is the slot's stored backup (an unreadable or empty live store counts
    /// as matching: there is nothing to reconcile).
    fn live_matches_slot_backup(&self, slot: &str, email: &str) -> bool {
        let Some(live) = self.read_credentials().filter(|l| !l.is_empty()) else {
            return true;
        };
        let backup = self.read_account_credentials(slot, email, false);
        !backup.is_empty() && same_lineage(&live, &backup)
    }

    /// What a switch to the slot that is already active should do: `noop`, `noop-diverged`, or
    /// `reconcile` (with the provenance that proves ownership).
    pub(crate) fn self_switch_action(
        &self,
        slot: &str,
        email: &str,
    ) -> (&'static str, Option<Provenance>) {
        if self.live_matches_slot_backup(slot, email) {
            return ("noop", None);
        }
        let provenance = self.prefetch_live_identity();
        if provenance.resolved.is_none() {
            info!(
                "Live credential diverges from Account-{slot}'s stored backup and ownership could not be \
                 verified; self-switch left everything untouched."
            );
            return ("noop-diverged", None);
        }
        ("reconcile", Some(provenance))
    }

    /// Resolve who the live credential belongs to (network, before any lock), when the local backup
    /// cannot already establish it.
    pub(crate) fn prefetch_live_identity(&self) -> Provenance {
        let mut result = Provenance::default();
        let Some(live) = self.read_credentials() else {
            return result;
        };
        result.live = Some(live.clone());
        if live.is_empty() {
            return result;
        }
        let Some((email, org)) = self.get_current_account() else {
            return result;
        };
        let data = self.get_sequence_data().ok().flatten().unwrap_or_default();
        let Some(slot) = Self::find_account_slot(&data, &email, &org) else {
            return result;
        };
        let backup = self.read_account_credentials(&slot, &email, false);
        if same_lineage(&backup, &live) {
            return result;
        }
        if let Some(token) = oauth::extract_access_token(&live) {
            result.resolved = self.api.fetch_oauth_profile(&token);
        }
        result
    }

    /// Who the outgoing live credential belongs to. Kinds: `own-bytes`, `own-family`, `own-rotated`,
    /// `wiped`, `oauth-absent`, `known-foreign`, `unresolved`, `alien`, `foreign`, `foreign-synced`,
    /// `api-key-mismatch`; plus the foreign slot for `foreign`/`foreign-synced`.
    fn classify_outgoing_credential(
        &self,
        current: &str,
        email: &str,
        original: &str,
        provenance: &Provenance,
        data: &JsonObject,
    ) -> (&'static str, Option<String>) {
        let (kind, foreign) =
            self.attribute_live_credential(current, email, original, provenance, data);
        let is_key_slot = account_of(data, current)
            .and_then(|a| a.get("kind"))
            .and_then(Value::as_str)
            == Some("api_key");
        if matches!(kind, "own-family" | "own-rotated" | "unresolved")
            && is_key_slot
            && !looks_like_api_key(Some(original))
        {
            return ("api-key-mismatch", None);
        }
        (kind, foreign)
    }

    fn attribute_live_credential(
        &self,
        current: &str,
        email: &str,
        original: &str,
        provenance: &Provenance,
        data: &JsonObject,
    ) -> (&'static str, Option<String>) {
        let backup = self.read_account_credentials(current, email, false);
        if !backup.is_empty() && backup == original {
            return ("own-bytes", None);
        }
        if !backup.is_empty()
            && oauth::credential_fingerprint(&backup) == oauth::credential_fingerprint(original)
        {
            return ("own-family", None);
        }
        let live_oauth = oauth::extract_oauth_data(original);
        if live_oauth.is_some() && !has_tokens(original) {
            return ("wiped", None);
        }
        if live_oauth.is_none()
            && !backup.is_empty()
            && oauth::extract_oauth_data(&backup).is_some()
        {
            return ("oauth-absent", None);
        }
        let resolved = match &provenance.resolved {
            Some(r) if provenance.live.as_deref() == Some(original) => r,
            _ => {
                let foreign = self
                    .lineage_of(current, email, original)
                    .and_then(|k| self.probe_verdict(&k))
                    == Some(false);
                return (
                    if foreign {
                        "known-foreign"
                    } else {
                        "unresolved"
                    },
                    None,
                );
            }
        };
        let r_email = resolved.email.as_deref().unwrap_or_default();
        let r_org = resolved.organization_uuid.as_deref().unwrap_or_default();
        let r_uuid = resolved.uuid.trim();
        let own_uuid = field(data, current, "uuid").trim().to_owned();
        let own_org = field(data, current, "organizationUuid");
        if !r_uuid.is_empty()
            && !own_uuid.is_empty()
            && r_uuid == own_uuid
            && (r_org.is_empty() || own_org.is_empty() || r_org == own_org)
        {
            return ("own-rotated", None);
        }
        let mut slot = if r_email.is_empty() {
            None
        } else {
            Self::find_account_slot(data, r_email, r_org)
        };
        if let Some(s) = &slot
            && !r_uuid.is_empty()
        {
            let stored = field(data, s, "uuid");
            if !stored.trim().is_empty() && stored.trim() != r_uuid {
                slot = None;
            }
        }
        if slot.is_none() && !r_uuid.is_empty() {
            slot = data
                .get("accounts")
                .and_then(Value::as_object)
                .and_then(|accounts| {
                    accounts.iter().find_map(|(num, acct)| {
                        let uuid = acct.get("uuid").and_then(Value::as_str).unwrap_or_default();
                        let org = acct
                            .get("organizationUuid")
                            .and_then(Value::as_str)
                            .unwrap_or_default();
                        (!uuid.is_empty() && uuid == r_uuid && org == r_org).then(|| num.clone())
                    })
                });
        }
        let Some(slot) = slot else {
            // An identity the profile fully answered that no slot holds.
            let org_known = resolved.organization_uuid.is_some();
            return (
                if !r_email.is_empty() && org_known {
                    "alien"
                } else {
                    "unresolved"
                },
                None,
            );
        };
        if slot == current {
            return ("own-rotated", None);
        }
        if r_uuid.is_empty() || field(data, &slot, "uuid").trim() != r_uuid {
            return ("alien", None);
        }
        let foreign_backup =
            self.read_account_credentials(&slot, &field(data, &slot, "email"), false);
        if !foreign_backup.is_empty() && same_lineage(&foreign_backup, original) {
            return ("foreign-synced", Some(slot));
        }
        ("foreign", Some(slot))
    }

    /// Keep a live credential that does not belong to the outgoing slot in the unclaimed stash.
    fn stash_live_credential(
        &self,
        original: &str,
        reason: &str,
        current: &str,
        resolved: Option<&AccountIdentity>,
    ) -> Result<String> {
        let mtime = std::fs::metadata(paths::credentials_path_in(&self.env))
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| jiff::Timestamp::try_from(t).ok())
            .map(|t| t.strftime("%Y-%m-%dT%H:%M:%SZ").to_string());
        let live_oauth_account = self
            .read_json(&self.claude_config_path(), false)
            .ok()
            .flatten()
            .and_then(|c| c.get("oauthAccount").cloned());
        let mut meta = Map::new();
        meta.insert("reason".into(), json!(reason));
        meta.insert("configSlot".into(), json!(current));
        meta.insert(
            "fingerprint".into(),
            json!(oauth::credential_fingerprint(original)),
        );
        meta.insert(
            "liveOauthAccount".into(),
            live_oauth_account.unwrap_or(Value::Null),
        );
        meta.insert(
            "resolvedIdentity".into(),
            resolved.map_or(Value::Null, |r| {
                serde_json::to_value(r).unwrap_or(Value::Null)
            }),
        );
        meta.insert("credentialsMtime".into(), json!(mtime));
        let entry_id = self.store.write_unclaimed_credential(original, &meta)?;
        warn!(
            "Live credential does not belong to Account-{current} ({reason}): stashed as {entry_id} \
             (credentials mtime {}). Something outside {PROG} rewrote the live login after the last switch.",
            mtime.as_deref().unwrap_or("unknown")
        );
        Ok(entry_id)
    }

    /// When the foreign owner's backup is older than the live credential, advance it (the safety copy is
    /// kept in the stash). Never fails outward.
    fn rescue_foreign_successor(
        &self,
        foreign_slot: &str,
        data: &JsonObject,
        original: &str,
    ) -> bool {
        let Some(live) = oauth::extract_oauth_data(original) else {
            return false;
        };
        let text = |o: &Map<String, Value>, k: &str| {
            o.get(k)
                .and_then(Value::as_str)
                .is_some_and(|s| !s.is_empty())
        };
        if !(text(&live, "accessToken") && text(&live, "refreshToken")) {
            return false;
        }
        let email = field(data, foreign_slot, "email");
        let (backup, unreadable) = self.read_account_credentials_ex(foreign_slot, &email, false);
        if unreadable {
            return false;
        }
        let stored = oauth::extract_oauth_data(&backup).unwrap_or_default();
        let number = |v: Option<&Value>| match v {
            None | Some(Value::Null) => Some(0.0),
            Some(Value::Number(n)) => n.as_f64().filter(|f| f.is_finite()),
            Some(_) => None,
        };
        let stored_has_tokens = text(&stored, "accessToken") || text(&stored, "refreshToken");
        if stored_has_tokens && !matches!(stored.get("expiresAt"), Some(Value::Number(_))) {
            return false;
        }
        let (Some(live_exp), Some(stored_exp)) = (
            number(live.get("expiresAt")),
            number(stored.get("expiresAt")),
        ) else {
            return false;
        };
        if live_exp <= stored_exp {
            return false;
        }
        match self.write_account_credentials(foreign_slot, &email, original) {
            Ok(()) => {
                info!(
                    "Resynced account {foreign_slot}'s backup to its newer live credential (found under another \
                     account's config label; the safety copy is kept)."
                );
                true
            }
            Err(err) => {
                warn!(
                    "Could not advance account {foreign_slot}'s backup to its newer live credential; the safety \
                     copy keeps it: {err}"
                );
                false
            }
        }
    }

    fn read_target_credentials(&self, num: &str, email: &str) -> Result<String> {
        let (creds, unreadable) = self.read_account_credentials_ex(num, email, false);
        if !creds.is_empty() {
            return Ok(creds);
        }
        if unreadable {
            return Err(Error::Switch(format!(
                "Account-{num}'s backup is in the macOS Keychain but it is unreadable right now (locked or no GUI \
                 session). Retry from a GUI terminal; do not re-add."
            )));
        }
        Err(Error::Switch(format!(
            "Account-{num} has no stored credentials. Re-add with: {PROG} --add-account --slot {num}"
        )))
    }

    fn read_target_config(&self, num: &str, email: &str) -> Result<(String, Value)> {
        let text = self.read_account_config(num, email)?;
        if text.is_empty() {
            return Err(Error::Switch(format!(
                "Account-{num} has no stored config backup. Re-add with: {PROG} --add-account --slot {num}"
            )));
        }
        let parsed: Value = serde_json::from_str(&text)
            .map_err(|e| Error::Switch(format!("Invalid backup config: {e}")))?;
        let oauth_section = parsed.get("oauthAccount").cloned().filter(|v| match v {
            Value::Object(o) => !o.is_empty(),
            Value::Null => false,
            _ => true,
        });
        let Some(oauth_section) = oauth_section else {
            return Err(Error::Switch("Invalid oauthAccount in backup".into()));
        };
        Ok((text, oauth_section))
    }

    /// Replace `oauthAccount` in the live `.claude.json`, leaving the rest alone (or write the backup's
    /// whole config when the live one is missing or unreadable).
    fn write_live_config(
        &self,
        target_text: &str,
        oauth_section: &Value,
        emit_output: bool,
        warnings: &mut Vec<String>,
    ) -> Result<()> {
        let config_path = self.claude_config_path();
        match self.read_json(&config_path, false)? {
            Some(mut existing) => {
                existing.insert("oauthAccount".into(), oauth_section.clone());
                self.write_json(&config_path, &existing)
            }
            None => {
                if config_path.exists() {
                    self.salvage_unreadable(&config_path, emit_output, warnings)?;
                }
                let whole: JsonObject = serde_json::from_str(target_text)?;
                self.write_json(&config_path, &whole)
            }
        }
    }

    /// Refuse or warn when the target has a live session-mode instance, and adopt a quiescent profile's
    /// newer credential before activating the backup.
    fn guard_target_session(
        &self,
        target: &str,
        emit_output: bool,
        warnings: &mut Vec<String>,
    ) -> Result<()> {
        let pre = self.get_sequence_data()?.unwrap_or_default();
        let email = field(&pre, target, "email");
        if email.is_empty() {
            return Ok(());
        }
        let org = field(&pre, target, "organizationUuid");
        let (sessions, unreadable) = session::scan_live_sessions(&self.session_dir(target, &email));
        let pids: Vec<String> = sessions.iter().map(|s| s.pid.to_string()).collect();
        if !pids.is_empty() || unreadable > 0 {
            if self.session_profile_ahead(target, &email, &org).is_some() {
                let who = if pids.is_empty() {
                    format!("{unreadable} session record(s) that could not be read")
                } else {
                    format!(
                        "a live session-mode Claude instance (PID {})",
                        pids.join(", ")
                    )
                };
                return Err(Error::Switch(format!(
                    "Account-{target} ({email}) has {who}, and its session profile's credential has rotated past \
                     the stored backup: the backup is a consumed generation, and activating it would fail with \
                     invalid_grant on its first refresh. Exit the session (its credential is adopted into the \
                     backup once nothing runs against it), or switch to another account."
                )));
            }
            if !pids.is_empty() {
                self.emit(
                    emit_output,
                    warnings,
                    format!(
                        "Account-{target} ({email}) has a live session-mode Claude instance (PID {}). Running the \
                         same account as both the default login and a session can make one copy's token go stale \
                         if the server rotates it. If the session later fails to authenticate, exit it and re-run \
                         '{PROG} run {target}'.",
                        pids.join(", ")
                    ),
                );
            }
            return Ok(());
        }
        let adopted = self.adopt_session_credential(target, &email, &org)?;
        if adopted.is_none() && self.session_profile_ahead(target, &email, &org).is_some() {
            return Err(Error::Switch(format!(
                "Account-{target} ({email}) has a session profile whose credential has rotated past the stored \
                 backup, and it could not be adopted into the backup: the profile's identity file (.claude.json) \
                 could not be read, or a Claude instance started against it just now. The backup is a consumed \
                 generation, and activating it would fail with invalid_grant on its first refresh. Re-login \
                 inside the profile ('{PROG} run {target}', then /login) so its identity is rewritten, or switch \
                 to another account."
            )));
        }
        Ok(())
    }

    /// Perform a switch to `target`. `force_activate` rewrites the live login from the stored backup
    /// without backing up the current one.
    pub fn perform_switch(
        &self,
        target: &str,
        emit_output: bool,
        force_activate: bool,
        provenance: Option<Provenance>,
    ) -> Result<SwitchOp> {
        self.refuse_session_shell()?;
        let mut warnings = Vec::new();
        self.guard_target_session(target, emit_output, &mut warnings)?;
        let provenance = match provenance {
            Some(p) => p,
            None if force_activate => Provenance::default(),
            None => self.prefetch_live_identity(),
        };

        let op = {
            let _slot = FileLock::lock(&self.lock_file, DEFAULT_LOCK_TIMEOUT)?;
            let _creds = claude_credentials_lock(&self.env, None)?;
            let _cfg = claude_config_lock(&self.env, None)?;
            self.switch_locked(
                target,
                emit_output,
                force_activate,
                &provenance,
                &mut warnings,
            )?
        };
        if let SwitchOutcome::Switched {
            from,
            to,
            email,
            org,
        } = op
        {
            if emit_output {
                println!("{} Account-{target} ({email})", accent("Switched to"));
                self.list_accounts(false, false, None);
                println!();
                self.print_switch_followup();
                println!();
            }
            self.replan_new_active(target, &email, &org);
            return Ok(SwitchOp { from, to, warnings });
        }
        let SwitchOutcome::Activated {
            from,
            to,
            email,
            org,
        } = op
        else {
            unreachable!("both outcomes handled")
        };
        if emit_output {
            println!("{} Account-{target} ({email})", accent("Activated"));
            println!();
            self.print_switch_followup();
            println!();
        }
        self.replan_new_active(target, &email, &org);
        Ok(SwitchOp { from, to, warnings })
    }

    fn switch_locked(
        &self,
        target: &str,
        emit_output: bool,
        force_activate: bool,
        provenance: &Provenance,
        warnings: &mut Vec<String>,
    ) -> Result<SwitchOutcome> {
        let mut data = self.get_sequence_data()?.unwrap_or_default();
        let target_email = account_of(&data, target)
            .and_then(|a| a.get("email"))
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| Error::AccountNotFound(format!("Account-{target} does not exist")))?;
        let target_org = field(&data, target, "organizationUuid");
        let to = account_ref(target.parse().ok(), &target_email);
        let mut current = data
            .get("activeAccountNumber")
            .filter(|v| !v.is_null())
            .map(|v| v.as_i64().map_or_else(|| v.to_string(), |n| n.to_string()));
        let identity = self.get_current_account();
        if let Some((email, org)) = &identity {
            current = Self::find_account_slot(&data, email, org);
        }

        let (Some((current_email, _)), Some(current_num), false) =
            (identity.clone(), current.clone(), force_activate)
        else {
            let from = match (&identity, &current) {
                (None, _) => Value::Null,
                (Some((email, _)), None) => account_ref(None, email),
                (Some((email, _)), Some(num)) => account_ref(num.parse().ok(), email),
            };
            self.activate_without_backup(
                target,
                &target_email,
                identity.is_some(),
                current.as_deref(),
                force_activate,
                emit_output,
                warnings,
                &mut data,
            )?;
            return Ok(SwitchOutcome::Activated {
                from,
                to,
                email: target_email,
                org: target_org,
            });
        };
        let from = account_ref(current_num.parse().ok(), &current_email);
        let config_path = self.claude_config_path();
        let original_creds = match self.read_credentials() {
            None => {
                return Err(Error::CredentialRead(
                    "Failed to read current credentials".into(),
                ));
            }
            Some(c) if c.is_empty() => {
                return Err(Error::CredentialRead(
                    "Current account credential is empty (Keychain unreadable?); refusing to overwrite its backup".into(),
                ));
            }
            Some(c) => c,
        };
        let original_config = match std::fs::read_to_string(&config_path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(Error::Config("Claude config file not found".into()));
            }
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                return Err(Error::Config(
                    "Permission denied reading Claude config".into(),
                ));
            }
            Err(e) => return Err(e.into()),
        };
        let mut tx = SwitchTransaction {
            original_credentials: original_creds.clone(),
            original_config: original_config.clone(),
            original_account_num: current_num.clone(),
            original_email: current_email.clone(),
            config_path: config_path.clone(),
            completed_steps: Vec::new(),
        };
        let body = (|| -> Result<()> {
            self.back_up_outgoing(
                &current_num,
                &current_email,
                &original_creds,
                &original_config,
                provenance,
                &mut data,
                emit_output,
                warnings,
            )?;
            let target_creds = self.read_target_credentials(target, &target_email)?;
            let (target_text, oauth_section) = self.read_target_config(target, &target_email)?;
            self.write_credentials(
                &self.prepare_credentials_for_activation(&target_creds, Some(&original_creds)),
            )?;
            tx.record_step("credentials_written");
            info!("Wrote target credentials");
            self.write_live_config(&target_text, &oauth_section, emit_output, warnings)?;
            tx.record_step("config_written");
            info!("Updated config file");
            data.insert(
                "activeAccountNumber".into(),
                json!(target.parse::<i64>().unwrap_or_default()),
            );
            data.insert("lastUpdated".into(), json!(get_timestamp()));
            self.write_json(&self.sequence_file, &data)?;
            tx.record_step("sequence_updated");
            info!("Switched from account {current_num} to {target}");
            Ok(())
        })();
        if let Err(err) = body {
            error!("Switch failed: {err}, attempting rollback");
            if tx.completed_steps.is_empty() {
                return Err(err);
            }
            if tx.rollback(self) {
                info!("Rollback successful");
                return Err(Error::Switch(format!(
                    "Switch failed and was rolled back: {err}"
                )));
            }
            error!("Rollback failed!");
            return Err(Error::Switch(format!(
                "Switch failed and rollback also failed: {err}. Manual recovery may be needed."
            )));
        }
        Ok(SwitchOutcome::Switched {
            from,
            to,
            email: target_email,
            org: target_org,
        })
    }

    /// Step 1 of a switch: back up the outgoing login according to who it belongs to.
    #[allow(clippy::too_many_arguments)]
    fn back_up_outgoing(
        &self,
        current: &str,
        email: &str,
        original_creds: &str,
        original_config: &str,
        provenance: &Provenance,
        data: &mut JsonObject,
        emit_output: bool,
        warnings: &mut Vec<String>,
    ) -> Result<()> {
        let (kind, foreign) =
            self.classify_outgoing_credential(current, email, original_creds, provenance, data);
        let resolved = provenance.resolved.as_ref();
        match kind {
            "foreign" | "alien" | "known-foreign" => {
                self.stash_live_credential(original_creds, kind, current, resolved)?;
                let foreign = foreign.unwrap_or_default();
                let msg = if kind == "foreign" && self.rescue_foreign_successor(&foreign, data, original_creds) {
                    format!(
                        "Credential ownership mismatch detected. The live credential belongs to Account-{foreign} \
                         and is newer than its stored backup, so Account-{foreign}'s backup was updated to it (a \
                         safety copy is kept). Nothing was written into Account-{current}."
                    )
                } else if kind == "foreign" {
                    format!(
                        "Credential ownership mismatch detected. The live credential was preserved and was not \
                         written into Account-{current}. If Account-{foreign} later cannot authenticate, log in \
                         as it and run: {PROG} add --slot {foreign}"
                    )
                } else if kind == "known-foreign" {
                    format!(
                        "The live credential was previously identified as another account's. It was preserved \
                         and not written into Account-{current}. If the owning account later cannot authenticate, \
                         log in as it and run: {PROG} add"
                    )
                } else {
                    format!(
                        "The live login does not match a managed account. It was preserved and not written into \
                         Account-{current}. If you need that account, log in as it and run: {PROG} add"
                    )
                };
                self.emit(emit_output, warnings, msg);
            }
            "foreign-synced" => self.emit(
                emit_output,
                warnings,
                format!(
                    "Credential ownership mismatch detected. The live credential already matches Account-{}'s \
                     stored backup, so nothing was written into Account-{current}.",
                    foreign.unwrap_or_default()
                ),
            ),
            "api-key-mismatch" => {
                if has_tokens(original_creds) {
                    self.stash_live_credential(original_creds, kind, current, resolved)?;
                    self.emit(
                        emit_output,
                        warnings,
                        format!(
                            "The live store holds a Claude login instead of Account-{current}'s API key. The \
                             stored key was kept and the login was preserved, not written into Account-{current}. \
                             If you need that account, log in as it and run: {PROG} add"
                        ),
                    );
                }
                self.write_account_config(current, email, original_config)?;
                info!("Backed up account {current} (config only; the live credentials are not an API key)");
            }
            "wiped" | "oauth-absent" => {
                self.write_account_config(current, email, original_config)?;
                let cause = if kind == "wiped" {
                    "The live credential's tokens were wiped (Claude Code clears them when a refresh is rejected)."
                } else {
                    "The live credential holds no Claude login (an API key, or an emptied credentials store)."
                };
                self.emit(
                    emit_output,
                    warnings,
                    format!(
                        "{cause} Account-{current}'s stored backup was kept. If the account cannot authenticate \
                         after switching back, log in with Claude Code and run: {PROG} add"
                    ),
                );
            }
            "unresolved" => {
                self.write_account_credentials(current, email, original_creds)?;
                self.write_account_config(current, email, original_config)?;
                info!(
                    "Backed up account {current} (lineage differs from the stored backup and ownership could not \
                     be verified)"
                );
            }
            "own-bytes" => {
                self.write_account_config(current, email, original_config)?;
                info!("Backed up account {current} (config only; credentials unchanged)");
            }
            _ => {
                self.write_account_credentials(current, email, original_creds)?;
                self.write_account_config(current, email, original_config)?;
                if kind == "own-rotated"
                    && let Some(uuid) = resolved.map(|r| r.uuid.clone()).filter(|u| !u.is_empty())
                    && let Some(acct) = data
                        .get_mut("accounts")
                        .and_then(|a| a.get_mut(current))
                        .and_then(Value::as_object_mut)
                    && acct.get("uuid").and_then(Value::as_str).is_none_or(str::is_empty)
                {
                    acct.insert("uuid".into(), json!(uuid));
                }
                info!("Backed up account {current}");
            }
        }
        Ok(())
    }

    /// Activation with nothing to back up: no live login, an unmanaged one, or `--force`.
    #[allow(clippy::too_many_arguments)]
    fn activate_without_backup(
        &self,
        target: &str,
        target_email: &str,
        has_identity: bool,
        current: Option<&str>,
        force_activate: bool,
        emit_output: bool,
        warnings: &mut Vec<String>,
        data: &mut JsonObject,
    ) -> Result<()> {
        let target_creds = self.read_target_credentials(target, target_email)?;
        let (target_text, oauth_section) = self.read_target_config(target, target_email)?;
        let config_path = self.claude_config_path();
        let live = self.read_credentials().ok_or_else(|| {
            Error::CredentialRead("Cannot snapshot live credentials before activation".into())
        })?;
        let rollback_creds = if !has_identity && live.is_empty() {
            None
        } else {
            Some(live)
        };
        let rollback_config = if config_path.exists() {
            Some(std::fs::read_to_string(&config_path).map_err(|e| {
                Error::Config(format!(
                    "Cannot snapshot live config before activation: {e}"
                ))
            })?)
        } else {
            None
        };
        if let Some(old) = rollback_creds
            .as_deref()
            .filter(|c| !c.is_empty() && *c != target_creds)
            && let Err(err) = self.stash_live_credential(
                old,
                "displaced-live-login",
                current.unwrap_or("unmanaged"),
                None,
            )
        {
            if !force_activate {
                return Err(Error::Switch(format!(
                    "Could not preserve the live credential before activation (safety-copy write failed: {err}); \
                     aborting rather than destroying it"
                )));
            }
            self.emit(
                emit_output,
                warnings,
                format!(
                    "Could not preserve the replaced live credential (safety-copy write failed: {err}) — proceeding \
                     because --force explicitly rewrites the live login."
                ),
            );
        }
        let mut creds_written = false;
        let mut config_written = false;
        let result = (|| -> Result<()> {
            self.write_credentials(
                &self.prepare_credentials_for_activation(&target_creds, rollback_creds.as_deref()),
            )?;
            creds_written = true;
            self.write_live_config(&target_text, &oauth_section, emit_output, warnings)?;
            config_written = true;
            data.insert(
                "activeAccountNumber".into(),
                json!(target.parse::<i64>().unwrap_or_default()),
            );
            data.insert("lastUpdated".into(), json!(get_timestamp()));
            self.write_json(&self.sequence_file, data)
        })();
        if let Err(err) = result {
            if config_written
                && let Some(text) = &rollback_config
                && let Err(e) = atomic_write_text(&config_path, text, WriteOptions::default())
            {
                error!("Failed to rollback config: {e}");
            }
            if creds_written
                && let Some(old) = &rollback_creds
                && let Err(e) = self.write_credentials(old)
            {
                error!("Failed to rollback credentials: {e}");
            }
            return Err(err);
        }
        if force_activate && has_identity {
            info!("Activated account {target} (forced, backup of current login skipped)");
        } else {
            info!("Activated account {target} (no prior live account)");
        }
        debug!("activation complete");
        Ok(())
    }

    /// How the new login takes effect, by the live store's backend.
    fn print_switch_followup(&self) {
        let backend =
            self.store
                .last_active_credentials_backend()
                .unwrap_or(if self.store.use_keychain() {
                    CredentialBackend::Keychain
                } else {
                    CredentialBackend::File
                });
        if backend == CredentialBackend::Keychain {
            println!(
                "{}",
                dimmed(
                    "Restart Claude Code to apply immediately — otherwise the session can take up to ~30 seconds \
                     to pick up the new account."
                )
            );
        } else {
            println!(
                "{}",
                dimmed("New account is active on your next message — no restart needed.")
            );
        }
    }
}

/// What the locked part of a switch did.
enum SwitchOutcome {
    Switched {
        from: Value,
        to: Value,
        email: String,
        org: String,
    },
    Activated {
        from: Value,
        to: Value,
        email: String,
        org: String,
    },
}

//! The account registry (`sequence.json`) and identity helpers (upstream `switcher.py` ~3524-4135):
//! reading and initialising the roster, the live identity, slot lookup, alias lookup, identifier
//! resolution, the org-field backfill, and the capture guards `add` uses.

use serde_json::{Map, Value, json};

use super::{JsonObject, Switcher, accounts, record, text};
use crate::credentials::looks_like_api_key;
use crate::error::{Error, Result};
use crate::models::{get_timestamp, org_tag_shown};
use crate::oauth::{self, AccountIdentity};
use crate::paths;
use crate::printer::{accent, muted};

/// Key of a cached ownership verdict for a credential lineage: `(slot, caller email, stored email,
/// stored org, stored uuid, fingerprint)`. Any component changing makes a lookup miss.
pub type LineageKey = (String, String, String, String, String, String);

/// A slot's stored identity (upstream `account_identity`'s dict).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SlotIdentity {
    pub email: String,
    pub organization_uuid: String,
    pub uuid: String,
}

impl Switcher {
    // --- the roster file ---------------------------------------------------------------------------

    /// Create `sequence.json` when it does not exist yet.
    pub fn init_sequence_file(&self) -> Result<()> {
        if self.sequence_file.exists() {
            return Ok(());
        }
        let init = json!({
            "activeAccountNumber": null,
            "lastUpdated": get_timestamp(),
            "sequence": [],
            "accounts": {},
        });
        let Value::Object(map) = init else {
            unreachable!("a json! object literal is an object")
        };
        self.write_json(&self.sequence_file, &map)
    }

    /// The roster. `Ok(None)` ONLY when it does not exist yet; a torn or unreadable file is an
    /// [`Error::Config`], never "no accounts" (a caller would rebuild the roster from nothing).
    pub fn get_sequence_data(&self) -> Result<Option<JsonObject>> {
        self.read_json(&self.sequence_file, true)
    }

    /// [`Self::get_sequence_data`], an absent roster read as empty (upstream `... or {}`).
    pub(crate) fn sequence_or_empty(&self) -> Result<JsonObject> {
        Ok(self.get_sequence_data()?.unwrap_or_default())
    }

    /// The roster after the org-field backfill has run.
    pub fn get_sequence_data_migrated(&self) -> Result<Option<JsonObject>> {
        let Some(data) = self.get_sequence_data()? else {
            return Ok(None);
        };
        if data.is_empty() {
            return Ok(Some(data));
        }
        let needs_migration = accounts(&data).is_some_and(|accts| {
            accts.values().any(|acc| {
                acc.as_object()
                    .is_some_and(|a| !a.contains_key("organizationUuid"))
            })
        });
        if !needs_migration {
            return Ok(Some(data));
        }
        self.migrate_org_fields()?;
        self.get_sequence_data()
    }

    /// The next free slot number: one past the highest.
    pub fn get_next_account_number(&self) -> Result<i64> {
        let data = self.get_sequence_data()?.unwrap_or_default();
        let max = accounts(&data)
            .map(|accts| {
                accts
                    .keys()
                    .filter_map(|k| k.trim().parse::<i64>().ok())
                    .max()
                    .unwrap_or(0)
            })
            .unwrap_or(0);
        Ok(max + 1)
    }

    /// Backfill `organizationUuid`/`organizationName` for records written before org support: the live
    /// config for the active account (authoritative), else the slot's config backup, else empty.
    pub fn migrate_org_fields(&self) -> Result<()> {
        let Some(mut data) = self.get_sequence_data()?.filter(|d| !d.is_empty()) else {
            return Ok(());
        };
        let live = self.live_oauth_account();
        let (live_email, live_org_uuid, live_org_name) = match &live {
            Some(acct) => (
                text(acct, "emailAddress").to_owned(),
                text(acct, "organizationUuid").to_owned(),
                text(acct, "organizationName").to_owned(),
            ),
            None => Default::default(),
        };
        let Some(Value::Object(accts)) = data.get_mut("accounts") else {
            return Ok(());
        };
        let mut updated = false;
        for (num, account) in accts.iter_mut() {
            let Some(account) = account.as_object_mut() else {
                continue;
            };
            if account.contains_key("organizationUuid") {
                continue;
            }
            let email = text(account, "email").to_owned();
            let (org_uuid, org_name) = if !live_email.is_empty() && email == live_email {
                (live_org_uuid.clone(), live_org_name.clone())
            } else {
                let config = self.read_account_config(num, &email).unwrap_or_default();
                backup_org_fields(&config)
            };
            account.insert("organizationUuid".into(), Value::String(org_uuid));
            account.insert("organizationName".into(), Value::String(org_name));
            updated = true;
        }
        if updated {
            data.insert("lastUpdated".into(), Value::String(get_timestamp()));
            self.write_json(&self.sequence_file, &data)?;
        }
        Ok(())
    }

    // --- the live identity -------------------------------------------------------------------------

    /// The live `.claude.json`'s `oauthAccount` object, best effort.
    fn live_oauth_account(&self) -> Option<JsonObject> {
        let config = self
            .read_json(&self.claude_config_path(), false)
            .ok()
            .flatten()?;
        config.get("oauthAccount")?.as_object().cloned()
    }

    /// Current `(email, organization_uuid)` from `.claude.json`.
    pub fn get_current_account(&self) -> Option<(String, String)> {
        self.get_current_identity_triple()
            .map(|(email, org, _)| (email, org))
    }

    /// `(email, org_uuid, account_uuid)` from ONE read of `.claude.json` (two reads could straddle a
    /// `/login` and pair one account's token with another's metadata).
    pub fn get_current_identity_triple(&self) -> Option<(String, String, String)> {
        let account = self.live_oauth_account()?;
        let email = text(&account, "emailAddress");
        if email.is_empty() {
            return None;
        }
        Some((
            email.to_owned(),
            text(&account, "organizationUuid").to_owned(),
            text(&account, "accountUuid").to_owned(),
        ))
    }

    /// Whether the live identity is `(email, org_uuid)` right now (the under-lock TOCTOU re-check).
    pub fn live_identity_matches(&self, email: &str, org_uuid: &str) -> bool {
        self.get_current_account()
            .is_some_and(|(e, o)| e == email && o == org_uuid)
    }

    /// Whether this slot is the DEFAULT profile's login while this process reads another profile.
    pub fn is_default_login_from_another_profile(&self, email: &str, org_uuid: &str) -> bool {
        let default_config = paths::default_global_config_path_in(&self.env);
        if self.claude_config_path() == default_config || !default_config.exists() {
            return false;
        }
        let account = self
            .read_json(&default_config, false)
            .ok()
            .flatten()
            .and_then(|c| c.get("oauthAccount").and_then(Value::as_object).cloned())
            .unwrap_or_default();
        text(&account, "emailAddress") == email && text(&account, "organizationUuid") == org_uuid
    }

    // --- slot identity -----------------------------------------------------------------------------

    /// A slot's stored identity (`""` for missing fields; the uuid trimmed).
    pub fn account_identity(&self, account_num: &str) -> Result<SlotIdentity> {
        let data = self.sequence_or_empty()?;
        let empty = Map::new();
        let acct = accounts(&data)
            .and_then(|a| a.get(account_num))
            .and_then(Value::as_object)
            .unwrap_or(&empty);
        Ok(SlotIdentity {
            email: text(acct, "email").to_owned(),
            organization_uuid: text(acct, "organizationUuid").to_owned(),
            uuid: text(acct, "uuid").trim().to_owned(),
        })
    }

    /// Whether an oracle-resolved identity is this slot's account, uuid first.
    ///
    /// `Some(true)`: same account. `Some(false)`: definitively another (safe to cache). `None`:
    /// unverifiable (the slot has no uuid and the resolved identity is too partial), never cached. On an
    /// exact (email, org) match for a uuid-less slot the resolved uuid is backfilled.
    pub fn resolved_matches_slot_identity(
        &self,
        account_num: &str,
        resolved: &AccountIdentity,
    ) -> Result<Option<bool>> {
        let own = self.account_identity(account_num)?;
        let r_uuid = resolved.uuid.trim();
        let r_email = resolved.email.as_deref().filter(|e| !e.is_empty());
        let r_org = resolved.organization_uuid.as_deref();
        if !r_uuid.is_empty() && !own.uuid.is_empty() {
            let org_ok = r_org.is_none_or(str::is_empty)
                || own.organization_uuid.is_empty()
                || r_org == Some(own.organization_uuid.as_str());
            return Ok(Some(r_uuid == own.uuid && org_ok));
        }
        if let Some(email) = r_email.filter(|e| *e == own.email) {
            let Some(org) = r_org else {
                return Ok(None);
            };
            if org == own.organization_uuid {
                self.backfill_account_uuid(account_num, r_uuid, Some(email), Some(org))?;
                return Ok(Some(true));
            }
            return Ok(Some(false));
        }
        if r_email.is_some() && r_org.is_some() {
            return Ok(Some(false));
        }
        Ok(None)
    }

    /// The probe-verdict key of a credential lineage, bound to the slot's full stored identity.
    pub fn lineage_key(
        &self,
        account_num: &str,
        email: &str,
        fingerprint: &str,
    ) -> Result<LineageKey> {
        let own = self.account_identity(account_num)?;
        Ok((
            account_num.to_owned(),
            email.to_owned(),
            own.email,
            own.organization_uuid,
            own.uuid,
            fingerprint.to_owned(),
        ))
    }

    /// The slot holding `(email, organization_uuid)`, if any. The org compares exactly (a stored `null`
    /// matches nothing; a missing one reads as `""`).
    pub fn find_account_slot(
        data: &JsonObject,
        email: &str,
        organization_uuid: &str,
    ) -> Option<String> {
        accounts(data)?.iter().find_map(|(num, account)| {
            let account = account.as_object()?;
            let email_ok = account.get("email").and_then(Value::as_str) == Some(email);
            let org_ok = match account.get("organizationUuid") {
                None => organization_uuid.is_empty(),
                Some(Value::String(org)) => org == organization_uuid,
                Some(_) => false,
            };
            (email_ok && org_ok).then(|| num.clone())
        })
    }

    /// Whether `(email, organization_uuid)` has a slot.
    pub fn account_exists(&self, email: &str, organization_uuid: &str) -> Result<bool> {
        Ok(self
            .get_sequence_data()?
            .is_some_and(|d| Self::find_account_slot(&d, email, organization_uuid).is_some()))
    }

    /// A slot's stored kind: `"api_key"` or `"oauth"` (the default, also for records without one).
    pub fn account_kind(&self, account_num: Option<&str>) -> Result<&'static str> {
        let Some(num) = account_num else {
            return Ok("oauth");
        };
        let data = self.sequence_or_empty()?;
        let is_key = accounts(&data)
            .and_then(|a| a.get(num))
            .and_then(|r| r.get("kind"))
            .and_then(Value::as_str)
            == Some("api_key");
        Ok(if is_key { "api_key" } else { "oauth" })
    }

    /// Public name of [`Self::account_kind`] for a known slot.
    pub fn account_kind_for(&self, account_num: &str) -> Result<&'static str> {
        self.account_kind(Some(account_num))
    }

    // --- capture guards (used by `add`) -------------------------------------------------------------

    /// Refuse when the active identity moved during an ownership check.
    pub fn reject_identity_drift_since_verify(
        &self,
        verified: &(String, String, String),
    ) -> Result<()> {
        let now = self.get_current_identity_triple();
        if now.as_ref() == Some(verified) {
            return Ok(());
        }
        let now_email = now.map(|t| t.0).filter(|e| !e.is_empty());
        Err(Error::Config(format!(
            "The active account changed while {} was being verified (now {}). Nothing was changed. \
             Re-run when no other login is in flight.",
            verified.0,
            now_email.as_deref().unwrap_or("unknown")
        )))
    }

    /// Refuse when the credential lineage advanced during an ownership check (a refresh of the SAME
    /// account moves no identity). An unreadable store is unverifiable, not a refusal.
    pub fn reject_credential_drift_since_verify(&self, verified: &str) -> Result<()> {
        let Ok(Some(now)) = self.read_capture_credentials() else {
            return Ok(());
        };
        if now.is_empty() {
            return Ok(());
        }
        match (
            oauth::credential_fingerprint(verified),
            oauth::credential_fingerprint(&now),
        ) {
            (Some(before), Some(after)) if before != after => Err(Error::Config(
                "The stored credential rotated while it was being verified. Nothing was changed. \
                 Registering the pre-rotation generation would hand the slot a credential the server \
                 has already retired. Re-run when no refresh is in flight."
                    .into(),
            )),
            _ => Ok(()),
        }
    }

    /// Guard for `add`: the captured token must be THIS account's, checked against the profile oracle,
    /// uuid first, with the org corroborated. Advisory: an unresolvable answer registers with a notice;
    /// only a resolved identity that DISAGREES refuses. A wiped credential (both tokens emptied by Claude
    /// Code after `invalid_grant`) is refused outright. An expired access token is unresolvable, never
    /// refreshed here (spending a grant needs the locked machinery). Makes a network call: never under a
    /// lock.
    pub fn reject_foreign_credential_capture(
        &self,
        creds: &str,
        email: &str,
        org_uuid: &str,
        account_uuid: &str,
    ) -> Result<String> {
        self.check_foreign_capture(creds, email, org_uuid, account_uuid, |token| {
            oauth::fetch_oauth_profile(token)
        })
    }

    pub(crate) fn check_foreign_capture(
        &self,
        creds: &str,
        email: &str,
        org_uuid: &str,
        account_uuid: &str,
        fetch_profile: impl FnOnce(&str) -> Option<AccountIdentity>,
    ) -> Result<String> {
        let unverified = |why: &str| {
            println!(
                "{} could not verify that the stored credential belongs to {email} ({why}). \
                 Registering anyway; re-run where the check can complete to confirm.",
                accent("Notice:")
            );
            Ok(creds.to_owned())
        };
        let oauth_data = oauth::extract_oauth_data(creds);
        if let Some(data) = &oauth_data {
            let has = |k: &str| super::truthy(data.get(k));
            if !has("accessToken") && !has("refreshToken") {
                return Err(Error::Config(format!(
                    "The stored credential for {email} has no access or refresh token (Claude Code \
                     clears both after a rejected refresh). Nothing was changed. Log in as {email} \
                     again, then re-run."
                )));
            }
        }
        let Some(token) = oauth::extract_access_token(creds).filter(|t| !t.is_empty()) else {
            return unverified("no access token to resolve");
        };
        if let Some(data) = &oauth_data
            && oauth::is_oauth_token_expired(data.get("expiresAt"))
        {
            return unverified("the access token is expired");
        }
        let Some(profile) = fetch_profile(&token) else {
            return unverified("the identity lookup did not resolve");
        };
        let seen_uuid = profile.uuid.trim();
        if !account_uuid.is_empty() {
            if seen_uuid != account_uuid {
                return Err(Error::Config(format!(
                    "The stored credential does not belong to {email}: the token resolves to account \
                     {seen_uuid}, not {account_uuid}. Nothing was changed. This happens when the config \
                     names one account while the credential store still holds another's token (e.g. a \
                     renamed .claude.json over a live keychain item). Log in as {email} in THIS \
                     environment, then re-run."
                )));
            }
        } else {
            let seen = profile.email.as_deref().unwrap_or_default().trim();
            if seen.is_empty() {
                return unverified("the resolved identity carries no address");
            }
            if seen.to_lowercase() != email.to_lowercase() {
                return Err(Error::Config(format!(
                    "The stored credential does not belong to {email}: the token resolves to {seen}. \
                     Nothing was changed. This happens when the config names one account while the \
                     credential store still holds another's token (e.g. a renamed .claude.json over a \
                     live keychain item). Log in as {email} in THIS environment, then re-run."
                )));
            }
        }
        let Some(resolved_org) = profile.organization_uuid.as_deref() else {
            return Ok(creds.to_owned()); // structurally absent: unverifiable
        };
        let seen_org = resolved_org.trim();
        if seen_org == org_uuid {
            return Ok(creds.to_owned());
        }
        let or_personal = |s: &str| {
            if s.is_empty() {
                "personal".to_owned()
            } else {
                s.to_owned()
            }
        };
        Err(Error::Config(format!(
            "The stored credential for {email} belongs to organization {}, not {}. Nothing was \
             changed. Two accounts can share an email across organizations. Log in as {email} in the \
             {} organization in THIS environment, then re-run.",
            or_personal(seen_org),
            or_personal(org_uuid),
            or_personal(org_uuid)
        )))
    }

    /// Guard for `add`: never capture a live managed API key as an OAuth account.
    pub fn reject_live_api_key_capture(&self, creds: &str) -> Result<()> {
        if looks_like_api_key(Some(creds)) {
            return Err(Error::Validation(format!(
                "Active login is an API-key account. Add it with '{prog} ai add-token sk-ant-...' instead \
                 of '{prog} ai add'.",
                prog = crate::PROG
            )));
        }
        Ok(())
    }

    /// Refuse registering a token whose `(email, personal org)` already exists as the other kind.
    pub fn reject_cross_kind_collision(&self, email: &str, is_api_key: bool) -> Result<()> {
        let Some(data) = self.get_sequence_data()?.filter(|d| !d.is_empty()) else {
            return Ok(());
        };
        let Some(slot) = Self::find_account_slot(&data, email, "") else {
            return Ok(());
        };
        let existing = self.account_kind(Some(&slot))?;
        let new_kind = if is_api_key { "api_key" } else { "oauth" };
        if existing == new_kind {
            return Ok(());
        }
        let label = |kind: &str| {
            if kind == "api_key" {
                "API-key"
            } else {
                "OAuth"
            }
        };
        Err(Error::Validation(format!(
            "'{email}' already exists as an {} account (slot {slot}); cannot add it as an {} account. \
             Pass a distinct --email.",
            label(existing),
            label(new_kind)
        )))
    }

    // --- display -----------------------------------------------------------------------------------

    /// The org context shown for an account: its org name, else `personal`.
    pub fn get_display_tag(_email: &str, org_name: &str, _org_uuid: &str) -> String {
        if org_name.is_empty() {
            "personal".to_owned()
        } else {
            org_name.to_owned()
        }
    }

    /// ` [org]` for a listing row, or `""` when `ui.orgTag` (`mode`) hides it.
    pub fn tag_suffix(
        mode: &str,
        emails: &[&str],
        email: &str,
        org_name: &str,
        org_uuid: &str,
    ) -> String {
        if !org_tag_shown(mode, email, emails) {
            return String::new();
        }
        format!(
            " {}",
            muted(&format!(
                "[{}]",
                Self::get_display_tag(email, org_name, org_uuid)
            ))
        )
    }

    // --- identifiers -------------------------------------------------------------------------------

    /// The slot whose alias matches (case-insensitively). An empty alias never matches.
    pub fn find_account_by_alias(&self, alias: &str) -> Result<Option<String>> {
        if alias.is_empty() {
            return Ok(None);
        }
        let Some(data) = self.get_sequence_data()? else {
            return Ok(None);
        };
        let key = alias.to_lowercase();
        Ok(accounts(&data).and_then(|accts| {
            accts.iter().find_map(|(num, account)| {
                let alias = account
                    .get("alias")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                (alias.to_lowercase() == key).then(|| num.clone())
            })
        }))
    }

    /// The slot already using `alias`, other than `exclude_num`.
    pub fn alias_in_use(&self, alias: &str, exclude_num: Option<&str>) -> Result<Option<String>> {
        Ok(self
            .find_account_by_alias(alias)?
            .filter(|num| Some(num.as_str()) != exclude_num))
    }

    /// Resolve a slot number, alias or email to a slot number (that precedence). A digit string is
    /// returned as given. An email matching several slots is an [`Error::Config`].
    pub fn resolve_account_identifier(&self, identifier: &str) -> Result<Option<String>> {
        if is_digits(identifier) {
            return Ok(Some(identifier.to_owned()));
        }
        let Some(data) = self.get_sequence_data()?.filter(|d| !d.is_empty()) else {
            return Ok(None);
        };
        if let Some(num) = self.find_account_by_alias(identifier)? {
            return Ok(Some(num));
        }
        let matches: Vec<&String> = accounts(&data)
            .map(|accts| {
                accts
                    .iter()
                    .filter(|(_, a)| a.get("email").and_then(Value::as_str) == Some(identifier))
                    .map(|(num, _)| num)
                    .collect()
            })
            .unwrap_or_default();
        match matches.as_slice() {
            [] => Ok(None),
            [one] => Ok(Some((*one).clone())),
            many => {
                let details: Vec<String> = many
                    .iter()
                    .map(|num| {
                        let org = record(&data, num)
                            .map(|r| text(r, "organizationName"))
                            .filter(|o| !o.is_empty())
                            .unwrap_or("personal");
                        format!("{num} [{org}]")
                    })
                    .collect();
                Err(Error::Config(format!(
                    "Email '{identifier}' is ambiguous — matches accounts: {}. Use account number \
                     instead (e.g., {} --switch-to 1).",
                    details.join(", "),
                    crate::PROG
                )))
            }
        }
    }
}

/// Python's `str.isdigit()` for the ASCII digits slot numbers use.
pub(crate) fn is_digits(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit())
}

/// `(organizationUuid, organizationName)` from a config backup's `oauthAccount`; empty on any problem.
fn backup_org_fields(config: &str) -> (String, String) {
    let parsed: Option<Value> = serde_json::from_str(config).ok();
    let account = parsed
        .as_ref()
        .and_then(|c| c.get("oauthAccount"))
        .and_then(Value::as_object);
    match account {
        Some(acct) => (
            text(acct, "organizationUuid").to_owned(),
            text(acct, "organizationName").to_owned(),
        ),
        None => Default::default(),
    }
}

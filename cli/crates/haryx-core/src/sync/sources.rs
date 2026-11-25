//! What this machine contributes to sync, and how merged values come back to it.
//!
//! - `settings`: the `haryx config` keys this machine has set, except [`LOCAL_ONLY_SETTINGS`].
//! - `app-prefs`: `<store>/sync/app-prefs.json`, an object the macOS and Windows apps own (shared
//!   keys, plus `macos.*` / `windows.*` ones only that platform reads). The CLI does not interpret it.
//! - `account`: per Claude account, its user-set fields (alias, disabled, threshold, priority) with
//!   its email and organization. Never a token, never usage.
//! - `credential`: per API-key or setup-token account, the key or token itself. These never renew,
//!   so every machine can hold the same one. A browser login is never a credential row.
//!
//! "Changed at" for a local side is the modification time of the file it lives in.

use std::path::Path;

use serde_json::{Map, Value};

use super::SyncError;
use super::merge::Values;
use crate::credentials::looks_like_api_key;
use crate::settings::{
    LOCAL_ONLY_SETTINGS, SETTING_SPECS, set_setting, settings_path, unset_setting,
};
use crate::switcher::Switcher;

pub const APP_PREFS_FILE: &str = "app-prefs.json";

/// Unix seconds of `path`'s last change; 0 when it does not exist.
pub fn mtime(path: &Path) -> i64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

fn local(err: impl std::fmt::Display) -> SyncError {
    SyncError::Local(err.to_string())
}

// --- settings ---------------------------------------------------------------------------------

fn syncable_keys() -> impl Iterator<Item = String> {
    SETTING_SPECS
        .iter()
        .map(|s| s.dotted())
        .filter(|k| !LOCAL_ONLY_SETTINGS.contains(&k.as_str()))
}

/// The syncable settings this machine has set, as `section.key` → JSON value.
pub fn read_settings(root: &Path) -> Result<(Values, i64), SyncError> {
    let path = settings_path(root);
    let raw: Map<String, Value> = match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text).map_err(|e| {
            local(format!(
                "{} is not valid JSON ({e}); fix it before syncing",
                path.display()
            ))
        })?,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Map::new(),
        Err(err) => return Err(local(err)),
    };
    let values = syncable_keys()
        .filter_map(|dotted| {
            let (section, key) = dotted.split_once('.')?;
            let value = raw.get(section)?.get(key)?.clone();
            (!value.is_null()).then_some((dotted, value))
        })
        .collect();
    Ok((values, mtime(&path)))
}

/// Make this machine's syncable settings equal `wanted`. Values this version cannot take are
/// skipped and named.
pub fn write_settings(root: &Path, wanted: &Values) -> Result<Vec<(String, String)>, SyncError> {
    let (current, _) = read_settings(root)?;
    let mut skipped = Vec::new();
    for dotted in syncable_keys() {
        match (current.get(&dotted), wanted.get(&dotted)) {
            (a, b) if a == b => {}
            (_, Some(value)) => {
                let raw = match value {
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                if let Err(err) = set_setting(root, &dotted, &raw) {
                    skipped.push((dotted.clone(), err.to_string()));
                }
            }
            (Some(_), None) => {
                unset_setting(root, &dotted).map_err(local)?;
            }
            (None, None) => {}
        }
    }
    Ok(skipped)
}

// --- app preferences --------------------------------------------------------------------------

pub fn app_prefs_path(root: &Path) -> std::path::PathBuf {
    root.join("sync").join(APP_PREFS_FILE)
}

pub fn read_app_prefs(root: &Path) -> Result<(Values, i64), SyncError> {
    let path = app_prefs_path(root);
    match std::fs::read_to_string(&path) {
        Ok(text) => {
            let values: Values = serde_json::from_str(&text)
                .map_err(|e| local(format!("{} is not a JSON object: {e}", path.display())))?;
            Ok((values, mtime(&path)))
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok((Values::new(), 0)),
        Err(err) => Err(local(err)),
    }
}

pub fn write_app_prefs(root: &Path, values: &Values) -> Result<(), SyncError> {
    let text = serde_json::to_string_pretty(values).map_err(local)?;
    crate::account::vault::write_private(&root.join("sync"), APP_PREFS_FILE, &text)
        .map_err(SyncError::Account)
}

// --- accounts and credentials -----------------------------------------------------------------

/// One Claude account on this machine.
#[derive(Debug, Clone)]
pub struct LocalAccount {
    pub num: String,
    pub email: String,
    pub organization_uuid: String,
    pub organization_name: String,
    pub settings: Map<String, Value>,
}

impl LocalAccount {
    /// What the account's row key is derived from: one email can sit in two organizations.
    pub fn label(&self, prefix: &str) -> String {
        label(prefix, &self.email, &self.organization_uuid)
    }

    /// The `account` row values: user settings plus identity.
    pub fn values(&self) -> Values {
        let mut values: Values = self.settings.clone().into_iter().collect();
        values.insert("email".into(), Value::String(self.email.clone()));
        values.insert(
            "organizationUuid".into(),
            Value::String(self.organization_uuid.clone()),
        );
        values.insert(
            "organizationName".into(),
            Value::String(self.organization_name.clone()),
        );
        values
    }
}

pub fn label(prefix: &str, email: &str, org: &str) -> String {
    format!("{prefix}:{}|{org}", email.trim().to_lowercase())
}

pub fn read_accounts(switcher: &Switcher) -> Result<(Vec<LocalAccount>, i64), SyncError> {
    let data = switcher
        .get_sequence_data()
        .map_err(local)?
        .unwrap_or_default();
    let text = |rec: &Value, key: &str| {
        rec.get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    let accounts = data
        .get("accounts")
        .and_then(Value::as_object)
        .map(|accounts| {
            accounts
                .iter()
                .filter(|(_, rec)| !text(rec, "email").is_empty())
                .map(|(num, rec)| LocalAccount {
                    num: num.clone(),
                    email: text(rec, "email"),
                    organization_uuid: text(rec, "organizationUuid"),
                    organization_name: text(rec, "organizationName"),
                    settings: Switcher::carried_settings(rec.as_object()),
                })
                .collect()
        })
        .unwrap_or_default();
    Ok((accounts, mtime(switcher.sequence_file())))
}

/// The key or token of an API-key or setup-token account; `None` for a browser login.
pub fn token_of(switcher: &Switcher, account: &LocalAccount) -> Option<String> {
    let creds = switcher.read_account_credentials(&account.num, &account.email, false);
    let creds = creds.trim();
    if looks_like_api_key(Some(creds)) {
        return Some(creds.to_owned());
    }
    let value: Value = serde_json::from_str(creds).ok()?;
    let oauth = value.get("claudeAiOauth")?.as_object()?;
    if !crate::oauth::is_setup_token(Some(oauth)) {
        return None;
    }
    oauth
        .get("accessToken")
        .and_then(Value::as_str)
        .map(str::to_owned)
}

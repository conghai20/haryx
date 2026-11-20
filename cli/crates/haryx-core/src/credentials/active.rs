//! The LIVE credential: Claude Code's OAuth login and managed API key.

use std::path::PathBuf;

use serde_json::{Map, Value};

use super::claude_keychain::{
    CLAUDE_CODE_KEYCHAIN_SERVICE, CLAUDE_CODE_MANAGED_KEYCHAIN_SERVICE,
    active_oauth_keychain_services, active_profile_is_default,
    active_store_is_the_default_secure_store,
};
use super::kind::{approved_form, looks_like_api_key};
use super::{ACTIVE_READ_ATTEMPTS, ActiveCredentials, CredentialBackend, CredentialStore, lock};
use crate::fsutil::{self, WriteOptions};
use crate::paths;
use crate::{Error, Result};

impl CredentialStore {
    fn credentials_path(&self) -> PathBuf {
        paths::credentials_path_in(&self.host.env)
    }

    fn global_config_path(&self) -> PathBuf {
        paths::global_config_path_in(&self.host.env)
    }

    fn oauth_services(&self) -> Vec<String> {
        active_oauth_keychain_services(&self.host.env)
    }

    /// Claude Code's active credential: the value if found, `Some("")` if not, `None` on a file read error.
    pub fn read_credentials(&self) -> Option<String> {
        self.read_active_credentials().value
    }

    /// Read one OAuth Keychain item with a bounded retry. Returns `(value, failed)`: `failed` only when
    /// every attempt raised; an absent item (rc 44) is not retried.
    fn read_one_oauth_keychain(&self, service: &str) -> (Option<String>, bool) {
        let account = self.host.keychain_account.clone();
        let mut last_error = None;
        for attempt in 0..ACTIVE_READ_ATTEMPTS {
            match self.kc_call(|kc| kc.get_password(service, &account)) {
                Ok(value) => return (value, false),
                Err(err) => {
                    last_error = Some(err);
                    if attempt + 1 < ACTIVE_READ_ATTEMPTS {
                        std::thread::sleep(self.tunables.active_read_retry_delay);
                    }
                }
            }
        }
        log::warn!(
            "Keychain read failed after {ACTIVE_READ_ATTEMPTS} attempt(s), trying file: {}",
            last_error.map(|e| e.to_string()).unwrap_or_default()
        );
        (None, true)
    }

    /// The item claude reads for this environment: the first resolved service.
    fn read_active_oauth_keychain(&self) -> (Option<String>, bool) {
        let services = self.oauth_services();
        let (value, failed) = self.read_one_oauth_keychain(&services[0]);
        (value.filter(|v| !v.is_empty()), failed)
    }

    /// The fallback item(s), only for an explicit `CLAUDE_CONFIG_DIR` naming the default profile.
    fn read_fallback_oauth_keychain(&self) -> (Option<String>, bool) {
        for service in self.oauth_services().iter().skip(1) {
            let (value, failed) = self.read_one_oauth_keychain(service);
            if value.as_deref().is_some_and(|v| !v.is_empty()) {
                return (value, false);
            }
            if failed {
                return (None, true);
            }
        }
        (None, false)
    }

    /// Read Claude Code's active credential, classifying the outcome.
    ///
    /// Order: (1) the OAuth Keychain item for THIS profile (macOS, when usable, bounded retry); (2) the
    /// plaintext `.credentials.json` Claude Code also falls back to; (3) the managed key (Keychain
    /// `Claude Code`, then `primaryApiKey`); (4) only with `CLAUDE_CONFIG_DIR` naming the default profile and
    /// nothing else answering, the unsuffixed OAuth item. Trying OAuth fully first means an OAuth login with
    /// only a file fallback is never misread as an API key. Non-mutating.
    ///
    /// The Keychain item is redirected to the profile's hashed name, not skipped: claude writes rotations
    /// keychain-only on macOS, so a custom profile frequently has no plaintext file at all.
    pub fn read_active_credentials(&self) -> ActiveCredentials {
        let mut keychain_failed = false;
        self.lift_pin_if_oauth_file_gone();
        let keychain_asked = self.use_keychain();
        if keychain_asked {
            let (value, failed) = self.read_active_oauth_keychain();
            keychain_failed = failed;
            // THIS read's own verdict, kept so a later success on some OTHER item cannot erase it.
            lock(&self.state).active_read_failed = failed;
            if let Some(value) = value {
                return ActiveCredentials::new(Some(value), false, false);
            }
        } else {
            let (residual_unverified, active_read_failed) = {
                let state = lock(&self.state);
                (
                    state.residual_verdict == Some(false),
                    state.active_read_failed,
                )
            };
            if residual_unverified || active_read_failed || self.keychain_unreadable() {
                // Keychain already known unusable this process: an absence below is "keychain
                // unavailable", not an empty slot.
                keychain_failed = true;
            }
        }

        // 2. OAuth plaintext file. After a FAILED keychain read it may hold a stale generation; the flag
        // travels as `degraded`.
        match std::fs::read(self.credentials_path()) {
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => {
                log::error!("Failed to read credentials file: {err}");
                return ActiveCredentials::new(None, keychain_failed, keychain_failed);
            }
            Ok(bytes) => match String::from_utf8(bytes) {
                Err(err) => {
                    log::error!("Failed to read credentials file: {err}");
                    return ActiveCredentials::new(None, keychain_failed, keychain_failed);
                }
                Ok(text) if !text.trim().is_empty() => {
                    return ActiveCredentials::new(Some(text), false, keychain_failed);
                }
                Ok(_) => {}
            },
        }

        // 3. Managed API key.
        let key = self.read_managed_key();
        if !key.is_empty() {
            return ActiveCredentials::new(Some(key), false, keychain_failed);
        }

        // 4. The fallback OAuth item, only when nothing above answered. An unreadable Keychain is a property
        // of the Keychain, so a failed read is not retried under a second name.
        if keychain_asked && !keychain_failed && self.oauth_services().len() > 1 {
            let (value, failed) = if self.use_keychain() {
                self.read_fallback_oauth_keychain()
            } else {
                // The managed-key lookup above failed; the fallback item was never asked.
                (None, true)
            };
            keychain_failed = failed;
            lock(&self.state).active_read_failed = failed;
            if let Some(value) = value {
                return ActiveCredentials::new(Some(value), false, false);
            }
        }
        ActiveCredentials::new(Some(String::new()), keychain_failed, keychain_failed)
    }

    /// The active managed API key, or "" when absent. Non-mutating.
    ///
    /// macOS Keychain `Claude Code` (when usable, default profile only) first, then `primaryApiKey`, mirroring
    /// Claude Code's `getApiKeyFromConfigOrMacOSKeychain`. The Keychain half is gated, not redirected: claude's
    /// managed-key service name under a custom profile is not pinned anywhere, and guessing it is worse.
    pub fn read_managed_key(&self) -> String {
        if active_profile_is_default(&self.host.env) && self.use_keychain() {
            let account = self.host.keychain_account.clone();
            match self.kc_call(|kc| kc.get_password(CLAUDE_CODE_MANAGED_KEYCHAIN_SERVICE, &account))
            {
                Ok(Some(value)) if !value.is_empty() => return value,
                Ok(_) => {}
                Err(err) => log::warn!("Managed-key Keychain read failed: {err}"),
            }
        }
        self.read_global_config()
            .and_then(|cfg| match cfg.get("primaryApiKey") {
                Some(Value::String(key)) if !key.is_empty() => Some(key.clone()),
                _ => None,
            })
            .unwrap_or_default()
    }

    /// `~/.claude.json` parsed, or `None` when absent, unreadable or not an object.
    pub fn read_global_config(&self) -> Option<Map<String, Value>> {
        let path = self.global_config_path();
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return None,
            Err(err) => {
                log::warn!("Failed to read global config: {err}");
                return None;
            }
        };
        match serde_json::from_str::<Value>(&text) {
            Ok(Value::Object(map)) => Some(map),
            Ok(_) => None,
            Err(err) => {
                log::warn!("Failed to read global config: {err}");
                None
            }
        }
    }

    /// Whether the global config exists. An error answering that counts as "exists": the callers use it to
    /// refuse overwriting a file they could not read.
    fn global_config_exists(&self) -> bool {
        self.global_config_path().try_exists().unwrap_or(true)
    }

    /// Atomically apply `mutator` to `~/.claude.json`, key-scoped: every other key (`oauthAccount`,
    /// projects, settings) is preserved. Refuses to overwrite a config that exists but could not be read: a
    /// torn config would otherwise be replaced by `{}` plus this mutation.
    pub fn update_global_config(
        &self,
        mutator: impl FnOnce(&mut Map<String, Value>),
    ) -> Result<()> {
        let path = self.global_config_path();
        let data = self.read_global_config();
        if data.is_none() && self.global_config_exists() {
            return Err(Error::CredentialWrite(format!(
                "{} exists but could not be read — refusing to overwrite it. Move or repair the file, then \
                 retry.",
                path.display()
            )));
        }
        let mut data = data.unwrap_or_default();
        mutator(&mut data);
        let write = || -> std::io::Result<()> {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let text = serde_json::to_string_pretty(&Value::Object(data))?;
            fsutil::atomic_write_text(&path, &text, WriteOptions::default())
        };
        write().map_err(|e| Error::CredentialWrite(format!("Failed to write global config: {e}")))
    }

    /// Atomically write Claude Code's plaintext active-credentials file (0600).
    pub fn write_active_credentials_file(&self, credentials: &str) -> std::io::Result<()> {
        let cred_file = self.credentials_path();
        if let Some(parent) = cred_file.parent() {
            std::fs::create_dir_all(parent)?;
        }
        fsutil::atomic_write_text(&cred_file, credentials, WriteOptions::default())
    }

    /// Best-effort removal of the active-credential Keychain item (macOS only).
    ///
    /// Claude Code reads the Keychain before the file, so once the store falls back to the file it must clear
    /// a stale item or Claude Code would resurrect it (Claude Code #30337). Clears only the FIRST resolved
    /// service: the one claude reads for this environment. Returns whether no active item can shadow the
    /// file (`true` off macOS). Not routed through the capability tracking, as upstream.
    pub fn delete_active_keychain_entry(&self) -> bool {
        if !self.is_macos() {
            return true;
        }
        let services = self.oauth_services();
        self.keychain
            .delete_password(&services[0], &self.host.keychain_account)
            .is_ok()
    }

    /// Write Claude Code's active credential, enforcing a single auth axis.
    ///
    /// The kind comes from the payload (raw `sk-ant-…` key vs OAuth JSON), and activating one axis clears the
    /// other, mirroring Claude Code's `saveApiKey`/`removeApiKey`.
    pub fn write_credentials(&self, credentials: &str) -> Result<()> {
        if looks_like_api_key(Some(credentials)) {
            self.write_managed_credentials(credentials.trim())
        } else {
            self.write_oauth_credentials(credentials)?;
            self.clear_managed_key();
            Ok(())
        }
    }

    /// Activate a managed API key, then clear OAuth (mutual exclusion).
    ///
    /// Always records `key[-20:]` in `customApiKeyResponses.approved` (Claude Code does this on every
    /// platform). Stores the key in the macOS Keychain when usable, else `primaryApiKey`.
    pub fn write_managed_credentials(&self, api_key: &str) -> Result<()> {
        let mut wrote_to_keychain = false;
        if self.use_keychain() {
            let account = self.host.keychain_account.clone();
            match self.kc_call(|kc| {
                kc.set_password(CLAUDE_CODE_MANAGED_KEYCHAIN_SERVICE, &account, api_key)
            }) {
                Ok(()) => wrote_to_keychain = true,
                Err(err) => {
                    log::warn!("Managed-key Keychain write failed, falling back to config: {err}")
                }
            }
        }

        let approved = approved_form(api_key);
        self.update_global_config(|cfg| {
            let mut responses = match cfg.remove("customApiKeyResponses") {
                Some(Value::Object(map)) => map,
                _ => Map::new(),
            };
            let mut approved_list = match responses.remove("approved") {
                Some(Value::Array(items)) => items,
                _ => Vec::new(),
            };
            if !approved_list
                .iter()
                .any(|v| v.as_str() == Some(approved.as_str()))
            {
                approved_list.push(Value::String(approved.clone()));
            }
            responses.insert("approved".into(), Value::Array(approved_list));
            responses
                .entry("rejected")
                .or_insert_with(|| Value::Array(Vec::new()));
            cfg.insert("customApiKeyResponses".into(), Value::Object(responses));
            if wrote_to_keychain {
                // The Keychain holds the key; keep it out of plaintext config.
                cfg.remove("primaryApiKey");
            } else {
                cfg.insert("primaryApiKey".into(), Value::String(api_key.to_string()));
            }
        })?;

        self.clear_oauth_credential();
        if self.is_macos() && !wrote_to_keychain {
            // A stale `Claude Code` item may remain and managed-key reads check it before `primaryApiKey`:
            // pin so a cooldown re-probe cannot read that residual. Unverified: nothing deleted the MANAGED
            // item.
            self.pin_file_mode(false);
        }
        self.set_last_backend(if wrote_to_keychain {
            CredentialBackend::Keychain
        } else {
            CredentialBackend::File
        });
        Ok(())
    }

    /// Clear any active managed API key (Claude Code `removeApiKey` semantics). Never fails.
    ///
    /// Deletes the macOS Keychain `Claude Code` item (best-effort) and drops `primaryApiKey`, leaving
    /// `customApiKeyResponses.approved` untouched. An unreadable (not absent) config is left in place with a
    /// warning: a stale `primaryApiKey` beside a fresh OAuth login must be tellable from "nothing to clear".
    pub fn clear_managed_key(&self) {
        if self.is_macos() {
            let _ = self.keychain.delete_password(
                CLAUDE_CODE_MANAGED_KEYCHAIN_SERVICE,
                &self.host.keychain_account,
            );
        }
        let cfg = self.read_global_config();
        if cfg.is_none() && self.global_config_exists() {
            log::warn!(
                "Could not clear primaryApiKey: the global config exists but could not be read \
                 (unreadable, not absent) — leaving it in place rather than overwriting it unread"
            );
            return;
        }
        let has_key = cfg
            .as_ref()
            .and_then(|c| c.get("primaryApiKey"))
            .is_some_and(|v| !v.is_null());
        if has_key {
            if let Err(err) = self.update_global_config(|c| {
                c.remove("primaryApiKey");
            }) {
                log::warn!("Failed to clear primaryApiKey: {err}");
            }
        }
    }

    /// Clear the active OAuth credential: Keychain item and plaintext file, best-effort.
    ///
    /// The store removes the file here itself, so its absence says nothing about Claude Code having moved the
    /// credential to the Keychain; the pin lift is disarmed first.
    pub fn clear_oauth_credential(&self) {
        lock(&self.state).pinned_to_oauth_file = false;
        self.delete_active_keychain_entry();
        let cred_file = self.credentials_path();
        if let Err(err) = std::fs::remove_file(&cred_file) {
            if err.kind() != std::io::ErrorKind::NotFound {
                log::warn!("Failed to remove credentials file: {err}");
            }
        }
    }

    /// Write Claude Code's active OAuth credential.
    ///
    /// macOS writes the Keychain when usable and this environment's store is the unsuffixed item; on success
    /// it rewrites an already-present `.credentials.json` (never creating one) so a running Claude Code's
    /// mtime-based cache invalidation fires (upstream #86). Otherwise it writes the file and best-effort
    /// clears the stale Keychain item (Claude Code #30337). A custom profile takes the file route
    /// deliberately (upstream #206): haryx does not author claude's hashed item, so it retires it and seeds
    /// the file; that route does not pin when the item is verifiably gone.
    pub fn write_oauth_credentials(&self, credentials: &str) -> Result<()> {
        let keychain_usable = self.use_keychain();
        let default_store =
            keychain_usable && active_store_is_the_default_secure_store(&self.host.env);
        let deliberate_file_route = keychain_usable && !default_store;
        if default_store {
            let account = self.host.keychain_account.clone();
            match self
                .kc_call(|kc| kc.set_password(CLAUDE_CODE_KEYCHAIN_SERVICE, &account, credentials))
            {
                Ok(()) => {
                    self.refresh_stale_credentials_file(credentials);
                    self.set_last_backend(CredentialBackend::Keychain);
                    return Ok(());
                }
                Err(err) => log::warn!("Keychain write failed, falling back to file: {err}"),
            }
        }

        self.write_active_credentials_file(credentials)
            .map_err(|e| Error::CredentialWrite(format!("Failed to write credentials: {e}")))?;
        let cleared = self.delete_active_keychain_entry();
        if self.is_macos() && !(deliberate_file_route && cleared) {
            let mut state = lock(&self.state);
            Self::pin_file_mode_locked(&mut state, cleared);
            state.pinned_to_oauth_file = true;
        }
        self.set_last_backend(CredentialBackend::File);
        Ok(())
    }

    /// After a Keychain write, rewrite an already-present `.credentials.json` with the same credential to
    /// bump its mtime. Never creates it; best-effort.
    fn refresh_stale_credentials_file(&self, credentials: &str) {
        if !self.credentials_path().exists() {
            return;
        }
        if let Err(err) = self.write_active_credentials_file(credentials) {
            log::warn!(
                "Could not refresh .credentials.json after Keychain write ({err}); a running session may \
                 not hot-reload until restart"
            );
        }
    }
}

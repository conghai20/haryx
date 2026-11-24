//! `run` (session mode): upstream `session.SessionManager`. Launches Claude Code as a stored account in
//! this terminal only, by pointing `CLAUDE_CONFIG_DIR` at a per-account profile under
//! `<store>/sessions/<num>-<email>/`; the default login and other terminals are untouched.
//!
//! A profile is seeded with the slot's credential (refreshed through the consume gate first), shares
//! settings/keybindings/CLAUDE.md/skills/commands/agents/rules from `~/.claude` by symlink, mirrors the
//! default profile's MCP servers, and (by default) shares conversation history, merging whatever the
//! profile accumulated on its own into `~/.claude` first.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use log::{info, warn};
use serde_json::{Map, Value, json};

use crate::claude_locks::{CONFIG_STALENESS, proper_lockfile};
use crate::credentials::{
    CredentialStore, StoreHost, Tunables, haryx_backup_namespace, looks_like_api_key,
};
use crate::error::{Error, Result};
use crate::fsutil::{
    WriteOptions, atomic_write_text, is_link, link_points_at, remove_link, replace_with_retry,
};
use crate::locking::FileLock;
use crate::printer::{accent, dimmed, muted, warning};
use crate::session::{
    self, clear_session_stale, is_session_stale, profile_is_quiescent, session_dir_for,
};
use crate::session_history::{MergeCounts, merge_history_into_source, mkdir_private};
use crate::settings::{atomic_write_json, explicit_share_history, load_run_settings};
use crate::switcher::Switcher;
use crate::{AI_PROG, PROG, oauth, paths, spawn};

/// Items shared from `~/.claude` into every profile (unless `--no-share`).
pub const SHARED_ITEMS: [&str; 7] = [
    "settings.json",
    "keybindings.json",
    "CLAUDE.md",
    "skills",
    "commands",
    "agents",
    "rules",
];
/// Conversation history, shared unless `--no-share-history` / `run.shareHistory false`.
pub const HISTORY_ITEMS: [&str; 2] = ["projects", "history.jsonl"];
const SHARE_MANIFEST: &str = ".cswap-shared.json";
const MCP_KEY: &str = "mcpServers";
const MCP_MIRROR_MARKER: &str = ".cswap-mcp-mirror-v1";
const MCP_DISPLACED_STASH: &str = ".cswap-mcp-displaced.json";
const MIGRATION_LOCK: &str = ".cswap-history-migration.lock";
const STAGED_LINK_SUFFIX: &str = ".cswap-link-staging";
const STAGED_CLEANUP_LOCK_TIMEOUT: Duration = Duration::from_secs(5);
const AUTH_STATUS_TIMEOUT: Duration = Duration::from_secs(10);
const BOOTSTRAP_LOCK_TIMEOUT: Duration = Duration::from_secs(30);
const MIGRATION_LOCK_TIMEOUT: Duration = Duration::from_secs(30);

/// Variables that would override the selected account inside Claude Code.
pub const AUTH_OVERRIDE_ENV_VARS: [&str; 5] = [
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "CLAUDE_CODE_OAUTH_TOKEN",
    "CLAUDE_CODE_OAUTH_TOKEN_FILE_DESCRIPTOR",
    "CLAUDE_CODE_API_KEY_FILE_DESCRIPTOR",
];

fn all_shareable(name: &str) -> bool {
    SHARED_ITEMS.contains(&name) || HISTORY_ITEMS.contains(&name)
}

fn load_json_object(path: &Path) -> Option<Map<String, Value>> {
    match serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()? {
        Value::Object(obj) => Some(obj),
        _ => None,
    }
}

/// Upstream `_session_validity` verdicts.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum Validity {
    Valid,
    Invalid,
    Unknown,
    Unreachable,
}

/// Options of one `run`.
#[derive(Debug, Clone, Default)]
pub struct RunRequest {
    pub share: bool,
    /// `None`: the `run.shareHistory` setting decides.
    pub share_history: Option<bool>,
    pub require_session: bool,
}

pub struct SessionManager<'a> {
    switcher: &'a Switcher,
    sessions_dir: PathBuf,
    merge: MergeCounts,
    deferral_reported: bool,
    own_history_found: bool,
    share_history_asked: bool,
}

impl<'a> SessionManager<'a> {
    pub fn new(switcher: &'a Switcher) -> Self {
        SessionManager {
            sessions_dir: switcher.backup_dir().join("sessions"),
            switcher,
            merge: MergeCounts::default(),
            deferral_reported: false,
            own_history_found: false,
            share_history_asked: false,
        }
    }

    fn store_root(&self) -> &Path {
        self.switcher.backup_dir()
    }

    pub(crate) fn claude_bin() -> Result<PathBuf> {
        spawn::resolve(
            std::ffi::OsStr::new("claude"),
            std::env::var_os("PATH").as_deref(),
        )
        .filter(|p| p.is_file())
        .ok_or_else(|| {
            Error::Session("'claude' was not found on PATH. Install Claude Code first.".into())
        })
    }

    fn resolve_share_history(&mut self, requested: Option<bool>) -> bool {
        self.share_history_asked = requested == Some(true);
        requested.unwrap_or_else(|| load_run_settings(self.store_root()).share_history)
    }

    fn ensure_not_api_key(&self, num: &str, email: &str) -> Result<()> {
        if self.switcher.account_kind(Some(num))? == "api_key" {
            return Err(Error::Session(format!(
                "Account-{num} ({email}) is an API-key account; '{AI_PROG} run' (session mode) does not support \
                 API-key accounts yet. Use '{PROG} --switch-to' to make it your default login instead."
            )));
        }
        Ok(())
    }

    /// The default (non-profile) login's identity, read from the default `.claude.json`.
    fn default_login_identity(&self) -> Option<(String, String)> {
        let config = load_json_object(&paths::default_global_config_path_in(
            self.switcher.path_env(),
        ))?;
        let oauth = config.get("oauthAccount")?.as_object()?;
        let email = oauth
            .get("emailAddress")?
            .as_str()
            .filter(|e| !e.is_empty())?;
        let org = oauth
            .get("organizationUuid")
            .and_then(Value::as_str)
            .unwrap_or_default();
        Some((email.to_owned(), org.to_owned()))
    }

    /// Whether the default login holds no usable token (signed out, or an expired access token with no
    /// refresh token). An unreadable store is never "signed out".
    fn default_login_signed_out(&self) -> bool {
        let mut env = self.switcher.path_env().clone();
        if env
            .secure_storage_dir
            .as_ref()
            .is_some_and(|d| !d.is_empty())
        {
            return false;
        }
        env.claude_config_dir = None;
        let Ok(ns) = haryx_backup_namespace(&env) else {
            return false;
        };
        let store = CredentialStore::with_keychain(
            StoreHost::new(&ns, env),
            Arc::new(crate::keychain::Security::system()),
            Tunables::default(),
        );
        let active = store.read_active_credentials();
        let Some(value) = active
            .value
            .filter(|_| !active.keychain_unavailable && !active.degraded)
        else {
            return false;
        };
        if value.trim().is_empty() {
            return true;
        }
        if looks_like_api_key(Some(&value)) {
            return false;
        }
        let Some(data) = oauth::extract_oauth_data(&value) else {
            return false;
        };
        let has = |k: &str| {
            data.get(k)
                .and_then(Value::as_str)
                .is_some_and(|s| !s.is_empty())
        };
        if has("refreshToken") {
            return false;
        }
        if !has("accessToken") {
            return true;
        }
        let now_ms = jiff::Timestamp::now().as_millisecond() as f64;
        matches!(data.get("expiresAt"), Some(Value::Number(n)) if n.as_f64().is_some_and(|e| e <= now_ms))
    }

    /// `run NUM|EMAIL`: never returns on success (the process becomes `claude`).
    pub fn run(
        &mut self,
        identifier: &str,
        claude_args: &[String],
        req: &RunRequest,
    ) -> Result<()> {
        let claude = Self::claude_bin()?;
        let share_history = self.resolve_share_history(req.share_history);
        let (num, email, org) = self.switcher.resolve_account(identifier)?;
        self.ensure_not_api_key(&num, &email)?;
        let preset = std::env::var_os("CLAUDE_CONFIG_DIR").filter(|v| !v.is_empty());
        let current = if preset.is_some() {
            self.default_login_identity()
        } else {
            self.switcher.get_current_account()
        };
        let is_default = current.as_ref() == Some(&(email.clone(), org.clone()));
        let signed_out = is_default && self.default_login_signed_out();
        if signed_out {
            println!(
                "{}",
                dimmed(&format!(
                    "Account-{num} ({email}) is the default login, but that login holds no usable token; \
                     launching from the stored login in a session profile."
                ))
            );
        }
        if is_default && !signed_out {
            if req.require_session {
                return Err(Error::Session(format!(
                    "Account-{num} ({email}) is the active default login, so this launch would run plain claude \
                     on the default login rather than in a session profile (a second copy of the active \
                     credential would drift). Switch the default login to another account first, or run \
                     `claude` directly."
                )));
            }
            println!(
                "{}",
                dimmed(&format!(
                    "Account-{num} ({email}) is already the active default login — launching claude directly."
                ))
            );
            self.history_on_the_fast_path(
                &session_dir_for(self.store_root(), &num, &email),
                share_history,
            );
            return exec(&claude, claude_args, &[], &["CLAUDE_CONFIG_DIR"]);
        }
        if let Some(preset) = &preset {
            warning(&format!(
                "CLAUDE_CONFIG_DIR is already set ({}); overriding it for this launch.",
                preset.to_string_lossy()
            ));
        }
        let scrubbed: Vec<&str> = AUTH_OVERRIDE_ENV_VARS
            .iter()
            .copied()
            .filter(|v| std::env::var_os(v).is_some_and(|x| !x.is_empty()))
            .collect();
        if !scrubbed.is_empty() {
            warning(&format!(
                "Ignoring {} for this session — it would override the selected account inside Claude Code.",
                scrubbed.join(", ")
            ));
        }
        let (session_dir, num, email) =
            self.setup_session(identifier, req.share, req.share_history)?;
        println!(
            "{} Account-{num} ({email}) {}",
            accent("Launching"),
            muted("[session mode]")
        );
        let set = [("CLAUDE_CONFIG_DIR", session_dir.into_os_string())];
        exec(&claude, claude_args, &set, &AUTH_OVERRIDE_ENV_VARS)
    }

    /// `run` with no mapping for the directory: plain `claude` on the default login.
    pub fn exec_default(&self, claude_args: &[String]) -> Result<()> {
        exec(&Self::claude_bin()?, claude_args, &[], &[])
    }

    fn history_on_the_fast_path(&mut self, session_dir: &Path, share_history: bool) {
        let own = HISTORY_ITEMS.iter().any(|n| {
            let p = session_dir.join(n);
            p.exists() && !is_link(&p)
        });
        if !own {
            return;
        }
        if share_history {
            self.sync_items(session_dir, &HISTORY_ITEMS, &HISTORY_ITEMS);
        } else {
            println!(
                "{}",
                dimmed(
                    "   This launch uses the default login's chat history (~/.claude), not the account's own."
                )
            );
        }
    }

    /// Create or reuse the account's profile; returns `(profile dir, slot, email)`.
    pub fn setup_session(
        &mut self,
        identifier: &str,
        share: bool,
        share_history: Option<bool>,
    ) -> Result<(PathBuf, String, String)> {
        let share_history = self.resolve_share_history(share_history);
        let (num, email, org) = self.switcher.resolve_account(identifier)?;
        self.ensure_not_api_key(&num, &email)?;
        let dir = session_dir_for(self.store_root(), &num, &email);
        let stale = is_session_stale(&dir) && profile_is_quiescent(&dir);
        if !stale && self.is_session_valid(&dir, &email, &org) {
            self.sync_sharing(&dir, share, share_history);
            return Ok((dir, num, email));
        }
        self.refresh_before_bootstrap(&num, &email)?;
        let _lock = FileLock::lock(self.switcher.lock_file_path(), BOOTSTRAP_LOCK_TIMEOUT)?;
        if is_session_stale(&dir) && profile_is_quiescent(&dir) {
            self.switcher.invalidate_session_credentials(&num, &email)?;
            clear_session_stale(&dir);
        }
        if self.is_session_valid(&dir, &email, &org) {
            if !self.profile_matches_backup(&dir, &num, &email) && profile_is_quiescent(&dir) {
                self.bootstrap(&dir, &num, &email)?;
            }
            self.sync_sharing(&dir, share, share_history);
            return Ok((dir, num, email));
        }
        self.bootstrap(&dir, &num, &email)?;
        self.sync_sharing(&dir, share, share_history);
        let mut verdict = self.session_validity(&dir, &email, &org);
        if verdict == Validity::Unknown && self.artifacts_say_usable(&dir, &email, &org) {
            verdict = Validity::Valid;
        }
        match verdict {
            Validity::Valid => Ok((dir, num, email)),
            Validity::Unknown | Validity::Unreachable => Err(Error::Session(format!(
                "Session profile for Account-{num} ({email}) could not be verified: `claude auth status` did not \
                 run or did not answer. The profile is left in place — check that `claude` is on PATH, then retry."
            ))),
            Validity::Invalid => {
                self.cleanup_failed_session(&dir);
                Err(Error::Session(format!(
                    "Session profile for Account-{num} ({email}) failed validation. Log in with that account and \
                     re-add it: {PROG} --add-account --slot {num}"
                )))
            }
        }
    }

    /// Freshen the stored grant through the consume gate before seeding a profile with it.
    fn refresh_before_bootstrap(&self, num: &str, email: &str) -> Result<()> {
        let pre = self.switcher.read_account_credentials(num, email, false);
        let has_rt = serde_json::from_str::<Value>(&pre).ok().is_none_or(|v| {
            v["claudeAiOauth"]["refreshToken"]
                .as_str()
                .is_some_and(|s| !s.is_empty())
        });
        if pre.is_empty() || !has_rt {
            return Ok(());
        }
        let outcome = self.switcher.consume_backup_grant(num, email, &pre);
        if outcome.error.is_some() && outcome.credentials.is_some() {
            if outcome.stashed {
                return Err(Error::Session(format!(
                    "Account-{num}'s refreshed credential could not be stored, so the backup still holds a spent \
                     grant. The successor is stashed — please retry, and the next run adopts it automatically."
                )));
            }
            return Err(Error::Session(format!(
                "Account-{num}'s refreshed credential could neither be stored nor stashed, so the backup holds a \
                 spent grant and the successor is gone. Fix the storage failure first; retrying before that spends \
                 nothing but earns a strike. If the slot strikes, log in again and re-add it: {PROG} \
                 --add-account --slot {num}"
            )));
        }
        match outcome.error.as_deref() {
            Some("live-login") => Err(live_login_error(num)),
            Some(_) => {
                warning(&format!(
                    "Could not refresh the token for Account-{num}; continuing with the stored credentials."
                ));
                Ok(())
            }
            None => Ok(()),
        }
    }

    fn profile_matches_backup(&self, dir: &Path, num: &str, email: &str) -> bool {
        let profile = session::read_session_credentials(&self.switcher.profile_keychain(), dir)
            .unwrap_or_default();
        let backup = self.switcher.read_account_credentials(num, email, false);
        profile.is_empty()
            || backup.is_empty()
            || oauth::credential_fingerprint(&profile) == oauth::credential_fingerprint(&backup)
    }

    /// Seed the profile with the slot's credential and identity.
    fn bootstrap(&self, dir: &Path, num: &str, email: &str) -> Result<()> {
        let stored = self.switcher.read_account_credentials(num, email, false);
        if self.switcher.live_login_holds(&stored) == Some(true) {
            return Err(live_login_error(num));
        }
        self.switcher.profile_keychain().delete_entry(dir);
        let (creds, unreadable) = self.switcher.read_account_credentials_ex(num, email, false);
        if creds.is_empty() {
            return Err(Error::Session(if unreadable {
                format!(
                    "Account-{num}'s backup is in the macOS Keychain but it is unreadable right now (locked or no \
                     GUI session). Retry from a GUI terminal; do not re-add."
                )
            } else {
                format!(
                    "Account-{num} has no stored credentials. Re-add with: {PROG} --add-account --slot {num}"
                )
            }));
        }
        let config_text = self.switcher.read_account_config(num, email)?;
        let config: Map<String, Value> = serde_json::from_str(&config_text).unwrap_or_default();
        let Some(oauth_account) = config
            .get("oauthAccount")
            .filter(|v| v.as_object().is_some_and(|o| !o.is_empty()))
        else {
            return Err(Error::Session(format!(
                "Account-{num} has no stored config backup. Re-add with: {PROG} --add-account --slot {num}"
            )));
        };
        std::fs::create_dir_all(dir)?;
        set_mode(dir, 0o700);
        atomic_write_text(
            &dir.join(".credentials.json"),
            &creds,
            WriteOptions::default(),
        )?;
        let config_path = dir.join(".claude.json");
        let mut existing = load_json_object(&config_path).unwrap_or_default();
        existing.insert("oauthAccount".into(), oauth_account.clone());
        existing.insert("hasCompletedOnboarding".into(), json!(true));
        if !existing.contains_key("theme") {
            let theme = config
                .get("theme")
                .and_then(Value::as_str)
                .filter(|t| !t.is_empty())
                .unwrap_or("dark");
            existing.insert("theme".into(), json!(theme));
        }
        let text = crate::models::pyfmt::json_dumps_pretty(&Value::Object(existing));
        atomic_write_text(&config_path, &text, WriteOptions::default())?;
        info!(
            "Bootstrapped session profile for account {num} at {}",
            dir.display()
        );
        Ok(())
    }

    fn cleanup_failed_session(&self, dir: &Path) {
        self.switcher.profile_keychain().delete_entry(dir);
        let _ = std::fs::remove_dir_all(dir);
        clear_session_stale(dir);
    }

    /// `claude auth status --json` against the profile.
    fn session_validity(&self, dir: &Path, email: &str, org: &str) -> Validity {
        if !dir.is_dir() {
            return Validity::Invalid;
        }
        let claude = Self::claude_bin().unwrap_or_else(|_| PathBuf::from("claude"));
        let opts = spawn::RunOptions {
            env: vec![(
                OsString::from("CLAUDE_CONFIG_DIR"),
                dir.as_os_str().to_owned(),
            )],
            env_remove: AUTH_OVERRIDE_ENV_VARS.iter().map(OsString::from).collect(),
            timeout: Some(AUTH_STATUS_TIMEOUT),
            ..Default::default()
        };
        let out = match spawn::run(
            &[
                claude.as_os_str(),
                "auth".as_ref(),
                "status".as_ref(),
                "--json".as_ref(),
            ],
            &opts,
        ) {
            Ok(out) => out,
            Err(spawn::SpawnError::Timeout { .. }) => return Validity::Unknown,
            Err(spawn::SpawnError::Io(_)) => return Validity::Unreachable,
        };
        if !out.success() {
            return Validity::Invalid;
        }
        let Ok(Value::Object(status)) = serde_json::from_str::<Value>(&out.stdout) else {
            return Validity::Unknown;
        };
        let text = |k: &str| status.get(k).and_then(Value::as_str);
        if status.get("loggedIn") != Some(&json!(true))
            || text("authMethod") != Some("claude.ai")
            || text("email") != Some(email)
        {
            return Validity::Invalid;
        }
        match text("orgId").filter(|o| !o.is_empty()) {
            Some(status_org) if !org.is_empty() && status_org != org => Validity::Invalid,
            _ => Validity::Valid,
        }
    }

    fn may_have_credential_material(&self, dir: &Path) -> bool {
        if std::fs::read_to_string(dir.join(".credentials.json")).is_ok_and(|t| !t.is_empty()) {
            return true;
        }
        session::read_session_credentials(&self.switcher.profile_keychain(), dir)
            .is_some_and(|c| !c.is_empty())
    }

    fn artifacts_say_usable(&self, dir: &Path, email: &str, org: &str) -> bool {
        self.may_have_credential_material(dir)
            && !session::session_identity_drifted(dir, email, org)
    }

    fn is_session_valid(&self, dir: &Path, email: &str, org: &str) -> bool {
        match self.session_validity(dir, email, org) {
            Validity::Unknown => self.artifacts_say_usable(dir, email, org),
            v => v == Validity::Valid,
        }
    }

    fn sync_sharing(&mut self, dir: &Path, share: bool, share_history: bool) {
        if !dir.is_dir() {
            return;
        }
        self.sync_mcp_servers(dir, share);
        let mut active: Vec<&str> = Vec::new();
        if share {
            active.extend(SHARED_ITEMS);
        }
        if share_history {
            active.extend(HISTORY_ITEMS);
        }
        let scope: Vec<&str> = SHARED_ITEMS
            .iter()
            .chain(HISTORY_ITEMS.iter())
            .copied()
            .collect();
        self.sync_items(dir, &active, &scope);
    }

    /// Link `active` items from `~/.claude` into the profile and unlink managed ones no longer wanted.
    fn sync_items(&mut self, dir: &Path, active: &[&str], scope: &[&str]) {
        self.merge = MergeCounts::default();
        self.deferral_reported = false;
        self.own_history_found = false;
        let source_root = home().join(".claude");
        let manifest_path = dir.join(SHARE_MANIFEST);
        let managed = read_manifest(&manifest_path);
        let out_of_scope: Vec<String> = managed
            .iter()
            .filter(|n| !scope.contains(&n.as_str()))
            .cloned()
            .collect();
        self.clear_staged_links(dir, &source_root, scope);
        for name in &managed {
            if scope.contains(&name.as_str()) && !active.contains(&name.as_str()) {
                let dest = dir.join(name);
                if HISTORY_ITEMS.contains(&name.as_str()) && dest.exists() && !is_link(&dest) {
                    continue;
                }
                remove_managed(&dest);
            }
        }
        if active.is_empty() && out_of_scope.is_empty() {
            let _ = std::fs::remove_file(&manifest_path);
            return;
        }
        let mut new_managed = out_of_scope;
        for name in active {
            let src = source_root.join(name);
            let dest = dir.join(name);
            let link_target = if is_link(&src) {
                crate::fsutil::canonicalize(&src).unwrap_or_else(|_| src.clone())
            } else {
                src.clone()
            };
            let is_history = HISTORY_ITEMS.contains(name);
            if is_history && !self.prepare_history_share(&src, &dest, dir) {
                continue;
            }
            if !src.exists() {
                if managed.iter().any(|m| m == name) {
                    remove_managed(&dest);
                }
                continue;
            }
            if is_link(&dest) {
                let in_place = link_points_at(&dest, &link_target);
                let ours =
                    in_place || managed.iter().any(|m| m == name) || link_points_at(&dest, &src);
                if !ours {
                    println!(
                        "{}",
                        dimmed(&format!(
                            "Not sharing {name}: the session profile already has its own link."
                        ))
                    );
                    continue;
                }
                if in_place {
                    new_managed.push((*name).to_owned());
                    continue;
                }
                if remove_link(&dest).is_err() {
                    continue;
                }
            } else if dest.exists() && !managed.iter().any(|m| m == name) {
                println!(
                    "{}",
                    dimmed(&format!(
                        "Not sharing {name}: the session profile already has its own copy."
                    ))
                );
                continue;
            }
            if dest.exists() {
                remove_managed(&dest);
            }
            if let Err(e) = make_link(&link_target, &dest) {
                warn!("Failed to share {name} into session: {e}");
                continue;
            }
            new_managed.push((*name).to_owned());
        }
        write_manifest(&manifest_path, &new_managed);
        self.report_quarantine();
        self.report_sharing_is_the_default();
    }

    fn clear_staged_links(&self, dir: &Path, source_root: &Path, scope: &[&str]) {
        let leftovers: Vec<PathBuf> = HISTORY_ITEMS
            .iter()
            .filter(|n| scope.contains(n))
            .map(|n| dir.join(format!("{n}{STAGED_LINK_SUFFIX}")))
            .collect();
        if !leftovers.iter().any(|s| is_link(s)) {
            return;
        }
        let result = (|| -> Result<()> {
            mkdir_private(source_root)?;
            let _lock = FileLock::lock(
                source_root.join(MIGRATION_LOCK),
                STAGED_CLEANUP_LOCK_TIMEOUT,
            )?;
            for staged in leftovers.iter().filter(|s| is_link(s)) {
                remove_link(staged)?;
                info!(
                    "Removed the staging link an interrupted launch left at {}",
                    staged.display()
                );
            }
            Ok(())
        })();
        if let Err(e) = result {
            warn!("Could not clear a staging link in {}: {e}", dir.display());
        }
    }

    /// Before linking history: merge the profile's own copy into `~/.claude` (when nothing runs on it).
    fn prepare_history_share(&mut self, src: &Path, dest: &Path, dir: &Path) -> bool {
        if dest.exists() && !is_link(dest) {
            self.own_history_found = true;
            if !profile_is_quiescent(dir) {
                self.report_deferral(dir, dest, false);
                return false;
            }
            let parent = src.parent().unwrap_or(src).to_path_buf();
            let locked = mkdir_private(&parent)
                .map_err(Error::from)
                .and_then(|()| FileLock::lock(parent.join(MIGRATION_LOCK), MIGRATION_LOCK_TIMEOUT));
            match locked {
                Ok(_lock) => {
                    if !self.migrate_under_lock(src, dest, dir) {
                        return false;
                    }
                }
                Err(Error::Lock(_)) => {
                    self.report_deferral(dir, dest, true);
                    return false;
                }
                Err(e) => {
                    warn!(
                        "Could not lock {} to merge {}: {e}",
                        parent.display(),
                        file_name(dest)
                    );
                    println!(
                        "{}",
                        dimmed(&format!(
                            "Not sharing {}: the shared history could not be locked (see log).",
                            file_name(dest)
                        ))
                    );
                    return false;
                }
            }
        }
        seed_history_source(src, dest)
    }

    fn migrate_under_lock(&mut self, src: &Path, dest: &Path, dir: &Path) -> bool {
        if !dest.exists() || is_link(dest) {
            return true;
        }
        if !profile_is_quiescent(dir) {
            self.report_deferral(dir, dest, false);
            return false;
        }
        let name = file_name(dest);
        if !seed_history_source(src, dest) {
            println!(
                "{}",
                dimmed(&format!(
                    "Not sharing {name}: {} could not be created (see log).",
                    src.display()
                ))
            );
            return false;
        }
        let link_target = if is_link(src) {
            crate::fsutil::canonicalize(src).unwrap_or_else(|_| src.to_path_buf())
        } else {
            src.to_path_buf()
        };
        let staged = dest.with_file_name(format!("{name}{STAGED_LINK_SUFFIX}"));
        let staged_ok = (|| -> std::io::Result<()> {
            if is_link(&staged) {
                remove_link(&staged)?;
            } else if staged.exists() {
                return Err(std::io::Error::other(format!(
                    "{} exists and is not a link",
                    staged.display()
                )));
            }
            make_link(&link_target, &staged)
        })();
        if let Err(e) = staged_ok {
            warn!("Could not link {name} into the session: {e}");
            println!(
                "{}",
                dimmed(&format!(
                    "Not sharing {name}: it could not be linked here (see log) — the account keeps its own."
                ))
            );
            return false;
        }
        let mut counts = std::mem::take(&mut self.merge);
        let merged = merge_history_into_source(
            src,
            dest,
            self.store_root(),
            &self.sessions_dir,
            &mut counts,
        );
        self.merge = counts;
        if let Err(e) = merged {
            discard_staged_link(&staged);
            warn!("Could not merge {name} into {}: {e}", src.display());
            println!(
                "{}",
                dimmed(&format!(
                    "Not sharing {name}: merging the profile's existing history failed (see log)."
                ))
            );
            return false;
        }
        if let Err(rename_error) = replace_with_retry(
            &staged,
            dest,
            crate::fsutil::DEFAULT_RETRY_ATTEMPTS,
            crate::fsutil::INITIAL_RETRY_DELAY,
        ) {
            if let Err(e) = make_link(&link_target, dest) {
                discard_staged_link(&staged);
                warn!("Failed to share {name} into session: {rename_error}; then {e}");
                println!(
                    "{}",
                    dimmed(&format!(
                        "Merged the profile's existing {name} into {}, but linking it back failed (see log) — \
                         retrying on the next launch.",
                        src.display()
                    ))
                );
                return false;
            }
            discard_staged_link(&staged);
        }
        println!(
            "{}",
            dimmed(&format!(
                "Merged the profile's existing {name} into {} — conversation history is now shared.",
                src.display()
            ))
        );
        true
    }

    fn relaunch_command(&self, slot: &str) -> String {
        if load_run_settings(self.store_root()).share_history {
            format!("{AI_PROG} run {slot}")
        } else {
            format!("{AI_PROG} run {slot} --share-history")
        }
    }

    fn report_deferral(&mut self, dir: &Path, dest: &Path, lock_busy: bool) {
        let account = file_name(dir);
        let slot = account.split('-').next().unwrap_or_default().to_owned();
        let blockers = if lock_busy {
            Vec::new()
        } else {
            session::scan_live_sessions(dir).0
        };
        let why = if lock_busy {
            "the migration lock is held by a peer process".to_owned()
        } else if blockers.is_empty() {
            "a session record could not be read".to_owned()
        } else {
            let list: Vec<String> = blockers
                .iter()
                .map(|b| format!("pid {} ({})", b.pid, b.cwd))
                .collect();
            format!("{} live session(s): {}", blockers.len(), list.join(", "))
        };
        warn!(
            "History migration of {} deferred for {account}: {why}",
            file_name(dest)
        );
        if self.deferral_reported {
            return;
        }
        self.deferral_reported = true;
        warning(&format!(
            "share-history: {account} keeps its own {} for now.",
            file_name(dest)
        ));
        let retry = self.relaunch_command(&slot);
        if lock_busy {
            println!(
                "{}",
                dimmed(&format!(
                    "   Another {PROG} launch is merging history into ~/.claude right now."
                ))
            );
            println!(
                "{}",
                dimmed(&format!("   Once it finishes, relaunch:  {retry}"))
            );
        } else if !blockers.is_empty() {
            for live in &blockers {
                let since = jiff::Timestamp::from_millisecond(live.started_at)
                    .map(|t| {
                        t.to_zoned(jiff::tz::TimeZone::system())
                            .strftime("%Y-%m-%d %H:%M")
                            .to_string()
                    })
                    .unwrap_or_default();
                println!(
                    "{}",
                    dimmed(&format!(
                        "   pid {}  {}  (since {since})",
                        live.pid, live.cwd
                    ))
                );
            }
            println!(
                "{}",
                dimmed(
                    "   Sessions started here will not appear in `claude --resume` under other accounts."
                )
            );
            println!(
                "{}",
                dimmed(&format!("   Close them, then relaunch:  {retry}"))
            );
        } else {
            println!(
                "{}",
                dimmed(
                    "   A session record in this profile could not be read, so it may still be in use."
                )
            );
            println!(
                "{}",
                dimmed(&format!(
                    "   Close anything using it, then relaunch:  {retry}"
                ))
            );
        }
    }

    fn report_sharing_is_the_default(&self) {
        if !self.own_history_found || explicit_share_history(self.store_root()).is_some() {
            return;
        }
        println!(
            "{}",
            dimmed(&format!(
                "Chat history is shared between accounts by default now (turn off: {AI_PROG} config set run.shareHistory false)."
            ))
        );
    }

    fn report_quarantine(&self) {
        let (n, Some(dir)) = (self.merge.quarantined, self.merge.run_dir.as_ref()) else {
            return;
        };
        if n == 0 {
            return;
        }
        let noun = if n == 1 { "file" } else { "files" };
        println!(
            "{}",
            dimmed(&format!(
                "Kept {n} conflicting history {noun} aside in {} — nothing was deleted.",
                dir.display()
            ))
        );
        warn!(
            "History merge kept {n} conflicting {noun} in {}",
            dir.display()
        );
    }

    /// Mirror the default profile's MCP servers into the profile (removing them again on `--no-share`).
    fn sync_mcp_servers(&self, dir: &Path, share: bool) {
        let config_path = dir.join(".claude.json");
        let marker = dir.join(MCP_MIRROR_MARKER);
        let source = if share {
            match self.read_mcp_source() {
                Some(s) => s,
                None => return,
            }
        } else if marker.exists() {
            Map::new()
        } else {
            return;
        };
        if !config_path.exists() {
            return;
        }
        if is_link(&config_path) || !config_path.is_file() {
            warn!(
                "Not syncing MCP servers: {} is not a regular file.",
                config_path.display()
            );
            return;
        }
        let Some(existing) = load_json_object(&config_path) else {
            return;
        };
        let Some(target) = existing
            .get(MCP_KEY)
            .cloned()
            .unwrap_or_else(|| json!({}))
            .as_object()
            .cloned()
        else {
            warn!("Not syncing MCP servers: the profile's {MCP_KEY} is not an object.");
            return;
        };
        if target == source && (!share || marker.exists()) {
            return;
        }
        let lock_dir = dir.join(".claude.json.lock");
        let result = (|| -> Result<()> {
            let _lock = proper_lockfile(&lock_dir, None, CONFIG_STALENESS)?;
            let source = if share {
                match self.read_mcp_source() {
                    Some(s) => s,
                    None => return Ok(()),
                }
            } else {
                source.clone()
            };
            if is_link(&config_path) || !config_path.is_file() {
                return Ok(());
            }
            let Some(mut existing) = load_json_object(&config_path) else {
                return Ok(());
            };
            let Some(target) = existing
                .get(MCP_KEY)
                .cloned()
                .unwrap_or_else(|| json!({}))
                .as_object()
                .cloned()
            else {
                return Ok(());
            };
            if target == source {
                if share {
                    ensure_marker(&marker);
                }
                return Ok(());
            }
            if share && !marker.exists() {
                let displaced: Map<String, Value> = target
                    .into_iter()
                    .filter(|(k, v)| source.get(k) != Some(v))
                    .collect();
                if !displaced.is_empty() && !stash_displaced_mcp(dir, displaced) {
                    return Ok(());
                }
            }
            if source.is_empty() {
                existing.remove(MCP_KEY);
            } else {
                existing.insert(MCP_KEY.into(), Value::Object(source));
            }
            if let Err(e) = atomic_write_json(&config_path, &Value::Object(existing)) {
                warn!("Could not sync MCP servers: {e}");
                return Ok(());
            }
            if share {
                ensure_marker(&marker);
            }
            Ok(())
        })();
        if let Err(e) = result {
            warn!("Could not sync MCP servers ({e}) — skipping this launch.");
        }
    }

    fn read_mcp_source(&self) -> Option<Map<String, Value>> {
        let config = load_json_object(&paths::default_global_config_path_in(
            self.switcher.path_env(),
        ))?;
        match config.get(MCP_KEY) {
            None => Some(Map::new()),
            Some(Value::Object(o)) => Some(o.clone()),
            Some(_) => None,
        }
    }
}

fn live_login_error(num: &str) -> Error {
    Error::Session(format!(
        "Account-{num}'s stored login is the live login right now, so a session would be a second copy of the same \
         login. Run '{PROG} --switch-to {num}' so {PROG} treats it as the active account, then start claude normally."
    ))
}

fn home() -> PathBuf {
    paths::home_dir().unwrap_or_else(|_| PathBuf::from("~"))
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn ensure_marker(marker: &Path) {
    if !marker.exists()
        && let Err(e) = std::fs::write(marker, "")
    {
        warn!("Could not write {}: {e}", file_name(marker));
    }
}

fn stash_displaced_mcp(dir: &Path, displaced: Map<String, Value>) -> bool {
    let stash = dir.join(MCP_DISPLACED_STASH);
    if is_link(&stash) || stash.exists() {
        let valid = !is_link(&stash)
            && stash.is_file()
            && load_json_object(&stash)
                .is_some_and(|d| d.get(MCP_KEY).is_some_and(Value::is_object));
        if !valid {
            warn!(
                "{MCP_DISPLACED_STASH} exists but is not a valid stash; leaving the profile's MCP servers in place."
            );
        }
        return valid;
    }
    if let Err(e) = atomic_write_json(&stash, &json!({"schemaVersion": 1, MCP_KEY: displaced})) {
        warn!("Could not stash the profile's MCP servers ({e}); leaving them in place.");
        return false;
    }
    println!(
        "{}",
        dimmed(&format!(
            "Session MCP servers now mirror your default profile; the profile's previous definitions were saved to \
             {MCP_DISPLACED_STASH}."
        ))
    );
    true
}

fn seed_history_source(src: &Path, dest: &Path) -> bool {
    if src.exists() {
        return true;
    }
    let made = if file_name(dest).ends_with(".jsonl") {
        src.parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::write(src, ""))
            .map(|()| set_mode(src, 0o600))
    } else {
        mkdir_private(src)
    };
    if let Err(e) = made {
        warn!("Could not create {}: {e}", src.display());
        return false;
    }
    true
}

fn discard_staged_link(staged: &Path) {
    if let Err(e) = remove_link(staged) {
        warn!("Could not remove {}: {e}", staged.display());
    }
}

fn read_manifest(path: &Path) -> Vec<String> {
    load_json_object(path)
        .and_then(|d| d.get("items").and_then(Value::as_array).cloned())
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .filter(|i| all_shareable(i))
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn write_manifest(path: &Path, items: &[String]) {
    let text = crate::models::pyfmt::json_dumps_pretty(&json!({"items": items, "mode": "symlink"}));
    let _ = atomic_write_text(path, &text, WriteOptions::default());
}

fn remove_managed(dest: &Path) {
    if is_link(dest) {
        let _ = remove_link(dest);
    } else if dest.is_file() {
        let _ = std::fs::remove_file(dest);
    } else if dest.is_dir() {
        let _ = std::fs::remove_dir_all(dest);
    }
}

/// The link for `target` at `dest` (upstream `_make_link`): a symlink, except that a directory on Windows
/// gets a junction. A file link needs Developer Mode on Windows; a failure is reported by the caller and
/// the account keeps its own copy.
fn make_link(target: &Path, dest: &Path) -> std::io::Result<()> {
    crate::fsutil::make_link(target, dest)
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode));
}

#[cfg(not(unix))]
fn set_mode(_path: &Path, _mode: u32) {}

/// While alive, this process's stdout goes to stderr: stdout belongs to `claude`, and a caller may read it
/// as data (`haryx run 2 -- -p ... --output-format stream-json`). [`exec`] restores the real stdout first.
pub struct StdoutToStderr {
    #[cfg(unix)]
    saved: Option<i32>,
}

impl StdoutToStderr {
    pub fn new() -> Self {
        use std::io::Write;
        let _ = std::io::stdout().flush();
        #[cfg(unix)]
        {
            // SAFETY: dup/dup2 on the process's own standard descriptors; failures leave stdout as is.
            let saved = unsafe {
                let saved = libc::dup(1);
                if saved >= 0 && libc::dup2(2, 1) >= 0 {
                    Some(saved)
                } else {
                    None
                }
            };
            StdoutToStderr { saved }
        }
        #[cfg(not(unix))]
        StdoutToStderr {}
    }

    fn restore(&self) {
        use std::io::Write;
        let _ = std::io::stdout().flush();
        #[cfg(unix)]
        if let Some(fd) = self.saved {
            // SAFETY: puts back the descriptor duplicated in `new`.
            unsafe {
                libc::dup2(fd, 1);
            }
        }
    }
}

impl Default for StdoutToStderr {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for StdoutToStderr {
    fn drop(&mut self) {
        self.restore();
    }
}

thread_local! {
    static REDIRECT: std::cell::RefCell<Option<StdoutToStderr>> = const { std::cell::RefCell::new(None) };
}

/// Send this thread's stdout notices to stderr until `claude` is started (or the process ends).
pub fn redirect_stdout_until_exec() {
    REDIRECT.with(|r| *r.borrow_mut() = Some(StdoutToStderr::new()));
}

/// Replace this process with `claude` (`set` added to and `remove` taken from the environment).
fn exec(claude: &Path, args: &[String], set: &[(&str, OsString)], remove: &[&str]) -> Result<()> {
    REDIRECT.with(|r| {
        if let Some(guard) = r.borrow_mut().take() {
            drop(guard);
        }
    });
    let mut command = std::process::Command::new(claude);
    command.args(args);
    for name in remove {
        command.env_remove(name);
    }
    for (k, v) in set {
        command.env(k, v);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let err = command.exec();
        Err(Error::Session(format!(
            "could not start {}: {err}",
            claude.display()
        )))
    }
    #[cfg(not(unix))]
    {
        let status = command.status()?;
        std::process::exit(status.code().unwrap_or(1));
    }
}

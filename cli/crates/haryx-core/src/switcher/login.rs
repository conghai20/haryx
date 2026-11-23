//! `login`: sign a slot in (again) through Claude Code's own `claude auth login`, then store that login.
//!
//! haryx never handles the browser sign-in itself; Claude Code does, unmodified. Two paths:
//!
//! - the slot is the active default login: `claude auth login` runs on the default profile and the
//!   result is captured exactly as `add --slot N` captures it;
//! - anything else (another slot, or a new account): `claude auth login` runs in a throwaway staging
//!   profile under the store, so the active login is never touched. The staging credential is checked
//!   against the slot's identity, written into the slot backup, and the staging profile (directory and
//!   its hashed Keychain item) is deleted.
//!
//! A login that signed in as a different account than the slot expects changes nothing.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

use log::info;
use serde_json::{Map, Value, json};

use super::add::{account, sequence_insert_sorted, text};
use super::{JsonObject, Switcher};
use crate::credentials::looks_like_api_key;
use crate::error::{Error, Result};
use crate::models::{get_timestamp, normalize_alias};
use crate::printer::{accent, dimmed, muted};
use crate::session;
use crate::session_manager::{AUTH_OVERRIDE_ENV_VARS, SessionManager};

/// Store subdirectory that holds staging profiles while a login runs.
const STAGING_DIR: &str = "login-staging";
/// Domain of the placeholder emails `add-token` gives setup-token and API-key slots.
const TOKEN_EMAIL_DOMAIN: &str = "@token.local";

/// What `haryx login` was asked to do.
#[derive(Debug, Clone, Default)]
pub struct LoginRequest {
    /// `NUM|EMAIL|ALIAS` of a managed slot to sign in again; `None` adds a new account.
    pub target: Option<String>,
    /// Where a new account lands (`--slot`); ignored with a target.
    pub slot: Option<i64>,
    /// Display alias for a new account.
    pub alias: Option<String>,
    /// `--sso`: passed through to `claude auth login`.
    pub sso: bool,
    /// Skip the overwrite confirmation when `--slot` displaces another account.
    pub assume_yes: bool,
}

/// What a finished login stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginOutcome {
    pub account_num: String,
    pub email: String,
    /// `true` when an existing slot was signed in again, `false` for a new slot.
    pub relogin: bool,
}

/// The account a fresh login belongs to, read from the profile Claude Code just wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
struct SignedIn {
    email: String,
    account_uuid: String,
    org_uuid: String,
    org_name: String,
    oauth_account: Value,
}

/// The managed slot a re-login targets.
struct Target {
    num: String,
    email: String,
    org: String,
}

/// A staging profile, removed (directory and Keychain item) when dropped.
struct Staging<'a> {
    switcher: &'a Switcher,
    dir: PathBuf,
}

impl Drop for Staging<'_> {
    fn drop(&mut self) {
        self.switcher.profile_keychain().delete_entry(&self.dir);
        if let Err(err) = std::fs::remove_dir_all(&self.dir)
            && err.kind() != std::io::ErrorKind::NotFound
        {
            log::warn!(
                "Could not remove the login staging profile at {}: {err}",
                self.dir.display()
            );
        }
    }
}

impl Switcher {
    /// `login [NUM|EMAIL]`: run Claude Code's sign-in and store the result. See the module docs.
    pub fn login_account(&self, req: &LoginRequest) -> Result<Option<LoginOutcome>> {
        self.prepare_store()?;
        let alias = req.alias.as_deref().map(normalize_alias).transpose()?;
        let target = req
            .target
            .as_deref()
            .map(|t| self.login_target(t))
            .transpose()?;
        let claude = SessionManager::claude_bin()?;

        if let Some(target) = &target
            && self.get_current_account() == Some((target.email.clone(), target.org.clone()))
        {
            return self.relogin_default(&claude, target, req.sso).map(Some);
        }

        let staging = self.new_staging()?;
        let hint = target.as_ref().map(|t| t.email.as_str());
        run_claude_login(&claude, Some(&staging.dir), hint, req.sso)?;
        let creds = self.read_staged_credentials(&staging.dir)?;
        let signed_in = read_signed_in(&staging.dir)?;

        match &target {
            Some(target) => {
                ensure_same_account(target, &signed_in)?;
                self.store_relogin(&target.num, &signed_in, &creds)?;
                Ok(Some(self.report(&target.num, &signed_in, true)))
            }
            None => self.store_new_login(req, alias.as_deref(), &signed_in, &creds),
        }
    }

    /// Resolve and vet the slot a re-login targets: token and API-key slots have no login to renew.
    fn login_target(&self, identifier: &str) -> Result<Target> {
        let (num, email, org) = self.resolve_account(identifier)?;
        let data = self.sequence()?;
        let is_api_key = text(account(&data, &num), "kind") == "api_key";
        if is_api_key || email.ends_with(TOKEN_EMAIL_DOMAIN) {
            return Err(Error::Validation(format!(
                "Account-{num} holds a token, not a Claude login, so it cannot be signed in again. \
                 Replace it with a login: {} login --slot {num}",
                crate::PROG
            )));
        }
        Ok(Target { num, email, org })
    }

    /// The slot is the active default login: sign in on the default profile, then capture it as
    /// `add --slot N` does (which verifies the identity before anything is written).
    fn relogin_default(&self, claude: &Path, target: &Target, sso: bool) -> Result<LoginOutcome> {
        run_claude_login(claude, None, Some(&target.email), sso)?;
        let now = self.get_current_account();
        if now != Some((target.email.clone(), target.org.clone())) {
            let who = now.map_or_else(|| "nobody".to_owned(), |(email, _)| email);
            return Err(Error::Validation(format!(
                "Claude Code is now signed in as {who}, but Account-{} is {}. Account-{} was not \
                 changed. To keep {who}, add it with: {} add",
                target.num,
                target.email,
                target.num,
                crate::PROG
            )));
        }
        let slot = target.num.parse::<i64>().ok();
        self.add_account(slot, None, true)?;
        Ok(LoginOutcome {
            account_num: target.num.clone(),
            email: target.email.clone(),
            relogin: true,
        })
    }

    fn new_staging(&self) -> Result<Staging<'_>> {
        let root = self.backup_dir.join(STAGING_DIR);
        std::fs::create_dir_all(&root)?;
        let stamp = jiff::Timestamp::now().as_nanosecond();
        let dir = root.join(format!("{}-{stamp}", std::process::id()));
        std::fs::create_dir(&dir)?;
        set_private(&dir);
        Ok(Staging {
            switcher: self,
            dir,
        })
    }

    /// The credential the sign-in left in the staging profile; a strict read, so a locked Keychain
    /// fails loudly instead of reading nothing.
    fn read_staged_credentials(&self, dir: &Path) -> Result<String> {
        let creds = session::read_config_dir_credentials(
            &self.profile_keychain(),
            dir.as_os_str(),
            true,
            None,
        )?
        .filter(|c| !c.trim().is_empty())
        .ok_or_else(|| {
            Error::CredentialRead(
                "Claude Code finished, but no login was saved. Was the sign-in completed?".into(),
            )
        })?;
        if looks_like_api_key(Some(creds.trim())) {
            return Err(Error::Validation(
                "That sign-in produced an API key, not a Claude subscription login. Use \
                 'add-token' for API keys."
                    .into(),
            ));
        }
        Ok(creds)
    }

    /// Write a fresh login into an existing slot: credential, the config backup's `oauthAccount`
    /// (other config kept), the identity fields of the roster row, and clear its dead-token strikes.
    fn store_relogin(&self, num: &str, who: &SignedIn, creds: &str) -> Result<()> {
        let existing = self
            .read_account_config(num, &who.email)
            .unwrap_or_default();
        let config = merged_config(&existing, &who.oauth_account);
        self.write_account_credentials(num, &who.email, creds)?;
        self.write_account_config(num, &who.email, &config)?;
        self.clear_dead(num, &who.email, &who.org_uuid);

        let mut data = self.sequence()?;
        if let Some(record) = data
            .get_mut("accounts")
            .and_then(|a| a.get_mut(num))
            .and_then(Value::as_object_mut)
        {
            record.insert("uuid".into(), json!(who.account_uuid));
            record.insert("organizationName".into(), json!(who.org_name));
        }
        data.insert("lastUpdated".into(), json!(get_timestamp()));
        self.write_json(&self.sequence_file, &data)?;
        info!("Signed in again: account {num} ({})", who.email);
        Ok(())
    }

    /// A new account (or a login for one already managed, which updates that slot). Never changes
    /// which account is active.
    fn store_new_login(
        &self,
        req: &LoginRequest,
        alias: Option<&str>,
        who: &SignedIn,
        creds: &str,
    ) -> Result<Option<LoginOutcome>> {
        if req.slot.is_none() && self.account_exists(&who.email, &who.org_uuid)? {
            let data = self.sequence()?;
            let num =
                Self::find_account_slot(&data, &who.email, &who.org_uuid).ok_or_else(|| {
                    Error::Config(format!(
                        "Existing account metadata for {} is inconsistent",
                        who.email
                    ))
                })?;
            self.store_relogin(&num, who, creds)?;
            return Ok(Some(self.report(&num, who, true)));
        }
        let Some(placement) = self.place(req.slot, &who.email, &who.org_uuid, req.assume_yes)?
        else {
            return Ok(None);
        };
        if let Some(alias) = alias
            && let Some(conflict) = self.alias_in_use(alias, Some(&placement.num))?
        {
            return Err(Error::Validation(format!(
                "Alias '{alias}' is already used by account {conflict}"
            )));
        }
        self.vacate(&placement)?;
        let num = placement.num.clone();
        let config = merged_config("", &who.oauth_account);
        self.write_account_credentials(&num, &who.email, creds)?;
        self.write_account_config(&num, &who.email, &config)?;
        self.clear_dead(&num, &who.email, &who.org_uuid);

        let mut record = Map::new();
        record.insert("email".into(), json!(who.email));
        record.insert("uuid".into(), json!(who.account_uuid));
        record.insert("organizationUuid".into(), json!(who.org_uuid));
        record.insert("organizationName".into(), json!(who.org_name));
        record.insert("added".into(), json!(get_timestamp()));
        record.extend(placement.carried.clone());
        if let Some(alias) = alias {
            record.insert("alias".into(), json!(alias));
        }
        let mut data = self.sequence()?;
        data.entry("accounts")
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .map(|a| a.insert(num.clone(), Value::Object(record)));
        sequence_insert_sorted(&mut data, &num);
        data.insert("lastUpdated".into(), json!(get_timestamp()));
        self.write_json(&self.sequence_file, &data)?;
        info!("Added account {num} by login: {}", who.email);
        Ok(Some(self.report(&num, who, false)))
    }

    fn report(&self, num: &str, who: &SignedIn, relogin: bool) -> LoginOutcome {
        let tag = Self::get_display_tag(&who.email, &who.org_name, &who.org_uuid);
        let verb = if relogin { "Signed in" } else { "Added" };
        println!(
            "{} Account {num}: {} {}",
            accent(verb),
            who.email,
            muted(&format!("[{tag}]"))
        );
        if !relogin {
            println!(
                "{}",
                dimmed("The active login was not changed. Switch to it with: haryx ai switch")
            );
        }
        LoginOutcome {
            account_num: num.to_owned(),
            email: who.email.clone(),
            relogin,
        }
    }
}

/// Run `claude auth login` with the terminal attached. `config_dir: None` keeps the default profile.
fn run_claude_login(
    claude: &Path,
    config_dir: Option<&Path>,
    email: Option<&str>,
    sso: bool,
) -> Result<()> {
    let mut command = Command::new(claude);
    command.args(login_args(email, sso));
    for var in AUTH_OVERRIDE_ENV_VARS {
        command.env_remove(var);
    }
    match config_dir {
        Some(dir) => command.env("CLAUDE_CONFIG_DIR", OsString::from(dir)),
        None => command.env_remove("CLAUDE_CONFIG_DIR"),
    };
    let status = command
        .status()
        .map_err(|e| Error::Session(format!("Could not start Claude Code: {e}")))?;
    if !status.success() {
        return Err(Error::Session(
            "Sign-in did not finish (Claude Code exited with an error). Nothing was changed."
                .into(),
        ));
    }
    Ok(())
}

/// Arguments of `claude auth login`: always the subscription flow, the expected email pre-filled.
fn login_args(email: Option<&str>, sso: bool) -> Vec<String> {
    let mut args = vec![
        "auth".to_owned(),
        "login".to_owned(),
        "--claudeai".to_owned(),
    ];
    if let Some(email) = email.filter(|e| !e.is_empty()) {
        args.extend(["--email".to_owned(), email.to_owned()]);
    }
    if sso {
        args.push("--sso".to_owned());
    }
    args
}

/// The identity Claude Code wrote into the staging profile's `.claude.json`.
fn read_signed_in(dir: &Path) -> Result<SignedIn> {
    let missing = || {
        Error::Config(
            "Claude Code saved a login but no account details, so it cannot be matched to a slot. \
             Nothing was changed."
                .into(),
        )
    };
    let text = std::fs::read_to_string(dir.join(".claude.json")).map_err(|_| missing())?;
    let config: Value = serde_json::from_str(&text).map_err(|_| missing())?;
    let oauth_account = config
        .get("oauthAccount")
        .filter(|v| v.is_object())
        .cloned()
        .ok_or_else(missing)?;
    let field = |k: &str| {
        oauth_account
            .get(k)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    let email = field("emailAddress");
    if email.is_empty() {
        return Err(missing());
    }
    Ok(SignedIn {
        email,
        account_uuid: field("accountUuid"),
        org_uuid: field("organizationUuid"),
        org_name: field("organizationName"),
        oauth_account,
    })
}

/// Refuse a sign-in as some other account: the email must match, and the org when both sides have one.
fn ensure_same_account(target: &Target, who: &SignedIn) -> Result<()> {
    let org_differs =
        !target.org.is_empty() && !who.org_uuid.is_empty() && target.org != who.org_uuid;
    if who.email == target.email && !org_differs {
        return Ok(());
    }
    Err(Error::Validation(format!(
        "You signed in as {}, but Account-{} is {}. Account-{} was not changed. To add {} as its own \
         account, run: {} login",
        who.email,
        target.num,
        target.email,
        target.num,
        who.email,
        crate::PROG
    )))
}

/// A config backup with `oauthAccount` replaced (other keys kept); an unreadable backup starts empty.
fn merged_config(existing: &str, oauth_account: &Value) -> String {
    let mut config: JsonObject = serde_json::from_str(existing).unwrap_or_default();
    config.insert("oauthAccount".into(), oauth_account.clone());
    Value::Object(config).to_string()
}

#[cfg(unix)]
fn set_private(dir: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
}

#[cfg(not(unix))]
fn set_private(_dir: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(email: &str, org: &str) -> Target {
        Target {
            num: "3".into(),
            email: email.into(),
            org: org.into(),
        }
    }

    fn signed_in(email: &str, org: &str) -> SignedIn {
        SignedIn {
            email: email.into(),
            account_uuid: "u".into(),
            org_uuid: org.into(),
            org_name: String::new(),
            oauth_account: json!({}),
        }
    }

    #[test]
    fn login_args_prefill_email_and_pass_sso() {
        assert_eq!(
            login_args(Some("a@x.com"), true),
            ["auth", "login", "--claudeai", "--email", "a@x.com", "--sso"]
        );
        assert_eq!(login_args(None, false), ["auth", "login", "--claudeai"]);
        assert_eq!(login_args(Some(""), false), ["auth", "login", "--claudeai"]);
    }

    #[test]
    fn same_account_requires_email_and_known_org_to_match() {
        assert!(ensure_same_account(&target("a@x.com", "o1"), &signed_in("a@x.com", "o1")).is_ok());
        assert!(ensure_same_account(&target("a@x.com", ""), &signed_in("a@x.com", "o1")).is_ok());
        assert!(
            ensure_same_account(&target("a@x.com", "o1"), &signed_in("b@x.com", "o1")).is_err()
        );
        assert!(
            ensure_same_account(&target("a@x.com", "o1"), &signed_in("a@x.com", "o2")).is_err()
        );
    }

    #[test]
    fn merged_config_replaces_only_oauth_account() {
        let merged = merged_config(
            r#"{"theme":"light","oauthAccount":{"emailAddress":"old"}}"#,
            &json!({"emailAddress": "new"}),
        );
        let v: Value = serde_json::from_str(&merged).unwrap();
        assert_eq!(v["theme"], "light");
        assert_eq!(v["oauthAccount"]["emailAddress"], "new");
        let fresh: Value =
            serde_json::from_str(&merged_config("not json", &json!({"emailAddress": "n"})))
                .unwrap();
        assert_eq!(fresh["oauthAccount"]["emailAddress"], "n");
    }

    #[test]
    fn signed_in_identity_is_read_from_the_profile() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(".claude.json"),
            r#"{"oauthAccount":{"emailAddress":"a@x.com","accountUuid":"u1","organizationUuid":"o1","organizationName":"Team"}}"#,
        )
        .unwrap();
        let who = read_signed_in(dir.path()).unwrap();
        assert_eq!(
            (
                who.email.as_str(),
                who.org_uuid.as_str(),
                who.org_name.as_str()
            ),
            ("a@x.com", "o1", "Team")
        );
        std::fs::write(dir.path().join(".claude.json"), r#"{"oauthAccount":null}"#).unwrap();
        assert!(read_signed_in(dir.path()).is_err());
    }
}

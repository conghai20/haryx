//! Adding and removing accounts: upstream `add_account`, `add_account_from_token`, `remove_account`.
//!
//! A live credential enters a slot only when it is known to belong there: the capture path rejects
//! API keys, foreign credentials and identity/credential drift between the verify and the write.

use std::io::{BufRead, Write};

use log::info;
use serde_json::{Map, Value, json};

use super::{JsonObject, Switcher};
use crate::credentials::looks_like_api_key;
use crate::error::{Error, Result};
use crate::models::{get_timestamp, normalize_alias};
use crate::oauth::SETUP_TOKEN_SCOPES;
use crate::printer::{accent, dimmed, muted, warning};
use crate::usage_store::Identities;

/// Ask a question on stdout and read one line; `None` on EOF or a read error.
pub fn prompt(question: &str) -> Option<String> {
    print!("{question}");
    std::io::stdout().flush().ok()?;
    let mut line = String::new();
    match std::io::stdin().lock().read_line(&mut line) {
        Ok(0) | Err(_) => None,
        Ok(_) => Some(line.trim_end_matches(['\n', '\r']).to_owned()),
    }
}

/// Read a secret from the terminal without echo (upstream `getpass.getpass`).
pub fn prompt_secret(question: &str) -> Option<String> {
    eprint!("{question}");
    let _ = std::io::stderr().flush();
    let _echo = EchoOff::new();
    let mut line = String::new();
    let read = std::io::stdin().lock().read_line(&mut line);
    eprintln!();
    match read {
        Ok(0) | Err(_) => None,
        Ok(_) => Some(line.trim_end_matches(['\n', '\r']).to_owned()),
    }
}

/// Terminal echo disabled for its lifetime (no-op when stdin is not a terminal).
struct EchoOff {
    #[cfg(unix)]
    saved: Option<libc::termios>,
}

impl EchoOff {
    #[cfg(unix)]
    fn new() -> Self {
        // SAFETY: tcgetattr/tcsetattr on fd 0 with a zeroed, properly sized termios; failures are
        // reported through the return code and leave the terminal untouched.
        unsafe {
            let mut term: libc::termios = std::mem::zeroed();
            if libc::tcgetattr(0, &mut term) != 0 {
                return EchoOff { saved: None };
            }
            let saved = term;
            term.c_lflag &= !libc::ECHO;
            if libc::tcsetattr(0, libc::TCSANOW, &term) != 0 {
                return EchoOff { saved: None };
            }
            EchoOff { saved: Some(saved) }
        }
    }

    #[cfg(not(unix))]
    fn new() -> Self {
        EchoOff {}
    }
}

impl Drop for EchoOff {
    fn drop(&mut self) {
        #[cfg(unix)]
        if let Some(saved) = self.saved {
            // SAFETY: restores the attributes read in `new` on the same fd.
            unsafe {
                libc::tcsetattr(0, libc::TCSANOW, &saved);
            }
        }
    }
}

fn confirm_overwrite(slot: i64) -> bool {
    match prompt(&format!("Overwrite slot {slot}? [y/N] ")) {
        None => {
            println!("\n{}", dimmed("Cancelled"));
            false
        }
        Some(answer) => {
            let yes = matches!(answer.trim().to_lowercase().as_str(), "y" | "yes");
            if !yes {
                println!("{}", dimmed("Cancelled"));
            }
            yes
        }
    }
}

pub(super) fn account<'a>(data: &'a JsonObject, num: &str) -> Option<&'a JsonObject> {
    data.get("accounts")?.get(num)?.as_object()
}

pub(super) fn text(record: Option<&JsonObject>, key: &str) -> String {
    record
        .and_then(|r| r.get(key))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

fn sequence_remove(data: &mut JsonObject, num: &str) {
    let n: i64 = num.parse().unwrap_or(-1);
    if let Some(seq) = data.get_mut("sequence").and_then(Value::as_array_mut) {
        seq.retain(|v| v.as_i64() != Some(n));
    }
}

pub(super) fn sequence_insert_sorted(data: &mut JsonObject, num: &str) {
    let n: i64 = num.parse().unwrap_or(0);
    let seq = data
        .entry("sequence")
        .or_insert_with(|| json!([]))
        .as_array_mut();
    if let Some(seq) = seq
        && !seq.iter().any(|v| v.as_i64() == Some(n))
    {
        seq.push(json!(n));
        seq.sort_by_key(|v| v.as_i64().unwrap_or(0));
    }
}

/// Where a capture lands: `(slot, slot to displace, slot to migrate from, carried settings)`.
pub(super) struct Placement {
    pub(super) num: String,
    displace: Option<(String, String, String)>,
    migrate_from: Option<String>,
    pub(super) carried: JsonObject,
}

/// What [`Switcher::store_token_account`] stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenStored {
    pub num: String,
    pub email: String,
    /// `token` or `API key`.
    pub kind_label: &'static str,
    /// An existing account with this email got the new token.
    pub updated: bool,
    /// `"3 → 5"` when the account moved slot.
    pub moved_from: Option<String>,
}

impl Switcher {
    pub(super) fn prepare_store(&self) -> Result<()> {
        self.refuse_session_shell()?;
        self.setup_directories()?;
        self.init_sequence_file()?;
        self.migrate_org_fields()
    }

    pub(super) fn sequence(&self) -> Result<JsonObject> {
        Ok(self.get_sequence_data()?.unwrap_or_default())
    }

    /// Resolve where an add lands for an explicit `slot` (or the next free number), asking before it
    /// displaces another account. `Ok(None)`: the person cancelled.
    pub(super) fn place(
        &self,
        slot: Option<i64>,
        email: &str,
        org_uuid: &str,
        assume_yes: bool,
    ) -> Result<Option<Placement>> {
        let Some(slot) = slot else {
            return Ok(Some(Placement {
                num: self.get_next_account_number()?.to_string(),
                displace: None,
                migrate_from: None,
                carried: Map::new(),
            }));
        };
        if slot < 1 {
            return Err(Error::Config("Slot number must be >= 1".into()));
        }
        let num = slot.to_string();
        let data = self.sequence()?;
        let migrate_from = if self.account_exists(email, org_uuid)? {
            Self::find_account_slot(&data, email, org_uuid).filter(|old| *old != num)
        } else {
            None
        };
        let mut displace = None;
        if let Some(existing) = account(&data, &num) {
            let existing_email = existing
                .get("email")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_owned();
            let existing_org = text(Some(existing), "organizationUuid");
            if existing_email != email || existing_org != org_uuid {
                let tag = Self::get_display_tag(
                    &existing_email,
                    &text(Some(existing), "organizationName"),
                    &existing_org,
                );
                warning(&format!("Slot {slot} already occupied"));
                println!("{existing_email} {}", muted(&format!("[{tag}]")));
                if !assume_yes && !confirm_overwrite(slot) {
                    return Ok(None);
                }
                displace = Some((num.clone(), existing_email, existing_org));
            }
        }
        let prior = account(&data, &num);
        let mut carried =
            if text(prior, "email") == email && text(prior, "organizationUuid") == org_uuid {
                Self::carried_settings(prior)
            } else {
                Map::new()
            };
        if let Some(old) = &migrate_from {
            carried.extend(Self::carried_settings(account(&data, old)));
        }
        Ok(Some(Placement {
            num,
            displace,
            migrate_from,
            carried,
        }))
    }

    /// Drop a displaced or migrated slot's files and roster row.
    pub(super) fn vacate(&self, placement: &Placement) -> Result<()> {
        if let Some((d_num, d_email, d_org)) = &placement.displace {
            self.delete_account_files(d_num, d_email)?;
            let mut data = self.sequence()?;
            sequence_remove(&mut data, d_num);
            if let Some(accounts) = data.get_mut("accounts").and_then(Value::as_object_mut) {
                accounts.remove(d_num);
            }
            self.write_json(&self.sequence_file, &data)?;
            self.prune_mappings(d_email, d_org)?;
        }
        if let Some(old) = &placement.migrate_from {
            let mut data = self.sequence()?;
            let old_email = text(account(&data, old), "email");
            self.delete_account_files(old, &old_email)?;
            sequence_remove(&mut data, old);
            if let Some(accounts) = data.get_mut("accounts").and_then(Value::as_object_mut) {
                accounts.remove(old);
            }
            self.write_json(&self.sequence_file, &data)?;
        }
        Ok(())
    }

    pub(super) fn clear_dead(&self, num: &str, email: &str, org_uuid: &str) {
        let ident: Identities = [(num.to_owned(), (email.to_owned(), org_uuid.to_owned()))].into();
        if let Err(err) = self.usage_store.clear_dead_token(&[num.to_owned()], &ident) {
            log::debug!("clear_dead_token failed for {num}: {err}");
        }
    }

    /// The live credential and config, after every capture guard.
    fn capture(&self, email: &str, org_uuid: &str, account_uuid: &str) -> Result<(String, String)> {
        let creds = match self.read_capture_credentials()? {
            None => {
                return Err(Error::CredentialRead(
                    "Failed to read credentials for current account".into(),
                ));
            }
            Some(c) if c.is_empty() => {
                return Err(Error::CredentialRead(
                    "No credentials found for current account".into(),
                ));
            }
            Some(c) => c,
        };
        self.reject_live_api_key_capture(&creds)?;
        let creds =
            self.reject_foreign_credential_capture(&creds, email, org_uuid, account_uuid)?;
        self.reject_credential_drift_since_verify(&creds)?;
        let config = match std::fs::read_to_string(self.claude_config_path()) {
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
        Ok((creds, config))
    }

    /// `add`: capture the live login into a slot (updating it when it is already managed).
    pub fn add_account(
        &self,
        slot: Option<i64>,
        alias: Option<&str>,
        assume_yes: bool,
    ) -> Result<()> {
        self.prepare_store()?;
        let alias = alias.map(normalize_alias).transpose()?;
        let identity = self.get_current_identity_triple().ok_or_else(|| {
            Error::Config("No active Claude account found. Please log in first.".into())
        })?;
        let (email, org_uuid, account_uuid) = identity.clone();

        if slot.is_none() && self.account_exists(&email, &org_uuid)? {
            return self.update_existing(&identity, alias.as_deref());
        }
        let Some(placement) = self.place(slot, &email, &org_uuid, assume_yes)? else {
            return Ok(());
        };
        if let Some(alias) = &alias
            && let Some(conflict) = self.alias_in_use(alias, Some(&placement.num))?
        {
            return Err(Error::Validation(format!(
                "Alias '{alias}' is already used by account {conflict}"
            )));
        }
        let (creds, config) = self.capture(&email, &org_uuid, &account_uuid)?;
        let config_data = self
            .read_json(&self.claude_config_path(), true)?
            .ok_or_else(|| Error::Config("Claude config file not found".into()))?;
        let oauth_account = config_data.get("oauthAccount");
        let field = |k: &str| {
            oauth_account
                .and_then(|o| o.get(k))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        let (acct_uuid, org, org_name) = (
            field("accountUuid"),
            field("organizationUuid"),
            field("organizationName"),
        );
        self.reject_identity_drift_since_verify(&identity)?;

        self.vacate(&placement)?;
        let num = placement.num.clone();
        self.write_account_credentials(&num, &email, &creds)?;
        self.write_account_config(&num, &email, &config)?;
        self.clear_dead(&num, &email, &org);

        let mut data = self.sequence()?;
        let mut record = Map::new();
        record.insert("email".into(), json!(email));
        record.insert("uuid".into(), json!(acct_uuid));
        record.insert("organizationUuid".into(), json!(org));
        record.insert("organizationName".into(), json!(org_name));
        record.insert("added".into(), json!(get_timestamp()));
        let mut carried = placement.carried.clone();
        if let Some(alias) = &alias {
            carried.insert("alias".into(), json!(alias)); // an explicit --alias wins over the carried one
        }
        record.extend(carried);
        data.entry("accounts")
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .map(|a| a.insert(num.clone(), Value::Object(record)));
        sequence_insert_sorted(&mut data, &num);
        data.insert(
            "activeAccountNumber".into(),
            json!(num.parse::<i64>().unwrap_or_default()),
        );
        data.insert("lastUpdated".into(), json!(get_timestamp()));
        self.write_json(&self.sequence_file, &data)?;

        let tag = Self::get_display_tag(&email, &org_name, &org);
        info!(
            "Added account {num}: {email} (org: {})",
            if org.is_empty() { "personal" } else { &org }
        );
        if let (Some(from), Some(slot)) = (&placement.migrate_from, slot) {
            println!("{}", dimmed(&format!("Moved from slot {from} → {slot}")));
        }
        println!(
            "{} Account {num}: {email} {}",
            accent("Added"),
            muted(&format!("[{tag}]"))
        );
        Ok(())
    }

    /// Re-capture an already managed login into its own slot.
    fn update_existing(
        &self,
        identity: &(String, String, String),
        alias: Option<&str>,
    ) -> Result<()> {
        let (email, org_uuid, account_uuid) = identity;
        let mut seq = self.sequence()?;
        let num = Self::find_account_slot(&seq, email, org_uuid).ok_or_else(|| {
            Error::Config(format!(
                "Existing account metadata for {email} is inconsistent"
            ))
        })?;
        let org_name = text(account(&seq, &num), "organizationName");
        if let Some(alias) = alias
            && let Some(conflict) = self.alias_in_use(alias, Some(&num))?
        {
            return Err(Error::Validation(format!(
                "Alias '{alias}' is already used by account {conflict}"
            )));
        }
        let (creds, config) = self.capture(email, org_uuid, account_uuid)?;
        self.reject_identity_drift_since_verify(identity)?;
        self.write_account_credentials(&num, email, &creds)?;
        self.write_account_config(&num, email, &config)?;
        self.clear_dead(&num, email, org_uuid);
        if let Some(alias) = alias
            && let Some(record) = seq
                .get_mut("accounts")
                .and_then(|a| a.get_mut(&num))
                .and_then(Value::as_object_mut)
        {
            record.insert("alias".into(), json!(alias));
        }
        seq.insert(
            "activeAccountNumber".into(),
            json!(num.parse::<i64>().unwrap_or_default()),
        );
        seq.insert("lastUpdated".into(), json!(get_timestamp()));
        self.write_json(&self.sequence_file, &seq)?;
        let tag = Self::get_display_tag(email, &org_name, org_uuid);
        info!("Updated credentials for account {num}: {email}");
        println!(
            "{} for Account {num} ({email} {}).",
            accent("Updated credentials"),
            muted(&format!("[{tag}]"))
        );
        Ok(())
    }

    /// `add-token`: a slot from a setup-token or an API key. `token` `"-"` reads stdin; empty prompts.
    pub fn add_account_from_token(
        &self,
        token: Option<&str>,
        email: Option<&str>,
        slot: Option<i64>,
        assume_yes: bool,
    ) -> Result<()> {
        self.refuse_session_shell()?;
        let raw = match token {
            Some("-") => {
                let mut line = String::new();
                std::io::stdin().lock().read_line(&mut line)?;
                line.trim_end_matches('\n').to_owned()
            }
            Some(t) if !t.is_empty() => t.to_owned(),
            _ => prompt_secret("Token: ").unwrap_or_default(),
        };
        let Some(stored) = self.store_token_account(&raw, email, slot, assume_yes)? else {
            return Ok(());
        };
        if stored.updated {
            println!(
                "{} for Account {} ({} {}).",
                accent(&format!("Updated {}", stored.kind_label)),
                stored.num,
                stored.email,
                muted("[personal]")
            );
            return Ok(());
        }
        if let Some(moved) = &stored.moved_from {
            println!("{}", dimmed(&format!("Moved from slot {moved}")));
        }
        println!(
            "{} Account {}: {} {} {}",
            accent("Added"),
            stored.num,
            stored.email,
            muted("[personal]"),
            muted(&format!("(from {})", stored.kind_label))
        );
        Ok(())
    }

    /// Store a setup-token or API key as an account, printing nothing (sync uses it). `None` when
    /// the placement was declined. An account with the same email is updated in place.
    pub fn store_token_account(
        &self,
        token: &str,
        email: Option<&str>,
        slot: Option<i64>,
        assume_yes: bool,
    ) -> Result<Option<TokenStored>> {
        let token = token.trim().to_owned();
        if token.is_empty() {
            return Err(Error::Validation("Token cannot be empty".into()));
        }
        let is_api_key = looks_like_api_key(Some(&token));
        if let Some(e) = email.filter(|e| !e.is_empty())
            && !Self::validate_email(e)
        {
            return Err(Error::Validation(format!("Invalid email format: {e}")));
        }
        self.setup_directories()?;
        self.init_sequence_file()?;
        self.migrate_org_fields()?;
        let mut slot = slot;
        let email = match email.filter(|e| !e.is_empty()) {
            Some(e) => e.to_owned(),
            None => {
                let n = match slot {
                    Some(n) => n,
                    None => {
                        let n = self.get_next_account_number()?;
                        slot = Some(n);
                        n
                    }
                };
                let label = if is_api_key { "api-key" } else { "setup-token" };
                format!("{label}-{n}@token.local")
            }
        };
        self.reject_cross_kind_collision(&email, is_api_key)?;
        let credentials = if is_api_key {
            token.clone()
        } else {
            json!({"claudeAiOauth": {"accessToken": token, "scopes": SETUP_TOKEN_SCOPES}})
                .to_string()
        };
        let config = json!({"oauthAccount": {"emailAddress": email, "accountUuid": "",
            "organizationUuid": null, "organizationName": null}})
        .to_string();
        let kind_label = if is_api_key { "API key" } else { "token" };

        if slot.is_none() && self.account_exists(&email, "")? {
            let mut seq = self.sequence()?;
            let num = Self::find_account_slot(&seq, &email, "").ok_or_else(|| {
                Error::Config(format!(
                    "Existing account metadata for {email} is inconsistent"
                ))
            })?;
            self.write_account_credentials(&num, &email, &credentials)?;
            self.write_account_config(&num, &email, &config)?;
            self.clear_dead(&num, &email, "");
            seq.insert("lastUpdated".into(), json!(get_timestamp()));
            self.write_json(&self.sequence_file, &seq)?;
            info!("Updated {kind_label} for account {num}: {email}");
            return Ok(Some(TokenStored {
                num,
                email,
                kind_label,
                updated: true,
                moved_from: None,
            }));
        }
        let Some(placement) = self.place(slot, &email, "", assume_yes)? else {
            return Ok(None);
        };
        self.vacate(&placement)?;
        let num = placement.num.clone();
        self.write_account_credentials(&num, &email, &credentials)?;
        self.write_account_config(&num, &email, &config)?;
        self.clear_dead(&num, &email, "");

        let mut data = self.sequence()?;
        let mut record = Map::new();
        record.insert("email".into(), json!(email));
        record.insert("uuid".into(), json!(""));
        record.insert("organizationUuid".into(), json!(""));
        record.insert("organizationName".into(), json!(""));
        record.insert("added".into(), json!(get_timestamp()));
        if is_api_key {
            record.insert("kind".into(), json!("api_key"));
        }
        record.extend(placement.carried.clone());
        data.entry("accounts")
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .map(|a| a.insert(num.clone(), Value::Object(record)));
        sequence_insert_sorted(&mut data, &num);
        data.insert("lastUpdated".into(), json!(get_timestamp()));
        self.write_json(&self.sequence_file, &data)?;
        info!("Added account {num} from {kind_label}: {email}");
        let moved_from = match (&placement.migrate_from, slot) {
            (Some(from), Some(slot)) => Some(format!("{from} → {slot}")),
            _ => None,
        };
        Ok(Some(TokenStored {
            num,
            email,
            kind_label,
            updated: false,
            moved_from,
        }))
    }

    /// `remove`: forget a slot after confirmation (`assume_yes` skips it).
    pub fn remove_account(&self, identifier: &str, assume_yes: bool) -> Result<()> {
        self.refuse_session_shell()?;
        if !self.sequence_file.exists() {
            return Err(Error::Config("No accounts are managed yet".into()));
        }
        self.get_sequence_data_migrated()?;
        let mut identifier = identifier.to_owned();
        if !identifier.bytes().all(|b| b.is_ascii_digit()) || identifier.is_empty() {
            let is_alias = self.find_account_by_alias(&identifier)?.is_some();
            if !is_alias && !Self::validate_email(&identifier) {
                return Err(Error::Validation(format!(
                    "Invalid account identifier: {identifier}"
                )));
            }
            if !is_alias {
                match self.choose_among_same_email(&identifier)? {
                    Choice::Pick(num) => identifier = num,
                    Choice::Cancelled => return Ok(()),
                    Choice::Unique => {}
                }
            }
        }
        let num = self
            .resolve_account_identifier(&identifier)?
            .ok_or_else(|| {
                Error::AccountNotFound(format!("No account found with identifier: {identifier}"))
            })?;
        let mut data = self.sequence()?;
        let record = account(&data, &num)
            .cloned()
            .ok_or_else(|| Error::AccountNotFound(format!("Account-{num} does not exist")))?;
        let email = text(Some(&record), "email");
        self.ensure_no_live_session(&num, &email, "--remove-account")?;
        if data
            .get("activeAccountNumber")
            .map(|v| v.to_string())
            .as_deref()
            == Some(num.as_str())
        {
            warning(&format!(
                "Warning: Account-{num} ({email}) is currently active"
            ));
        }
        if !assume_yes {
            let answer = prompt(&format!(
                "Are you sure you want to permanently remove Account-{num} ({email})? [y/N] "
            ))
            .unwrap_or_default();
            if answer.to_lowercase() != "y" {
                println!("{}", dimmed("Cancelled"));
                return Ok(());
            }
        }
        self.delete_account_files(&num, &email)?;
        if let Some(accounts) = data.get_mut("accounts").and_then(Value::as_object_mut) {
            accounts.remove(&num);
        }
        sequence_remove(&mut data, &num);
        data.insert("lastUpdated".into(), json!(get_timestamp()));
        self.write_json(&self.sequence_file, &data)?;
        info!("Removed account {num}: {email}");
        println!("{} Account-{num} ({email})", accent("Removed"));
        self.prune_mappings(&email, &text(Some(&record), "organizationUuid"))
    }

    /// One email managed under several orgs: list them and ask which slot.
    fn choose_among_same_email(&self, email: &str) -> Result<Choice> {
        let data = self.sequence()?;
        let mut matches: Vec<String> = data
            .get("accounts")
            .and_then(Value::as_object)
            .map(|a| {
                a.iter()
                    .filter(|(_, acc)| acc.get("email").and_then(Value::as_str) == Some(email))
                    .map(|(num, _)| num.clone())
                    .collect()
            })
            .unwrap_or_default();
        if matches.len() <= 1 {
            return Ok(Choice::Unique);
        }
        matches.sort_by_key(|n| n.parse::<i64>().unwrap_or(0));
        println!("Multiple accounts found for '{email}':");
        for num in &matches {
            let acc = account(&data, num);
            let tag = Self::get_display_tag(
                &text(acc, "email"),
                &text(acc, "organizationName"),
                &text(acc, "organizationUuid"),
            );
            println!("  {num}: {email} {}", muted(&format!("[{tag}]")));
        }
        let choice = prompt("Enter account number to remove: ").unwrap_or_default();
        let choice = choice.trim().to_owned();
        if choice.bytes().all(|b| b.is_ascii_digit()) && matches.contains(&choice) {
            Ok(Choice::Pick(choice))
        } else {
            println!("{}", dimmed("Cancelled"));
            Ok(Choice::Cancelled)
        }
    }
}

enum Choice {
    Unique,
    Pick(String),
    Cancelled,
}

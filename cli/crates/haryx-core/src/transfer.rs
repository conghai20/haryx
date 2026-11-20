//! Port of `claude_swap/transfer.py`: `export`, `import`, `import-usage`, plus the haryx-only
//! `import --from-cswap`, which copies (never moves) an existing cswap store into haryx's own.
//!
//! The export envelope (version 1) is the same document cswap writes, so files move between the two
//! tools in both directions.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

use crate::credentials::{
    BackupNamespace, StoreHost, Tunables, cswap_backup_namespaces, looks_like_api_key,
};
use crate::error::{Error, Result};
use crate::fsutil::{WriteOptions, atomic_write_text};
use crate::json_output::{SCHEMA_VERSION, usage_from_json};
use crate::keychain::Security;
use crate::models::pyfmt::json_dumps_pretty;
use crate::models::{Platform, get_timestamp, normalize_alias};
use crate::oauth::credential_fingerprint;
use crate::switcher::{JsonObject, Switcher};
use crate::usage_store::Identities;
use crate::{PROG, VERSION};

/// The export envelope's format version.
pub const FORMAT_VERSION: i64 = 1;

fn eprint(msg: &str) {
    eprintln!("{msg}");
}

fn transfer(msg: impl Into<String>) -> Error {
    Error::Transfer(msg.into())
}

fn platform_tag() -> &'static str {
    match Platform::detect() {
        Platform::MacOs => "macos",
        Platform::Linux => "linux",
        Platform::Wsl => "wsl",
        Platform::Windows => "windows",
        Platform::Unknown => "unknown",
    }
}

fn expand_user(path: &str) -> PathBuf {
    match path.strip_prefix("~/") {
        Some(rest) => dirs::home_dir().map_or_else(|| PathBuf::from(path), |h| h.join(rest)),
        None if path == "~" => dirs::home_dir().unwrap_or_else(|| PathBuf::from(path)),
        None => PathBuf::from(path),
    }
}

fn parse_payload(text: &str, label: &str) -> Result<JsonObject> {
    match serde_json::from_str::<Value>(text) {
        Ok(Value::Object(obj)) => Ok(obj),
        Ok(_) => Err(transfer(format!("{label} must be a JSON object"))),
        Err(e) => Err(transfer(format!("{label} is not valid JSON: {e}"))),
    }
}

fn record_str(record: &Value, key: &str) -> String {
    record
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

/// Build the export envelope and how many accounts it carries.
pub fn build_export(switcher: &Switcher, account: Option<&str>, full: bool) -> Result<Value> {
    let data = switcher.get_sequence_data_migrated()?.unwrap_or_default();
    let accounts_map = data
        .get("accounts")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    if accounts_map.is_empty() {
        return Err(transfer(format!(
            "no accounts to export — run {PROG} --add-account first"
        )));
    }
    let explicit = account.is_some();
    let targets: Vec<String> = match account {
        Some(id) => {
            let resolved = switcher
                .resolve_account_identifier(id)?
                .filter(|n| accounts_map.contains_key(n))
                .ok_or_else(|| transfer(format!("account not found: {id}")))?;
            vec![resolved]
        }
        None => {
            let mut nums: Vec<String> = accounts_map.keys().cloned().collect();
            nums.sort_by_key(|n| n.parse::<i64>().unwrap_or(0));
            nums
        }
    };
    let current = switcher.get_current_account();
    let mut payload = Vec::new();
    for num in targets {
        let record = &accounts_map[&num];
        let email = record_str(record, "email");
        let org_uuid = record_str(record, "organizationUuid");
        let is_active = current
            .as_ref()
            .is_some_and(|(e, o)| *e == email && *o == org_uuid);
        let (creds_text, config_text) = if is_active {
            let creds = switcher
                .read_credentials()
                .filter(|c| !c.is_empty())
                .ok_or_else(|| {
                    Error::CredentialRead(format!(
                        "failed to read live credentials for active account {email}"
                    ))
                })?;
            let config_path = switcher.claude_config_path();
            let config = std::fs::read_to_string(&config_path)
                .map_err(|_| Error::Config("Claude config file not found".into()))?;
            (creds, config)
        } else {
            let creds = switcher.read_account_credentials(&num, &email, false);
            let config = switcher.read_account_config(&num, &email)?;
            if creds.is_empty() || config.is_empty() {
                if explicit {
                    if creds.is_empty() {
                        return Err(Error::CredentialRead(format!(
                            "no backup credentials found for account {num} ({email})"
                        )));
                    }
                    return Err(Error::Config(format!(
                        "no backup config found for account {num} ({email})"
                    )));
                }
                eprint(&format!(
                    "Skipping Account-{num} ({email}): no stored credentials/config — re-add with: {PROG} \
                     --add-account --slot {num}"
                ));
                continue;
            }
            (creds, config)
        };
        let mut config_obj = parse_payload(&config_text, &format!("config for {email}"))?;
        if !full {
            let Some(oauth) = config_obj
                .get("oauthAccount")
                .filter(|v| v.is_object())
                .cloned()
            else {
                return Err(transfer(format!(
                    "config for {email} is missing oauthAccount — cannot export"
                )));
            };
            config_obj = Map::from_iter([("oauthAccount".to_owned(), oauth)]);
        }
        let is_api_key = looks_like_api_key(Some(&creds_text));
        let creds_payload = if is_api_key {
            Value::String(creds_text.trim().to_owned())
        } else {
            let obj = parse_payload(&creds_text, &format!("credentials for {email}"))?;
            match obj.get("claudeAiOauth").filter(|_| !full) {
                Some(oauth) => json!({"claudeAiOauth": oauth}),
                None => Value::Object(obj),
            }
        };
        let mut entry = Map::new();
        entry.insert(
            "number".into(),
            json!(num.parse::<i64>().unwrap_or_default()),
        );
        entry.insert("email".into(), json!(email));
        entry.insert("uuid".into(), json!(record_str(record, "uuid")));
        entry.insert("organizationUuid".into(), json!(org_uuid));
        entry.insert(
            "organizationName".into(),
            json!(record_str(record, "organizationName")),
        );
        entry.insert("added".into(), json!(record_str(record, "added")));
        entry.insert("credentials".into(), creds_payload);
        entry.insert("config".into(), Value::Object(config_obj));
        if is_api_key {
            entry.insert("kind".into(), json!("api_key"));
        }
        let alias = record_str(record, "alias");
        if !alias.is_empty() {
            entry.insert("alias".into(), json!(alias));
        }
        payload.push(Value::Object(entry));
    }
    if payload.is_empty() {
        return Err(transfer(format!(
            "no exportable accounts — all managed slots are missing stored credentials/config. Re-add with: \
             {PROG} --add-account --slot <number>"
        )));
    }
    let exported: HashSet<i64> = payload
        .iter()
        .filter_map(|a| a["number"].as_i64())
        .collect();
    let active = data
        .get("activeAccountNumber")
        .and_then(Value::as_i64)
        .filter(|n| exported.contains(n));
    Ok(json!({
        "version": FORMAT_VERSION,
        "exportedAt": get_timestamp(),
        "exportedFrom": platform_tag(),
        "swapVersion": VERSION,
        "encrypted": false,
        "activeAccountNumber": active,
        "accounts": payload,
    }))
}

/// `export DEST` (`-` for stdout).
pub fn export_accounts(
    switcher: &Switcher,
    destination: &str,
    account: Option<&str>,
    full: bool,
) -> Result<()> {
    let envelope = build_export(switcher, account, full)?;
    let count = envelope["accounts"].as_array().map_or(0, Vec::len);
    let serialized = json_dumps_pretty(&envelope);
    if destination == "-" {
        let mut out = std::io::stdout().lock();
        writeln!(out, "{serialized}")?;
        out.flush()?;
        return Ok(());
    }
    let path = expand_user(destination);
    if path.is_dir() {
        return Err(transfer(format!(
            "export destination must be a file path, not a directory: {}",
            path.display()
        )));
    }
    let options = WriteOptions {
        follow_symlinks: false,
        ..WriteOptions::default()
    };
    atomic_write_text(&path, &format!("{serialized}\n"), options)?;
    eprint(&format!(
        "Exported {count} account(s) to {}",
        path.display()
    ));
    Ok(())
}

fn read_source(source: &str, what: &str) -> Result<String> {
    if source == "-" {
        let mut text = String::new();
        std::io::stdin().read_to_string(&mut text).map_err(|_| {
            transfer(format!(
                "piped input is not UTF-8 text — decrypt an encrypted backup first (e.g. gpg -d backup.gpg | {PROG} \
                 --import -)"
            ))
        })?;
        return Ok(text);
    }
    let path = expand_user(source);
    if !path.exists() {
        return Err(transfer(format!("{what} not found: {}", path.display())));
    }
    if path.is_dir() {
        return Err(transfer(format!(
            "import source must be a file path, not a directory: {}",
            path.display()
        )));
    }
    let bytes = std::fs::read(&path)
        .map_err(|e| transfer(format!("could not read {}: {e}", path.display())))?;
    String::from_utf8(bytes).map_err(|_| {
        transfer(format!(
            "{} is not UTF-8 text — if it is an encrypted backup, decrypt it first (e.g. gpg -d backup.gpg | {PROG} \
             --import -)",
            path.display()
        ))
    })
}

/// One validated account of an envelope.
struct Incoming {
    email: String,
    exported_num: String,
    org_uuid: String,
    org_name: String,
    uuid: String,
    added: String,
    is_api_key: bool,
    alias: Option<String>,
    creds_text: String,
    config_text: String,
}

fn validate_account(raw: &Value) -> Result<(String, String)> {
    let Some(obj) = raw.as_object() else {
        return Err(transfer("account entry must be a JSON object"));
    };
    let email = obj
        .get("email")
        .and_then(Value::as_str)
        .filter(|e| Switcher::validate_email(e));
    let Some(email) = email else {
        return Err(transfer(format!(
            "invalid or missing email in imported account: {}",
            obj.get("email").map_or("None".into(), Value::to_string)
        )));
    };
    let number = obj
        .get("number")
        .and_then(|v| if v.is_boolean() { None } else { v.as_i64() });
    let Some(number) = number.filter(|n| *n >= 1) else {
        return Err(transfer(format!(
            "invalid slot number in imported account ({email}): {}",
            obj.get("number").map_or("None".into(), Value::to_string)
        )));
    };
    for field in [
        "organizationUuid",
        "organizationName",
        "uuid",
        "added",
        "alias",
    ] {
        if let Some(v) = obj.get(field).filter(|v| !v.is_null() && !v.is_string()) {
            return Err(transfer(format!(
                "{field} for {email} must be a string, got {}",
                type_name(v)
            )));
        }
    }
    if let Some(alias) = obj.get("alias").and_then(Value::as_str) {
        normalize_alias(alias).map_err(|e| transfer(format!("invalid alias for {email}: {e}")))?;
    }
    Ok((email.to_owned(), number.to_string()))
}

fn type_name(v: &Value) -> &'static str {
    match v {
        Value::Bool(_) => "bool",
        Value::Number(n) if n.is_i64() || n.is_u64() => "int",
        Value::Number(_) => "float",
        Value::Array(_) => "list",
        Value::Object(_) => "dict",
        Value::String(_) => "str",
        Value::Null => "NoneType",
    }
}

fn normalize_accounts(switcher: &Switcher, accounts: &[Value]) -> Result<Vec<Incoming>> {
    let local = switcher.get_sequence_data_migrated()?.unwrap_or_default();
    let local_aliases: HashMap<String, (String, String)> = local
        .get("accounts")
        .and_then(Value::as_object)
        .map(|a| {
            a.values()
                .filter_map(|acc| {
                    let alias = acc
                        .get("alias")
                        .and_then(Value::as_str)
                        .filter(|s| !s.is_empty())?;
                    Some((
                        alias.to_lowercase(),
                        (
                            record_str(acc, "email"),
                            record_str(acc, "organizationUuid"),
                        ),
                    ))
                })
                .collect()
        })
        .unwrap_or_default();
    let mut out = Vec::new();
    let mut seen_keys = HashSet::new();
    let mut seen_aliases = HashSet::new();
    for raw in accounts {
        let (email, exported_num) = validate_account(raw)?;
        let org_uuid = record_str(raw, "organizationUuid");
        let Some(config) = raw.get("config").and_then(Value::as_object) else {
            return Err(transfer(format!(
                "config for {email} must be a JSON object"
            )));
        };
        let creds = raw.get("credentials").cloned().unwrap_or(Value::Null);
        let is_api_key =
            raw.get("kind").and_then(Value::as_str) == Some("api_key") || creds.is_string();
        let creds_text = if is_api_key {
            match creds.as_str().filter(|c| looks_like_api_key(Some(c))) {
                Some(key) => key.trim().to_owned(),
                None => {
                    return Err(transfer(format!(
                        "API-key credentials for {email} must be a raw sk-ant-… key (not an OAuth token)"
                    )));
                }
            }
        } else if creds.is_object() {
            creds.to_string()
        } else {
            return Err(transfer(format!(
                "credentials for {email} must be a JSON object"
            )));
        };
        if !seen_keys.insert((email.clone(), org_uuid.clone())) {
            let org = if org_uuid.is_empty() {
                "personal"
            } else {
                &org_uuid
            };
            return Err(transfer(format!(
                "duplicate account in export: {email} (org={org})"
            )));
        }
        let mut alias = raw
            .get("alias")
            .and_then(Value::as_str)
            .filter(|a| !a.is_empty())
            .map(str::to_owned);
        if let Some(a) = alias.clone() {
            let key = normalize_alias(&a)?;
            if !seen_aliases.insert(key.clone()) {
                return Err(transfer(format!("duplicate alias in export: {key}")));
            }
            match local_aliases.get(&key) {
                Some(owner) if *owner != (email.clone(), org_uuid.clone()) => {
                    eprint(&format!(
                        "Warning: alias '{key}' for {email} already used by an existing account, dropping the \
                         imported alias"
                    ));
                    alias = None;
                }
                _ => alias = Some(key),
            }
        }
        out.push(Incoming {
            org_name: record_str(raw, "organizationName"),
            uuid: record_str(raw, "uuid"),
            added: Some(record_str(raw, "added"))
                .filter(|a| !a.is_empty())
                .unwrap_or_else(get_timestamp),
            config_text: json_dumps_pretty(&Value::Object(config.clone())),
            email,
            exported_num,
            org_uuid,
            is_api_key,
            alias,
            creds_text,
        });
    }
    Ok(out)
}

/// `import FILE` (`-` for stdin).
pub fn import_accounts(switcher: &Switcher, source: &str, force: bool) -> Result<()> {
    let text = read_source(source, "import file")?;
    import_envelope_text(switcher, &text, force)
}

/// Import an envelope document.
pub fn import_envelope_text(switcher: &Switcher, text: &str, force: bool) -> Result<()> {
    let envelope: Value = serde_json::from_str(text)
        .map_err(|e| transfer(format!("export file is not valid JSON: {e}")))?;
    if !envelope.is_object() {
        return Err(transfer("export file must be a JSON object"));
    }
    let version = &envelope["version"];
    if version.as_i64() != Some(FORMAT_VERSION) || version.is_boolean() {
        return Err(transfer(format!(
            "unsupported export version: {} (expected {FORMAT_VERSION})",
            if version.is_null() {
                "None".into()
            } else {
                version.to_string()
            }
        )));
    }
    if envelope["encrypted"] == json!(true) {
        return Err(transfer(format!(
            "encrypted exports are not supported in this version — decrypt before piping (e.g. gpg -d backup.gpg | \
             {PROG} --import -)"
        )));
    }
    let accounts = envelope["accounts"]
        .as_array()
        .filter(|a| !a.is_empty())
        .cloned();
    let Some(accounts) = accounts else {
        return Err(transfer("export file has no accounts to import"));
    };
    let incoming = normalize_accounts(switcher, &accounts)?;
    switcher.setup_directories()?;
    switcher.init_sequence_file()?;
    let envelope_active = envelope["activeAccountNumber"]
        .as_i64()
        .map(|n| n.to_string());
    let mut counts = Counts::default();
    let mut resolved_active: Option<String> = None;
    let mut written = BTreeSet::new();
    for entry in &incoming {
        let is_envelope_active = envelope_active.as_deref() == Some(entry.exported_num.as_str());
        if let Some(slot) = import_one(switcher, entry, force, &mut counts)? {
            if is_envelope_active {
                resolved_active = Some(slot.0.clone());
            }
            if slot.1 {
                written.insert(slot.0);
            }
        }
    }
    finish_import(switcher, resolved_active, &counts, &written)
}

#[derive(Default)]
struct Counts {
    imported: usize,
    skipped: usize,
    overwritten: usize,
    replaced: usize,
}

/// Import one account. Returns `(slot, written)`; `written` is false for a skip.
fn import_one(
    switcher: &Switcher,
    entry: &Incoming,
    force: bool,
    counts: &mut Counts,
) -> Result<Option<(String, bool)>> {
    let mut data = switcher.get_sequence_data_migrated()?.unwrap_or_else(|| {
        let mut d = Map::new();
        d.insert("activeAccountNumber".into(), Value::Null);
        d.insert("lastUpdated".into(), json!(get_timestamp()));
        d.insert("sequence".into(), json!([]));
        d.insert("accounts".into(), json!({}));
        d
    });
    let existing = Switcher::find_account_slot(&data, &entry.email, &entry.org_uuid);
    let ident = |num: &str| -> Identities {
        [(
            num.to_owned(),
            (entry.email.clone(), entry.org_uuid.clone()),
        )]
        .into()
    };
    let had_slot = existing.is_some();
    let (target, outcome, strike) = match existing {
        Some(slot) if force => {
            let row = switcher
                .usage_store
                .entries(&ident(&slot), &[])
                .remove(&slot)
                .unwrap_or_default();
            let had_strike = row.auth_dead_strikes > 0;
            let same = row.struck_fingerprint.is_some()
                && credential_fingerprint(&entry.creds_text) == row.struck_fingerprint;
            (slot, "overwrote", (had_strike, same))
        }
        Some(slot) if switcher.slot_token_dead(&slot, &entry.email) => {
            (slot, "replaced", (false, false))
        }
        Some(slot) => {
            eprint(&format!(
                "Skipped {} (already exists, use --force)",
                entry.email
            ));
            counts.skipped += 1;
            return Ok(Some((slot, false)));
        }
        None => {
            let free = !data
                .get("accounts")
                .and_then(|a| a.get(&entry.exported_num))
                .is_some();
            let slot = if free {
                entry.exported_num.clone()
            } else {
                switcher.get_next_account_number()?.to_string()
            };
            (slot, "imported", (false, false))
        }
    };
    if had_slot {
        let pids = switcher.live_session_pids(&target, &entry.email);
        if !pids.is_empty() {
            let list: Vec<String> = pids.iter().map(i32::to_string).collect();
            eprint(&format!(
                "Warning: {} (slot {target}) has a live session-mode instance (PID {}); its session profile keeps \
                 the pre-import credentials until it is restarted via '{PROG} run'.",
                entry.email,
                list.join(", ")
            ));
        }
    }
    switcher.write_account_credentials(&target, &entry.email, &entry.creds_text)?;
    switcher.write_account_config(&target, &entry.email, &entry.config_text)?;
    if let Err(err) = switcher
        .usage_store
        .clear_dead_token(std::slice::from_ref(&target), &ident(&target))
    {
        log::debug!("clear_dead_token failed for {target}: {err}");
    }
    let mut record = Map::new();
    record.insert("email".into(), json!(entry.email));
    record.insert("uuid".into(), json!(entry.uuid));
    record.insert("organizationUuid".into(), json!(entry.org_uuid));
    record.insert("organizationName".into(), json!(entry.org_name));
    record.insert("added".into(), json!(entry.added));
    if entry.is_api_key {
        record.insert("kind".into(), json!("api_key"));
    }
    if let Some(alias) = &entry.alias {
        record.insert("alias".into(), json!(alias));
    }
    data.entry("accounts")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .map(|a| a.insert(target.clone(), Value::Object(record)));
    let n: i64 = target.parse().unwrap_or_default();
    if let Some(seq) = data
        .entry("sequence")
        .or_insert_with(|| json!([]))
        .as_array_mut()
        && !seq.iter().any(|v| v.as_i64() == Some(n))
    {
        seq.push(json!(n));
        seq.sort_by_key(|v| v.as_i64().unwrap_or(0));
    }
    data.insert("lastUpdated".into(), json!(get_timestamp()));
    switcher.write_json(switcher.sequence_file(), &data)?;
    match outcome {
        "overwrote" => {
            eprint(&format!("Overwrote {} (slot {target})", entry.email));
            if strike.0 {
                eprint("  └ cleared this slot's stored dead-token strike");
                if strike.1 {
                    eprint(
                        "  └ this import holds the same credential generation the strike condemned; another \
                         permanent auth failure will quarantine it again — recover with a newer export or a re-login",
                    );
                }
            }
            counts.overwritten += 1;
        }
        "replaced" => {
            eprint(&format!(
                "Replaced {} (slot {target} was quarantined: refresh token dead)",
                entry.email
            ));
            counts.replaced += 1;
        }
        _ => {
            eprint(&format!("Imported {} → slot {target}", entry.email));
            counts.imported += 1;
        }
    }
    Ok(Some((target, true)))
}

fn finish_import(
    switcher: &Switcher,
    resolved_active: Option<String>,
    c: &Counts,
    written: &BTreeSet<String>,
) -> Result<()> {
    let mut final_data = switcher.get_sequence_data()?;
    if let (Some(data), Some(slot)) = (final_data.as_mut(), resolved_active)
        && matches!(data.get("activeAccountNumber"), None | Some(Value::Null))
            | (data.get("activeAccountNumber").and_then(Value::as_i64) == Some(0))
    {
        data.insert(
            "activeAccountNumber".into(),
            json!(slot.parse::<i64>().unwrap_or_default()),
        );
        data.insert("lastUpdated".into(), json!(get_timestamp()));
        switcher.write_json(switcher.sequence_file(), data)?;
    }
    let mut summary = format!(
        "Done: {} imported, {} overwritten, {} skipped",
        c.imported, c.overwritten, c.skipped
    );
    if c.replaced > 0 {
        summary.push_str(&format!(", {} replaced (dead token)", c.replaced));
    }
    eprint(&summary);
    if let (Some((email, org)), Some(data)) = (switcher.get_current_account(), final_data.as_ref())
        && let Some(live) = Switcher::find_account_slot(data, &email, &org)
        && written.contains(&live)
    {
        eprint(&format!(
            "Note: {email} is your current live login — activate the imported credentials with: {PROG} \
             --switch-to {live} --force"
        ));
    }
    Ok(())
}

/// `import-usage FILE`: adopt readings from another machine's `list --json`.
pub fn import_usage(switcher: &Switcher, source: &str, hold_s: Option<f64>) -> Result<()> {
    let text = read_source(source, "usage file")?;
    let document = parse_payload(&text, "usage document")?;
    if document.get("schemaVersion").and_then(Value::as_i64) != Some(i64::from(SCHEMA_VERSION)) {
        return Err(transfer(format!(
            "unsupported usage document schemaVersion: {} (expected {SCHEMA_VERSION}, as printed by '{PROG} list \
             --json')",
            document
                .get("schemaVersion")
                .map_or("None".into(), Value::to_string)
        )));
    }
    let Some(rows) = document.get("accounts").and_then(Value::as_array) else {
        return Err(transfer("usage document has no accounts list"));
    };
    let local = switcher.get_sequence_data_migrated()?.unwrap_or_default();
    let mut readings: BTreeMap<String, (Value, f64)> = BTreeMap::new();
    let mut identities: Identities = BTreeMap::new();
    let mut skipped = 0;
    for row in rows {
        if !row.is_object() {
            return Err(transfer("each account row must be a JSON object"));
        }
        let email = row
            .get("email")
            .and_then(Value::as_str)
            .filter(|e| !e.is_empty());
        let org = match row.get("organizationUuid") {
            None | Some(Value::Null) => Some(String::new()),
            Some(Value::String(s)) => Some(s.clone()),
            Some(_) => None,
        };
        let (Some(email), Some(org)) = (email, org) else {
            return Err(transfer(
                "each account row needs an email and an organizationUuid",
            ));
        };
        if row.get("usageStatus").and_then(Value::as_str) != Some("ok") {
            skipped += 1;
            continue;
        }
        let age = row
            .get("usageAgeSeconds")
            .filter(|v| !v.is_boolean())
            .and_then(Value::as_f64)
            .filter(|a| a.is_finite() && *a >= 0.0);
        let Some(age) = age else {
            return Err(transfer(format!(
                "usageAgeSeconds for {email} must be a non-negative number"
            )));
        };
        let usage = usage_from_json(row.get("usage").unwrap_or(&Value::Null))
            .map_err(|e| transfer(format!("usage for {email}: {e}")))?;
        let Some(num) = Switcher::find_account_slot(&local, email, &org) else {
            skipped += 1;
            continue;
        };
        if readings.contains_key(&num) {
            let shown = if org.is_empty() {
                "personal".to_owned()
            } else {
                org.clone()
            };
            return Err(transfer(format!(
                "duplicate account in usage document: {email} (org={shown})"
            )));
        }
        readings.insert(num.clone(), (usage, age));
        identities.insert(num, (email.to_owned(), org));
    }
    let adopted = switcher.usage_store.adopt(&readings, &identities, hold_s)?;
    for (num, (email, _)) in &identities {
        if adopted.contains(num) {
            eprint(&format!("Adopted usage for {email} → slot {num}"));
        } else {
            eprint(&format!(
                "Kept slot {num}'s own reading for {email}: it is newer"
            ));
        }
    }
    let mut summary = format!(
        "Done: {} adopted, {} kept, {skipped} skipped",
        adopted.len(),
        readings.len() - adopted.len()
    );
    if let Some(hold) = hold_s.filter(|_| !readings.is_empty()) {
        summary.push_str(&if hold > 0.0 {
            format!("; fetching held for up to {hold:.0}s")
        } else {
            "; holds lifted".to_owned()
        });
    }
    eprint(&summary);
    Ok(())
}

/// The cswap store to read for `--from-cswap`: the first candidate that holds a roster.
pub fn find_cswap_store(switcher: &Switcher) -> Option<BackupNamespace> {
    cswap_backup_namespaces(switcher.path_env())
        .into_iter()
        .find(|ns| ns.sequence_file.is_file())
}

/// `import --from-cswap`: copy every account of an existing cswap store (and its settings and folder
/// mappings, when haryx has none yet). The cswap store is only read.
pub fn import_from_cswap(switcher: &Switcher, force: bool) -> Result<()> {
    let Some(ns) = find_cswap_store(switcher) else {
        let tried: Vec<String> = cswap_backup_namespaces(switcher.path_env())
            .iter()
            .map(|ns| ns.store_root.display().to_string())
            .collect();
        return Err(transfer(format!(
            "no cswap store found (looked in: {})",
            tried.join(", ")
        )));
    };
    eprint(&format!(
        "Reading cswap store at {}",
        ns.store_root.display()
    ));
    let host = StoreHost::new(&ns, switcher.path_env().clone());
    let cswap = Switcher::new(
        host,
        std::sync::Arc::new(Security::system()),
        Tunables::default(),
        switcher.debug(),
    );
    let envelope = build_export(&cswap, None, true)?;
    import_envelope_text(switcher, &envelope.to_string(), force)?;
    for file in ["settings.json", "mappings.json"] {
        copy_if_absent(&ns.store_root.join(file), &switcher.backup_dir().join(file))?;
    }
    Ok(())
}

fn copy_if_absent(from: &Path, to: &Path) -> Result<()> {
    if !from.is_file() || to.exists() {
        return Ok(());
    }
    let text = std::fs::read_to_string(from)?;
    atomic_write_text(to, &text, WriteOptions::default())?;
    eprint(&format!(
        "Copied {}",
        to.file_name()
            .map(|n| n.to_string_lossy())
            .unwrap_or_default()
    ));
    Ok(())
}

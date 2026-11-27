//! Sync between a user's machines, end-to-end encrypted (plan: `docs/haryx-account-plan.md`).
//!
//! A sync pass, per document: decrypt the server's copy, three-way merge it with this machine's
//! ([`merge`]), write back what changed there, then apply what changed here ([`sources`]). Writes
//! name the version they read; when another machine got there first the pass starts over (twice
//! at most), with nothing applied here yet. Local data is always the source of truth: the server
//! is a relay, and a missing row is re-uploaded, never read as "delete".
//!
//! The server is not trusted with integrity either: every document names its own kind, key and
//! version inside the encryption, so a row moved, swapped or replayed does not open. A row that
//! does not open is replaced by this machine's copy; it never locks sync. Only the `keyring` row's
//! key check decides that the key changed.

pub mod crypto;
pub mod merge;
pub mod profiles;
pub mod remote;
pub mod sources;
pub mod state;

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::account::{AccountError, Vault};
use crate::switcher::Switcher;
use crypto::SyncKey;
use merge::{Doc, Values};
use remote::{Remote, Row};
use sources::LocalAccount;
use state::{OverwriteRecord, RemoteAccount, RowState, State};

/// The vault entry holding this machine's sync key.
const KEY_NAME: &str = "sync-key";
const ATTEMPTS: usize = 3;

pub const KIND_KEYRING: &str = "keyring";
pub const KIND_SETTINGS: &str = "settings";
pub const KIND_APP_PREFS: &str = "app-prefs";
pub const KIND_ACCOUNT: &str = "account";
pub const KIND_CREDENTIAL: &str = "credential";
pub const KIND_PROFILES: &str = "profiles";
/// Per-account usage readings and poll state, shared outside the sync pass (`crate::usage_share`).
pub const KIND_USAGE: &str = "usage";

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum SyncError {
    #[error("This machine is not signed in to HaryX. Run 'haryx login' first.")]
    NotSignedIn,
    #[error(
        "Sync is locked on this machine. Run 'haryx sync unlock' and enter your sync passphrase."
    )]
    Locked,
    #[error("That is not your sync passphrase.")]
    WrongPassphrase,
    #[error(
        "The sync passphrase was reset on another machine. Run 'haryx sync unlock' with the new one."
    )]
    KeyChanged,
    #[error("Another machine kept changing the same data. Run 'haryx sync' again.")]
    Busy,
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    Local(String),
    #[error("The sync server answered: {0}")]
    Server(String),
    #[error("{0}")]
    Account(#[from] AccountError),
}

impl SyncError {
    /// The `code` of a `--json` error (`packages/contracts/haryx/sync-result.json`).
    pub fn code(&self) -> &'static str {
        match self {
            SyncError::NotSignedIn => "not_signed_in",
            SyncError::Locked => "locked",
            SyncError::WrongPassphrase => "wrong_passphrase",
            SyncError::KeyChanged => "key_changed",
            SyncError::Busy => "busy",
            SyncError::Invalid(_) => "invalid",
            SyncError::Local(_) => "local",
            SyncError::Server(_) => "server",
            SyncError::Account(err) => err.code(),
        }
    }
}

/// Everything a sync operation needs. `root` is the haryx store root.
pub struct Context<'a> {
    pub root: &'a Path,
    pub switcher: &'a Switcher,
    pub vault: &'a Vault,
    pub remote: &'a dyn Remote,
    pub objects: &'a dyn profiles::Objects,
    pub user: &'a str,
    pub project: &'a str,
    pub now: i64,
}

/// The decrypted body of every row but the keyring.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Document {
    /// The row this was written as: checked on open, so the server cannot move or replay it.
    pub kind: String,
    pub key: String,
    pub version: i64,
    pub values: Values,
    /// When the writer's copy last changed (Unix seconds): the tiebreak for conflicts.
    pub changed_at: i64,
}

/// The sync key as this machine keeps it: tied to one HaryX user and project.
#[derive(Serialize, Deserialize)]
struct StoredKey {
    user: String,
    project: String,
    key: String,
}

/// This machine's sync key for `user`/`project`, when sync is unlocked here.
pub(crate) fn unlocked_key(
    vault: &Vault,
    user: &str,
    project: &str,
) -> Result<Option<SyncKey>, SyncError> {
    load_key(vault, user, project)
}

fn load_key(vault: &Vault, user: &str, project: &str) -> Result<Option<SyncKey>, SyncError> {
    let Some(text) = vault.read(KEY_NAME)? else {
        return Ok(None);
    };
    match serde_json::from_str::<StoredKey>(&text) {
        Ok(stored) if stored.user == user && stored.project == project => {
            SyncKey::from_secret(&stored.key).map(Some)
        }
        _ => Ok(None),
    }
}

fn store_key(ctx: &Context<'_>, key: &SyncKey) -> Result<(), SyncError> {
    let stored = StoredKey {
        user: ctx.user.to_owned(),
        project: ctx.project.to_owned(),
        key: key.to_secret(),
    };
    let text = serde_json::to_string(&stored).map_err(|e| SyncError::Local(e.to_string()))?;
    Ok(ctx.vault.write(KEY_NAME, &text)?)
}

/// Forget this machine's sync key and state (signing out, a reset, a key changed elsewhere).
pub fn forget_local(root: &Path, vault: &Vault) -> Result<(), SyncError> {
    vault.remove(KEY_NAME)?;
    State::remove(root)
}

pub fn is_unlocked(vault: &Vault, user: &str, project: &str) -> bool {
    load_key(vault, user, project).is_ok_and(|k| k.is_some())
}

fn require_key(ctx: &Context<'_>) -> Result<SyncKey, SyncError> {
    load_key(ctx.vault, ctx.user, ctx.project)?.ok_or(SyncError::Locked)
}

fn keyring(rows: &[Row]) -> Option<&Row> {
    rows.iter()
        .find(|r| r.kind == KIND_KEYRING && r.key.is_empty())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Unlocked {
    /// The account had no sync passphrase yet: this machine set it.
    pub created: bool,
}

/// Unlock sync on this machine. The first machine of an account sets the passphrase; every other
/// machine enters it.
pub fn unlock(ctx: &Context<'_>, passphrase: &str) -> Result<Unlocked, SyncError> {
    let rows = ctx.remote.list()?;
    let previous = load_key(ctx.vault, ctx.user, ctx.project)?;
    let (key, created) = match keyring(&rows) {
        Some(row) => (SyncKey::unwrap(&row.body, passphrase)?, false),
        None => {
            crypto::check_passphrase(passphrase)?;
            let key = SyncKey::generate();
            if ctx
                .remote
                .insert(KIND_KEYRING, "", &key.wrap(passphrase)?)?
                .is_none()
            {
                // Another machine set a passphrase a moment ago: use that one.
                return Err(SyncError::Busy);
            }
            (key, true)
        }
    };
    if previous.is_some_and(|p| p.to_secret() != key.to_secret()) {
        State::remove(ctx.root)?;
    }
    store_key(ctx, &key)?;
    Ok(Unlocked { created })
}

/// Whether this HaryX account has a sync passphrase yet (the first machine sets it).
pub fn has_passphrase(remote: &dyn Remote) -> Result<bool, SyncError> {
    Ok(keyring(&remote.list()?).is_some())
}

/// Re-wrap the sync key with `passphrase`. Nothing else changes.
pub fn change_passphrase(ctx: &Context<'_>, passphrase: &str) -> Result<(), SyncError> {
    let key = require_key(ctx)?;
    let rows = ctx.remote.list()?;
    let row = keyring(&rows)
        .filter(|r| crypto::keyring_check(&r.body) == Some(key.check()))
        .ok_or(SyncError::KeyChanged)?;
    let body = key.wrap(passphrase)?;
    ctx.remote
        .update(KIND_KEYRING, "", &body, row.version)?
        .map(drop)
        .ok_or(SyncError::Busy)
}

/// Delete everything this account has on the server and forget this machine's key. The next
/// `unlock` sets a new passphrase, and the next sync uploads this machine's data again.
pub fn reset(ctx: &Context<'_>) -> Result<(), SyncError> {
    // Files first: their names are only known from the index row the second step deletes.
    profiles::delete_all_objects(ctx)?;
    ctx.remote.delete_all()?;
    forget_local(ctx.root, ctx.vault)
}

/// Remove an account (and its token, if any) from the server, so other machines stop offering it.
/// Returns how many rows went.
pub fn forget(ctx: &Context<'_>, email: &str) -> Result<usize, SyncError> {
    let key = require_key(ctx)?;
    let wanted = email.trim().to_lowercase();
    let mut gone = 0;
    for row in ctx.remote.list()? {
        if row.kind != KIND_ACCOUNT && row.kind != KIND_CREDENTIAL && row.kind != KIND_USAGE {
            continue;
        }
        let Opened::Doc(doc) = open(&key, &row) else {
            continue;
        };
        let matches = doc
            .values
            .get("email")
            .and_then(Value::as_str)
            .is_some_and(|e| e.to_lowercase() == wanted);
        if matches {
            ctx.remote.delete(&row.kind, &row.key)?;
            gone += 1;
        }
    }
    let mut state = State::load(ctx.root, ctx.user, ctx.project);
    state
        .other_machine_accounts
        .retain(|a| a.email.to_lowercase() != wanted);
    state.save(ctx.root)?;
    Ok(gone)
}

/// A server row, decrypted.
pub(crate) enum Opened {
    Doc(Document),
    /// Not sealed with this key, or sealed as another row: garbage or tampering. Replaced.
    Foreign,
    /// Sealed with this key but in a shape this version does not read (a newer HaryX). Left alone.
    Unreadable,
}

pub(crate) fn open(key: &SyncKey, row: &Row) -> Opened {
    let Some(plain) = key.open(&row.body) else {
        return Opened::Foreign;
    };
    let Ok(doc) = serde_json::from_slice::<Document>(&plain) else {
        return Opened::Unreadable;
    };
    if doc.kind != row.kind || doc.key != row.key || doc.version != row.version {
        return Opened::Foreign;
    }
    Opened::Doc(doc)
}

/// Write `values` as row `kind`/`row_key`: an update of the row at `current`, or an insert.
/// `None`: another machine wrote first.
fn write_row(
    ctx: &Context<'_>,
    key: &SyncKey,
    kind: &str,
    row_key: &str,
    values: Values,
    changed_at: i64,
    current: Option<i64>,
) -> Result<Option<Row>, SyncError> {
    let doc = Document {
        kind: kind.to_owned(),
        key: row_key.to_owned(),
        version: current.map_or(1, |v| v + 1),
        values,
        changed_at,
    };
    let text = serde_json::to_vec(&doc).map_err(|e| SyncError::Local(e.to_string()))?;
    let body = key.seal(&text)?;
    match current {
        Some(version) => ctx.remote.update(kind, row_key, &body, version),
        None => ctx.remote.insert(kind, row_key, &body),
    }
}

/// What one `haryx sync` did (`packages/contracts/haryx/sync-result.json`).
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    /// Documents written to the server.
    pub pushed: usize,
    /// Documents that changed this machine.
    pub pulled: usize,
    /// The kinds that changed this machine (`settings`, `app-prefs`, `account`, `credential`).
    pub changed: Vec<String>,
    pub overwritten: Vec<OverwriteRecord>,
    pub other_machine_accounts: Vec<RemoteAccount>,
    /// Values this machine could not take, with why.
    pub skipped: Vec<String>,
}

impl Report {
    fn changed(&mut self, kind: &str) {
        self.pulled += 1;
        if !self.changed.iter().any(|k| k == kind) {
            self.changed.push(kind.to_owned());
        }
    }
}

/// Sync this machine with the server.
pub fn run(ctx: &Context<'_>) -> Result<Report, SyncError> {
    let key = require_key(ctx)?;
    let mut state = State::load(ctx.root, ctx.user, ctx.project);
    for _ in 0..ATTEMPTS {
        let rows = ctx.remote.list()?;
        let same_key = keyring(&rows).is_some_and(|r| {
            crypto::keyring_check(&r.body).is_some_and(|check| check == key.check())
        });
        if !same_key {
            forget_local(ctx.root, ctx.vault)?;
            return Err(SyncError::KeyChanged);
        }
        match pass(ctx, &key, &mut state, &rows) {
            Ok(Some(report)) => {
                state.last_sync_at = Some(ctx.now);
                state.other_machine_accounts = report.other_machine_accounts.clone();
                state.record(report.overwritten.clone());
                state.save(ctx.root)?;
                return Ok(report);
            }
            Ok(None) => continue,
            Err(other) => {
                // What was written before the error is already on the server; keep its base.
                state.save(ctx.root)?;
                return Err(other);
            }
        }
    }
    Err(SyncError::Busy)
}

/// A server document, decrypted.
struct RemoteDoc {
    doc: Document,
    version: i64,
}

/// The server's rows for one pass: those that opened, and the version and state of those that
/// did not, by `(kind, key)`.
#[derive(Default)]
struct Server {
    docs: BTreeMap<(String, String), RemoteDoc>,
    broken: BTreeMap<(String, String), (i64, bool)>,
}

/// A field this machine could not take, and why.
pub type Skipped = Vec<(String, String)>;

/// How a merged document is applied to this machine; returns what it could not take.
type Apply<'f> = Box<dyn FnOnce(&Values) -> Result<Skipped, SyncError> + 'f>;

/// One document to sync: what it is called, this machine's copy, and how to apply a merge here.
struct Item<'f> {
    kind: &'static str,
    key: String,
    /// How a field of it is named in the overwrite log (`you@example.com: alias`).
    name: String,
    secret: bool,
    values: Values,
    changed_at: i64,
    apply: Apply<'f>,
}

/// One pass over every document. `Ok(None)`: another machine wrote first; start over.
fn pass(
    ctx: &Context<'_>,
    key: &SyncKey,
    state: &mut State,
    rows: &[Row],
) -> Result<Option<Report>, SyncError> {
    let mut remote = Server::default();
    for row in rows
        .iter()
        .filter(|r| r.kind != KIND_KEYRING && r.kind != KIND_PROFILES && r.kind != KIND_USAGE)
    {
        let at = (row.kind.clone(), row.key.clone());
        match open(key, row) {
            Opened::Doc(doc) => {
                let version = row.version;
                remote.docs.insert(at, RemoteDoc { doc, version });
            }
            Opened::Foreign => {
                remote.broken.insert(at, (row.version, false));
            }
            Opened::Unreadable => {
                remote.broken.insert(at, (row.version, true));
            }
        }
    }
    let mut report = Report::default();

    // Tokens first: an imported one becomes an account whose settings then sync below.
    for item in credential_items(ctx, key, state, &remote.docs, &mut report.skipped)? {
        if !sync_item(ctx, key, state, &remote, item, &mut report)? {
            return Ok(None);
        }
    }
    let (accounts, accounts_at) = sources::read_accounts(ctx.switcher)?;
    let mut local_account_keys = Vec::new();
    for account in accounts {
        let row_key = key.row_key(&account.label(KIND_ACCOUNT));
        local_account_keys.push(row_key.clone());
        let item = account_item(ctx, &account, row_key, accounts_at);
        if !sync_item(ctx, key, state, &remote, item, &mut report)? {
            return Ok(None);
        }
    }
    report.other_machine_accounts = remote
        .docs
        .iter()
        .filter(|((kind, k), _)| kind == KIND_ACCOUNT && !local_account_keys.contains(k))
        .map(|(_, r)| {
            let text = |f: &str| {
                r.doc
                    .values
                    .get(f)
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned()
            };
            RemoteAccount {
                email: text("email"),
                organization_name: text("organizationName"),
                alias: text("alias"),
            }
        })
        .collect();

    let (settings, settings_at) = sources::read_settings(ctx.root)?;
    let settings_item = Item {
        kind: KIND_SETTINGS,
        key: String::new(),
        name: String::new(),
        secret: false,
        values: settings,
        changed_at: settings_at,
        apply: Box::new(|wanted| sources::write_settings(ctx.root, wanted)),
    };
    if !sync_item(ctx, key, state, &remote, settings_item, &mut report)? {
        return Ok(None);
    }
    let (prefs, prefs_at) = sources::read_app_prefs(ctx.root)?;
    let prefs_item = Item {
        kind: KIND_APP_PREFS,
        key: String::new(),
        name: String::new(),
        secret: false,
        values: prefs,
        changed_at: prefs_at,
        apply: Box::new(|wanted| {
            sources::write_app_prefs(ctx.root, wanted).map(|()| Skipped::new())
        }),
    };
    if !sync_item(ctx, key, state, &remote, prefs_item, &mut report)? {
        return Ok(None);
    }
    Ok(Some(report))
}

fn account_item<'f>(
    ctx: &'f Context<'_>,
    account: &LocalAccount,
    row_key: String,
    changed_at: i64,
) -> Item<'f> {
    let num = account.num.clone();
    Item {
        kind: KIND_ACCOUNT,
        key: row_key,
        name: account.email.clone(),
        secret: false,
        values: account.values(),
        changed_at,
        apply: Box::new(move |wanted| {
            let settings = wanted.clone().into_iter().collect();
            ctx.switcher
                .apply_synced_settings(&num, &settings)
                .map_err(|e| SyncError::Local(e.to_string()))
        }),
    }
}

/// Token accounts here, plus tokens only the server has that this machine never declined.
fn credential_items<'f>(
    ctx: &'f Context<'_>,
    key: &SyncKey,
    state: &State,
    remote: &BTreeMap<(String, String), RemoteDoc>,
    skipped: &mut Vec<String>,
) -> Result<Vec<Item<'f>>, SyncError> {
    let (accounts, accounts_at) = sources::read_accounts(ctx.switcher)?;
    let mut items = Vec::new();
    let mut local_keys = Vec::new();
    for account in &accounts {
        let Some(token) = sources::token_of(ctx.switcher, account) else {
            continue;
        };
        let row_key = key.row_key(&account.label(KIND_CREDENTIAL));
        local_keys.push(row_key.clone());
        let values = credential_values(&token, &account.email, &account.organization_uuid);
        items.push(credential_item(
            ctx,
            row_key,
            account.email.clone(),
            values,
            accounts_at,
        ));
    }
    for ((kind, row_key), r) in remote {
        let seen_before = state.rows.contains_key(&State::row_id(kind, row_key));
        // Synced here before and gone now: removed on this machine on purpose. Leave it removed.
        if kind != KIND_CREDENTIAL || local_keys.contains(row_key) || seen_before {
            continue;
        }
        let text = |f: &str| {
            r.doc
                .values
                .get(f)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        let email = text("email");
        // The same account signed in here through the browser: its login is worth more than a
        // token, and importing would replace it.
        let signed_in_here = accounts.iter().any(|a| {
            a.email.eq_ignore_ascii_case(&email) && a.organization_uuid == text("organizationUuid")
        });
        if signed_in_here {
            skipped.push(format!(
                "{email}: token: this machine has its own sign-in for that account"
            ));
            continue;
        }
        // A new token: this machine's side is empty, so the merge takes the server's copy whole.
        items.push(credential_item(
            ctx,
            row_key.clone(),
            email,
            Values::new(),
            0,
        ));
    }
    Ok(items)
}

fn credential_values(token: &str, email: &str, org: &str) -> Values {
    [
        ("token", token),
        ("email", email),
        ("organizationUuid", org),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), Value::String(v.to_owned())))
    .collect()
}

fn credential_item<'f>(
    ctx: &'f Context<'_>,
    row_key: String,
    email: String,
    values: Values,
    changed_at: i64,
) -> Item<'f> {
    Item {
        kind: KIND_CREDENTIAL,
        key: row_key,
        name: email,
        secret: true,
        values,
        changed_at,
        apply: Box::new(move |wanted| {
            let text = |f: &str| wanted.get(f).and_then(Value::as_str).unwrap_or_default();
            if text("token").is_empty() {
                return Ok(vec![("token".into(), "a synced token had no value".into())]);
            }
            ctx.switcher
                .store_token_account(text("token"), Some(text("email")), None, true)
                .map(|_| Skipped::new())
                .map_err(|e| SyncError::Local(e.to_string()))
        }),
    }
}

/// The field of a credential row that holds the secret.
const SECRET_FIELD: &str = "token";

/// `values` with the secret replaced by a keyed digest: what a credential row is merged as and
/// what `state.json` keeps as its base, so the token itself never lands in that file.
fn digested(key: &SyncKey, values: &Values) -> Values {
    let mut out = values.clone();
    if let Some(secret) = values.get(SECRET_FIELD) {
        let digest = format!("sha:{}", key.row_key(&format!("secret:{secret}")));
        out.insert(SECRET_FIELD.to_owned(), Value::String(digest));
    }
    out
}

/// `merged` (digested) with the real secret of whichever side it came from.
fn restored(key: &SyncKey, merged: &Values, sides: &[&Values]) -> Values {
    let mut out = merged.clone();
    if let Some(wanted) = merged.get(SECRET_FIELD) {
        let real = sides
            .iter()
            .find(|side| digested(key, side).get(SECRET_FIELD) == Some(wanted))
            .and_then(|side| side.get(SECRET_FIELD));
        match real {
            Some(secret) => out.insert(SECRET_FIELD.to_owned(), secret.clone()),
            None => out.remove(SECRET_FIELD),
        };
    }
    out
}

/// Merge one document, write it there, apply it here. `Ok(false)`: the server moved; start over.
fn sync_item(
    ctx: &Context<'_>,
    key: &SyncKey,
    state: &mut State,
    remote: &Server,
    item: Item<'_>,
    report: &mut Report,
) -> Result<bool, SyncError> {
    let id = State::row_id(item.kind, &item.key);
    let at = (item.kind.to_owned(), item.key.clone());
    let broken = remote.broken.get(&at).copied();
    if let Some((_, true)) = broken {
        report.skipped.push(format!(
            "{}: the server's copy was written by a newer HaryX; update to sync it",
            if item.name.is_empty() {
                item.kind
            } else {
                item.name.as_str()
            }
        ));
        return Ok(true);
    }
    let view = |values: &Values| {
        if item.secret {
            digested(key, values)
        } else {
            values.clone()
        }
    };
    let theirs = remote.docs.get(&at);
    let local = view(&item.values);
    let their_values = theirs.map(|r| view(&r.doc.values));
    let merged = merge::merge(
        Doc {
            values: &local,
            changed_at: item.changed_at,
        },
        state.rows.get(&id).map(|r| &r.base),
        theirs.zip(their_values.as_ref()).map(|(r, values)| Doc {
            values,
            changed_at: r.doc.changed_at,
        }),
    );
    let wanted = match (item.secret, theirs) {
        (false, _) => merged.values.clone(),
        (true, Some(r)) => restored(key, &merged.values, &[&item.values, &r.doc.values]),
        (true, None) => restored(key, &merged.values, &[&item.values]),
    };

    // There first: if another machine got in between, nothing has changed here yet.
    let changed_at = item.changed_at.max(theirs.map_or(0, |r| r.doc.changed_at));
    let version = match (theirs, broken) {
        (Some(r), _) if their_values.as_ref() == Some(&merged.values) => r.version,
        (theirs, broken) => {
            let current = theirs.map(|r| r.version).or(broken.map(|(v, _)| v));
            let written = write_row(
                ctx,
                key,
                item.kind,
                &item.key,
                wanted.clone(),
                changed_at,
                current,
            )?;
            let Some(row) = written else {
                return Ok(false);
            };
            report.pushed += 1;
            row.version
        }
    };

    // Then here. What this machine could not take stays as it is here in the base too, so the
    // next pass does not read it as removed here and remove it everywhere.
    let mut base = merged.values.clone();
    if merged.values != local {
        for (field, why) in (item.apply)(&wanted)? {
            match local.get(&field) {
                Some(value) => base.insert(field.clone(), value.clone()),
                None => base.remove(&field),
            };
            report.skipped.push(if item.name.is_empty() {
                format!("{field}: {why}")
            } else {
                format!("{}: {field}: {why}", item.name)
            });
        }
        report.changed(item.kind);
    }
    report
        .overwritten
        .extend(merged.overwritten.into_iter().map(|o| OverwriteRecord {
            at: ctx.now,
            kind: item.kind.to_owned(),
            label: if item.name.is_empty() {
                o.key.clone()
            } else {
                format!("{}: {}", item.name, o.key)
            },
            kept: o.kept,
            lost: if item.secret { None } else { o.lost },
        }));
    state.rows.insert(id, RowState { version, base });
    Ok(true)
}

/// `haryx sync status`: local only, no network (`packages/contracts/haryx/sync-status.json`).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub signed_in: bool,
    pub unlocked: bool,
    /// RFC 3339, UTC.
    pub last_sync_at: Option<String>,
    pub other_machine_accounts: Vec<RemoteAccount>,
    pub overwritten: Vec<OverwriteRecord>,
}

pub fn status(root: &Path, vault: &Vault, signed_in_as: Option<(&str, &str)>) -> Status {
    let state = signed_in_as
        .map(|(user, project)| State::load(root, user, project))
        .unwrap_or_default();
    Status {
        signed_in: signed_in_as.is_some(),
        unlocked: signed_in_as.is_some_and(|(u, p)| is_unlocked(vault, u, p)),
        last_sync_at: state.last_sync_at.and_then(|t| {
            jiff::Timestamp::from_second(t)
                .ok()
                .map(|ts| ts.to_string())
        }),
        other_machine_accounts: state.other_machine_accounts,
        overwritten: state.overwritten,
    }
}

#[cfg(test)]
mod tests;

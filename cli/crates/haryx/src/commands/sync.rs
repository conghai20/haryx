//! `haryx sync …`: the command side of [`haryx_core::sync`].
//!
//! `--json` shapes: `packages/contracts/haryx/sync-result.json`, `sync-status.json`,
//! `sync-profiles.json` and `app-prefs.json`. Every JSON failure is
//! `{"ok": false, "code": …, "message": …}` on stdout with exit code 1.

use std::io::{BufRead as _, IsTerminal as _, Read as _};

use haryx_core::account::{Backend, Session, SessionStore};
use haryx_core::printer::{accent, dimmed, warning};
use haryx_core::profile::{Category, ProfileEnv};
use haryx_core::switcher::{Switcher, prompt, prompt_secret};
use haryx_core::sync::merge::{Side, Values};
use haryx_core::sync::profiles::{self, Storage};
use haryx_core::sync::remote::Postgrest;
use haryx_core::sync::{self, Context, Report, SyncError, sources};
use haryx_core::{PROG, paths};
use serde_json::{Value, json};

use super::{CommandError, guard, profile};
use crate::cli::{PartFlags, PluginsChoice, SecretsChoice, SyncAction};

pub fn run(action: &SyncAction, debug: bool) -> Result<(), CommandError> {
    guard::refuse_root()?;
    let json = wants_json(action);
    let result = match action {
        SyncAction::PrefsGet => prefs_get(),
        SyncAction::PrefsSet => prefs_set(),
        SyncAction::Status { json } => status(*json),
        other => with_session(other, debug),
    };
    result.map_err(|err| failure(&err, json))
}

fn wants_json(action: &SyncAction) -> bool {
    match action {
        SyncAction::Run { json }
        | SyncAction::Status { json }
        | SyncAction::HasPassphrase { json }
        | SyncAction::Unlock { json, .. }
        | SyncAction::Passphrase { json, .. }
        | SyncAction::Reset { json, .. }
        | SyncAction::Forget { json, .. }
        | SyncAction::ProfileList { json } => *json,
        SyncAction::PrefsGet | SyncAction::PrefsSet => true,
        SyncAction::ProfilePush { .. }
        | SyncAction::ProfilePull { .. }
        | SyncAction::ProfileRemove { .. } => false,
    }
}

fn failure(err: &SyncError, json: bool) -> CommandError {
    if json {
        println!(
            "{}",
            json!({"ok": false, "code": err.code(), "message": err.to_string()})
        );
        return CommandError::silent(1);
    }
    CommandError {
        message: format!("Error: {err}"),
        exit_code: 1,
    }
}

fn local(err: impl std::fmt::Display) -> SyncError {
    SyncError::Local(err.to_string())
}

fn now() -> i64 {
    jiff::Timestamp::now().as_second()
}

fn store_root() -> Result<std::path::PathBuf, SyncError> {
    paths::store_root().map_err(local)
}

/// The session for the compiled-in (or configured) project, renewed if needed.
fn session(store: &SessionStore, backend: &Backend) -> Result<Session, SyncError> {
    store.current(backend, now())?.ok_or(SyncError::NotSignedIn)
}

fn with_session(action: &SyncAction, debug: bool) -> Result<(), SyncError> {
    let switcher = Switcher::from_process(debug).map_err(local)?;
    let root = switcher.backup_dir().to_path_buf();
    let store = SessionStore::for_store(&root);
    let backend = Backend::from_env();
    let session = session(&store, &backend)?;
    let remote = Postgrest::new(backend.base(), backend.key(), &session.access_token);
    let objects = Storage::new(backend.base(), backend.key(), &session.access_token);
    let ctx = Context {
        root: &root,
        switcher: &switcher,
        vault: store.vault(),
        remote: &remote,
        objects: &objects,
        user: &session.user.id,
        project: backend.base(),
        now: now(),
    };
    match action {
        SyncAction::Run { json } => run_sync(&ctx, *json),
        SyncAction::HasPassphrase { .. } => {
            let has = sync::has_passphrase(&remote)?;
            println!("{}", json!({"ok": true, "hasPassphrase": has}));
            Ok(())
        }
        SyncAction::Unlock {
            passphrase_stdin,
            json,
        } => unlock(&ctx, *passphrase_stdin, *json),
        SyncAction::Passphrase {
            passphrase_stdin,
            json,
        } => change_passphrase(&ctx, *passphrase_stdin, *json),
        SyncAction::Reset { yes, json } => reset(&ctx, *yes, *json),
        SyncAction::Forget { email, json } => {
            let removed = sync::forget(&ctx, email)?;
            if *json {
                println!("{}", json!({"ok": true, "removed": removed}));
            } else if removed == 0 {
                println!("No account {email} is synced.");
            } else {
                println!("{email} is no longer offered to your other machines.");
            }
            Ok(())
        }
        SyncAction::ProfileList { json } => profile_list(&ctx, *json),
        SyncAction::ProfilePush { name } => profile_push(&ctx, name, debug),
        SyncAction::ProfilePull { name, force } => profile_pull(&ctx, name, *force, debug),
        SyncAction::ProfileRemove { name } => {
            if profiles::remove(&ctx, name)? {
                println!("Removed profile '{name}' from the server.");
            } else {
                println!("No profile '{name}' on the server.");
            }
            Ok(())
        }
        SyncAction::Status { .. } | SyncAction::PrefsGet | SyncAction::PrefsSet => {
            unreachable!("handled without a session")
        }
    }
}

// --- sync -------------------------------------------------------------------------------------

fn run_sync(ctx: &Context<'_>, json: bool) -> Result<(), SyncError> {
    let report = sync::run(ctx)?;
    if json {
        let mut value = serde_json::to_value(&report).map_err(local)?;
        if let Some(map) = value.as_object_mut() {
            map.insert("ok".into(), Value::Bool(true));
        }
        println!("{value}");
        return Ok(());
    }
    print_report(&report);
    Ok(())
}

fn print_report(report: &Report) {
    if report.pushed == 0 && report.pulled == 0 {
        println!("Already in sync.");
    } else {
        println!(
            "Synced: {} sent, {} received{}.",
            report.pushed,
            report.pulled,
            if report.changed.is_empty() {
                String::new()
            } else {
                format!(" ({} changed here)", report.changed.join(", "))
            }
        );
    }
    for o in &report.overwritten {
        let kept = match o.kept {
            Side::Local => "this machine's value",
            Side::Remote => "the other machine's value",
        };
        println!("{}", dimmed(&format!("  {}: kept {kept}", o.label)));
    }
    for s in &report.skipped {
        warning(&format!("  Not applied: {s}"));
    }
    if !report.other_machine_accounts.is_empty() {
        println!("On your other machines, not signed in here:");
        for a in &report.other_machine_accounts {
            let alias = if a.alias.is_empty() {
                String::new()
            } else {
                format!(" ({})", a.alias)
            };
            println!("  {}{alias}", a.email);
        }
        println!(
            "{}",
            dimmed(&format!(
                "Sign one in with '{} login', then run '{PROG} sync'.",
                haryx_core::AI_PROG
            ))
        );
    }
}

fn status(json: bool) -> Result<(), SyncError> {
    let root = store_root()?;
    let store = SessionStore::for_store(&root);
    let backend = Backend::from_env();
    let signed_in = store
        .load()?
        .filter(|s| s.project == backend.base())
        .map(|s| s.user.id);
    let status = sync::status(
        &root,
        store.vault(),
        signed_in.as_deref().map(|u| (u, backend.base())),
    );
    if json {
        println!("{}", serde_json::to_value(&status).map_err(local)?);
        return Ok(());
    }
    match (status.signed_in, status.unlocked) {
        (false, _) => println!("Not signed in to HaryX. Run '{PROG} login' to sign in."),
        (true, false) => println!("Signed in, but sync is locked here. Run '{PROG} sync unlock'."),
        (true, true) => println!(
            "Sync is unlocked. Last synced: {}.",
            status.last_sync_at.as_deref().unwrap_or("never")
        ),
    }
    for a in &status.other_machine_accounts {
        println!("  On another machine: {}", a.email);
    }
    Ok(())
}

// --- passphrase -------------------------------------------------------------------------------

fn read_stdin_line() -> Result<String, SyncError> {
    let mut line = String::new();
    std::io::stdin()
        .lock()
        .read_line(&mut line)
        .map_err(local)?;
    Ok(line.trim_end_matches(['\n', '\r']).to_owned())
}

/// The passphrase from stdin, or asked for (twice when it is being set).
fn passphrase(from_stdin: bool, setting: bool) -> Result<String, SyncError> {
    if from_stdin {
        return read_stdin_line();
    }
    if !std::io::stdin().is_terminal() {
        return Err(SyncError::Invalid(
            "No terminal to ask for the passphrase; use --passphrase-stdin".into(),
        ));
    }
    let ask = |q: &str| prompt_secret(q).ok_or_else(|| SyncError::Invalid("No passphrase".into()));
    if !setting {
        return ask("Sync passphrase: ");
    }
    println!(
        "{}",
        dimmed(
            "Choose a sync passphrase, at least 12 characters. HaryX cannot recover it; any machine \
             still unlocked can set a new one."
        )
    );
    let first = ask("New sync passphrase: ")?;
    sync::crypto::check_passphrase(&first)?;
    if ask("Again: ")? != first {
        return Err(SyncError::Invalid("The passphrases differ".into()));
    }
    Ok(first)
}

fn unlock(ctx: &Context<'_>, from_stdin: bool, json: bool) -> Result<(), SyncError> {
    let setting = !sync::has_passphrase(ctx.remote)?;
    let pass = passphrase(from_stdin, setting)?;
    let unlocked = sync::unlock(ctx, &pass)?;
    if json {
        println!("{}", json!({"ok": true, "created": unlocked.created}));
    } else if unlocked.created {
        println!(
            "{} Your sync passphrase is set. Run '{PROG} sync'.",
            accent("Unlocked.")
        );
    } else {
        println!("{} Run '{PROG} sync'.", accent("Unlocked."));
    }
    Ok(())
}

fn change_passphrase(ctx: &Context<'_>, from_stdin: bool, json: bool) -> Result<(), SyncError> {
    let pass = passphrase(from_stdin, true)?;
    sync::change_passphrase(ctx, &pass)?;
    if json {
        println!("{}", json!({"ok": true}));
    } else {
        println!("Sync passphrase changed. Other machines stay unlocked.");
    }
    Ok(())
}

fn reset(ctx: &Context<'_>, yes: bool, json: bool) -> Result<(), SyncError> {
    if !yes {
        if !std::io::stdin().is_terminal() {
            return Err(SyncError::Invalid("Confirm with --yes".into()));
        }
        let answer = prompt(
            "Delete everything synced from the server and lock this machine? Local data stays. [y/N] ",
        )
        .unwrap_or_default();
        if !answer.trim().eq_ignore_ascii_case("y") {
            println!("Nothing changed.");
            return Ok(());
        }
    }
    sync::reset(ctx)?;
    if json {
        println!("{}", json!({"ok": true}));
    } else {
        println!(
            "Server copy deleted. Run '{PROG} sync unlock' to set a new passphrase, then '{PROG} sync'."
        );
    }
    Ok(())
}

// --- app preferences --------------------------------------------------------------------------

fn prefs_get() -> Result<(), SyncError> {
    let (values, _) = sources::read_app_prefs(&store_root()?)?;
    println!("{}", serde_json::to_value(values).map_err(local)?);
    Ok(())
}

fn prefs_set() -> Result<(), SyncError> {
    let mut text = String::new();
    std::io::stdin().read_to_string(&mut text).map_err(local)?;
    let values: Values = serde_json::from_str(&text)
        .map_err(|e| SyncError::Invalid(format!("prefs set expects one JSON object: {e}")))?;
    let root = store_root()?;
    if sources::read_app_prefs(&root)?.0 != values {
        sources::write_app_prefs(&root, &values)?;
    }
    println!("{}", json!({"ok": true}));
    Ok(())
}

// --- profiles ---------------------------------------------------------------------------------

fn profile_list(ctx: &Context<'_>, json: bool) -> Result<(), SyncError> {
    let list = profiles::list(ctx)?;
    if json {
        println!("{}", json!({"profiles": list}));
        return Ok(());
    }
    if list.is_empty() {
        println!("No profiles on the server. Upload one with '{PROG} sync profile push NAME'.");
    }
    for p in list {
        println!("  {}  {} KB", p.name, p.bytes.div_ceil(1024));
    }
    Ok(())
}

/// `<store>/sync/tmp` (0700) and the name of a work file in it for profile `name`.
fn work_file(name: &str) -> Result<(std::path::PathBuf, String), SyncError> {
    haryx_core::profile::store::validate_name(name)
        .map_err(|e| SyncError::Invalid(e.to_string()))?;
    let dir = store_root()?.join("sync").join("tmp");
    Ok((dir, format!("{name}-{}.haryx-profile", std::process::id())))
}

fn profile_push(ctx: &Context<'_>, name: &str, debug: bool) -> Result<(), SyncError> {
    let env = ProfileEnv::from_process().map_err(local)?;
    let (dir, file_name) = work_file(name)?;
    // An empty private file first, so the export lands in a folder only this user can read.
    haryx_core::account::vault::write_private(&dir, &file_name, "")?;
    let file = dir.join(file_name);
    let parts = PartFlags {
        exclude: vec![Category::History],
        ..PartFlags::default()
    };
    let exported = profile::export(
        &env,
        name,
        &file.to_string_lossy(),
        &parts,
        SecretsChoice::Redact,
        PluginsChoice::Ref,
        debug,
    );
    let bytes = exported
        .map_err(|e| SyncError::Invalid(e.message))
        .and_then(|()| std::fs::read(&file).map_err(local));
    let _ = std::fs::remove_file(&file);
    let entry = profiles::push(ctx, name, &bytes?)?;
    println!(
        "Uploaded profile '{}' ({} KB, encrypted; secrets redacted, no history).",
        entry.name,
        entry.bytes.div_ceil(1024)
    );
    Ok(())
}

fn profile_pull(ctx: &Context<'_>, name: &str, force: bool, debug: bool) -> Result<(), SyncError> {
    let bytes = profiles::pull(ctx, name)?
        .ok_or_else(|| SyncError::Invalid(format!("No profile '{name}' on the server")))?;
    let env = ProfileEnv::from_process().map_err(local)?;
    let (dir, file_name) = work_file(name)?;
    haryx_core::account::vault::write_private(&dir, &file_name, &bytes)?;
    let file = dir.join(file_name);
    let imported = profile::import(
        &env,
        name,
        &file.to_string_lossy(),
        false,
        false,
        force,
        false,
        false,
        debug,
    );
    let _ = std::fs::remove_file(&file);
    imported.map_err(|e| SyncError::Invalid(e.message))
}

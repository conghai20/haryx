//! `haryx account consents|consent|export|machines|machine rename`: what HaryX keeps about the
//! signed-in user on its server, as the Privacy center shows it on the web.
//!
//! `--json` shapes: `consents` and `consent grant|withdraw` print `consents.json`; `machines`
//! prints `machines.json` and `machine rename` one of its items; `export` prints
//! `{"ok": true, "path": …}`. A failure is `{"ok": false, "code": …, "message": …}` on stdout with
//! exit code 1, as for `sync`.

use std::io::IsTerminal as _;
use std::path::{Path, PathBuf};

use haryx_core::account::api::{self as account_api, POLICY_VERSION, Source};
use haryx_core::account::{Backend, SessionStore};
use haryx_core::contracts::{Consent, Consents, Machine, Purpose};
use haryx_core::fsutil::{WriteOptions, atomic_write_bytes};
use haryx_core::printer::{accent, dimmed};
use haryx_core::switcher::prompt;
use haryx_core::sync::SyncError;
use haryx_core::sync::remote::Api;
use haryx_core::{PROG, machine};
use serde_json::json;

use super::CommandError;
use super::sync::{failure, session};
use crate::cli::AccountAction;

pub fn run(
    action: &AccountAction,
    root: &Path,
    store: &SessionStore,
    backend: &Backend,
) -> Result<(), CommandError> {
    let api = session(store, backend)
        .map(|s| Api::new(backend, store, &s.access_token))
        .map_err(|err| failure(&err, action.wants_json()))?;
    let this_machine = machine::machine_id(root);
    let result = match action {
        AccountAction::Consents { json } => {
            account_api::consents(&api, account_api::TIMEOUT).map(|c| print_consents(&c, *json))
        }
        AccountAction::Consent {
            grant: true,
            purpose,
            source,
            json,
            ..
        } => grant(&api, root, *purpose, *source, *json),
        AccountAction::Consent {
            grant: false,
            purpose,
            source,
            yes,
            json,
        } => withdraw(&api, *purpose, *source, *yes, *json),
        AccountAction::Export { path, json } => export(&api, path.as_deref(), *json),
        AccountAction::Machines { json } => {
            account_api::machines(&api, &this_machine).map(|m| print_machines(&m.machines, *json))
        }
        AccountAction::RenameMachine { id, name, json } => {
            account_api::rename_machine(&api, id, name, &this_machine)
                .map(|m| print_renamed(&m, *json))
        }
        _ => unreachable!("not a server verb"),
    };
    result.map_err(|err| failure(&err, action.wants_json()))
}

fn label(purpose: Purpose) -> &'static str {
    match purpose {
        Purpose::Sync => "Sync",
        Purpose::Machines => "Show your machines on HaryX web",
    }
}

/// What withdrawing `purpose` erases, said before it is.
fn erased(purpose: Purpose) -> &'static str {
    match purpose {
        Purpose::Sync => {
            "Withdrawing Sync stops sync on every machine and deletes HaryX's server copy of your \
             synced data: settings, accounts, credentials, profile files and usage. What is on \
             each machine stays."
        }
        Purpose::Machines => {
            "Withdrawing it deletes your machines from HaryX web: their names, systems, versions \
             and when they were last seen. Sign-in and sync are not affected."
        }
    }
}

fn consent_line(c: &Consent) -> String {
    let state = if c.granted { "on" } else { "off" };
    let detail = match (&c.version, &c.at) {
        (Some(version), Some(at)) => dimmed(&format!(
            " ({} {at}, privacy policy {version})",
            if c.granted { "since" } else { "withdrawn" }
        )),
        _ => String::new(),
    };
    format!("  {}: {}{detail}", label(c.purpose), accent(state))
}

fn print_consents(consents: &Consents, json: bool) {
    if json {
        println!("{}", json!(consents));
        return;
    }
    println!("Your HaryX consents:");
    for c in &consents.consents {
        println!("{}", consent_line(c));
    }
    println!(
        "{}",
        dimmed(&format!(
            "Change one with '{PROG} account consent grant|withdraw sync|machines'."
        ))
    );
}

fn grant(
    api: &Api,
    root: &Path,
    purpose: Purpose,
    source: Source,
    json: bool,
) -> Result<(), SyncError> {
    let consents = account_api::grant(api, purpose, source)?;
    // Show this machine at once rather than at its next sync.
    if purpose == Purpose::Machines
        && let Err(err) = account_api::heartbeat(api, root)
    {
        log::warn!("could not add this machine to HaryX web yet: {err}");
    }
    if json {
        println!("{}", json!(consents));
    } else {
        println!(
            "{} is on for your HaryX ID (privacy policy {POLICY_VERSION}).",
            label(purpose)
        );
    }
    Ok(())
}

fn withdraw(
    api: &Api,
    purpose: Purpose,
    source: Source,
    yes: bool,
    json: bool,
) -> Result<(), SyncError> {
    if !yes {
        if !std::io::stdin().is_terminal() {
            return Err(SyncError::Invalid(format!(
                "{} Confirm with --yes.",
                erased(purpose)
            )));
        }
        println!("{}", erased(purpose));
        let answer = prompt(&format!("Withdraw '{}'? [y/N] ", label(purpose))).unwrap_or_default();
        if !answer.trim().eq_ignore_ascii_case("y") {
            println!("Nothing changed.");
            return Ok(());
        }
    }
    let consents = account_api::withdraw(api, purpose, source)?;
    if json {
        println!("{}", json!(consents));
    } else {
        println!(
            "Withdrawn: {}. Its data is gone from the server.",
            label(purpose)
        );
    }
    Ok(())
}

fn export_path(path: Option<&Path>) -> Result<PathBuf, SyncError> {
    let path = path.map(Path::to_path_buf).unwrap_or_else(|| {
        PathBuf::from(format!("haryx-export-{}.json", jiff::Zoned::now().date()))
    });
    if path.is_absolute() {
        return Ok(path);
    }
    let cwd = std::env::current_dir().map_err(|e| SyncError::Local(e.to_string()))?;
    Ok(cwd.join(path))
}

fn export(api: &Api, path: Option<&Path>, json: bool) -> Result<(), SyncError> {
    let path = export_path(path)?;
    let body = account_api::export(api)?;
    let private = WriteOptions {
        mode: 0o600,
        follow_symlinks: false,
    };
    atomic_write_bytes(&path, &body, private)
        .map_err(|e| SyncError::Local(format!("Could not write {}: {e}", path.display())))?;
    if json {
        println!("{}", json!({"ok": true, "path": path}));
    } else {
        println!(
            "Saved your HaryX data to {}.",
            accent(&path.display().to_string())
        );
        println!(
            "{}",
            dimmed("Synced data in it stays encrypted: only your sync passphrase opens it.")
        );
    }
    Ok(())
}

fn machine_line(m: &Machine) -> String {
    let this = if m.is_this_machine {
        " (this machine)"
    } else {
        ""
    };
    let out = if m.signed_in { "" } else { ", signed out" };
    format!(
        "  {}{this}  {} {}, last seen {}{out}\n{}",
        accent(&m.name),
        m.os,
        m.version,
        m.last_seen_at,
        dimmed(&format!("    id {}", m.machine_id))
    )
}

fn print_machines(machines: &[Machine], json: bool) {
    if json {
        println!("{}", json!({"machines": machines}));
        return;
    }
    if machines.is_empty() {
        println!(
            "No machines are shown on HaryX web. Run '{PROG} account consent grant machines' to \
             show this one."
        );
        return;
    }
    println!("Your machines on HaryX web:");
    for m in machines {
        println!("{}", machine_line(m));
    }
}

fn print_renamed(machine: &Machine, json: bool) {
    if json {
        println!("{}", json!(machine));
    } else {
        println!("Renamed to {}.", accent(&machine.name));
    }
}

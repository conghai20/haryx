//! `haryx login`, `logout`, `whoami`, `account delete`: the command side of
//! [`haryx_core::account`]. The other `account` verbs are in [`super::privacy`].
//!
//! `--json` shapes: `packages/contracts/haryx/whoami.json` and `login-events.json`; `account
//! delete --json` answers `{"ok": false, "code": "unsupported", "message": …}`: a WeCoAp ID is
//! deleted on its account page, not by HaryX (plan A10).

use haryx_core::account::{
    self, AccountError, Backend, DeviceCode, LOGIN_TIMEOUT, Provider, SessionStore, User, browser,
};

use std::time::Duration;

use haryx_core::account::api as account_api;
use haryx_core::contracts::{ConsentState, Purpose};
use haryx_core::printer::{accent, dimmed};
use haryx_core::sync::SyncError;
use haryx_core::sync::remote::Api;
use haryx_core::{AI_PROG, PROG, paths, sync};
use serde_json::{Value, json};

use super::{CommandError, guard, privacy};
use crate::cli::AccountAction;

/// `whoami` answers without the consents rather than wait longer for them.
const CONSENTS_TIMEOUT: Duration = Duration::from_secs(5);

pub fn run(action: &AccountAction) -> Result<(), CommandError> {
    guard::refuse_root()?;
    let (root, store) = session_store()?;
    let backend = Backend::from_env();
    match *action {
        AccountAction::Login {
            provider,
            no_browser,
            json,
        } => login(&backend, &store, provider, no_browser, json),
        AccountAction::Logout { json } => logout(&backend, &store, &root, json),
        AccountAction::Whoami { json } => whoami(&backend, &store, json),
        AccountAction::Delete { json, .. } => delete(&backend, json),
        _ => privacy::run(action, &root, &store, &backend),
    }
}

fn session_store() -> Result<(std::path::PathBuf, SessionStore), CommandError> {
    let root = paths::store_root().map_err(|err| CommandError {
        message: format!("Error: {err}"),
        exit_code: 1,
    })?;
    let store = SessionStore::for_store(&root);
    Ok((root, store))
}

/// Signed out here: the sync key and sync state go too, so a later sign-in as someone else starts
/// clean. Best effort; the session is already gone.
fn forget_sync(root: &std::path::Path, store: &SessionStore) {
    if let Err(err) = sync::forget_local(root, store.vault()) {
        log::warn!("could not clear this machine's sync key: {err}");
    }
}

fn user_json(user: &User) -> Value {
    json!({"id": user.id, "email": user.email, "provider": user.provider})
}

/// The `whoami.json` shape.
fn status_json(user: Option<&User>) -> Value {
    json!({"signedIn": user.is_some(), "user": user.map(user_json)})
}

fn print_line(value: &Value) {
    println!("{value}");
}

fn provider_label(user: &User) -> &str {
    match user.provider.as_str() {
        "github" => "GitHub",
        "google" => "Google",
        "apple" => "Apple",
        "email" => "email",
        other => other,
    }
}

fn login(
    backend: &Backend,
    store: &SessionStore,
    provider: Option<Provider>,
    no_browser: bool,
    json: bool,
) -> Result<(), CommandError> {
    let with = provider
        .map(|p| format!(" with {}", p.label()))
        .unwrap_or_default();
    let show_url = |url: &str| {
        let opened = browser::open(url);
        if json {
            print_line(&json!({"event": "browser", "url": url, "opened": opened}));
        } else if opened {
            println!("Opening your browser to sign in to HaryX{with}…");
            println!(
                "{}",
                dimmed(&format!("If it did not open, visit:\n  {url}"))
            );
        } else {
            println!("Open this link to sign in to HaryX{with}:\n  {url}");
        }
    };
    // The same `browser` event, never opened: the link is for another device.
    let show_device = |device: &DeviceCode| {
        if json {
            print_line(&json!({"event": "browser", "url": device.url, "opened": false}));
        } else {
            println!(
                "To sign in to HaryX, open this link on any device:\n  {}\nand check that it \
                 shows the code {}.",
                device.url,
                accent(&device.user_code)
            );
        }
    };
    let signed_in = if no_browser {
        account::login_device(backend, store, LOGIN_TIMEOUT, show_device)
    } else {
        account::login(backend, store, provider, LOGIN_TIMEOUT, show_url)
    };
    match signed_in {
        Ok(user) if json => {
            print_line(&json!({"event": "signed_in", "user": user_json(&user)}));
            Ok(())
        }
        Ok(user) => {
            println!(
                "Signed in to HaryX as {} ({}).",
                accent(&user.email),
                provider_label(&user)
            );
            Ok(())
        }
        Err(err) => Err(failure(&err, json)),
    }
}

fn logout(
    backend: &Backend,
    store: &SessionStore,
    root: &std::path::Path,
    json: bool,
) -> Result<(), CommandError> {
    let user = account::logout(backend, store).map_err(|err| failure(&err, json))?;
    forget_sync(root, store);
    if json {
        print_line(&status_json(None));
    } else if let Some(user) = user {
        println!("Signed out of HaryX ({}).", user.email);
    } else {
        println!("This machine was not signed in to HaryX.");
    }
    Ok(())
}

/// The consents `whoami` shows: `Ok(None)` when they cannot be read now (offline, another
/// issuer), `Err` with the reason when this machine turns out to be signed out (a renewal refused,
/// as after a sign-out from the web).
fn consents_now(
    backend: &Backend,
    store: &SessionStore,
) -> Result<Option<Vec<ConsentState>>, String> {
    let now = jiff::Timestamp::now().as_second();
    let read = store
        .current(backend, now)
        .map_err(SyncError::from)
        .and_then(|session| {
            let Some(session) = session else {
                return Ok(None);
            };
            let api = Api::new(backend, store, &session.access_token);
            account_api::consents(&api, CONSENTS_TIMEOUT).map(Some)
        });
    match read {
        Ok(consents) => Ok(consents.map(|c| {
            c.consents
                .into_iter()
                .map(|c| ConsentState {
                    purpose: c.purpose,
                    granted: c.granted,
                    version: c.version,
                })
                .collect()
        })),
        Err(SyncError::Account(AccountError::SignedOut(why))) => Err(why),
        Err(err) => {
            log::debug!("whoami without consents: {err}");
            Ok(None)
        }
    }
}

fn not_signed_in(why: &str, json: bool) -> Result<(), CommandError> {
    if json {
        print_line(&status_json(None));
    } else {
        println!("Not signed in to HaryX: {why}.");
    }
    Ok(())
}

fn consents_text(consents: &[ConsentState]) -> String {
    let state = |purpose| {
        let on = consents.iter().any(|c| c.purpose == purpose && c.granted);
        if on { "on" } else { "off" }
    };
    format!(
        "Sync: {}. Shown on HaryX web: {}.",
        state(Purpose::Sync),
        state(Purpose::Machines)
    )
}

fn whoami(backend: &Backend, store: &SessionStore, json: bool) -> Result<(), CommandError> {
    let session = match store.load() {
        // A session from the sign-in this replaced: signed out, and the text says what to do.
        Err(AccountError::SignedOut(why)) => return not_signed_in(&why, json),
        other => other.map_err(|err| failure(&err, json))?,
    };
    let consents = match &session {
        Some(_) => match consents_now(backend, store) {
            Err(why) => return not_signed_in(&why, json),
            Ok(consents) => consents,
        },
        None => None,
    };
    let user = session.as_ref().map(|s| &s.user);
    if json {
        let mut status = status_json(user);
        if let (Some(consents), Some(map)) = (&consents, status.as_object_mut()) {
            map.insert("consents".into(), json!(consents));
        }
        print_line(&status);
    } else if let Some(user) = user {
        println!(
            "Signed in to HaryX as {} ({}).",
            accent(&user.email),
            provider_label(user)
        );
        if let Some(consents) = &consents {
            println!("{}", dimmed(&consents_text(consents)));
        }
    } else {
        println!("Not signed in to HaryX. Run '{PROG} login' to sign in.");
        println!(
            "{}",
            dimmed(&format!("Claude accounts are listed by '{AI_PROG} list'."))
        );
    }
    Ok(())
}

/// With `--json`, the error is an event on stdout and only the exit code remains.
fn failure(err: &AccountError, json: bool) -> CommandError {
    if json {
        print_line(&json!({"event": "error", "code": err.code(), "message": err.to_string()}));
        return CommandError::silent(1);
    }
    CommandError {
        message: format!("Error: {err}"),
        exit_code: 1,
    }
}

/// HaryX does not delete accounts: the WeCoAp ID is shared by every WeCoAp product, and is
/// deleted on its own account page (plan A10).
fn delete(backend: &Backend, json: bool) -> Result<(), CommandError> {
    let message = format!(
        "Your HaryX account is your WeCoAp ID. Delete it on the WeCoAp account page: {}",
        backend.account_url()
    );
    if json {
        print_line(&json!({"ok": false, "code": "unsupported", "message": message}));
        return Err(CommandError::silent(1));
    }
    Err(CommandError {
        message: format!("Error: {message}"),
        exit_code: 1,
    })
}

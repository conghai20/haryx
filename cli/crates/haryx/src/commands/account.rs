//! `haryx login`, `logout`, `whoami`, `account delete`: the command side of
//! [`haryx_core::account`].
//!
//! `--json` shapes: `packages/contracts/haryx/whoami.json` and `login-events.json`; `account
//! delete --json` answers `{"ok": false, "code": "unsupported", "message": …}`: a WeCoAp ID is
//! deleted on its account page, not by HaryX (plan A10).

use haryx_core::account::{
    self, AccountError, Backend, DeviceCode, LOGIN_TIMEOUT, Provider, SessionStore, User, browser,
};

use haryx_core::printer::{accent, dimmed};
use haryx_core::{AI_PROG, PROG, paths, sync};
use serde_json::{Value, json};

use super::{CommandError, guard};
use crate::cli::AccountAction;

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
        AccountAction::Whoami { json } => whoami(&store, json),
        AccountAction::Delete { json, .. } => delete(&backend, json),
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

fn whoami(store: &SessionStore, json: bool) -> Result<(), CommandError> {
    let session = match store.load() {
        // A session from the sign-in this replaced: signed out, and the text says what to do.
        Err(AccountError::SignedOut(why)) => {
            if !json {
                println!("Not signed in to HaryX: {why}.");
                return Ok(());
            }
            None
        }
        other => other.map_err(|err| failure(&err, json))?,
    };
    let user = session.as_ref().map(|s| &s.user);
    if json {
        print_line(&status_json(user));
    } else if let Some(user) = user {
        println!(
            "Signed in to HaryX as {} ({}).",
            accent(&user.email),
            provider_label(user)
        );
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

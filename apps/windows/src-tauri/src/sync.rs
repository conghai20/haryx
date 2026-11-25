//! HaryX account sync, run through the CLI (`haryx sync --json`): at launch, every 15 minutes, 5 s
//! after a synced preference changes, and after a Claude sign-in. Only one sync or preference push
//! runs at a time. The app never holds the HaryX session or the sync key; the CLI does.
//!
//! Automatic runs happen only when the Sync preference is on and `sync status` (local, no network)
//! says this PC is signed in and unlocked. A failure is reported to the UI as status, never a dialog.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tauri::{AppHandle, Emitter, Manager};

use crate::cli::{Cli, DEFAULT_TIMEOUT, Output};
use crate::prefsync::{self, Synced};
use crate::settings::{self, Prefs};
use crate::state::AppState;

/// Between automatic syncs.
pub const PERIOD: Duration = Duration::from_secs(15 * 60);
/// After a synced preference changes, so a burst of changes syncs once.
pub const DEBOUNCE: Duration = Duration::from_secs(5);

#[derive(Default)]
pub struct Syncer {
    /// Held by a sync or a preference push: never two at once.
    lock: Mutex<()>,
    running: AtomicBool,
    /// The last `sync --json` outcome, for a window opened later.
    last: Mutex<Option<Value>>,
    nudge: Mutex<Option<Sender<Duration>>>,
}

impl Syncer {
    /// What the UI draws (`SyncInfo` in `src/lib/types.ts`).
    pub fn info(&self) -> Value {
        json!({
            "running": self.running.load(Ordering::SeqCst),
            "last": *self.last.lock().unwrap_or_else(|e| e.into_inner()),
        })
    }
}

/// A sync's outcome from the CLI's output: its JSON when it printed an object (success or the
/// `{"ok": false, "code", "message"}` failure), else a `local` failure with its message.
pub fn outcome(out: &Output) -> Value {
    match serde_json::from_str::<Value>(out.stdout.trim()) {
        Ok(value) if value.is_object() => value,
        _ => failure("local", &out.error_message()),
    }
}

fn failure(code: &str, message: &str) -> Value {
    json!({ "ok": false, "code": code, "message": message })
}

/// Whether a sync result says the app preferences changed on this PC.
pub fn prefs_changed(result: &Value) -> bool {
    result.get("ok").and_then(Value::as_bool) == Some(true)
        && result
            .get("changed")
            .and_then(Value::as_array)
            .is_some_and(|kinds| kinds.iter().any(|k| k == "app-prefs"))
}

/// Whether `sync status --json` says sync can run here.
pub fn ready(status: &Value) -> bool {
    let flag = |k: &str| status.get(k).and_then(Value::as_bool) == Some(true);
    flag("signedIn") && flag("unlocked")
}

/// Start the loop: a sync at launch, then one every [`PERIOD`] or when [`request`]ed.
pub fn start_loop(app: &AppHandle) {
    let (tx, rx) = mpsc::channel::<Duration>();
    *app.state::<Syncer>()
        .nudge
        .lock()
        .unwrap_or_else(|e| e.into_inner()) = Some(tx);
    let app = app.clone();
    std::thread::spawn(move || {
        let mut due = Instant::now();
        loop {
            match rx.recv_timeout(due.saturating_duration_since(Instant::now())) {
                // A later request moves the deadline: that is the debounce.
                Ok(delay) => due = Instant::now() + delay,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    run(&app, true);
                    due = Instant::now() + PERIOD;
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            }
        }
    });
}

/// Ask for an automatic sync `delay` from now (a newer request replaces an older one).
pub fn request(app: &AppHandle, delay: Duration) {
    if let Some(tx) = app
        .state::<Syncer>()
        .nudge
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
    {
        let _ = tx.send(delay);
    }
}

fn cli(app: &AppHandle) -> Result<Cli, String> {
    app.state::<Arc<AppState>>().cli()
}

/// Run one sync and publish it. `auto` runs only when the preference is on and this PC is signed in
/// and unlocked, and returns `None` otherwise (nothing to report).
pub fn run(app: &AppHandle, auto: bool) -> Option<Value> {
    let syncer = app.state::<Syncer>();
    let Ok(_guard) = syncer.lock.try_lock() else {
        return (!auto).then(|| failure("busy", "A sync is already running."));
    };
    let cli = match cli(app) {
        Ok(cli) => cli,
        Err(e) if auto => {
            log::debug!("sync skipped: {e}");
            return None;
        }
        Err(e) => return Some(failure("local", &e)),
    };
    if auto {
        let enabled = app.state::<Arc<AppState>>().prefs().sync_enabled;
        if !enabled
            || !cli
                .json(&["sync", "status", "--json"])
                .is_ok_and(|s| ready(&s))
        {
            return None;
        }
    }
    syncer.running.store(true, Ordering::SeqCst);
    let _ = app.emit("sync", syncer.info());
    let result = match cli.run(&["sync", "--json"], None, &[], DEFAULT_TIMEOUT) {
        Ok(out) => outcome(&out),
        Err(e) => failure("local", &e.to_string()),
    };
    if prefs_changed(&result) {
        pull_prefs(app, &cli);
    }
    if result.get("ok").and_then(Value::as_bool) == Some(true) {
        // Account metadata may have arrived.
        app.state::<Arc<AppState>>().refresh_soon();
    }
    *syncer.last.lock().unwrap_or_else(|e| e.into_inner()) = Some(result.clone());
    syncer.running.store(false, Ordering::SeqCst);
    let _ = app.emit("sync", syncer.info());
    Some(result)
}

/// Read `sync prefs get` and apply what this app understands.
fn pull_prefs(app: &AppHandle, cli: &Cli) {
    let remote = match cli.json(&["sync", "prefs", "get"]) {
        Ok(Value::Object(map)) => map,
        Ok(_) => return,
        Err(e) => {
            log::warn!("sync prefs get: {e}");
            return;
        }
    };
    let state = app.state::<Arc<AppState>>();
    let current = state.prefs();
    let next = prefsync::apply(&current, &remote);
    if next == current {
        return;
    }
    if let Err(e) = settings::save(&state.config_dir, &next) {
        log::warn!("saving synced preferences: {e}");
        return;
    }
    *state.prefs.lock().unwrap_or_else(|e| e.into_inner()) = next.clone();
    crate::tray::apply_theme(app);
    let _ = app.emit("prefs", &next);
}

/// A preference change was saved: push the synced keys that moved, then sync shortly after.
pub fn prefs_saved(app: &AppHandle, old: &Prefs, new: &Prefs) {
    if !new.sync_enabled {
        return;
    }
    if !old.sync_enabled {
        request(app, Duration::ZERO);
    }
    let changed = prefsync::changed(old, new);
    if changed.is_empty() {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        if push_prefs(&app, changed) {
            request(&app, DEBOUNCE);
        }
    });
}

/// `prefs get`, lay `changed` over it, `prefs set`. Only when signed in, so a PC that never signs in
/// writes nothing. Returns whether it pushed.
fn push_prefs(app: &AppHandle, changed: Synced) -> bool {
    let Ok(cli) = cli(app) else { return false };
    let syncer = app.state::<Syncer>();
    let _guard = syncer.lock.lock().unwrap_or_else(|e| e.into_inner());
    let signed_in = cli
        .json(&["sync", "status", "--json"])
        .is_ok_and(|s| s.get("signedIn").and_then(Value::as_bool) == Some(true));
    if !signed_in {
        return false;
    }
    let remote = match cli.json(&["sync", "prefs", "get"]) {
        Ok(Value::Object(map)) => map,
        Ok(_) => Synced::new(),
        Err(e) => {
            log::warn!("sync prefs get: {e}");
            return false;
        }
    };
    let body = Value::Object(prefsync::overlay(remote, changed)).to_string();
    match cli.run(&["sync", "prefs", "set"], Some(&body), &[], DEFAULT_TIMEOUT) {
        Ok(out) if out.success() => true,
        Ok(out) => {
            log::warn!("sync prefs set: {}", out.error_message());
            false
        }
        Err(e) => {
            log::warn!("sync prefs set: {e}");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RESULT: &str = include_str!("../../../../packages/contracts/haryx/sync-result.json");
    const STATUS: &str = include_str!("../../../../packages/contracts/haryx/sync-status.json");

    fn out(status: i32, stdout: &str) -> Output {
        Output {
            status,
            stdout: stdout.into(),
            stderr: String::new(),
        }
    }

    #[test]
    fn outcome_keeps_the_cli_json_on_success_and_failure() {
        let ok = outcome(&out(0, RESULT));
        assert_eq!(ok["ok"], true);
        assert_eq!(ok["otherMachineAccounts"][0]["email"], "work@example.com");
        let failed = outcome(&out(
            1,
            r#"{"ok":false,"code":"key_changed","message":"Unlock again."}"#,
        ));
        assert_eq!(failed["code"], "key_changed");
        let garbled = outcome(&Output {
            status: 2,
            stdout: "panic".into(),
            stderr: "Error: boom".into(),
        });
        assert_eq!(garbled["ok"], false);
        assert_eq!(garbled["code"], "local");
        assert_eq!(garbled["message"], "Error: boom");
    }

    #[test]
    fn app_prefs_reload_only_when_they_changed() {
        let fixture: Value = serde_json::from_str(RESULT).unwrap();
        assert!(
            !prefs_changed(&fixture),
            "the fixture changed settings and accounts"
        );
        assert!(prefs_changed(
            &json!({"ok": true, "changed": ["app-prefs"]})
        ));
        assert!(!prefs_changed(
            &json!({"ok": false, "changed": ["app-prefs"]})
        ));
        assert!(!prefs_changed(&json!({"ok": true})));
    }

    #[test]
    fn ready_needs_signed_in_and_unlocked() {
        let status: Value = serde_json::from_str(STATUS).unwrap();
        assert!(ready(&status));
        assert!(!ready(&json!({"signedIn": true, "unlocked": false})));
        assert!(!ready(&json!({"signedIn": false, "unlocked": true})));
        assert!(!ready(&json!({})));
    }
}

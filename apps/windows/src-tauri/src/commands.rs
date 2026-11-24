//! What the UI may ask the backend to do. Every CLI call goes through [`haryx`], which only runs the
//! haryx program, never a shell, and only with a first word from [`ALLOWED_VERBS`].

use std::path::PathBuf;
use std::sync::Arc;

use serde::Serialize;
use serde_json::{Value, json};
use tauri::{AppHandle, Emitter, Manager, State};

use crate::autoswitch::AutoEvent;
use crate::cli::DEFAULT_TIMEOUT;
use crate::settings::{self, Prefs};
use crate::state::{self, AppState};
use crate::terminals::{self, Job, Terminal};

/// First words the UI may send. `run`, `tui` and `watch` need a terminal and go through
/// [`open_terminal`]; long-running `auto` goes through [`auto_start`].
pub const ALLOWED_VERBS: &[&str] = &[
    "list",
    "status",
    "switch",
    "alias",
    "threshold",
    "priority",
    "move",
    "swap",
    "map",
    "unmap",
    "disable",
    "enable",
    "remove",
    "config",
    "export",
    "import",
    "import-usage",
    "unclaimed",
    "upgrade",
    "purge",
    "add",
    "add-token",
    "ai",
];

/// `auto --once` is allowed (a single tick); a looping `auto` is not.
pub fn is_allowed(args: &[String]) -> bool {
    match args.first().map(String::as_str) {
        Some("auto") => args.iter().any(|a| a == "--once"),
        Some(verb) => ALLOWED_VERBS.contains(&verb),
        None => false,
    }
}

#[derive(Debug, Serialize)]
pub struct RunResult {
    pub ok: bool,
    pub status: i32,
    pub stdout: String,
    pub message: String,
}

/// Run `haryx <args>` (with optional stdin, e.g. a token or a `y` confirmation the UI already asked
/// for). `passphrase` opens an encrypted setup profile; it reaches the CLI only as
/// `HARYX_PROFILE_PASSPHRASE`, never in argv. Refreshes the accounts afterwards.
#[tauri::command]
pub async fn haryx(
    state: State<'_, Arc<AppState>>,
    args: Vec<String>,
    stdin: Option<String>,
    passphrase: Option<String>,
) -> Result<RunResult, String> {
    if !is_allowed(&args) {
        return Err(format!(
            "not allowed: haryx {}",
            args.first().map_or("", String::as_str)
        ));
    }
    let state = Arc::clone(&state);
    tauri::async_runtime::spawn_blocking(move || {
        let cli = state.cli()?;
        let argv: Vec<&str> = args.iter().map(String::as_str).collect();
        let env: Vec<(&str, &str)> = passphrase
            .as_deref()
            .filter(|p| !p.is_empty())
            .map(|p| vec![("HARYX_PROFILE_PASSPHRASE", p)])
            .unwrap_or_default();
        let out = cli
            .run(&argv, stdin.as_deref(), &env, DEFAULT_TIMEOUT)
            .map_err(|e| e.to_string())?;
        state.refresh_soon();
        Ok(RunResult {
            ok: out.success(),
            status: out.status,
            message: if out.success() {
                crate::cli::strip_ansi(out.stdout.trim().lines().last().unwrap_or(""))
            } else {
                out.error_message()
            },
            stdout: out.stdout,
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Run a `--json` command and return its parsed output.
#[tauri::command]
pub async fn haryx_json(
    state: State<'_, Arc<AppState>>,
    args: Vec<String>,
) -> Result<Value, String> {
    if !is_allowed(&args) {
        return Err("not allowed".into());
    }
    let state = Arc::clone(&state);
    tauri::async_runtime::spawn_blocking(move || {
        let argv: Vec<&str> = args.iter().map(String::as_str).collect();
        state.cli()?.json(&argv)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Everything the UI needs to draw itself at once.
#[tauri::command]
pub fn snapshot(state: State<'_, Arc<AppState>>) -> Value {
    let terminals: Vec<Value> = state
        .terminals
        .available()
        .into_iter()
        .map(|t| json!({ "id": t, "title": t.title() }))
        .collect();
    json!({
        "accounts": *state.accounts_json.lock().unwrap_or_else(|e| e.into_inner()),
        "error": *state.last_error.lock().unwrap_or_else(|e| e.into_inner()),
        "sessions": *state.sessions_json.lock().unwrap_or_else(|e| e.into_inner()),
        "prefs": state.prefs(),
        "auto": { "running": state.runner.is_running(), "log": state.runner.log.lock().unwrap_or_else(|e| e.into_inner()).snapshot() },
        "terminals": terminals,
        "cliPath": state.cli().ok().map(|c| c.exe.display().to_string()),
        "storeRoot": state::store_root().map(|p| p.display().to_string()),
        "platform": std::env::consts::OS,
    })
}

#[tauri::command]
pub fn refresh(state: State<'_, Arc<AppState>>) {
    state.refresh_soon();
}

#[tauri::command]
pub fn set_prefs(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    prefs: Prefs,
) -> Result<(), String> {
    settings::save(&state.config_dir, &prefs).map_err(|e| e.to_string())?;
    let bundled = bundled_bin(&app);
    *state.cli.write().unwrap_or_else(|e| e.into_inner()) =
        state::locate_cli(&prefs, bundled.as_deref());
    *state.prefs.lock().unwrap_or_else(|e| e.into_inner()) = prefs.clone();
    crate::autostart::apply(&app, &prefs);
    let _ = app.emit("prefs", &prefs);
    state.refresh_soon();
    Ok(())
}

/// The folder the installer puts the bundled haryx.exe in.
pub fn bundled_bin(app: &AppHandle) -> Option<PathBuf> {
    app.path().resource_dir().ok().map(|d| d.join("bin"))
}

#[tauri::command]
pub fn auto_start(app: AppHandle, state: State<'_, Arc<AppState>>) -> Result<(), String> {
    let cli = state.cli()?;
    let dry_run = state.prefs().auto_switch_dry_run;
    let emitter = app.clone();
    state.runner.start(&cli, dry_run, move |event: &AutoEvent| {
        let _ = emitter.emit("auto-event", event);
        let state = emitter.state::<Arc<AppState>>();
        if event.kind == "switch" {
            state.refresh_soon();
        }
        if let Some(title) = &event.notification_title
            && state.prefs().notify_auto_switch
        {
            crate::notify::show(&emitter, title, &event.summary);
        }
    })
}

#[tauri::command]
pub fn auto_stop(state: State<'_, Arc<AppState>>) {
    state.runner.stop();
}

/// Open a terminal in `folder` (else the preferred Claude folder, else the home folder) running `job`.
#[tauri::command]
pub fn open_terminal(
    state: State<'_, Arc<AppState>>,
    job: Job,
    terminal: Option<Terminal>,
    folder: Option<String>,
) -> Result<(), String> {
    let prefs = state.prefs();
    let terminal = terminal
        .or(prefs.terminal)
        .or_else(|| state.terminals.available().first().copied())
        .ok_or("no terminal found")?;
    let folder = folder
        .map(PathBuf::from)
        .or(prefs.claude_folder)
        .or_else(|| std::env::var_os("USERPROFILE").map(PathBuf::from))
        .ok_or("no folder to open")?;
    let haryx = state.cli()?.exe;
    let argv = terminals::launch_argv(terminal, &job, &haryx, &folder, &state.terminals)?;
    terminals::launch(&argv, &folder)
}

#[tauri::command]
pub fn open_dashboard(app: AppHandle, pane: Option<String>) {
    crate::tray::show_dashboard(&app, pane.as_deref());
}

#[tauri::command]
pub fn hide_flyout(app: AppHandle) {
    crate::tray::hide_flyout(&app);
}

#[tauri::command]
pub fn quit(app: AppHandle, state: State<'_, Arc<AppState>>) {
    state.runner.stop();
    app.exit(0);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(words: &[&str]) -> Vec<String> {
        words.iter().map(|w| (*w).to_string()).collect()
    }

    #[test]
    fn only_allowlisted_verbs_run() {
        assert!(is_allowed(&args(&["switch", "2"])));
        assert!(is_allowed(&args(&["ai", "wake", "status", "--json"])));
        assert!(is_allowed(&args(&["auto", "--once", "--json"])));
        assert!(
            !is_allowed(&args(&["auto", "--json"])),
            "the loop has its own command"
        );
        assert!(!is_allowed(&args(&["run", "2"])), "run needs a terminal");
        assert!(!is_allowed(&args(&["tui"])));
        assert!(!is_allowed(&[]));
    }
}

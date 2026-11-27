//! What the UI may ask the backend to do. Every CLI call goes through [`haryx`], which only runs the
//! haryx program, never a shell, and only with a first word from [`ALLOWED_VERBS`].

use std::path::PathBuf;
use std::sync::Arc;

use serde::Serialize;
use serde_json::{Value, json};
use tauri::{AppHandle, Emitter, Manager, State};

use crate::autoswitch::AutoEvent;
use crate::cli::DEFAULT_TIMEOUT;
use crate::login::Logins;
use crate::settings::{self, Prefs};
use crate::state::{self, AppState};
use crate::terminals::{self, Job, Terminal};
use crate::updater::{self, UpdateMode};

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
    // HaryX account and sync. The long-running `login` has its own command
    // ([`account_login_start`]); passphrases and preferences go on stdin, never in argv.
    "login",
    "logout",
    "whoami",
    "account",
    "sync",
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
        "update": crate::updater::snapshot_json(&state, &state.update.lock().unwrap_or_else(|e| e.into_inner()).clone()),
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
    let old = state.prefs();
    let bundled = bundled_bin(&app);
    *state.cli.write().unwrap_or_else(|e| e.into_inner()) =
        state::locate_cli(&prefs, bundled.as_deref());
    *state.prefs.lock().unwrap_or_else(|e| e.into_inner()) = prefs.clone();
    crate::autostart::apply(&app, &prefs);
    crate::tray::apply_theme(&app);
    let _ = app.emit("prefs", &prefs);
    crate::sync::prefs_saved(&app, &old, &prefs);
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
    if terminal == Terminal::Warp {
        let app_data = std::env::var_os("APPDATA").ok_or("APPDATA is not set")?;
        let dir = PathBuf::from(app_data).join(r"warp\Warp\data\launch_configurations");
        return terminals::open_warp(&job, &haryx, &folder, &dir);
    }
    let argv = terminals::launch_argv(
        terminal,
        &job,
        &haryx,
        &folder,
        &state.terminals,
        prefs.shell,
    )?;
    terminals::launch(&argv, &folder)
}

/// Open the dashboard on `pane`; `account` also selects that account on Accounts.
#[tauri::command]
pub fn open_dashboard(app: AppHandle, pane: Option<String>, account: Option<u32>) {
    match account {
        Some(slot) => crate::tray::reveal_account(&app, slot),
        None => crate::tray::show_dashboard(&app, pane.as_deref()),
    }
}

/// The folder mappings `haryx map` keeps (`<store root>/mappings.json`), as macOS reads them.
#[tauri::command]
pub fn mappings() -> Vec<crate::models::Mapping> {
    state::store_root()
        .and_then(|root| std::fs::read_to_string(root.join("mappings.json")).ok())
        .map(|text| crate::models::parse_mappings(&text))
        .unwrap_or_default()
}

/// Post a test toast, so a person can see notifications reach them (and fix Windows' settings if not).
#[tauri::command]
pub fn notify_test(app: AppHandle) {
    crate::notify::show(
        &app,
        "HaryX notifications work",
        "You'll hear from your mascots here.",
    );
}

/// The CLI's log in the data folder (rotated copies sit next to it).
pub const LOG_FILE: &str = "haryx.log";

/// Show haryx's data folder (`what: "store"`) or its log file (`"log"`) in Explorer. The UI names
/// the target, never a path.
#[tauri::command]
pub fn reveal(what: String) -> Result<(), String> {
    let root = state::store_root().ok_or("haryx's data folder is unknown")?;
    let args: Vec<std::ffi::OsString> = match what.as_str() {
        "store" if root.is_dir() => vec![root.into_os_string()],
        "store" => return Err("haryx has no data folder yet: add an account first".into()),
        "log" => {
            let log = root.join(LOG_FILE);
            if !log.is_file() {
                return Err(format!(
                    "No log yet: haryx writes {} on its first command",
                    log.display()
                ));
            }
            if cfg!(windows) {
                vec!["/select,".into(), log.into_os_string()]
            } else {
                vec!["-R".into(), log.into_os_string()]
            }
        }
        other => return Err(format!("cannot show {other}")),
    };
    let opener = if cfg!(windows) {
        "explorer.exe"
    } else {
        "open"
    };
    std::process::Command::new(opener)
        .args(&args)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("cannot open Explorer: {e}"))
}

#[tauri::command]
pub fn hide_flyout(app: AppHandle) {
    crate::tray::hide_flyout(&app);
}

#[tauri::command]
pub fn quit(app: AppHandle, state: State<'_, Arc<AppState>>) {
    state.runner.stop();
    updater::install_on_quit(&state);
    app.exit(0);
}

/// Open one of HaryX's GitHub pages in the browser.
#[tauri::command]
pub fn open_link(url: String) -> Result<(), String> {
    updater::open_link(&url)
}

/// Look up the latest release (`refresh` skips the day-old cache).
#[tauri::command]
pub async fn update_check(app: AppHandle, refresh: bool) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || updater::check(&app, refresh))
        .await
        .map_err(|e| e.to_string())?
}

/// Download and check the available installer.
#[tauri::command]
pub async fn update_download(app: AppHandle) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || updater::download(&app))
        .await
        .map_err(|e| e.to_string())?
}

/// Install the downloaded update and reopen HaryX.
#[tauri::command]
pub fn update_restart(app: AppHandle, state: State<'_, Arc<AppState>>) -> Result<(), String> {
    let installer = updater::ready_installer(&state).ok_or("no update is ready")?;
    updater::launch_installer(&installer, true)?;
    state.runner.stop();
    app.exit(0);
    Ok(())
}

/// Change the update mode or the skipped version (`skip: ""` clears it).
#[tauri::command]
pub fn update_set(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    mode: Option<UpdateMode>,
    skip: Option<String>,
) -> Result<(), String> {
    let old = state.prefs();
    let mut prefs = old.clone();
    if let Some(mode) = mode {
        prefs.update_mode = mode;
    }
    if let Some(skip) = skip {
        prefs.skipped_version = Some(skip).filter(|s| !s.is_empty());
    }
    settings::save(&state.config_dir, &prefs).map_err(|e| e.to_string())?;
    *state.prefs.lock().unwrap_or_else(|e| e.into_inner()) = prefs.clone();
    let _ = app.emit("prefs", &prefs);
    crate::sync::prefs_saved(&app, &old, &prefs);
    let update = state
        .update
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    let _ = app.emit("update", updater::snapshot_json(&state, &update));
    if prefs.update_mode == UpdateMode::Automatic
        && update.phase == "idle"
        && update.available(prefs.skipped_version.as_deref()).is_some()
    {
        std::thread::spawn(move || {
            let _ = updater::download(&app);
        });
    }
    Ok(())
}

/// Sign in (again, into `slot`, or a new account) in the app's own sign-in window.
#[tauri::command]
pub fn login_start(app: AppHandle, slot: Option<u32>, title: String) -> Result<u64, String> {
    crate::login::start(&app, slot, title)
}

#[tauri::command]
pub fn login_get(logins: State<'_, Logins>, id: u64) -> Option<crate::login::View> {
    logins.view(id)
}

/// Type a line (the pasted code, or a y/n answer) into the run, with the Return a terminal sends.
#[tauri::command]
pub fn login_send(logins: State<'_, Logins>, id: u64, line: String) -> Result<(), String> {
    logins.send(id, &line)
}

#[tauri::command]
pub fn login_cancel(logins: State<'_, Logins>, id: u64) {
    logins.cancel(id);
}

/// Open the sign-in link the run printed. The UI names the run, never the URL.
#[tauri::command]
pub fn login_open(logins: State<'_, Logins>, id: u64) -> Result<(), String> {
    let url = logins.url(id).ok_or("no sign-in link yet")?;
    updater::open_in_browser(&url)
}

/// Start the HaryX sign-in (`haryx login --provider <provider> --json`); its events arrive as
/// `account-login`.
#[tauri::command]
pub fn account_login_start(
    app: AppHandle,
    login: State<'_, crate::account::AccountLogin>,
    provider: String,
) -> Result<(), String> {
    login.start(&app, &provider)
}

#[tauri::command]
pub fn account_login_cancel(login: State<'_, crate::account::AccountLogin>) {
    login.cancel();
}

/// Open the sign-in page again. The UI names the run, never the URL.
#[tauri::command]
pub fn account_login_open(login: State<'_, crate::account::AccountLogin>) -> Result<(), String> {
    updater::open_in_browser(&login.url().ok_or("no sign-in link yet")?)
}

/// Sync now (Sync now, or after unlocking); never alongside another sync.
#[tauri::command]
pub async fn sync_now(app: AppHandle) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        crate::sync::run(&app, false).unwrap_or(Value::Null)
    })
    .await
    .map_err(|e| e.to_string())
}

/// Sync after a synced account setting changed here (a group added, removed, renamed or deleted),
/// as the automatic sync does: only when sync is on and this PC is signed in and unlocked, and
/// quietly skipped otherwise or while another sync runs (the 15-minute loop picks the change up).
#[tauri::command]
pub async fn sync_after_change(app: AppHandle) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        crate::sync::run(&app, true).unwrap_or(Value::Null)
    })
    .await
    .map_err(|e| e.to_string())
}

/// Whether a sync is running, and the last one's outcome.
#[tauri::command]
pub fn sync_info(syncer: State<'_, crate::sync::Syncer>) -> Value {
    syncer.info()
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

    #[test]
    fn account_and_sync_verbs_are_allowed_and_nothing_else_new() {
        for words in [
            &["login", "--provider", "github", "--json"][..],
            &["logout", "--json"],
            &["whoami", "--json"],
            &["account", "delete", "--yes", "--json"],
            &["sync", "--json"],
            &["sync", "status", "--json"],
            &["sync", "unlock", "--passphrase-stdin", "--json"],
            &["sync", "prefs", "set"],
        ] {
            assert!(is_allowed(&args(words)), "{words:?}");
        }
        for verb in [
            "run",
            "tui",
            "watch",
            "shell",
            "cmd",
            "powershell",
            "server",
            "logins",
        ] {
            assert!(!is_allowed(&args(&[verb])), "{verb}");
        }
        assert_eq!(
            ALLOWED_VERBS.len(),
            28,
            "a new verb needs its own review and test"
        );
    }
}

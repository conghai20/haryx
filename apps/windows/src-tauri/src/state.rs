//! Shared app state and the two background loops: accounts (`list --json`, every refresh interval, at
//! once when nudged, and when `sequence.json` changes) and sessions (`ai sessions --json`, every 4 s).
//! Each result goes to the UI as an event and through the notification rules.

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, SystemTime};

use serde_json::{Value, json};
use tauri::{AppHandle, Emitter, Manager};

use crate::autoswitch::Runner;
use crate::cli::{self, Cli};
use crate::models::{Account, AccountList, SessionList};
use crate::rules::{self, Dedupe};
use crate::settings::{self, Prefs};
use crate::terminals::{self, Installed};

/// How often running sessions are read (cheap: a folder listing per profile).
pub const SESSIONS_INTERVAL: Duration = Duration::from_secs(4);

pub struct AppState {
    pub config_dir: PathBuf,
    pub prefs: Mutex<Prefs>,
    pub cli: RwLock<Option<Cli>>,
    pub runner: Runner,
    pub terminals: Installed,
    /// The last good `list --json`, as the UI gets it.
    pub accounts_json: Mutex<Option<Value>>,
    pub accounts: Mutex<Vec<Account>>,
    pub last_error: Mutex<Option<String>>,
    pub sessions_json: Mutex<Value>,
    pub sessions: Mutex<SessionList>,
    /// In-app update progress (`updater`).
    pub update: Mutex<crate::updater::UpdateState>,
    dedupe: Mutex<Dedupe>,
    nudge: Mutex<Option<Sender<()>>>,
}

impl AppState {
    pub fn new(config_dir: PathBuf, bundled_bin: Option<PathBuf>) -> Self {
        let prefs = settings::load(&config_dir);
        let cli = locate_cli(&prefs, bundled_bin.as_deref());
        AppState {
            config_dir,
            prefs: Mutex::new(prefs),
            cli: RwLock::new(cli),
            runner: Runner::default(),
            terminals: detect_terminals(),
            accounts_json: Mutex::new(None),
            accounts: Mutex::new(Vec::new()),
            last_error: Mutex::new(None),
            sessions_json: Mutex::new(json!({"sessions": [], "unreadable": 0})),
            sessions: Mutex::new(SessionList {
                sessions: Vec::new(),
                unreadable: 0,
            }),
            update: Mutex::new(crate::updater::UpdateState::new(env!("CARGO_PKG_VERSION"))),
            dedupe: Mutex::new(Dedupe::default()),
            nudge: Mutex::new(None),
        }
    }

    pub fn cli(&self) -> Result<Cli, String> {
        self.cli
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .ok_or_else(|| cli::CliError::NotFound.to_string())
    }

    pub fn prefs(&self) -> Prefs {
        self.prefs.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Ask the accounts loop to refresh now.
    pub fn refresh_soon(&self) {
        if let Some(tx) = self
            .nudge
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
        {
            let _ = tx.send(());
        }
    }

    /// The active account, if the last list had one.
    pub fn active_account(&self) -> Option<Account> {
        self.accounts
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .find(|a| a.active)
            .cloned()
    }

    /// Whether a Claude session runs as the active account.
    pub fn active_is_running(&self) -> bool {
        let Some(active) = self.active_account() else {
            return false;
        };
        self.sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .sessions
            .iter()
            .any(|s| s.account == Some(active.number))
    }
}

pub fn locate_cli(prefs: &Prefs, bundled: Option<&std::path::Path>) -> Option<Cli> {
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from);
    let candidates = cli::candidates(
        prefs.haryx_path.as_deref(),
        std::env::var_os("PATH").as_deref(),
        home.as_deref(),
        bundled,
    );
    cli::locate(&candidates).map(Cli::new)
}

fn detect_terminals() -> Installed {
    let env_path = |name: &str| std::env::var_os(name).map(PathBuf::from);
    terminals::detect(
        std::env::var_os("PATH").as_deref(),
        env_path("LOCALAPPDATA").as_deref(),
        env_path("ProgramFiles").as_deref(),
        env_path("SystemRoot").as_deref(),
    )
}

/// haryx's store, where `sequence.json` lives: `HARYX_HOME`, else the platform default the CLI uses.
pub fn store_root() -> Option<PathBuf> {
    if let Some(home) = std::env::var_os("HARYX_HOME").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(home));
    }
    if cfg!(windows) {
        return std::env::var_os("LOCALAPPDATA").map(|d| PathBuf::from(d).join("haryx"));
    }
    let home = PathBuf::from(std::env::var_os("HOME")?);
    if cfg!(target_os = "macos") {
        Some(home.join("Library/Application Support/HaryX"))
    } else {
        Some(home.join(".local/share/haryx"))
    }
}

fn modified(path: &std::path::Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// Start both loops. `on_change` runs after every accounts or sessions update (tray refresh).
pub fn start_loops(app: &AppHandle, on_change: impl Fn(&AppHandle) + Send + Sync + 'static) {
    let (tx, rx) = mpsc::channel();
    let state = app.state::<Arc<AppState>>();
    *state.nudge.lock().unwrap_or_else(|e| e.into_inner()) = Some(tx.clone());
    let on_change = Arc::new(on_change);
    {
        let app = app.clone();
        let on_change = Arc::clone(&on_change);
        std::thread::spawn(move || accounts_loop(&app, &rx, on_change.as_ref()));
    }
    let app = app.clone();
    std::thread::spawn(move || sessions_loop(&app, &tx, on_change.as_ref()));
}

fn accounts_loop(
    app: &AppHandle,
    nudged: &Receiver<()>,
    on_change: &(dyn Fn(&AppHandle) + Send + Sync),
) {
    loop {
        refresh_accounts(app);
        on_change(app);
        let interval = app.state::<Arc<AppState>>().prefs().refresh_interval();
        match nudged.recv_timeout(interval) {
            Ok(()) => while nudged.try_recv().is_ok() {},
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        }
    }
}

/// Run `list --json` once, publish it, and send the notices it raises.
pub fn refresh_accounts(app: &AppHandle) {
    let state = app.state::<Arc<AppState>>();
    let result = state.cli().and_then(|cli| cli.json(&["list", "--json"]));
    match result {
        Ok(value) => {
            let parsed: Result<AccountList, _> = serde_json::from_value(value.clone());
            *state
                .accounts_json
                .lock()
                .unwrap_or_else(|e| e.into_inner()) = Some(value.clone());
            *state.last_error.lock().unwrap_or_else(|e| e.into_inner()) = None;
            let _ = app.emit("accounts", json!({ "list": value, "error": null }));
            if let Ok(list) = parsed {
                let previous = std::mem::replace(
                    &mut *state.accounts.lock().unwrap_or_else(|e| e.into_inner()),
                    list.accounts.clone(),
                );
                let notices = rules::evaluate(&previous, &list.accounts, jiff::Timestamp::now());
                let fresh = state
                    .dedupe
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .fresh(notices);
                let prefs = state.prefs();
                for notice in fresh.into_iter().filter(|n| prefs.notifies(n.category)) {
                    crate::notify::show(app, &notice.title, &notice.body);
                }
            }
        }
        Err(message) => {
            *state.last_error.lock().unwrap_or_else(|e| e.into_inner()) = Some(message.clone());
            let _ = app.emit("accounts", json!({ "list": null, "error": message }));
        }
    }
}

fn sessions_loop(
    app: &AppHandle,
    nudge: &Sender<()>,
    on_change: &(dyn Fn(&AppHandle) + Send + Sync),
) {
    let sequence = store_root().map(|r| r.join("sequence.json"));
    let mut seen = sequence.as_deref().and_then(modified);
    loop {
        std::thread::sleep(SESSIONS_INTERVAL);
        // A switch made outside the app (a terminal, auto-switch) shows up at once.
        let now = sequence.as_deref().and_then(modified);
        if now != seen {
            seen = now;
            let _ = nudge.send(());
        }
        let state = app.state::<Arc<AppState>>();
        let Ok(cli) = state.cli() else { continue };
        let Ok(value) = cli.json(&["ai", "sessions", "--json"]) else {
            continue;
        };
        let changed = *state
            .sessions_json
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            != value;
        if !changed {
            continue;
        }
        if let Ok(list) = serde_json::from_value::<SessionList>(value.clone()) {
            *state.sessions.lock().unwrap_or_else(|e| e.into_inner()) = list;
        }
        *state
            .sessions_json
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = value.clone();
        let _ = app.emit("sessions", value);
        on_change(app);
    }
}

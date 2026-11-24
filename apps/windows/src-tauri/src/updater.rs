//! In-app updates. The bundled haryx does the network work: `upgrade --check --json` finds the latest
//! release (contract: `packages/contracts/haryx/upgrade-check.json`) and `upgrade --download-app
//! DIR --json` downloads this machine's setup.exe and checks its SHA-256. The app then runs that
//! installer silently once it has quit (a per-user install: no UAC prompt) and reopens itself.
//!
//! Modes (`Prefs::update_mode`): `automatic` downloads in the background and installs on quit or on
//! "Restart now"; `notify` (default) checks daily and shows the update; `manual` checks only when
//! asked.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tauri::{AppHandle, Emitter, Manager};

use crate::cli::Cli;
use crate::settings;
use crate::state::AppState;

/// How often the loop wakes to see whether a daily check is due.
pub const LOOP_INTERVAL: Duration = Duration::from_secs(3600);
pub const CHECK_EVERY_S: u64 = 24 * 3600;
/// Downloading an installer can take a while on a slow line.
pub const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(600);
const CHECK_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum UpdateMode {
    Automatic,
    #[default]
    Notify,
    Manual,
}

/// What the UI shows: the last check, and where an update stands.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateState {
    pub check: Option<Value>,
    /// `idle`, `checking`, `downloading`, `ready`, `failed`.
    pub phase: &'static str,
    pub error: Option<String>,
    /// The downloaded installer and its version, once ready.
    pub installer: Option<PathBuf>,
    pub ready_version: Option<String>,
    pub app_version: String,
}

impl UpdateState {
    pub fn new(app_version: &str) -> Self {
        UpdateState {
            phase: "idle",
            app_version: app_version.to_owned(),
            ..UpdateState::default()
        }
    }

    /// The release on offer: newer than this app and not skipped.
    pub fn available(&self, skipped: Option<&str>) -> Option<String> {
        let latest = self.check.as_ref()?.get("latest")?.as_str()?;
        (is_newer(latest, &self.app_version) && Some(latest) != skipped).then(|| latest.to_owned())
    }
}

/// Whether dotted release `latest` is newer than `current` (`0.1.10` > `0.1.9`). A pre-release
/// suffix sorts below its release.
pub fn is_newer(latest: &str, current: &str) -> bool {
    fn split(v: &str) -> (Vec<u64>, bool) {
        let v = v.strip_prefix('v').unwrap_or(v);
        let core: String = v
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '.')
            .collect();
        let parts = core
            .split('.')
            .filter(|p| !p.is_empty())
            .map(|p| p.parse().unwrap_or(0))
            .collect();
        (parts, core.len() < v.len())
    }
    let (l, l_pre) = split(latest);
    let (r, r_pre) = split(current);
    for i in 0..l.len().max(r.len()) {
        let (x, y) = (
            l.get(i).copied().unwrap_or(0),
            r.get(i).copied().unwrap_or(0),
        );
        if x != y {
            return x > y;
        }
    }
    !l_pre && r_pre
}

/// Whether a background check is due.
pub fn check_due(mode: UpdateMode, last_check: Option<u64>, now: u64) -> bool {
    mode != UpdateMode::Manual && last_check.is_none_or(|t| now.saturating_sub(t) >= CHECK_EVERY_S)
}

/// PowerShell single-quoted literal.
fn ps_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// The hidden PowerShell that waits for this app (`pid`) to exit, runs the installer silently, and
/// reopens the app when `relaunch`.
pub fn install_script(installer: &Path, app_exe: &Path, pid: u32, relaunch: bool) -> String {
    let mut script = format!(
        "Wait-Process -Id {pid} -ErrorAction SilentlyContinue; \
         $p = Start-Process -FilePath {} -ArgumentList '/S' -Wait -PassThru; \
         Remove-Item -LiteralPath {} -ErrorAction SilentlyContinue",
        ps_quote(&installer.display().to_string()),
        ps_quote(&installer.display().to_string()),
    );
    if relaunch {
        script.push_str(&format!(
            "; Start-Process -FilePath {}",
            ps_quote(&app_exe.display().to_string())
        ));
    }
    script
}

/// Start the installer script detached; it runs once the app has exited.
pub fn launch_installer(installer: &Path, relaunch: bool) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let script = install_script(installer, &exe, std::process::id(), relaunch);
    let mut command = std::process::Command::new("powershell.exe");
    command.args([
        "-NoProfile",
        "-NonInteractive",
        "-WindowStyle",
        "Hidden",
        "-Command",
        &script,
    ]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(DETACHED_PROCESS | CREATE_NO_WINDOW);
    }
    command
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("cannot start the installer: {e}"))
}

/// Only HaryX's own GitHub pages open from the app.
pub const LINK_PREFIX: &str = "https://github.com/conghai20/haryx";

pub fn is_allowed_link(url: &str) -> bool {
    url == LINK_PREFIX
        || url
            .strip_prefix(LINK_PREFIX)
            .is_some_and(|rest| rest.starts_with('/') && !rest.contains(['"', ' ', '\\']))
}

/// Open one of HaryX's GitHub pages in the default browser.
pub fn open_link(url: &str) -> Result<(), String> {
    if !is_allowed_link(url) {
        return Err(format!("not opening {url}"));
    }
    let opener = if cfg!(windows) {
        "explorer.exe"
    } else {
        "open"
    };
    std::process::Command::new(opener)
        .arg(url)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("cannot open {url}: {e}"))
}

fn now_s() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

// ------------------------------------------------------------------------------------- actions

/// The haryx that checks: the bundled one (same version as the app), else the one in use.
fn checker(app: &AppHandle, state: &AppState) -> Result<Cli, String> {
    let bundled = crate::commands::bundled_bin(app)
        .map(|dir| dir.join(crate::cli::program_name()))
        .filter(|p| p.is_file());
    match bundled {
        Some(exe) => Ok(Cli::new(exe)),
        None => state.cli(),
    }
}

fn set(app: &AppHandle, f: impl FnOnce(&mut UpdateState)) {
    let state = app.state::<Arc<AppState>>();
    let snapshot = {
        let mut update = state.update.lock().unwrap_or_else(|e| e.into_inner());
        f(&mut update);
        update.clone()
    };
    let _ = app.emit("update", snapshot_json(&state, &snapshot));
}

/// The update state as the UI gets it (with the skipped version and mode from the prefs).
pub fn snapshot_json(state: &AppState, update: &UpdateState) -> Value {
    let prefs = state.prefs();
    json!({
        "state": update,
        "available": update.available(prefs.skipped_version.as_deref()),
        "mode": prefs.update_mode,
        "skipped": prefs.skipped_version,
        "lastCheck": prefs.last_update_check,
    })
}

/// Look up the latest release.
pub fn check(app: &AppHandle, refresh: bool) -> Result<(), String> {
    let state = Arc::clone(&app.state::<Arc<AppState>>());
    let cli = checker(app, &state)?;
    set(app, |u| {
        if u.phase != "ready" {
            u.phase = "checking";
        }
    });
    let mut args = vec!["upgrade", "--check", "--json"];
    if refresh {
        args.push("--refresh");
    }
    let result = cli
        .run(&args, None, &[], CHECK_TIMEOUT)
        .map_err(|e| e.to_string())
        .and_then(|out| {
            serde_json::from_str::<Value>(out.stdout.trim()).map_err(|_| {
                format!(
                    "this haryx ({}) is too old to check for updates",
                    cli.exe.display()
                )
            })
        });
    let mut prefs = state.prefs();
    prefs.last_update_check = Some(now_s());
    let _ = settings::save(&state.config_dir, &prefs);
    *state.prefs.lock().unwrap_or_else(|e| e.into_inner()) = prefs;
    let error = match &result {
        Ok(v) => v.get("error").and_then(Value::as_str).map(str::to_owned),
        Err(e) => Some(e.clone()),
    };
    set(app, |u| {
        if let Ok(v) = result {
            u.check = Some(v);
        }
        if u.phase != "ready" {
            u.phase = if error.is_some() { "failed" } else { "idle" };
            u.error = error.clone();
        }
    });
    error.map_or(Ok(()), Err)
}

/// Download and check the available installer into the app's `updates` folder.
pub fn download(app: &AppHandle) -> Result<(), String> {
    let state = Arc::clone(&app.state::<Arc<AppState>>());
    let cli = checker(app, &state)?;
    // Local (not roaming) storage: an installer has no business following the user around.
    let dir = app
        .path()
        .app_cache_dir()
        .map_err(|e| e.to_string())?
        .join("updates");
    set(app, |u| {
        u.phase = "downloading";
        u.error = None;
    });
    let dir_text = dir.display().to_string();
    let result = cli
        .run(
            &["upgrade", "--download-app", &dir_text, "--json"],
            None,
            &[],
            DOWNLOAD_TIMEOUT,
        )
        .map_err(|e| e.to_string())
        .and_then(|out| {
            let v: Value = serde_json::from_str(out.stdout.trim())
                .map_err(|_| "this haryx cannot download app updates".to_string())?;
            if let Some(err) = v.get("error").and_then(Value::as_str) {
                return Err(err.to_owned());
            }
            let path = v
                .get("path")
                .and_then(Value::as_str)
                .ok_or("no installer path")?;
            let version = v.get("version").and_then(Value::as_str).unwrap_or("");
            Ok((PathBuf::from(path), version.to_owned()))
        });
    set(app, |u| match &result {
        Ok((path, version)) => {
            u.phase = "ready";
            u.installer = Some(path.clone());
            u.ready_version = Some(version.clone());
        }
        Err(err) => {
            u.phase = "failed";
            u.error = Some(err.clone());
        }
    });
    result.map(|_| ())
}

/// The downloaded installer, if one is ready.
pub fn ready_installer(state: &AppState) -> Option<PathBuf> {
    let update = state.update.lock().unwrap_or_else(|e| e.into_inner());
    (update.phase == "ready")
        .then(|| update.installer.clone())
        .flatten()
}

/// On quit: a downloaded update goes in once the app has closed, without reopening it.
pub fn install_on_quit(state: &AppState) {
    if let Some(installer) = ready_installer(state) {
        let _ = launch_installer(&installer, false);
    }
}

/// Background loop: a daily check unless manual; in automatic mode, download what it finds.
pub fn start_loop(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        // Let the app settle (tray, first accounts read) before going online.
        std::thread::sleep(Duration::from_secs(20));
        loop {
            tick(&app);
            std::thread::sleep(LOOP_INTERVAL);
        }
    });
}

fn tick(app: &AppHandle) {
    let state = Arc::clone(&app.state::<Arc<AppState>>());
    let prefs = state.prefs();
    if check_due(prefs.update_mode, prefs.last_update_check, now_s()) {
        let _ = check(app, false);
    }
    let offered = {
        let update = state.update.lock().unwrap_or_else(|e| e.into_inner());
        update.phase == "idle" && update.available(prefs.skipped_version.as_deref()).is_some()
    };
    if prefs.update_mode == UpdateMode::Automatic && offered {
        let _ = download(app);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_versions() {
        assert!(is_newer("0.1.10", "0.1.9"));
        assert!(is_newer("v0.2.0", "0.1.4"));
        assert!(!is_newer("0.1.4", "0.1.4"));
        assert!(!is_newer("0.2", "0.2.0"));
        assert!(is_newer("1.0.0", "1.0.0-rc1"));
        assert!(!is_newer("1.0.0-rc1", "1.0.0"));
    }

    #[test]
    fn offers_a_newer_release_unless_skipped() {
        let mut state = UpdateState::new("0.1.4");
        assert_eq!(state.available(None), None);
        state.check = Some(json!({"latest": "0.2.0"}));
        assert_eq!(state.available(None).as_deref(), Some("0.2.0"));
        assert_eq!(state.available(Some("0.2.0")), None);
        state.check = Some(json!({"latest": "0.1.4"}));
        assert_eq!(state.available(None), None);
        state.check = Some(json!({"latest": null, "error": "offline"}));
        assert_eq!(state.available(None), None);
    }

    #[test]
    fn daily_checks_unless_manual() {
        let day = CHECK_EVERY_S;
        assert!(check_due(UpdateMode::Notify, None, 1000));
        assert!(!check_due(UpdateMode::Notify, Some(1000), 1000 + day - 1));
        assert!(check_due(UpdateMode::Automatic, Some(1000), 1000 + day));
        assert!(!check_due(UpdateMode::Manual, None, 1000));
    }

    #[test]
    fn install_script_waits_installs_silently_and_relaunches() {
        let script = install_script(
            Path::new(r"C:\Users\o'neil\AppData\Local\HaryX\updates\HaryX-0.2.0-x64-setup.exe"),
            Path::new(r"C:\Users\o'neil\AppData\Local\HaryX\HaryX.exe"),
            4242,
            true,
        );
        assert!(script.starts_with("Wait-Process -Id 4242"));
        assert!(script.contains(r"-FilePath 'C:\Users\o''neil\AppData\Local\HaryX\updates\HaryX-0.2.0-x64-setup.exe' -ArgumentList '/S' -Wait"));
        assert!(script.ends_with(
            r"Start-Process -FilePath 'C:\Users\o''neil\AppData\Local\HaryX\HaryX.exe'"
        ));
        let quiet = install_script(Path::new("s.exe"), Path::new("a.exe"), 1, false);
        assert!(!quiet.contains("a.exe"), "no relaunch on quit");
    }

    #[test]
    fn only_haryx_github_links_open() {
        assert!(is_allowed_link("https://github.com/conghai20/haryx"));
        assert!(is_allowed_link(
            "https://github.com/conghai20/haryx/releases/tag/v0.2.0"
        ));
        assert!(!is_allowed_link("https://github.com/conghai20/haryx-evil"));
        assert!(!is_allowed_link("https://example.com"));
        assert!(!is_allowed_link(
            "https://github.com/conghai20/haryx/x\" & calc"
        ));
    }

    #[test]
    fn modes_serialize_lowercase() {
        assert_eq!(
            serde_json::to_value(UpdateMode::Automatic).unwrap(),
            "automatic"
        );
        let mode: UpdateMode = serde_json::from_value(json!("manual")).unwrap();
        assert_eq!(mode, UpdateMode::Manual);
    }
}

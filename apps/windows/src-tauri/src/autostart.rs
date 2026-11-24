//! Launch at login (HKCU Run key on Windows), following the dashboard's switch.

use tauri::AppHandle;
use tauri_plugin_autostart::ManagerExt;

use crate::settings::Prefs;

/// Make the OS setting match `prefs.launch_at_login`; failures are logged.
pub fn apply(app: &AppHandle, prefs: &Prefs) {
    let manager = app.autolaunch();
    let enabled = manager.is_enabled().unwrap_or(false);
    let result = match (prefs.launch_at_login, enabled) {
        (true, false) => manager.enable(),
        (false, true) => manager.disable(),
        _ => Ok(()),
    };
    if let Err(e) = result {
        log::warn!("launch at login: {e}");
    }
}

//! The app's own preferences (not haryx's settings, which the CLI owns): a small JSON file in the app's
//! config folder, written atomically. Unknown or missing keys fall back to defaults, so an old or
//! hand-edited file never stops the app.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::terminals::Terminal;
use crate::updater::UpdateMode;

pub const FILE_NAME: &str = "preferences.json";
/// Polling bounds for `list --json`: never hammer the usage endpoint, never go stale for long.
pub const MIN_REFRESH_S: u64 = 30;
pub const MAX_REFRESH_S: u64 = 900;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Prefs {
    /// Seconds between account refreshes.
    pub refresh_seconds: u64,
    /// The terminal quick actions open.
    pub terminal: Option<Terminal>,
    /// Where "Claude in a folder" starts, when no folder is picked.
    pub claude_folder: Option<PathBuf>,
    /// A specific haryx.exe instead of the one found on PATH.
    pub haryx_path: Option<PathBuf>,
    /// Start auto-switch when the app starts.
    pub auto_switch_on_launch: bool,
    /// Auto-switch only reports what it would do.
    pub auto_switch_dry_run: bool,
    pub notify_switched: bool,
    pub notify_usage: bool,
    pub notify_login: bool,
    pub notify_reset: bool,
    pub notify_auto_switch: bool,
    /// Show the active account's mascot in the tray instead of the HaryX mark.
    pub tray_mascot: bool,
    /// Start HaryX when you sign in to Windows.
    pub launch_at_login: bool,
    /// How much HaryX does on its own when a release comes out.
    pub update_mode: UpdateMode,
    /// A release the user chose to skip.
    pub skipped_version: Option<String>,
    /// When the last update check ran (Unix seconds).
    pub last_update_check: Option<u64>,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs {
            refresh_seconds: 60,
            terminal: None,
            claude_folder: None,
            haryx_path: None,
            auto_switch_on_launch: false,
            auto_switch_dry_run: false,
            notify_switched: true,
            notify_usage: true,
            notify_login: true,
            notify_reset: true,
            notify_auto_switch: true,
            tray_mascot: false,
            launch_at_login: false,
            update_mode: UpdateMode::Notify,
            skipped_version: None,
            last_update_check: None,
        }
    }
}

impl Prefs {
    /// The refresh interval, clamped to the allowed range.
    pub fn refresh_interval(&self) -> std::time::Duration {
        std::time::Duration::from_secs(self.refresh_seconds.clamp(MIN_REFRESH_S, MAX_REFRESH_S))
    }

    pub fn notifies(&self, category: crate::rules::Category) -> bool {
        use crate::rules::Category;
        match category {
            Category::Switched => self.notify_switched,
            Category::Usage => self.notify_usage,
            Category::Login => self.notify_login,
            Category::Reset => self.notify_reset,
        }
    }
}

pub fn load(dir: &Path) -> Prefs {
    std::fs::read_to_string(dir.join(FILE_NAME))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

pub fn save(dir: &Path, prefs: &Prefs) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let text = serde_json::to_string_pretty(prefs).map_err(std::io::Error::other)?;
    let tmp = dir.join(format!("{FILE_NAME}.tmp"));
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, dir.join(FILE_NAME))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_defaults_missing_keys() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(load(tmp.path()), Prefs::default());
        let prefs = Prefs {
            refresh_seconds: 120,
            terminal: Some(Terminal::Pwsh),
            ..Prefs::default()
        };
        save(tmp.path(), &prefs).unwrap();
        assert_eq!(load(tmp.path()), prefs);
        std::fs::write(
            tmp.path().join(FILE_NAME),
            r#"{"refreshSeconds": 45, "bogus": 1}"#,
        )
        .unwrap();
        let loaded = load(tmp.path());
        assert_eq!(loaded.refresh_seconds, 45);
        assert!(loaded.notify_usage);
    }

    #[test]
    fn corrupt_file_gives_defaults() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join(FILE_NAME), "{not json").unwrap();
        assert_eq!(load(tmp.path()), Prefs::default());
    }

    #[test]
    fn refresh_interval_is_clamped() {
        let fast = Prefs {
            refresh_seconds: 1,
            ..Prefs::default()
        };
        assert_eq!(fast.refresh_interval().as_secs(), MIN_REFRESH_S);
        let slow = Prefs {
            refresh_seconds: 99_999,
            ..Prefs::default()
        };
        assert_eq!(slow.refresh_interval().as_secs(), MAX_REFRESH_S);
    }
}

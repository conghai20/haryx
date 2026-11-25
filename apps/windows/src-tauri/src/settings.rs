//! The app's own preferences (not haryx's settings, which the CLI owns): a small JSON file in the app's
//! config folder, written atomically. Unknown or missing keys fall back to defaults, so an old or
//! hand-edited file never stops the app.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::mascot::MascotStyle;
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
    /// A Claude session that ran 1 minute or longer finished.
    pub notify_session: bool,
    pub notify_auto_switch: bool,
    /// Show the active account's mascot in the tray instead of the HaryX mark.
    pub tray_mascot: bool,
    /// Mascot choices by account identity (`email|org`); accounts not listed get a hashed species.
    pub mascot_styles: BTreeMap<String, MascotStyle>,
    /// Start HaryX when you sign in to Windows.
    pub launch_at_login: bool,
    /// Light or dark, or whatever Windows uses.
    pub theme: Theme,
    /// How much HaryX does on its own when a release comes out.
    pub update_mode: UpdateMode,
    /// A release the user chose to skip.
    pub skipped_version: Option<String>,
    /// When the last update check ran (Unix seconds).
    pub last_update_check: Option<u64>,
    /// A weekly usage summary toast (off by default), and when it last went out (Unix seconds).
    pub weekly_summary: bool,
    pub last_weekly_summary: Option<u64>,
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
            notify_session: true,
            notify_auto_switch: true,
            tray_mascot: false,
            mascot_styles: BTreeMap::new(),
            launch_at_login: false,
            theme: Theme::System,
            update_mode: UpdateMode::Notify,
            skipped_version: None,
            last_update_check: None,
            weekly_summary: false,
            last_weekly_summary: None,
        }
    }
}

/// The app's look. An unknown value (a newer or hand-edited file) reads as `System`, so it never
/// discards the rest of the preferences.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    Light,
    Dark,
    #[default]
    #[serde(other)]
    System,
}

impl Theme {
    /// The window theme to force; `None` follows Windows.
    pub fn window_theme(self) -> Option<tauri::Theme> {
        match self {
            Theme::System => None,
            Theme::Light => Some(tauri::Theme::Light),
            Theme::Dark => Some(tauri::Theme::Dark),
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
            Category::Session => self.notify_session,
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
        assert!(
            loaded.notify_session,
            "an older file gains the new toggle, on"
        );
    }

    #[test]
    fn theme_round_trips_and_an_unknown_one_means_system() {
        let tmp = tempfile::tempdir().unwrap();
        let dark = Prefs {
            theme: Theme::Dark,
            ..Prefs::default()
        };
        save(tmp.path(), &dark).unwrap();
        assert_eq!(load(tmp.path()).theme, Theme::Dark);
        std::fs::write(
            tmp.path().join(FILE_NAME),
            r#"{"refreshSeconds": 45, "theme": "sepia"}"#,
        )
        .unwrap();
        let loaded = load(tmp.path());
        assert_eq!(loaded.theme, Theme::System);
        assert_eq!(
            loaded.refresh_seconds, 45,
            "an unknown theme must not discard the other keys"
        );
        assert_eq!(Theme::System.window_theme(), None);
        assert_eq!(Theme::Light.window_theme(), Some(tauri::Theme::Light));
    }

    #[test]
    fn mascot_styles_round_trip_and_default_empty() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join(FILE_NAME), r#"{"trayMascot": true}"#).unwrap();
        assert!(load(tmp.path()).mascot_styles.is_empty());
        let prefs = Prefs {
            mascot_styles: BTreeMap::from([
                (
                    "a@x.com|".to_string(),
                    MascotStyle {
                        species: "cat".into(),
                        hue: Some(0.25),
                    },
                ),
                (
                    "b@x.com|org".to_string(),
                    MascotStyle {
                        species: "robot".into(),
                        hue: None,
                    },
                ),
            ]),
            ..Prefs::default()
        };
        save(tmp.path(), &prefs).unwrap();
        let text = std::fs::read_to_string(tmp.path().join(FILE_NAME)).unwrap();
        assert!(text.contains(r#""mascotStyles""#));
        assert!(
            !text.contains(r#""hue": null"#),
            "an unset hue is left out, as on macOS"
        );
        assert_eq!(load(tmp.path()), prefs);
        // The UI may send an explicit null hue.
        std::fs::write(
            tmp.path().join(FILE_NAME),
            r#"{"mascotStyles": {"a@x.com|": {"species": "frog", "hue": null}}}"#,
        )
        .unwrap();
        assert_eq!(load(tmp.path()).mascot_styles["a@x.com|"].hue, None);
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

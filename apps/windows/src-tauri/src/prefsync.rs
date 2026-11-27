//! Which app preferences sync between machines, and how they map to the flat `haryx sync prefs`
//! object (contract: `packages/contracts/haryx/app-prefs.json`). Shared keys have no prefix and mean
//! the same on macOS; Windows-only keys are `windows.*`. Paths, terminal choices and machine state
//! (last checks, a skipped version) never leave this PC.

use serde_json::{Map, Value, json};

use crate::settings::{Prefs, Theme};
use crate::updater::UpdateMode;

/// The app's synced keys, as `haryx sync prefs` holds them.
pub type Synced = Map<String, Value>;

pub const THEME: &str = "theme";
pub const WEEKLY_SUMMARY: &str = "weeklySummary";
pub const MASCOT_STYLES: &str = "mascotStyles";
pub const UPDATE_MODE: &str = "updateMode";
pub const TRAY_MASCOT: &str = "windows.trayMascot";

type Toggle = fn(&mut Prefs) -> &mut bool;

/// Notification toggles: the synced key and the field it maps to. The key is
/// `notifications.<category>` with macOS's `Notice.Category` raw value, so both apps share it.
const NOTIFICATIONS: &[(&str, Toggle)] = &[
    ("notifications.switched", |p| &mut p.notify_switched),
    ("notifications.usage", |p| &mut p.notify_usage),
    ("notifications.login", |p| &mut p.notify_login),
    ("notifications.reset", |p| &mut p.notify_reset),
    ("notifications.session", |p| &mut p.notify_session),
    ("notifications.autoSwitch", |p| &mut p.notify_auto_switch),
];

/// This PC's synced keys and values.
pub fn to_synced(prefs: &Prefs) -> Synced {
    let mut copy = prefs.clone();
    let mut out = Synced::new();
    out.insert(THEME.into(), json!(prefs.theme));
    out.insert(WEEKLY_SUMMARY.into(), json!(prefs.weekly_summary));
    out.insert(MASCOT_STYLES.into(), json!(prefs.mascot_styles));
    out.insert(UPDATE_MODE.into(), json!(prefs.update_mode));
    out.insert(TRAY_MASCOT.into(), json!(prefs.tray_mascot));
    for (key, field) in NOTIFICATIONS {
        out.insert((*key).into(), json!(*field(&mut copy)));
    }
    out
}

/// The keys whose value differs between `old` and `new` (what a change has to push).
pub fn changed(old: &Prefs, new: &Prefs) -> Synced {
    let before = to_synced(old);
    to_synced(new)
        .into_iter()
        .filter(|(k, v)| before.get(k) != Some(v))
        .collect()
}

/// `remote` (everything `prefs get` returned, other platforms' keys included) with `ours` laid over
/// it, so a push never drops a `macos.*` or unknown key.
pub fn overlay(mut remote: Synced, ours: Synced) -> Synced {
    remote.extend(ours);
    remote
}

/// `prefs` with every synced key from `remote` this app understands applied. Unknown keys, `macos.*`
/// and values of the wrong type are ignored, leaving the local value.
pub fn apply(prefs: &Prefs, remote: &Synced) -> Prefs {
    let mut next = prefs.clone();
    let flag = |key: &str| remote.get(key).and_then(Value::as_bool);
    if let Some(theme) = remote.get(THEME).and_then(Value::as_str).and_then(theme) {
        next.theme = theme;
    }
    if let Some(mode) = remote
        .get(UPDATE_MODE)
        .and_then(|v| serde_json::from_value::<UpdateMode>(v.clone()).ok())
    {
        next.update_mode = mode;
    }
    if let Some(on) = flag(WEEKLY_SUMMARY) {
        next.weekly_summary = on;
    }
    if let Some(on) = flag(TRAY_MASCOT) {
        next.tray_mascot = on;
    }
    if let Some(styles) = remote.get(MASCOT_STYLES).and_then(Value::as_object) {
        // An entry that does not decode is dropped; the rest still apply.
        next.mascot_styles = styles
            .iter()
            .filter_map(|(k, v)| Some((k.clone(), serde_json::from_value(v.clone()).ok()?)))
            .collect();
    }
    for (key, field) in NOTIFICATIONS {
        if let Some(on) = flag(key) {
            *field(&mut next) = on;
        }
    }
    next
}

/// Only the three known themes; `Theme`'s own fallback would turn an unknown one into System.
fn theme(value: &str) -> Option<Theme> {
    match value {
        "system" => Some(Theme::System),
        "light" => Some(Theme::Light),
        "dark" => Some(Theme::Dark),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mascot::MascotStyle;
    use std::collections::BTreeMap;

    const FIXTURE: &str = include_str!("../../../../packages/contracts/haryx/app-prefs.json");

    fn fixture() -> Synced {
        serde_json::from_str(FIXTURE).unwrap()
    }

    #[test]
    fn applies_the_contract_fixture() {
        let local = Prefs {
            theme: Theme::Dark,
            notify_usage: false,
            notify_session: false,
            tray_mascot: true,
            ..Prefs::default()
        };
        let next = apply(&local, &fixture());
        assert_eq!(next.theme, Theme::System);
        assert!(next.notify_usage && next.notify_switched && next.notify_session);
        assert!(next.weekly_summary);
        assert!(!next.tray_mascot, "windows.trayMascot applies");
        assert_eq!(
            next.mascot_styles["you@example.com|"],
            MascotStyle {
                species: "cat".into(),
                hue: Some(0.55)
            }
        );
        // Keys the fixture leaves out keep their local value.
        assert_eq!(next.update_mode, local.update_mode);
        assert_eq!(next.notify_login, local.notify_login);
    }

    #[test]
    fn ignores_unknown_keys_wrong_types_and_macos_keys() {
        let local = Prefs {
            theme: Theme::Dark,
            ..Prefs::default()
        };
        let remote: Synced = serde_json::from_value(json!({
            "theme": "sepia",
            "weeklySummary": "yes",
            "updateMode": "sometimes",
            "mascotStyles": ["not", "a", "map"],
            "macos.menuBarPercent": false,
            "windows.somethingNew": 3,
            "notifications.usage": 0,
        }))
        .unwrap();
        assert_eq!(apply(&local, &remote), local);
        let remote: Synced = serde_json::from_value(json!({
            "mascotStyles": { "a@x.com|": { "species": "cat", "hue": 0.5 }, "b@x.com|": 7 },
            "updateMode": "manual",
        }))
        .unwrap();
        let next = apply(&local, &remote);
        assert_eq!(next.update_mode, UpdateMode::Manual);
        assert_eq!(
            next.mascot_styles,
            BTreeMap::from([(
                "a@x.com|".to_string(),
                MascotStyle {
                    species: "cat".into(),
                    hue: Some(0.5)
                }
            )])
        );
    }

    #[test]
    fn round_trips_and_never_syncs_local_state() {
        let prefs = Prefs {
            theme: Theme::Light,
            notify_reset: false,
            notify_auto_switch: false,
            weekly_summary: true,
            update_mode: UpdateMode::Automatic,
            terminal: Some(crate::terminals::Terminal::Pwsh),
            claude_folder: Some("C:\\work".into()),
            haryx_path: Some("C:\\haryx.exe".into()),
            skipped_version: Some("9.9.9".into()),
            last_update_check: Some(5),
            ..Prefs::default()
        };
        let synced = to_synced(&prefs);
        let back = apply(&Prefs::default(), &synced);
        assert_eq!(to_synced(&back), synced);
        assert_eq!(back.terminal, None);
        let text = serde_json::to_string(&synced).unwrap();
        for local in [
            "C:\\\\",
            "9.9.9",
            "terminal",
            "Folder",
            "haryxPath",
            "lastUpdate",
            "skipped",
            "refresh",
            "launchAtLogin",
            "syncEnabled",
        ] {
            assert!(
                !text.contains(local),
                "{local} must stay on this PC: {text}"
            );
        }
        for key in synced.keys() {
            assert!(!key.starts_with("macos."), "{key}");
        }
    }

    #[test]
    fn changed_lists_only_what_moved_and_overlay_keeps_other_keys() {
        let old = Prefs::default();
        let new = Prefs {
            theme: Theme::Dark,
            terminal: Some(crate::terminals::Terminal::Cmd),
            ..Prefs::default()
        };
        let diff = changed(&old, &new);
        assert_eq!(diff.keys().collect::<Vec<_>>(), vec![THEME]);
        let merged = overlay(fixture(), diff);
        assert_eq!(merged[THEME], json!("dark"));
        assert_eq!(merged["macos.menuBarPercent"], json!(true));
        assert!(changed(&old, &old).is_empty());
    }
}

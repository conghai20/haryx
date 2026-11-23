//! Ported from `tests/test_settings.py`.

use std::path::Path;

use serde_json::json;
use tempfile::TempDir;

use super::*;

fn root() -> TempDir {
    tempfile::tempdir().unwrap()
}

fn write(root: &Path, text: &str) {
    std::fs::write(settings_path(root), text).unwrap();
}

fn write_json(root: &Path, value: Value) {
    write(root, &value.to_string());
}

fn read_json(root: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(settings_path(root)).unwrap()).unwrap()
}

fn auto(f: impl FnOnce(&mut AutoSwitchSettings)) -> AutoSwitchSettings {
    let mut settings = AutoSwitchSettings::default();
    f(&mut settings);
    settings
}

fn config_err(result: Result<SettingValue>) -> String {
    match result {
        Err(Error::Config(message)) => message,
        other => panic!("expected a config error, got {other:?}"),
    }
}

// ------------------------------------------------------------------------------------ load_settings

#[test]
fn missing_corrupt_or_non_object_file_gives_defaults() {
    let dir = root();
    assert_eq!(load_settings(dir.path()), AutoSwitchSettings::default());
    for text in ["{not json", "[1, 2]"] {
        write(dir.path(), text);
        assert_eq!(load_settings(dir.path()), AutoSwitchSettings::default());
    }
}

#[test]
fn partial_section_fills_defaults() {
    let dir = root();
    write_json(
        dir.path(),
        json!({"schemaVersion": 1, "autoswitch": {"threshold": 80}}),
    );
    let loaded = load_settings(dir.path());
    assert_eq!(loaded.threshold, 80.0);
    assert_eq!(loaded.interval_seconds, 60.0);
}

#[test]
fn values_are_clamped() {
    let dir = root();
    write_json(
        dir.path(),
        json!({"autoswitch": {"threshold": 200, "intervalSeconds": 1, "hysteresisPct": -5, "unhealthyTicks": 0}}),
    );
    let loaded = load_settings(dir.path());
    assert_eq!(loaded.threshold, 99.9);
    assert_eq!(loaded.interval_seconds, 15.0);
    assert_eq!(loaded.hysteresis_pct, 0.0);
    assert_eq!(loaded.unhealthy_ticks, 1);
}

#[test]
fn bad_types_fall_back_to_defaults() {
    let dir = root();
    write_json(
        dir.path(),
        json!({"autoswitch": {"threshold": "high", "includeApiKeyAccounts": 1}}),
    );
    let loaded = load_settings(dir.path());
    assert_eq!(loaded.threshold, 90.0);
    assert!(loaded.include_api_key_accounts);
}

#[test]
fn quoted_bools_are_read_by_their_word() {
    let dir = root();
    for (word, expected) in [("false", false), ("TRUE", true), ("maybe", false)] {
        write_json(
            dir.path(),
            json!({"autoswitch": {"includeApiKeyAccounts": word}}),
        );
        assert_eq!(
            load_settings(dir.path()).include_api_key_accounts,
            expected,
            "{word}"
        );
    }
}

#[test]
fn strategies() {
    let dir = root();
    write_json(dir.path(), json!({"autoswitch": {"strategy": "chaos"}}));
    assert_eq!(load_settings(dir.path()).strategy, "best");
    write_json(
        dir.path(),
        json!({"autoswitch": {"strategy": "consume-first"}}),
    );
    assert_eq!(load_settings(dir.path()).strategy, "consume-first");
    let dir = root();
    set_setting(dir.path(), "autoswitch.strategy", "priority-consume-first").unwrap();
    assert_eq!(load_settings(dir.path()).strategy, "priority-consume-first");
}

#[test]
fn garbage_model_value_falls_back_to_none() {
    let dir = root();
    write_json(dir.path(), json!({"autoswitch": {"model": 123}}));
    assert_eq!(load_settings(dir.path()).model, None);
}

// ------------------------------------------------------------------------------------ save_settings

#[test]
fn save_round_trips() {
    let dir = root();
    let custom = auto(|s| {
        s.threshold = 85.0;
        s.cooldown_seconds = 60.0;
        s.model = Some("Fable".into());
    });
    save_settings(dir.path(), &custom).unwrap();
    assert_eq!(load_settings(dir.path()), custom);
    assert_eq!(
        read_json(dir.path())["autoswitch"]["weeklyThreshold"],
        Value::Null
    );
}

#[test]
fn save_keeps_unknown_keys_and_their_order() {
    let dir = root();
    write(
        dir.path(),
        r#"{"schemaVersion": 1, "futureSection": {"x": 1}, "autoswitch": {"threshold": 80, "futureKnob": true}}"#,
    );
    save_settings(dir.path(), &auto(|s| s.threshold = 70.0)).unwrap();
    let raw = read_json(dir.path());
    assert_eq!(raw["futureSection"], json!({"x": 1}));
    assert_eq!(raw["autoswitch"]["futureKnob"], json!(true));
    assert_eq!(raw["autoswitch"]["threshold"], json!(70.0));
    let keys: Vec<&String> = raw.as_object().unwrap().keys().collect();
    assert_eq!(keys, ["schemaVersion", "futureSection", "autoswitch"]);
}

#[cfg(unix)]
#[test]
fn saved_file_mode_is_0600() {
    use std::os::unix::fs::PermissionsExt;
    let dir = root();
    save_settings(dir.path(), &AutoSwitchSettings::default()).unwrap();
    let mode = std::fs::metadata(settings_path(dir.path()))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600);
}

// ------------------------------------------------------------------------------ other sections

#[test]
fn ui_settings() {
    let dir = root();
    assert_eq!(load_ui_settings(dir.path()), UiSettings::default());
    write_json(dir.path(), json!({"ui": {"theme": "light"}}));
    assert_eq!(load_ui_settings(dir.path()).theme, "light");
    write_json(dir.path(), json!({"ui": {"theme": "purple"}}));
    assert_eq!(load_ui_settings(dir.path()).theme, "auto");

    let dir = root();
    assert_eq!(
        set_setting(dir.path(), "ui.theme", "light").unwrap(),
        SettingValue::Str("light".into())
    );
    assert_eq!(
        read_json(dir.path()),
        json!({"schemaVersion": 1, "ui": {"theme": "light"}})
    );
    assert!(unset_setting(dir.path(), "ui.theme").unwrap());
    assert!(read_json(dir.path()).get("ui").is_none());
    assert!(config_err(set_setting(dir.path(), "ui.theme", "purple")).contains("dark, light"));
}

#[test]
fn org_tag_defaults_auto_round_trips_and_ignores_bad_values() {
    let dir = root();
    assert_eq!(load_ui_settings(dir.path()).org_tag, "auto");
    set_setting(dir.path(), "ui.orgTag", "off").unwrap();
    assert_eq!(load_ui_settings(dir.path()).org_tag, "off");
    assert!(
        config_err(set_setting(dir.path(), "ui.orgTag", "sometimes")).contains("auto, on, off")
    );
    write_json(
        dir.path(),
        json!({"ui": {"orgTag": "no", "theme": "light"}}),
    );
    assert_eq!(
        load_ui_settings(dir.path()),
        UiSettings {
            theme: "light".into(),
            org_tag: "auto".into()
        }
    );
    write_json(
        dir.path(),
        json!({"ui": {"orgTag": "on", "theme": "purple"}}),
    );
    assert_eq!(
        load_ui_settings(dir.path()),
        UiSettings {
            theme: "auto".into(),
            org_tag: "on".into()
        }
    );
}

#[test]
fn run_settings_and_explicit_share_history() {
    let dir = root();
    assert!(load_run_settings(dir.path()).share_history);
    assert_eq!(explicit_share_history(dir.path()), None);
    write_json(dir.path(), json!({"run": {"shareHistory": false}}));
    assert!(!load_run_settings(dir.path()).share_history);
    // "no" is truthy as a string and falsy in intent: neither reading is taken, the default is.
    write_json(dir.path(), json!({"run": {"shareHistory": "no"}}));
    assert!(load_run_settings(dir.path()).share_history);
    assert_eq!(explicit_share_history(dir.path()), None);

    for chosen in [true, false] {
        let dir = root();
        set_setting(dir.path(), "run.shareHistory", &chosen.to_string()).unwrap();
        assert_eq!(explicit_share_history(dir.path()), Some(chosen));
        assert_eq!(load_run_settings(dir.path()).share_history, chosen);
        assert_eq!(
            read_json(dir.path()),
            json!({"schemaVersion": 1, "run": {"shareHistory": chosen}})
        );
        assert!(unset_setting(dir.path(), "run.shareHistory").unwrap());
        assert_eq!(explicit_share_history(dir.path()), None);
    }
}

#[test]
fn update_settings() {
    let dir = root();
    assert!(load_update_settings(dir.path()).check);
    write_json(dir.path(), json!({"updates": {"check": false}}));
    assert!(!load_update_settings(dir.path()).check);
    write_json(dir.path(), json!({"updates": {"check": "false"}}));
    assert!(load_update_settings(dir.path()).check);

    let dir = root();
    assert_eq!(
        set_setting(dir.path(), "updates.check", "false").unwrap(),
        SettingValue::Bool(false)
    );
    assert_eq!(
        read_json(dir.path()),
        json!({"schemaVersion": 1, "updates": {"check": false}})
    );
    assert!(unset_setting(dir.path(), "updates.check").unwrap());
    assert!(read_json(dir.path()).get("updates").is_none());
}

#[test]
fn usage_settings() {
    let dir = root();
    assert_eq!(load_usage_settings(dir.path()), UsageSettings::default());
    write_json(dir.path(), json!({"usage": {"headerProbe": "true"}}));
    assert!(!load_usage_settings(dir.path()).header_probe);
    write_json(
        dir.path(),
        json!({"usage": {"headerProbe": true, "fableProbe": "yes"}}),
    );
    assert_eq!(
        load_usage_settings(dir.path()),
        UsageSettings {
            header_probe: true,
            fable_probe: false
        }
    );

    let dir = root();
    set_setting(dir.path(), "usage.headerProbe", "true").unwrap();
    assert_eq!(
        read_json(dir.path()),
        json!({"schemaVersion": 1, "usage": {"headerProbe": true}})
    );
    assert!(!load_usage_settings(dir.path()).fable_probe);
    set_setting(dir.path(), "usage.fableProbe", "true").unwrap();
    assert_eq!(
        load_usage_settings(dir.path()),
        UsageSettings {
            header_probe: true,
            fable_probe: true
        }
    );
    assert!(unset_setting(dir.path(), "usage.fableProbe").unwrap());
    assert!(!load_usage_settings(dir.path()).fable_probe);
}

// ------------------------------------------------------------------------------------------ registry

#[test]
fn registry_defaults_match_the_structs() {
    let keys: Vec<String> = SETTING_SPECS.iter().map(SettingSpec::dotted).collect();
    assert_eq!(keys.len(), 15);
    let defaults: Vec<(String, SettingValue)> = SETTING_SPECS
        .iter()
        .map(|spec| (spec.dotted(), spec.default()))
        .collect();
    let expected = [
        ("autoswitch.threshold", SettingValue::Float(90.0)),
        ("autoswitch.weeklyThreshold", SettingValue::None),
        ("autoswitch.intervalSeconds", SettingValue::Float(60.0)),
        ("autoswitch.cooldownSeconds", SettingValue::Float(300.0)),
        ("autoswitch.hysteresisPct", SettingValue::Float(10.0)),
        ("autoswitch.strategy", SettingValue::Str("best".into())),
        (
            "autoswitch.includeApiKeyAccounts",
            SettingValue::Bool(false),
        ),
        ("autoswitch.unhealthyTicks", SettingValue::Int(3)),
        ("autoswitch.model", SettingValue::None),
        ("ui.theme", SettingValue::Str("auto".into())),
        ("ui.orgTag", SettingValue::Str("auto".into())),
        ("run.shareHistory", SettingValue::Bool(true)),
        ("updates.check", SettingValue::Bool(true)),
        ("usage.headerProbe", SettingValue::Bool(false)),
        ("usage.fableProbe", SettingValue::Bool(false)),
    ];
    let expected: Vec<(String, SettingValue)> = expected
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v))
        .collect();
    assert_eq!(defaults, expected);
}

#[test]
fn every_spec_maps_to_a_field() {
    // A spec whose (section, field) pair matches no struct field would report None as its value.
    let mut loaded = LoadedSettings::default();
    loaded.autoswitch.weekly_threshold = Some(95.0);
    loaded.autoswitch.model = Some("x".into());
    for spec in &SETTING_SPECS {
        assert_ne!(loaded.value(spec), SettingValue::None, "{}", spec.dotted());
    }
}

// --------------------------------------------------------------------------------- set / unset

#[test]
fn set_writes_a_minimal_file() {
    let dir = root();
    assert_eq!(
        set_setting(dir.path(), "autoswitch.threshold", "80").unwrap(),
        SettingValue::Float(80.0)
    );
    let text = std::fs::read_to_string(settings_path(dir.path())).unwrap();
    assert_eq!(
        text,
        "{\n  \"schemaVersion\": 1,\n  \"autoswitch\": {\n    \"threshold\": 80.0\n  }\n}"
    );
}

#[test]
fn set_int_kind_coerces_and_rejects_floats() {
    let dir = root();
    assert_eq!(
        set_setting(dir.path(), "autoswitch.unhealthyTicks", "5").unwrap(),
        SettingValue::Int(5)
    );
    assert!(
        config_err(set_setting(dir.path(), "autoswitch.unhealthyTicks", "3.5")).contains("integer")
    );
    assert!(
        config_err(set_setting(
            dir.path(),
            "autoswitch.unhealthyTicks",
            "99999999999999999999999"
        ))
        .contains("between 1 and 100")
    );
}

#[test]
fn set_rejects_out_of_range_without_writing() {
    let dir = root();
    let message = config_err(set_setting(dir.path(), "autoswitch.threshold", "200"));
    assert_eq!(message, "autoswitch.threshold must be between 50 and 99.9");
    for raw in ["nan", "inf"] {
        assert!(
            config_err(set_setting(dir.path(), "autoswitch.threshold", raw)).contains("between")
        );
    }
    assert!(!settings_path(dir.path()).exists());
}

#[test]
fn set_rejects_unknown_key_listing_the_valid_ones() {
    let dir = root();
    let message = config_err(set_setting(dir.path(), "autoswitch.bogus", "1"));
    assert!(
        message
            .starts_with("unknown setting 'autoswitch.bogus'\nValid keys: autoswitch.threshold, ")
    );
}

#[test]
fn set_string_kind() {
    let dir = root();
    assert_eq!(
        set_setting(dir.path(), "autoswitch.model", " Fable ").unwrap(),
        SettingValue::Str("Fable".into())
    );
    assert_eq!(read_json(dir.path())["autoswitch"]["model"], "Fable");
    assert_eq!(load_settings(dir.path()).model.as_deref(), Some("Fable"));
    let dir = root();
    let message = config_err(set_setting(dir.path(), "autoswitch.model", "   "));
    assert!(
        message.contains("haryx ai config unset autoswitch.model"),
        "{message}"
    );
    assert!(!settings_path(dir.path()).exists());
}

#[test]
fn set_bool_words_strictly() {
    let dir = root();
    assert_eq!(
        set_setting(dir.path(), "autoswitch.includeApiKeyAccounts", "FALSE").unwrap(),
        SettingValue::Bool(false)
    );
    assert!(
        config_err(set_setting(
            dir.path(),
            "autoswitch.includeApiKeyAccounts",
            "falsy"
        ))
        .contains("true or false")
    );
}

#[test]
fn set_on_a_corrupt_file_fails_and_preserves_it() {
    let dir = root();
    write(dir.path(), "{not json");
    assert!(
        config_err(set_setting(dir.path(), "autoswitch.threshold", "80"))
            .contains("not valid JSON")
    );
    assert_eq!(
        std::fs::read_to_string(settings_path(dir.path())).unwrap(),
        "{not json"
    );
    write(dir.path(), "[1]");
    assert!(
        config_err(set_setting(dir.path(), "autoswitch.threshold", "80"))
            .contains("not a JSON object")
    );
}

#[test]
fn set_replaces_a_non_object_section_in_place() {
    let dir = root();
    write(dir.path(), r#"{"autoswitch": "best", "other": 1}"#);
    set_setting(dir.path(), "autoswitch.threshold", "80").unwrap();
    let raw = read_json(dir.path());
    let keys: Vec<&String> = raw.as_object().unwrap().keys().collect();
    assert_eq!(keys, ["autoswitch", "other", "schemaVersion"]);
    assert_eq!(raw["autoswitch"], json!({"threshold": 80.0}));
}

#[test]
fn unset() {
    let dir = root();
    assert!(!unset_setting(dir.path(), "autoswitch.threshold").unwrap());
    assert!(!settings_path(dir.path()).exists());

    set_setting(dir.path(), "autoswitch.threshold", "80").unwrap();
    assert!(unset_setting(dir.path(), "autoswitch.threshold").unwrap());
    assert!(read_json(dir.path()).get("autoswitch").is_none());

    write_json(dir.path(), json!({"autoswitch": {"threshold": 80}}));
    assert!(unset_setting(dir.path(), "autoswitch.threshold").unwrap());
    assert_eq!(read_json(dir.path())["schemaVersion"], 1);
}

#[test]
fn effective_settings_mark_presence_not_value_equality() {
    let dir = root();
    let rows = effective_settings(dir.path());
    assert_eq!(rows.len(), SETTING_SPECS.len());
    assert!(rows.iter().all(|row| !row.is_set));

    set_setting(dir.path(), "autoswitch.threshold", "90").unwrap();
    let rows = effective_settings(dir.path());
    let is_set = |key: &str| rows.iter().find(|r| r.spec.dotted() == key).unwrap().is_set;
    assert!(is_set("autoswitch.threshold"));
    assert!(!is_set("autoswitch.intervalSeconds"));
}

#[test]
fn format_values() {
    assert_eq!(format_setting_value(&SettingValue::None), "(none)");
    assert_eq!(format_setting_value(&SettingValue::Bool(true)), "true");
    assert_eq!(format_setting_value(&SettingValue::Float(90.0)), "90");
    assert_eq!(format_setting_value(&SettingValue::Float(99.9)), "99.9");
    assert_eq!(format_setting_value(&SettingValue::Int(3)), "3");
    assert_eq!(
        format_setting_value(&SettingValue::Str("best".into())),
        "best"
    );
}

#[test]
fn model_names_are_trimmed_and_deduped() {
    assert!(parse_model_names(None).is_empty());
    assert!(parse_model_names(Some("")).is_empty());
    assert_eq!(
        parse_model_names(Some(" Fable, opus,fable,,Opus ")),
        ["Fable", "opus"]
    );
}

mod unrecognized_keys;

// -------------------------------------------------------------------------------- merged_with_cli

#[test]
fn merged_with_cli_overlays_and_clamps() {
    let base = auto(|s| {
        s.threshold = 80.0;
        s.cooldown_seconds = 10.0;
    });
    assert_eq!(
        merged_with_cli(&base, &AutoSwitchOverrides::default()),
        base
    );

    let merged = merged_with_cli(
        &base,
        &AutoSwitchOverrides {
            threshold: Some(60.0),
            interval: Some(30.0),
            ..Default::default()
        },
    );
    assert_eq!(merged.threshold, 60.0);
    assert_eq!(merged.interval_seconds, 30.0);
    assert_eq!(merged.cooldown_seconds, 10.0);

    let merged = merged_with_cli(
        &AutoSwitchSettings::default(),
        &AutoSwitchOverrides {
            interval: Some(1.0),
            weekly_threshold: Some(970.0),
            include_api_key_accounts: Some(true),
            model: Some("Fable".into()),
            strategy: Some("consume-first".into()),
            ..Default::default()
        },
    );
    assert_eq!(merged.interval_seconds, 15.0);
    assert_eq!(merged.weekly_threshold, Some(99.9));
    assert!(merged.include_api_key_accounts);
    assert_eq!(merged.model.as_deref(), Some("Fable"));
    assert_eq!(merged.strategy, "consume-first");
}

#[test]
fn threshold_alone_leaves_a_configured_weekly_line_in_force() {
    let merged = merged_with_cli(
        &auto(|s| {
            s.threshold = 90.0;
            s.weekly_threshold = Some(97.0);
        }),
        &AutoSwitchOverrides {
            threshold: Some(85.0),
            ..Default::default()
        },
    );
    assert_eq!(
        (merged.threshold, merged.weekly_threshold),
        (85.0, Some(97.0))
    );
}

// ------------------------------------------------------------------------------- weekly threshold

#[test]
fn weekly_threshold_load_clamps_numbers_and_drops_garbage() {
    let dir = root();
    assert_eq!(load_settings(dir.path()).weekly_threshold, None);
    for (raw, expected) in [
        ("97", Some(97.0)),
        ("970", Some(99.9)),
        ("10", Some(50.0)),
        ("\"banana\"", None),
        ("null", None),
    ] {
        write(
            dir.path(),
            &format!(r#"{{"autoswitch": {{"weeklyThreshold": {raw}}}}}"#),
        );
        assert_eq!(
            load_settings(dir.path()).weekly_threshold,
            expected,
            "{raw}"
        );
    }
}

#[test]
fn non_finite_or_huge_numbers_fall_back_to_defaults() {
    // serde_json rejects NaN/Infinity literals and integers beyond f64 range, so the whole file reads as
    // corrupt: both lines fall back to their defaults (upstream clamps a huge integer to 99.9 instead).
    let dir = root();
    let huge = format!("1{}", "0".repeat(400));
    for literal in ["NaN", "Infinity", "-Infinity", huge.as_str()] {
        write(
            dir.path(),
            &format!(
                r#"{{"autoswitch": {{"threshold": {literal}, "weeklyThreshold": {literal}}}}}"#
            ),
        );
        let loaded = load_settings(dir.path());
        assert_eq!((loaded.threshold, loaded.weekly_threshold), (90.0, None));
    }
}

#[test]
fn weekly_threshold_set_validates_loudly_and_unset_clears() {
    let dir = root();
    assert!(
        config_err(set_setting(dir.path(), "autoswitch.weeklyThreshold", "970"))
            .contains("between 50 and 99.9")
    );
    let message = config_err(set_setting(
        dir.path(),
        "autoswitch.weeklyThreshold",
        "banana",
    ));
    assert_eq!(
        message,
        "autoswitch.weeklyThreshold expects a number, got 'banana'; \
         use 'haryx ai config unset autoswitch.weeklyThreshold' to clear it"
    );
    assert!(!settings_path(dir.path()).exists());
    // A required number gets no unset hint.
    let message = config_err(set_setting(dir.path(), "autoswitch.threshold", "high"));
    assert_eq!(message, "autoswitch.threshold expects a number, got 'high'");

    set_setting(dir.path(), "autoswitch.threshold", "90").unwrap();
    assert_eq!(
        set_setting(dir.path(), "autoswitch.weeklyThreshold", "80").unwrap(),
        SettingValue::Float(80.0)
    );
    let loaded = load_settings(dir.path());
    assert_eq!(
        (loaded.threshold, loaded.weekly_threshold),
        (90.0, Some(80.0))
    );
    assert!(unset_setting(dir.path(), "autoswitch.weeklyThreshold").unwrap());
    assert_eq!(load_settings(dir.path()).weekly_threshold, None);
}

// ------------------------------------------------------------------------------ atomic_write_json

#[cfg(unix)]
mod symlinks {
    use std::os::unix::fs::{PermissionsExt, symlink};

    use super::*;

    #[test]
    fn write_preserves_the_link_and_updates_the_target() {
        let dir = root();
        let (repo, live) = (dir.path().join("repo"), dir.path().join("live"));
        std::fs::create_dir(&repo).unwrap();
        std::fs::create_dir(&live).unwrap();
        let tracked = repo.join("settings.json");
        std::fs::write(&tracked, r#"{"tracked": true}"#).unwrap();
        let link = live.join("settings.json");
        symlink(&tracked, &link).unwrap();

        atomic_write_json(&link, &json!({"written": "through"})).unwrap();

        assert!(link.symlink_metadata().unwrap().file_type().is_symlink());
        let written: Value =
            serde_json::from_str(&std::fs::read_to_string(&tracked).unwrap()).unwrap();
        assert_eq!(written, json!({"written": "through"}));
        // No temp file was left beside the link or the target.
        for d in [&repo, &live] {
            assert_eq!(std::fs::read_dir(d).unwrap().count(), 1);
        }
    }

    #[test]
    fn dangling_link_writes_where_it_points() {
        let dir = root();
        // `fsutil::publish_target` resolves the link's parent strictly, so the target's directory exists
        // here (upstream also creates a missing one); only the file itself is missing.
        std::fs::create_dir(dir.path().join("gone")).unwrap();
        let target = dir.path().join("gone").join("settings.json");
        let link = dir.path().join("settings.json");
        symlink(&target, &link).unwrap();
        atomic_write_json(&link, &json!({"dangling": "ok"})).unwrap();
        assert!(link.symlink_metadata().unwrap().file_type().is_symlink());
        assert!(
            std::fs::read_to_string(&target)
                .unwrap()
                .contains("dangling")
        );
    }

    #[test]
    fn hardening_stays_on_the_directory_haryx_owns() {
        let dir = root();
        let (repo, live) = (dir.path().join("repo"), dir.path().join("live"));
        std::fs::create_dir(&repo).unwrap();
        std::fs::set_permissions(&repo, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::create_dir(&live).unwrap();
        let tracked = repo.join("settings.json");
        std::fs::write(&tracked, "{}").unwrap();
        let link = live.join("settings.json");
        symlink(&tracked, &link).unwrap();

        atomic_write_json(&link, &json!({"x": 1})).unwrap();

        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&repo), 0o755, "foreign dir untouched");
        assert_eq!(mode(&live), 0o700, "our dir hardened");
        assert_eq!(mode(&tracked), 0o600, "file still 0600");
    }
}

#[test]
fn plain_file_write() {
    let dir = root();
    let path = dir.path().join("nested").join("settings.json");
    atomic_write_json(&path, &json!({"plain": 1})).unwrap();
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "{\n  \"plain\": 1\n}"
    );
}

//! Ported from `TestUnrecognizedKeys` in `tests/test_settings.py`.

use super::*;

#[test]
fn typoed_keys_are_named_with_the_key_they_resemble() {
    let dir = root();
    write(
        dir.path(),
        r#"{"schemaVersion": 1, "autoswitch": {"threshold": 98.0, "hysteresisPCT": 5, "cooldownSecond": 600}}"#,
    );
    assert_eq!(
        unrecognized_keys(dir.path()),
        [
            UnrecognizedKey::new("autoswitch.hysteresisPCT", Some("autoswitch.hysteresisPct")),
            UnrecognizedKey::new(
                "autoswitch.cooldownSecond",
                Some("autoswitch.cooldownSeconds")
            ),
        ]
    );
    let loaded = load_settings(dir.path());
    assert_eq!(loaded.threshold, 98.0);
    assert_eq!(loaded.hysteresis_pct, 10.0);
    assert_eq!(loaded.cooldown_seconds, 300.0);
}

#[test]
fn mistyped_section_and_unrelated_keys() {
    let dir = root();
    write_json(dir.path(), json!({"autoSwitch": {"threshold": 70}}));
    assert_eq!(
        unrecognized_keys(dir.path()),
        [UnrecognizedKey::new("autoSwitch", Some("autoswitch"))]
    );
    assert_eq!(load_settings(dir.path()), AutoSwitchSettings::default());

    write(
        dir.path(),
        r#"{"futureSection": {"x": 1}, "autoswitch": {"graceSeconds": 30}, "ui": {"font": "mono"}}"#,
    );
    assert_eq!(
        unrecognized_keys(dir.path()),
        [
            UnrecognizedKey::new("futureSection", None),
            UnrecognizedKey::new("autoswitch.graceSeconds", None),
            UnrecognizedKey::new("ui.font", None),
        ]
    );
}

#[test]
fn suggestion_only_for_a_near_miss() {
    let cases = [
        ("Strategy", Some("strategy")),
        ("treshold", Some("threshold")),
        ("models", Some("model")),
        ("weekly_threshold", Some("weeklyThreshold")),
        ("interval", Some("intervalSeconds")),
        ("cooldown", Some("cooldownSeconds")),
        ("hysteresis", Some("hysteresisPct")),
        ("graceSeconds", None),
        ("holdSeconds", None),
        ("maxThreshold", None),
        ("order", None),
        ("th", None),
    ];
    for (written, meant) in cases {
        let dir = root();
        write_json(dir.path(), json!({"autoswitch": {written: 1}}));
        let found = unrecognized_keys(dir.path());
        assert_eq!(found.len(), 1, "{written}");
        assert_eq!(found[0].dotted, format!("autoswitch.{written}"));
        assert_eq!(
            found[0].suggestion,
            meant.map(|m| format!("autoswitch.{m}")),
            "{written}"
        );
    }
}

#[test]
fn setting_written_flat_at_the_top_level_is_told_where_it_belongs() {
    let dir = root();
    write_json(dir.path(), json!({"autoswitch.threshold": 80}));
    let found = unrecognized_keys(dir.path());
    assert_eq!(
        found,
        [UnrecognizedKey::new(
            "autoswitch.threshold",
            Some("autoswitch.threshold")
        )]
    );
    assert_eq!(
        found[0].describe(),
        "'autoswitch.threshold' is not a key this version reads, so it has no effect; \
         it belongs inside 'autoswitch' as 'threshold'"
    );
    assert_eq!(load_settings(dir.path()), AutoSwitchSettings::default());
}

#[test]
fn key_is_quoted_so_stray_whitespace_shows() {
    let dir = root();
    write_json(
        dir.path(),
        json!({"autoswitch": {"threshold ": 80, "a\nb": 1}}),
    );
    let described: Vec<String> = unrecognized_keys(dir.path())
        .iter()
        .map(UnrecognizedKey::describe)
        .collect();
    assert!(described[0].starts_with("'autoswitch.threshold ' is not a key"));
    assert!(described[0].ends_with("did you mean autoswitch.threshold?"));
    assert!(!described[1].contains('\n'));
    let plain = UnrecognizedKey::new("autoswitch.futureKnob", None).describe();
    assert_eq!(
        plain,
        "'autoswitch.futureKnob' is not a key this version reads, so it has no effect"
    );
}

#[test]
fn every_registered_key_is_recognized() {
    let dir = root();
    let mut raw = Map::new();
    raw.insert("schemaVersion".into(), json!(1));
    for spec in &SETTING_SPECS {
        let section = raw
            .entry(spec.section)
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .unwrap();
        section.insert(spec.json_key.into(), spec.default().to_json());
    }
    write_json(dir.path(), Value::Object(raw));
    assert!(unrecognized_keys(dir.path()).is_empty());
}

#[test]
fn nothing_unrecognized_in_missing_corrupt_empty_or_scalar_sections() {
    let dir = root();
    assert!(unrecognized_keys(dir.path()).is_empty());
    for text in ["{not json", "[1, 2]", "{}", r#"{"autoswitch": "best"}"#] {
        write(dir.path(), text);
        assert!(unrecognized_keys(dir.path()).is_empty(), "{text}");
    }
}

//! `haryx config` end to end, ported from upstream `tests/test_config_cli.py`. Every run gets its own
//! store through `HARYX_HOME` (and a temp `HOME`), so the real store is never touched.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::{Value, json};
use tempfile::TempDir;

struct Sandbox {
    dir: TempDir,
}

struct Run {
    code: i32,
    out: String,
    err: String,
}

impl Sandbox {
    fn new() -> Self {
        Sandbox {
            dir: tempfile::tempdir().unwrap(),
        }
    }

    fn store(&self) -> PathBuf {
        self.dir.path().join("store")
    }

    fn settings(&self) -> PathBuf {
        self.store().join("settings.json")
    }

    fn config(&self, argv: &[&str]) -> Run {
        let output = Command::new(env!("CARGO_BIN_EXE_haryx"))
            .arg("config")
            .args(argv)
            .env("HOME", self.dir.path())
            .env("HARYX_HOME", self.store())
            .env_remove("XDG_DATA_HOME")
            .stdin(Stdio::null())
            .output()
            .unwrap();
        Run {
            code: output.status.code().unwrap_or(-1),
            out: String::from_utf8_lossy(&output.stdout).into_owned(),
            err: String::from_utf8_lossy(&output.stderr).into_owned(),
        }
    }

    fn ok(&self, argv: &[&str]) -> Run {
        let run = self.config(argv);
        assert_eq!(run.code, 0, "{argv:?}: {}", run.err);
        run
    }

    fn write_settings(&self, value: &Value) {
        std::fs::create_dir_all(self.store()).unwrap();
        std::fs::write(self.settings(), value.to_string()).unwrap();
    }

    fn read_settings(&self) -> Value {
        read_json(&self.settings())
    }
}

fn read_json(path: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn line_with<'a>(out: &'a str, needle: &str) -> &'a str {
    out.lines().find(|l| l.contains(needle)).unwrap()
}

const KEYS: [&str; 15] = [
    "autoswitch.threshold",
    "autoswitch.weeklyThreshold",
    "autoswitch.intervalSeconds",
    "autoswitch.cooldownSeconds",
    "autoswitch.hysteresisPct",
    "autoswitch.strategy",
    "autoswitch.includeApiKeyAccounts",
    "autoswitch.unhealthyTicks",
    "autoswitch.model",
    "ui.theme",
    "ui.orgTag",
    "run.shareHistory",
    "updates.check",
    "usage.headerProbe",
    "usage.fableProbe",
];

// ------------------------------------------------------------------------------------------------ list

#[test]
fn lists_all_keys_as_defaults() {
    let sb = Sandbox::new();
    let run = sb.ok(&[]);
    for key in KEYS {
        assert!(run.out.contains(key), "{key}");
    }
    assert_eq!(run.out.matches("(default)").count(), 18);
    assert_eq!(sb.ok(&["list"]).out, run.out);
    assert!(!sb.settings().exists(), "listing writes nothing");
}

#[test]
fn set_keys_are_not_marked_default_even_when_equal_to_it() {
    let sb = Sandbox::new();
    sb.ok(&["set", "autoswitch.cooldownSeconds", "600"]);
    sb.ok(&["set", "autoswitch.threshold", "90"]);
    let out = sb.ok(&[]).out;
    let cooldown = line_with(&out, "cooldownSeconds");
    assert!(cooldown.contains("600") && !cooldown.contains("(default)"));
    assert!(!line_with(&out, "threshold").contains("(default)"));
}

#[test]
fn json_list() {
    let sb = Sandbox::new();
    let payload: Value = serde_json::from_str(&sb.ok(&["--json"]).out).unwrap();
    assert_eq!(payload["schemaVersion"], 1);
    assert!(payload["path"].as_str().unwrap().ends_with("settings.json"));
    let settings = payload["settings"].as_array().unwrap();
    assert_eq!(settings.len(), 18);
    let by_key = |key: &str| settings.iter().find(|e| e["key"] == key).unwrap().clone();
    assert_eq!(
        by_key("autoswitch.threshold"),
        json!({"key": "autoswitch.threshold", "value": 90.0, "isSet": false})
    );
    assert_eq!(by_key("autoswitch.weeklyThreshold")["value"], Value::Null);
    assert_eq!(by_key("autoswitch.includeApiKeyAccounts")["value"], false);
    assert_eq!(by_key("updates.check")["value"], true);
    assert_eq!(payload["unrecognized"], json!([]));
    // Same text as Python's json.dumps(indent=2).
    assert!(
        sb.ok(&["--json"])
            .out
            .contains("\n      \"value\": 90.0,\n")
    );
}

#[test]
fn list_warns_about_keys_nothing_reads() {
    let sb = Sandbox::new();
    sb.write_settings(&json!({
        "schemaVersion": 1,
        "autoswitch": {"threshold": 98.0, "hysteresisPCT": 5, "futureKnob": true},
    }));
    let run = sb.ok(&[]);
    assert!(line_with(&run.out, "hysteresisPct").contains("(default)"));
    assert!(!run.out.contains("hysteresisPCT"));
    assert!(run.err.contains(
        "Warning: settings.json: 'autoswitch.hysteresisPCT' is not a key this version reads, \
         so it has no effect; did you mean autoswitch.hysteresisPct?"
    ));
    assert!(run.err.contains(
        "settings.json: 'autoswitch.futureKnob' is not a key this version reads, so it has no effect\n"
    ));

    let sb = Sandbox::new();
    sb.ok(&["set", "autoswitch.threshold", "80"]);
    assert!(!sb.ok(&[]).err.contains("settings.json"));
}

#[test]
fn json_list_names_keys_nothing_reads() {
    let sb = Sandbox::new();
    sb.write_settings(&json!({
        "autoSwitch": {"threshold": 70},
        "autoswitch": {"cooldownSecond": 600, "futureKnob": true},
    }));
    let run = sb.ok(&["--json"]);
    assert!(!run.err.contains("settings.json"));
    let payload: Value = serde_json::from_str(&run.out).unwrap();
    assert_eq!(
        payload["unrecognized"],
        json!([
            {"key": "autoSwitch", "suggestion": "autoswitch"},
            {"key": "autoswitch.cooldownSecond", "suggestion": "autoswitch.cooldownSeconds"},
            {"key": "autoswitch.futureKnob", "suggestion": null},
        ])
    );
}

// --------------------------------------------------------------------------------------- set / get

#[test]
fn set_then_get() {
    let sb = Sandbox::new();
    assert_eq!(
        sb.ok(&["set", "autoswitch.threshold", "80"]).out,
        "autoswitch.threshold = 80\n"
    );
    assert_eq!(sb.ok(&["get", "autoswitch.threshold"]).out, "80\n");
    let raw = sb.read_settings();
    assert_eq!(
        raw,
        json!({"schemaVersion": 1, "autoswitch": {"threshold": 80.0}})
    );
}

#[test]
fn set_bool_words() {
    let sb = Sandbox::new();
    let run = sb.ok(&["set", "autoswitch.includeApiKeyAccounts", "no"]);
    assert!(run.out.contains("= false"));
    assert_eq!(
        sb.read_settings()["autoswitch"]["includeApiKeyAccounts"],
        false
    );
}

#[test]
fn fable_probe_alone_says_it_needs_the_header_probe() {
    let sb = Sandbox::new();
    let run = sb.ok(&["set", "usage.fableProbe", "true"]);
    assert!(run.out.contains("usage.fableProbe = true"));
    assert!(
        run.err
            .contains("does nothing until usage.headerProbe is true")
    );
    assert_eq!(sb.ok(&["set", "usage.headerProbe", "true"]).err, "");
}

#[test]
fn set_preserves_unknown_keys() {
    let sb = Sandbox::new();
    sb.write_settings(&json!({
        "schemaVersion": 1,
        "futureSection": {"x": 1},
        "autoswitch": {"threshold": 80, "futureKnob": true},
    }));
    sb.ok(&["set", "autoswitch.threshold", "70"]);
    let raw = sb.read_settings();
    assert_eq!(raw["futureSection"], json!({"x": 1}));
    assert_eq!(raw["autoswitch"]["futureKnob"], true);
    assert_eq!(raw["autoswitch"]["threshold"], 70.0);
}

#[test]
fn get_json_with_trailing_and_leading_flag() {
    let sb = Sandbox::new();
    sb.ok(&["set", "autoswitch.threshold", "80"]);
    for argv in [
        &["get", "autoswitch.threshold", "--json"][..],
        &["--json", "get", "autoswitch.threshold"],
    ] {
        let payload: Value = serde_json::from_str(&sb.ok(argv).out).unwrap();
        assert_eq!(
            payload,
            json!({"schemaVersion": 1, "key": "autoswitch.threshold", "value": 80.0, "isSet": true})
        );
    }
}

// ------------------------------------------------------------------------------------- validation

#[test]
fn invalid_values_exit_1_with_the_reason() {
    let sb = Sandbox::new();
    for (argv, needle) in [
        (
            &["set", "autoswitch.threshold", "30"][..],
            "between 50 and 99.9",
        ),
        (&["set", "autoswitch.bogus", "1"], "unknown setting"),
        (
            &["set", "autoswitch.includeApiKeyAccounts", "falsy"],
            "true or false",
        ),
        (&["set", "autoswitch.threshold", "high"], "expects a number"),
        (
            &["set", "autoswitch.unhealthyTicks", "3.5"],
            "expects an integer",
        ),
        (
            &["set", "autoswitch.strategy", "chaos"],
            "must be one of: best",
        ),
        (&["get", "autoswitch.bogus"], "autoswitch.threshold"),
    ] {
        let run = sb.config(argv);
        assert_eq!(run.code, 1, "{argv:?}");
        assert!(run.err.starts_with("Error: "), "{argv:?}: {}", run.err);
        assert!(run.err.contains(needle), "{argv:?}: {}", run.err);
        assert!(run.out.is_empty());
    }
    assert!(!sb.settings().exists());
}

#[test]
fn unknown_key_json_error_envelope() {
    let sb = Sandbox::new();
    let run = sb.config(&["--json", "get", "autoswitch.bogus"]);
    assert_eq!(run.code, 1);
    assert!(run.err.is_empty(), "{}", run.err);
    let payload: Value = serde_json::from_str(&run.out).unwrap();
    assert_eq!(payload["schemaVersion"], 1);
    assert_eq!(payload["error"]["type"], "ConfigError");
    assert!(
        payload["error"]["message"]
            .as_str()
            .unwrap()
            .contains("unknown setting")
    );
}

#[test]
fn corrupt_file_set_exits_1_and_leaves_the_file_untouched() {
    let sb = Sandbox::new();
    std::fs::create_dir_all(sb.store()).unwrap();
    std::fs::write(sb.settings(), "{not json").unwrap();
    let run = sb.config(&["set", "autoswitch.threshold", "80"]);
    assert_eq!(run.code, 1);
    assert!(run.err.contains("not valid JSON"));
    assert_eq!(std::fs::read_to_string(sb.settings()).unwrap(), "{not json");
    // Reading still works: a corrupt file lists the defaults.
    assert_eq!(sb.ok(&[]).out.matches("(default)").count(), 18);
}

#[test]
fn usage_errors_exit_2() {
    let sb = Sandbox::new();
    for argv in [
        &["set", "autoswitch.threshold"][..],
        &["frobnicate"],
        &["--json", "set", "autoswitch.threshold", "80"],
    ] {
        assert_eq!(sb.config(argv).code, 2, "{argv:?}");
    }
    assert!(!sb.settings().exists());
}

// ------------------------------------------------------------------------------------- unset / path

#[test]
fn unset_restores_the_default() {
    let sb = Sandbox::new();
    sb.ok(&["set", "autoswitch.threshold", "80"]);
    assert_eq!(
        sb.ok(&["unset", "autoswitch.threshold"]).out,
        "autoswitch.threshold unset (default: 90)\n"
    );
    assert_eq!(sb.ok(&["get", "autoswitch.threshold"]).out, "90\n");
    assert!(sb.read_settings().get("autoswitch").is_none());

    let run = sb.ok(&["unset", "autoswitch.threshold"]);
    assert_eq!(run.err, "autoswitch.threshold is not set; nothing to do\n");
}

#[test]
fn path_prints_the_settings_location() {
    let sb = Sandbox::new();
    assert_eq!(sb.ok(&["path"]).out.trim(), sb.settings().to_string_lossy());
    assert!(!sb.store().exists(), "path creates nothing");
}

#[test]
fn help_lists_keys_from_the_registry() {
    let sb = Sandbox::new();
    let out = sb.ok(&["--help"]).out;
    for key in KEYS {
        assert!(out.contains(key), "{key}");
    }
    assert!(out.contains("unset"));
    assert!(out.contains("Poll interval for the haryx auto loop"));
}

#[test]
fn weekly_threshold_round_trip() {
    let sb = Sandbox::new();
    let run = sb.config(&["set", "autoswitch.weeklyThreshold", "970"]);
    assert_eq!(run.code, 1);
    assert!(run.err.contains("between 50 and 99.9"));
    let run = sb.config(&["set", "autoswitch.weeklyThreshold", "banana"]);
    assert_eq!(run.code, 1);
    assert!(run.err.contains("expects a number"));
    assert!(
        run.err
            .contains("haryx ai config unset autoswitch.weeklyThreshold")
    );
    assert!(!sb.settings().exists());

    sb.ok(&["set", "autoswitch.weeklyThreshold", "97"]);
    assert!(
        sb.ok(&["get", "autoswitch.weeklyThreshold"])
            .out
            .contains("97")
    );
    let get_json = |sb: &Sandbox| -> Value {
        serde_json::from_str(&sb.ok(&["get", "--json", "autoswitch.weeklyThreshold"]).out).unwrap()
    };
    assert_eq!(get_json(&sb)["value"], 97.0);
    assert!(
        sb.ok(&["unset", "autoswitch.weeklyThreshold"])
            .out
            .contains("(none)")
    );
    assert_eq!(get_json(&sb)["value"], Value::Null);
}

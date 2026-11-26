//! Tool settings persisted at `<store root>/settings.json`. Port of `claude_swap/settings.py`.
//!
//! One versioned JSON file for user-tunable preferences, written atomically with the store's 0600/0700
//! modes. v1 carries the `autoswitch`, `ui`, `run`, `updates` and `usage` sections; other sections can be
//! added additively. Unknown keys (future fields, other tools' experiments) survive a round trip.
//!
//! Reading is forgiving: a missing or corrupt file yields defaults with a logged warning, never a crash,
//! so a bad hand edit degrades to default behaviour. A key nothing reads (a typo, or a key from another
//! version) is kept in the file but changes nothing; [`unrecognized_keys`] finds those: `haryx config`
//! lists them and the auto-switch loop logs them as it starts.

mod specs;
mod unrecognized;

use std::io;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use crate::AI_PROG;
use crate::error::{Error, Result};
use crate::fsutil::{WriteOptions, atomic_write_text, publish_target};
use crate::models::pyfmt::{json_dumps_pretty, repr_json};

pub use specs::{
    LOCAL_ONLY_SETTINGS, SETTING_SPECS, SettingKind, SettingSpec, SettingValue, setting_spec,
};
pub use unrecognized::UnrecognizedKey;

pub const SETTINGS_SCHEMA_VERSION: i64 = 1;
pub const SETTINGS_FILENAME: &str = "settings.json";
/// Values of `autoswitch.strategy`; `best` is the default, the others are opt-in.
pub const AUTOSWITCH_STRATEGIES: &[&str] = &[
    "best",
    "weekly-first",
    "consume-first",
    "priority-consume-first",
];

/// The top-level sections, in registry order.
pub(crate) const SECTIONS: [&str; 6] = ["autoswitch", "ui", "run", "updates", "usage", "history"];

/// Policy knobs for the auto-switch engine (`haryx auto`).
///
/// `threshold` is the switch line for the 5-hour window and, while `weekly_threshold` is unset, for every
/// window: each of an account's usage windows (5h, 7d, and any per-model window named by `model`) is
/// judged against its own line, and the engine looks for a better account once any of them is at or over
/// it. 90 rather than 95 leaves margin for the macOS ~30s Keychain pickup tail and for heavy subagent
/// turns burning past the mark before a swap lands. `weekly_threshold` gives the 7-day window and the
/// per-model windows (they all reset weekly) their own line: the weekly window is the budget, so it is
/// worth running closer to its ceiling than the 5-hour window, which recycles in hours. A proactive
/// candidate must itself sit below each of its lines and beat the active account by at least
/// `hysteresis_pct` points of headroom measured against those lines, so two accounts hovering at the line
/// never ping-pong while a strictly better account is always taken. Under `consume-first`,
/// `hysteresis_pct` is instead the room a landing needs under each of the target's own lines.
#[derive(Clone, Debug, PartialEq)]
pub struct AutoSwitchSettings {
    pub threshold: f64,
    /// Switch line for the 7-day window and every per-model scoped window; `None`: `threshold` governs.
    pub weekly_threshold: Option<f64>,
    pub interval_seconds: f64,
    pub cooldown_seconds: f64,
    pub hysteresis_pct: f64,
    /// One of [`AUTOSWITCH_STRATEGIES`].
    pub strategy: String,
    pub include_api_key_accounts: bool,
    pub unhealthy_ticks: i64,
    /// Comma-separated model display name(s) (`Fable`, `Fable,Opus`) or `all`: each named model's
    /// per-model weekly limit is folded into the binding window. `None`: account-wide 5h/7d only.
    pub model: Option<String>,
}

impl Default for AutoSwitchSettings {
    fn default() -> Self {
        AutoSwitchSettings {
            threshold: 90.0,
            weekly_threshold: None,
            interval_seconds: 60.0,
            cooldown_seconds: 300.0,
            hysteresis_pct: 10.0,
            strategy: "best".to_owned(),
            include_api_key_accounts: false,
            unhealthy_ticks: 3,
            model: None,
        }
    }
}

/// Appearance preferences (`ui`). `theme`: `dark`, `light` or `auto` (follows terminal-background
/// detection). `org_tag`: when listings draw the `[org]` tag: `on`, `off`, or `auto` (only on accounts
/// sharing an email with another managed account).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UiSettings {
    pub theme: String,
    pub org_tag: String,
}

impl Default for UiSettings {
    fn default() -> Self {
        UiSettings {
            theme: "auto".to_owned(),
            org_tag: "auto".to_owned(),
        }
    }
}

/// Defaults for `haryx run` (`run`); its flags override them. `share_history`: sessions of every account
/// read and write the one chat history in `~/.claude`, so the account only decides whose quota is used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RunSettings {
    pub share_history: bool,
}

impl Default for RunSettings {
    fn default() -> Self {
        RunSettings {
            share_history: true,
        }
    }
}

/// Update notification (`updates`). `check` turns the daily "newer version available" lookup on or off;
/// `haryx upgrade` works either way.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UpdateSettings {
    pub check: bool,
}

impl Default for UpdateSettings {
    fn default() -> Self {
        UpdateSettings { check: true }
    }
}

/// Usage collection preferences (`usage`).
///
/// `header_probe`: the usage endpoint never answers a setup-token account, so read its 5h/7d utilization
/// from the rate-limit headers of a 1-output-token Haiku request instead. Each probe is a real, if tiny,
/// request against that account's quota, so it is opt-in. `fable_probe`: those headers report Fable's
/// weekly limit only on a Fable response, so also send a 1-token Fable request per poll. A second opt-in
/// because it spends Fable quota; it acts only with `header_probe` on. `share_between_machines`:
/// read and write the shared per-account usage rows (`crate::usage_share`), so this user's machines
/// spend one usage budget per account; it acts only while signed in with sync unlocked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UsageSettings {
    pub header_probe: bool,
    pub fable_probe: bool,
    pub share_between_machines: bool,
}

impl Default for UsageSettings {
    fn default() -> Self {
        UsageSettings {
            header_probe: false,
            fable_probe: false,
            share_between_machines: true,
        }
    }
}

/// Usage history preferences (`history`, `docs/usage-history-plan.md`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistorySettings {
    pub enabled: bool,
    /// `ask` (until the user chooses), `all` or `none`.
    pub import: String,
    /// Days of day files kept; 0 keeps everything.
    pub keep_days: i64,
}

impl Default for HistorySettings {
    fn default() -> Self {
        HistorySettings {
            enabled: true,
            import: "ask".into(),
            keep_days: 90,
        }
    }
}

/// Load the history section; a missing file or a bad value gives the default for that key.
pub fn load_history_settings(root: &Path) -> HistorySettings {
    let default = HistorySettings::default();
    let Some(section) = read_section(root, "history") else {
        return default;
    };
    let enabled = match section.get("enabled") {
        Some(Value::Bool(v)) => *v,
        _ => default.enabled,
    };
    let import = match section.get("import") {
        Some(Value::String(v)) if ["ask", "all", "none"].contains(&v.as_str()) => v.clone(),
        _ => default.import.clone(),
    };
    let keep_days = match section.get("keepDays").and_then(Value::as_i64) {
        Some(v) if (0..=36_500).contains(&v) => v,
        _ => default.keep_days,
    };
    HistorySettings {
        enabled,
        import,
        keep_days,
    }
}

/// Every section's effective settings, as the loaders return them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LoadedSettings {
    pub autoswitch: AutoSwitchSettings,
    pub ui: UiSettings,
    pub run: RunSettings,
    pub updates: UpdateSettings,
    pub usage: UsageSettings,
    pub history: HistorySettings,
}

impl LoadedSettings {
    /// Load every section from `<root>/settings.json`.
    pub fn load(root: &Path) -> Self {
        LoadedSettings {
            autoswitch: load_settings(root),
            ui: load_ui_settings(root),
            run: load_run_settings(root),
            updates: load_update_settings(root),
            usage: load_usage_settings(root),
            history: load_history_settings(root),
        }
    }

    /// The value `spec` names in these settings.
    pub fn value(&self, spec: &SettingSpec) -> SettingValue {
        specs::field_value(self, spec)
    }
}

/// One row of [`effective_settings`].
#[derive(Clone, Debug, PartialEq)]
pub struct EffectiveSetting {
    pub spec: &'static SettingSpec,
    pub value: SettingValue,
    /// The key is present in the file (an explicit value equal to the default still counts).
    pub is_set: bool,
}

/// `haryx auto` flags that override settings.json; `None` means "not given".
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AutoSwitchOverrides {
    pub threshold: Option<f64>,
    pub weekly_threshold: Option<f64>,
    pub interval: Option<f64>,
    pub cooldown: Option<f64>,
    pub include_api_key_accounts: Option<bool>,
    pub model: Option<String>,
    pub strategy: Option<String>,
}

pub fn settings_path(root: &Path) -> PathBuf {
    root.join(SETTINGS_FILENAME)
}

/// Split a comma-separated model list, trimmed and case-insensitively deduped (first spelling wins).
/// Shared by the auto engine and the manual switch strategies so both read `autoswitch.model` alike.
pub fn parse_model_names(value: Option<&str>) -> Vec<String> {
    let mut seen: Vec<String> = Vec::new();
    for part in value.unwrap_or_default().split(',') {
        let name = part.trim();
        if !name.is_empty() && !seen.iter().any(|s| s.to_lowercase() == name.to_lowercase()) {
            seen.push(name.to_owned());
        }
    }
    seen
}

// ---------------------------------------------------------------------------------------------- read

/// Read settings.json as a JSON object; missing → empty, unreadable or not an object → empty with a
/// warning.
fn read_raw(path: &Path) -> Map<String, Value> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Map::new(),
        Err(err) => {
            log::warn!("Could not read {} ({err}); using defaults", path.display());
            return Map::new();
        }
    };
    match serde_json::from_str::<Value>(&text) {
        Ok(Value::Object(raw)) => raw,
        Ok(_) => {
            log::warn!("{} is not a JSON object; using defaults", path.display());
            Map::new()
        }
        Err(err) => {
            log::warn!("Could not read {} ({err}); using defaults", path.display());
            Map::new()
        }
    }
}

fn read_section(root: &Path, section: &str) -> Option<Map<String, Value>> {
    match read_raw(&settings_path(root)).remove(section) {
        Some(Value::Object(map)) => Some(map),
        _ => None,
    }
}

/// The JSON keys the loaders read in `section`, in registry order.
pub(crate) fn section_keys(section: &str) -> Vec<&'static str> {
    SETTING_SPECS
        .iter()
        .filter(|spec| spec.section == section)
        .map(|spec| spec.json_key)
        .collect()
}

/// Keys in settings.json that nothing reads: a typo'd or misplaced key, or one written by another
/// version. They survive every write; they just do nothing, which the file alone does not show.
pub fn unrecognized_keys(root: &Path) -> Vec<UnrecognizedKey> {
    unrecognized::unrecognized(&read_raw(&settings_path(root)))
}

/// Log one warning per key [`unrecognized_keys`] finds. For a process that keeps running to call once as
/// it starts, not for the loaders: they run on every command.
pub fn log_unrecognized_keys(root: &Path) {
    for item in unrecognized_keys(root) {
        log::warn!("settings.json: {}", item.describe());
    }
}

/// Load the autoswitch section; a missing or corrupt file or field gives the default, and numbers are
/// clamped into their spec's range.
pub fn load_settings(root: &Path) -> AutoSwitchSettings {
    match read_section(root, "autoswitch") {
        Some(section) => specs::clamp_autoswitch(&section),
        None => AutoSwitchSettings::default(),
    }
}

/// Load the ui section; a missing or corrupt file or a bad value gives its default.
pub fn load_ui_settings(root: &Path) -> UiSettings {
    let Some(section) = read_section(root, "ui") else {
        return UiSettings::default();
    };
    let choice = |key: &str| {
        let spec = setting_spec(&format!("ui.{key}")).ok()?;
        let default = spec.default().as_str().unwrap_or_default().to_owned();
        let value = section.get(key);
        match value {
            None => Some(default),
            Some(Value::String(s)) if spec.choices.contains(&s.as_str()) => Some(s.clone()),
            Some(other) => {
                log::warn!(
                    "settings.json: unsupported {} {}; using {}",
                    spec.dotted(),
                    repr_json(other),
                    repr_json(&Value::String(default.clone()))
                );
                Some(default)
            }
        }
    };
    let defaults = UiSettings::default();
    UiSettings {
        theme: choice("theme").unwrap_or(defaults.theme),
        org_tag: choice("orgTag").unwrap_or(defaults.org_tag),
    }
}

/// Load the run section; a missing or corrupt file or a non-boolean value gives the default.
pub fn load_run_settings(root: &Path) -> RunSettings {
    explicit_share_history(root)
        .map(|share_history| RunSettings { share_history })
        .unwrap_or_default()
}

/// `run.shareHistory` as the user set it; `None` when they never did (a non-boolean counts as never
/// set). [`load_run_settings`] answers "what applies"; this answers "who decided".
pub fn explicit_share_history(root: &Path) -> Option<bool> {
    read_section(root, "run")?.get("shareHistory")?.as_bool()
}

/// Load the updates section; a missing or corrupt file or a non-boolean gives the default.
pub fn load_update_settings(root: &Path) -> UpdateSettings {
    let default = UpdateSettings::default();
    let Some(section) = read_section(root, "updates") else {
        return default;
    };
    match section.get("check") {
        None => default,
        Some(Value::Bool(check)) => UpdateSettings { check: *check },
        Some(other) => {
            log::warn!(
                "settings.json: updates.check {} is not true or false; using {}",
                repr_json(other),
                repr_json(&Value::Bool(default.check))
            );
            default
        }
    }
}

/// Load the usage section; a missing or corrupt file gives the defaults, and a non-boolean value gives
/// that key's default (the other key still applies).
pub fn load_usage_settings(root: &Path) -> UsageSettings {
    let default = UsageSettings::default();
    let Some(section) = read_section(root, "usage") else {
        return default;
    };
    let flag = |key: &str, fallback: bool| match section.get(key) {
        None => fallback,
        Some(Value::Bool(value)) => *value,
        Some(other) => {
            log::warn!(
                "settings.json: usage.{key} must be true or false, got {}; using {}",
                repr_json(other),
                repr_json(&Value::Bool(fallback))
            );
            fallback
        }
    };
    UsageSettings {
        header_probe: flag("headerProbe", default.header_probe),
        fable_probe: flag("fableProbe", default.fable_probe),
        share_between_machines: flag("shareBetweenMachines", default.share_between_machines),
    }
}

/// (spec, effective value, explicitly set?) per key, in registry order. "Set" means the key is present in
/// the raw file, so `haryx config`'s "(default)" marker reflects the file, not value equality.
pub fn effective_settings(root: &Path) -> Vec<EffectiveSetting> {
    let raw = read_raw(&settings_path(root));
    let loaded = LoadedSettings::load(root);
    SETTING_SPECS
        .iter()
        .map(|spec| EffectiveSetting {
            spec,
            value: loaded.value(spec),
            is_set: matches!(raw.get(spec.section), Some(Value::Object(s)) if s.contains_key(spec.json_key)),
        })
        .collect()
}

/// Overlay the given `haryx auto` flags onto `settings`, clamped like a loaded file. With no flag given
/// the settings come back unchanged.
pub fn merged_with_cli(
    settings: &AutoSwitchSettings,
    overrides: &AutoSwitchOverrides,
) -> AutoSwitchSettings {
    let mut section = specs::autoswitch_to_json(settings);
    let mut put = |key: &str, value: Option<Value>| {
        if let Some(value) = value {
            section.insert(key.to_owned(), value);
        }
    };
    put("threshold", overrides.threshold.map(Value::from));
    put(
        "weeklyThreshold",
        overrides.weekly_threshold.map(Value::from),
    );
    put("intervalSeconds", overrides.interval.map(Value::from));
    put("cooldownSeconds", overrides.cooldown.map(Value::from));
    put(
        "includeApiKeyAccounts",
        overrides.include_api_key_accounts.map(Value::from),
    );
    put("model", overrides.model.clone().map(Value::from));
    put("strategy", overrides.strategy.clone().map(Value::from));
    if *overrides == AutoSwitchOverrides::default() {
        return settings.clone();
    }
    specs::clamp_autoswitch(&section)
}

// --------------------------------------------------------------------------------------------- write

/// Write the autoswitch section (every key), preserving unknown keys and sections.
pub fn save_settings(root: &Path, settings: &AutoSwitchSettings) -> Result<()> {
    let path = settings_path(root);
    let mut raw = read_raw(&path);
    stamp_schema_version(&mut raw);
    let section = section_mut(&mut raw, "autoswitch");
    for (key, value) in specs::autoswitch_to_json(settings) {
        section.insert(key, value);
    }
    atomic_write_json(&path, &Value::Object(raw))
}

fn stamp_schema_version(raw: &mut Map<String, Value>) {
    raw.entry("schemaVersion")
        .or_insert_with(|| Value::from(SETTINGS_SCHEMA_VERSION));
}

/// Strictly parse a CLI-provided string for `haryx config set`.
///
/// Unlike the forgiving clamp on load, out-of-range or mistyped values are an [`Error::Config`] so the
/// user learns about the problem when setting the value, not by degraded behaviour at `haryx auto` time.
pub fn parse_setting_value(spec: &SettingSpec, raw_value: &str) -> Result<SettingValue> {
    let dotted = spec.dotted();
    match spec.kind {
        SettingKind::Bool => bool_word(raw_value).map(SettingValue::Bool).ok_or_else(|| {
            Error::Config(format!(
                "{dotted} expects true or false (or 1/0, yes/no), got '{raw_value}'"
            ))
        }),
        SettingKind::Choice => {
            if spec.choices.contains(&raw_value) {
                Ok(SettingValue::Str(raw_value.to_owned()))
            } else {
                Err(Error::Config(format!(
                    "{dotted} must be one of: {}",
                    spec.choices.join(", ")
                )))
            }
        }
        SettingKind::String => {
            let value = raw_value.trim();
            if value.is_empty() {
                return Err(Error::Config(format!(
                    "{dotted} expects a non-empty value; use '{AI_PROG} config unset {dotted}' to clear it"
                )));
            }
            Ok(SettingValue::Str(value.to_owned()))
        }
        SettingKind::Int | SettingKind::Float => parse_number(spec, raw_value),
    }
}

fn parse_number(spec: &SettingSpec, raw_value: &str) -> Result<SettingValue> {
    let dotted = spec.dotted();
    let text = raw_value.trim();
    let parsed = match spec.kind {
        SettingKind::Int => {
            parse_int(text).map(|(value, as_float)| (SettingValue::Int(value), as_float))
        }
        _ => text
            .parse::<f64>()
            .ok()
            .map(|v| (SettingValue::Float(v), v)),
    };
    let Some((value, number)) = parsed else {
        let noun = if spec.kind == SettingKind::Int {
            "an integer"
        } else {
            "a number"
        };
        // An optional number has no spelling for "unset" here; point at the command that clears it.
        let hint = if spec.default() == SettingValue::None {
            format!("; use '{AI_PROG} config unset {dotted}' to clear it")
        } else {
            String::new()
        };
        return Err(Error::Config(format!(
            "{dotted} expects {noun}, got '{raw_value}'{hint}"
        )));
    };
    let (lo, hi) = (spec.lo.unwrap_or(f64::MIN), spec.hi.unwrap_or(f64::MAX));
    if !(lo <= number && number <= hi) {
        return Err(Error::Config(format!(
            "{dotted} must be between {} and {}",
            format_setting_value(&SettingValue::Float(lo)),
            format_setting_value(&SettingValue::Float(hi))
        )));
    }
    // An integer too large for i64 reached here only as out of range; `value` is exact otherwise.
    Ok(value)
}

/// Python's `int(text)` for a trimmed decimal: the value (saturated) and its magnitude as a float, so an
/// integer too large for `i64` still fails the range test rather than the type test.
fn parse_int(text: &str) -> Option<(i64, f64)> {
    let digits = text.strip_prefix(['+', '-']).unwrap_or(text);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    match text.parse::<i64>() {
        Ok(value) => Some((value, value as f64)),
        Err(_) => {
            let magnitude = if text.starts_with('-') {
                f64::MIN
            } else {
                f64::MAX
            };
            Some((0, magnitude))
        }
    }
}

/// The words a boolean setting accepts, case-insensitively and trimmed.
pub(crate) fn bool_word(text: &str) -> Option<bool> {
    match text.trim().to_lowercase().as_str() {
        "true" | "1" | "yes" => Some(true),
        "false" | "0" | "no" => Some(false),
        _ => None,
    }
}

/// Render a settings value the way settings.json writes it: `(none)`, `true`/`false`, a whole float
/// without its `.0`.
pub fn format_setting_value(value: &SettingValue) -> String {
    match value {
        SettingValue::None => "(none)".to_owned(),
        SettingValue::Bool(b) => b.to_string(),
        SettingValue::Int(i) => i.to_string(),
        SettingValue::Float(f) if f.is_finite() && f.fract() == 0.0 => format!("{f:.0}"),
        SettingValue::Float(f) => f.to_string(),
        SettingValue::Str(s) => s.clone(),
    }
}

/// Raw read for the config write path: a corrupt file is an error, never `{}`. A read-modify-write
/// starting from `{}` would replace a malformed (maybe hand-recoverable) file with a near-empty one.
fn read_raw_for_write(path: &Path) -> Result<Map<String, Value>> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Map::new()),
        Err(err) => {
            return Err(Error::Config(format!(
                "could not read {}: {err}",
                path.display()
            )));
        }
    };
    match serde_json::from_str::<Value>(&text) {
        Ok(Value::Object(raw)) => Ok(raw),
        Ok(_) => Err(Error::Config(format!(
            "{} is not a JSON object; fix or delete it before changing settings",
            path.display()
        ))),
        Err(err) => Err(Error::Config(format!(
            "{} is not valid JSON ({err}); fix or delete it before changing settings",
            path.display()
        ))),
    }
}

/// Validate and persist one key for `haryx config set`; returns the value.
///
/// Writes only the given key (plus schemaVersion), deliberately not [`save_settings`], which writes every
/// known key and would freeze the current defaults into the file, pinning users to them if a later
/// version changes a default. Unknown keys and sections in the file survive.
pub fn set_setting(root: &Path, dotted_key: &str, raw_value: &str) -> Result<SettingValue> {
    let spec = setting_spec(dotted_key)?;
    let value = parse_setting_value(spec, raw_value)?;
    let path = settings_path(root);
    let mut raw = read_raw_for_write(&path)?;
    stamp_schema_version(&mut raw);
    section_mut(&mut raw, spec.section).insert(spec.json_key.to_owned(), value.to_json());
    atomic_write_json(&path, &Value::Object(raw))?;
    Ok(value)
}

/// The object under `name`, replacing a missing or non-object value with `{}` in place, so the section
/// keeps its position in the file (as a Python dict assignment does).
fn section_mut<'a>(raw: &'a mut Map<String, Value>, name: &str) -> &'a mut Map<String, Value> {
    let slot = raw.entry(name).or_insert(Value::Null);
    if !slot.is_object() {
        *slot = Value::Object(Map::new());
    }
    // Invariant: the slot holds an object, set just above.
    slot.as_object_mut().expect("section slot is an object")
}

/// Remove one key from settings.json; `false` (and no write) when it was not set. An emptied section is
/// removed entirely.
pub fn unset_setting(root: &Path, dotted_key: &str) -> Result<bool> {
    let spec = setting_spec(dotted_key)?;
    let path = settings_path(root);
    let mut raw = read_raw_for_write(&path)?;
    let Some(Value::Object(section)) = raw.get_mut(spec.section) else {
        return Ok(false);
    };
    if section.shift_remove(spec.json_key).is_none() {
        return Ok(false);
    }
    if section.is_empty() {
        raw.shift_remove(spec.section);
    }
    stamp_schema_version(&mut raw);
    atomic_write_json(&path, &Value::Object(raw))?;
    Ok(true)
}

/// Atomically write JSON (Python `json.dumps(indent=2)` text) with the store's 0600/0700 modes.
///
/// Shared by settings.json and the auto-switch state file. **Writes THROUGH a symlink, never over it**: a
/// rename onto a symlinked path detaches the link, and the link target silently stops receiving updates.
/// So a dangling link still writes where it points; the temp file is created beside the RESOLVED target
/// (the rename stays on one filesystem); and the 0700 hardening stays on the directory haryx owns
/// (`path`'s parent), never the resolved parent, which may belong to something else. The written file
/// still gets 0600.
pub fn atomic_write_json(path: &Path, data: &Value) -> Result<()> {
    let target = publish_target(path)?;
    if let Some(parent) = target.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    #[cfg(unix)]
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        use std::os::unix::fs::PermissionsExt;
        // `path`'s parent, NOT the target's: see above.
        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))?;
    }
    atomic_write_text(&target, &json_dumps_pretty(data), WriteOptions::default())?;
    Ok(())
}

#[cfg(test)]
mod tests;

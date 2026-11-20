//! The settings registry: one [`SettingSpec`] per user-tunable key, and the forgiving clamp on load.

use serde_json::{Map, Value};

use super::{AUTOSWITCH_STRATEGIES, AutoSwitchSettings, LoadedSettings, bool_word};
use crate::PROG;
use crate::error::{Error, Result};
use crate::models::pyfmt::repr_json;
use crate::models::{ACCOUNT_THRESHOLD_MAX, ACCOUNT_THRESHOLD_MIN};

/// The type of a setting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingKind {
    Float,
    Int,
    Bool,
    /// One of the spec's `choices`.
    Choice,
    /// A non-empty string (or unset).
    String,
}

/// A setting's value: what `config get` shows and `config set` writes. `None` is an unset optional key.
#[derive(Clone, Debug, PartialEq)]
pub enum SettingValue {
    None,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
}

impl SettingValue {
    /// The JSON form written to settings.json and printed by `--json`.
    pub fn to_json(&self) -> Value {
        match self {
            SettingValue::None => Value::Null,
            SettingValue::Bool(b) => Value::Bool(*b),
            SettingValue::Int(i) => Value::from(*i),
            SettingValue::Float(f) => Value::from(*f),
            SettingValue::Str(s) => Value::String(s.clone()),
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            SettingValue::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            SettingValue::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            SettingValue::Float(f) => Some(*f),
            SettingValue::Int(i) => Some(*i as f64),
            _ => None,
        }
    }
}

/// Metadata for one settings.json key.
///
/// Single source of truth for bounds and choices: both the lenient clamp on load and the strict
/// validation in `haryx config set` ([`super::parse_setting_value`]) read from here.
#[derive(Clone, Debug, PartialEq)]
pub struct SettingSpec {
    /// Top-level JSON section.
    pub section: &'static str,
    /// camelCase key inside the section.
    pub json_key: &'static str,
    /// snake_case field of the section's settings struct.
    pub field: &'static str,
    pub kind: SettingKind,
    pub lo: Option<f64>,
    pub hi: Option<f64>,
    pub choices: &'static [&'static str],
    /// Help text; `{prog}` stands for the program name (see [`SettingSpec::help_text`]).
    pub help: &'static str,
}

impl SettingSpec {
    const fn new(
        section: &'static str,
        json_key: &'static str,
        field: &'static str,
        kind: SettingKind,
        help: &'static str,
    ) -> Self {
        SettingSpec {
            section,
            json_key,
            field,
            kind,
            lo: None,
            hi: None,
            choices: &[],
            help,
        }
    }

    const fn range(self, lo: f64, hi: f64) -> Self {
        SettingSpec {
            lo: Some(lo),
            hi: Some(hi),
            ..self
        }
    }

    const fn choices(self, choices: &'static [&'static str]) -> Self {
        SettingSpec { choices, ..self }
    }

    /// `section.jsonKey`, the name `haryx config` takes.
    pub fn dotted(&self) -> String {
        format!("{}.{}", self.section, self.json_key)
    }

    /// The help text with the program name filled in.
    pub fn help_text(&self) -> String {
        self.help.replace("{prog}", PROG)
    }

    /// The value used when the file does not set this key.
    pub fn default(&self) -> SettingValue {
        field_value(&LoadedSettings::default(), self)
    }
}

use SettingKind::{Bool, Choice, Float, Int, String as Text};

/// Every key, in registry order (the order `haryx config` lists them).
pub static SETTING_SPECS: [SettingSpec; 15] = [
    SettingSpec::new(
        "autoswitch",
        "threshold",
        "threshold",
        Float,
        "Switch when the 5h window (or any window, if no weekly line) reaches this pct",
    )
    .range(ACCOUNT_THRESHOLD_MIN, ACCOUNT_THRESHOLD_MAX),
    SettingSpec::new(
        "autoswitch",
        "weeklyThreshold",
        "weekly_threshold",
        Float,
        "Switch line for the 7d and per-model windows (unset: threshold)",
    )
    .range(ACCOUNT_THRESHOLD_MIN, ACCOUNT_THRESHOLD_MAX),
    SettingSpec::new(
        "autoswitch",
        "intervalSeconds",
        "interval_seconds",
        Float,
        "Poll interval for the {prog} auto loop, in seconds",
    )
    .range(15.0, 3600.0),
    SettingSpec::new(
        "autoswitch",
        "cooldownSeconds",
        "cooldown_seconds",
        Float,
        "Minimum seconds between proactive switches",
    )
    .range(0.0, 86400.0),
    SettingSpec::new(
        "autoswitch",
        "hysteresisPct",
        "hysteresis_pct",
        Float,
        "A target must beat the active account by this many pct of headroom",
    )
    .range(0.0, 50.0),
    SettingSpec::new(
        "autoswitch",
        "strategy",
        "strategy",
        Choice,
        "How auto-switch picks the target account",
    )
    .choices(AUTOSWITCH_STRATEGIES),
    SettingSpec::new(
        "autoswitch",
        "includeApiKeyAccounts",
        "include_api_key_accounts",
        Bool,
        "Allow rotating onto managed API-key accounts (bill per token)",
    ),
    SettingSpec::new(
        "autoswitch",
        "unhealthyTicks",
        "unhealthy_ticks",
        Int,
        "Consecutive failed polls before an account is unhealthy",
    )
    .range(1.0, 100.0),
    SettingSpec::new(
        "autoswitch",
        "model",
        "model",
        Text,
        "Also switch on these models' weekly limits (e.g. Fable, Fable,Opus, or all)",
    ),
    SettingSpec::new(
        "ui",
        "theme",
        "theme",
        Choice,
        "Color theme; auto follows the terminal background",
    )
    .choices(&["dark", "light", "auto"]),
    SettingSpec::new(
        "ui",
        "orgTag",
        "org_tag",
        Choice,
        "Show the [org] tag beside accounts; auto only where accounts share an email",
    )
    .choices(&["auto", "on", "off"]),
    SettingSpec::new(
        "run",
        "shareHistory",
        "share_history",
        Bool,
        "{prog} run shares one chat history across accounts",
    ),
    SettingSpec::new(
        "updates",
        "check",
        "check",
        Bool,
        "Check for a newer release once a day and print a hint when one exists",
    ),
    SettingSpec::new(
        "usage",
        "headerProbe",
        "header_probe",
        Bool,
        "Read setup-token usage from a 1-token Haiku request's rate-limit headers (opt-in: uses quota)",
    ),
    SettingSpec::new(
        "usage",
        "fableProbe",
        "fable_probe",
        Bool,
        "With usage.headerProbe, also read Fable's weekly limit from a 1-token Fable request (opt-in: uses Fable quota)",
    ),
];

/// Look up a spec by dotted key; an unknown key is an [`Error::Config`] listing the valid ones.
pub fn setting_spec(dotted_key: &str) -> Result<&'static SettingSpec> {
    SETTING_SPECS
        .iter()
        .find(|spec| spec.dotted() == dotted_key)
        .ok_or_else(|| {
            let valid: Vec<String> = SETTING_SPECS.iter().map(SettingSpec::dotted).collect();
            Error::Config(format!(
                "unknown setting '{dotted_key}'\nValid keys: {}",
                valid.join(", ")
            ))
        })
}

/// The value `spec` names in `loaded` (upstream `getattr(section_settings, spec.field)`).
pub(super) fn field_value(loaded: &LoadedSettings, spec: &SettingSpec) -> SettingValue {
    let a = &loaded.autoswitch;
    let opt_str = |v: &Option<String>| v.clone().map_or(SettingValue::None, SettingValue::Str);
    match (spec.section, spec.field) {
        ("autoswitch", "threshold") => SettingValue::Float(a.threshold),
        ("autoswitch", "weekly_threshold") => a
            .weekly_threshold
            .map_or(SettingValue::None, SettingValue::Float),
        ("autoswitch", "interval_seconds") => SettingValue::Float(a.interval_seconds),
        ("autoswitch", "cooldown_seconds") => SettingValue::Float(a.cooldown_seconds),
        ("autoswitch", "hysteresis_pct") => SettingValue::Float(a.hysteresis_pct),
        ("autoswitch", "strategy") => SettingValue::Str(a.strategy.clone()),
        ("autoswitch", "include_api_key_accounts") => {
            SettingValue::Bool(a.include_api_key_accounts)
        }
        ("autoswitch", "unhealthy_ticks") => SettingValue::Int(a.unhealthy_ticks),
        ("autoswitch", "model") => opt_str(&a.model),
        ("ui", "theme") => SettingValue::Str(loaded.ui.theme.clone()),
        ("ui", "org_tag") => SettingValue::Str(loaded.ui.org_tag.clone()),
        ("run", "share_history") => SettingValue::Bool(loaded.run.share_history),
        ("updates", "check") => SettingValue::Bool(loaded.updates.check),
        ("usage", "header_probe") => SettingValue::Bool(loaded.usage.header_probe),
        ("usage", "fable_probe") => SettingValue::Bool(loaded.usage.fable_probe),
        _ => SettingValue::None,
    }
}

/// The autoswitch section as settings.json writes it (every key, `null` for an unset optional one).
pub(super) fn autoswitch_to_json(settings: &AutoSwitchSettings) -> Map<String, Value> {
    let loaded = LoadedSettings {
        autoswitch: settings.clone(),
        ..LoadedSettings::default()
    };
    autoswitch_specs()
        .map(|spec| {
            (
                spec.json_key.to_owned(),
                field_value(&loaded, spec).to_json(),
            )
        })
        .collect()
}

fn autoswitch_specs() -> impl Iterator<Item = &'static SettingSpec> {
    SETTING_SPECS
        .iter()
        .filter(|spec| spec.section == "autoswitch")
}

/// Build the autoswitch settings from a (possibly hand-edited) section: numbers clamped into their
/// spec's range, bad types and unsupported choices back to the default.
pub(super) fn clamp_autoswitch(section: &Map<String, Value>) -> AutoSwitchSettings {
    let value = |key: &str| {
        autoswitch_specs()
            .find(|spec| spec.json_key == key)
            .map(|spec| match section.get(key) {
                Some(raw) => clamp_value(spec, raw),
                None => spec.default(),
            })
            .unwrap_or(SettingValue::None)
    };
    let defaults = AutoSwitchSettings::default();
    let float = |key: &str, default: f64| value(key).as_f64().unwrap_or(default);
    AutoSwitchSettings {
        threshold: float("threshold", defaults.threshold),
        weekly_threshold: value("weeklyThreshold").as_f64(),
        interval_seconds: float("intervalSeconds", defaults.interval_seconds),
        cooldown_seconds: float("cooldownSeconds", defaults.cooldown_seconds),
        hysteresis_pct: float("hysteresisPct", defaults.hysteresis_pct),
        strategy: value("strategy")
            .as_str()
            .map_or(defaults.strategy, str::to_owned),
        include_api_key_accounts: value("includeApiKeyAccounts")
            .as_bool()
            .unwrap_or(defaults.include_api_key_accounts),
        unhealthy_ticks: match value("unhealthyTicks") {
            SettingValue::Int(i) => i,
            _ => defaults.unhealthy_ticks,
        },
        model: value("model").as_str().map(str::to_owned),
    }
}

/// The forgiving read of one value present in the file.
fn clamp_value(spec: &SettingSpec, raw: &Value) -> SettingValue {
    match spec.kind {
        Float | Int => clamp_number(spec, raw),
        Bool => match raw {
            // Never truthiness of a string: "false" is a non-empty string. A quoted boolean is read by
            // the words `haryx config set` accepts; anything else reverts to the default.
            Value::String(text) => bool_word(text).map_or_else(
                || {
                    log::warn!(
                        "settings.json: {} expects a boolean, got {}; using {}",
                        spec.dotted(),
                        repr_json(raw),
                        repr_json(&spec.default().to_json())
                    );
                    spec.default()
                },
                SettingValue::Bool,
            ),
            other => SettingValue::Bool(truthy(other)),
        },
        // A non-empty string keeps as-is; anything else reverts to the default (None), so a null or
        // garbage value disables the filter.
        Text => match raw {
            Value::String(text) if !text.is_empty() => SettingValue::Str(text.clone()),
            _ => spec.default(),
        },
        Choice => match raw {
            Value::String(text) if spec.choices.contains(&text.as_str()) => {
                SettingValue::Str(text.clone())
            }
            _ => {
                log::warn!(
                    "settings.json: unsupported {} {}; using {}",
                    spec.dotted(),
                    repr_json(raw),
                    repr_json(&spec.default().to_json())
                );
                spec.default()
            }
        },
    }
}

/// A JSON number clamped into `[lo, hi]`; anything else (bool, string, null) is the default. serde_json
/// never yields a non-finite number, and an integer beyond `f64` range fails to parse the whole file.
fn clamp_number(spec: &SettingSpec, raw: &Value) -> SettingValue {
    let Some(number) = raw.as_f64().filter(|n| n.is_finite()) else {
        return spec.default();
    };
    let (lo, hi) = (spec.lo.unwrap_or(f64::MIN), spec.hi.unwrap_or(f64::MAX));
    let clamped = number.clamp(lo, hi);
    match spec.kind {
        // Truncation toward zero, as Python's int(float).
        Int => SettingValue::Int(clamped.trunc() as i64),
        _ => SettingValue::Float(clamped),
    }
}

/// Python truthiness of a non-string JSON value.
fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(items) => !items.is_empty(),
        Value::Object(map) => !map.is_empty(),
    }
}

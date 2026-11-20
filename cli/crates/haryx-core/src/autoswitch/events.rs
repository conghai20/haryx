//! Auto-switch events: the engine's only output channel (`autoswitch.py` event classes).
//!
//! Each event serializes to one JSONL line (`{"schemaVersion":1,"event":kind,"ts":...,...}`) and
//! renders one human line; both shapes are part of the `auto --json` contract.

use serde_json::{Map, Value, json};

use crate::PROG;
use crate::json_output::SCHEMA_VERSION;
use crate::lines::binding_window;
use crate::oauth::UsageWindow;

use super::pct_label;

/// Ordered `account number → value` map (accounts in slot order).
pub type ByAccount<T> = Vec<(String, T)>;

/// One auto-switch event (kind + payload); `ts` is stamped at construction.
#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    pub ts: String,
    pub body: EventBody,
}

#[derive(Clone, Debug, PartialEq)]
pub enum EventBody {
    Poll {
        /// `{"number": int, "email": str}`, or `None` with no active account.
        active: Option<Value>,
        headroom: ByAccount<Option<f64>>,
        threshold: f64,
        fetch_errors: ByAccount<String>,
        windows: ByAccount<Vec<(String, f64)>>,
        weekly_threshold: Option<f64>,
    },
    Switch {
        trigger: String,
        from: Option<Value>,
        to: Option<Value>,
        warnings: Vec<String>,
        dry_run: bool,
    },
    NoSwitch {
        reason: String,
        detail: String,
    },
    Quarantined {
        number: String,
        email: String,
        reason: String,
    },
    Unquarantined {
        number: String,
        email: String,
        reason: String,
    },
    AllExhausted {
        earliest_reset_at: Option<String>,
    },
    Sleep {
        seconds: f64,
        until: String,
    },
    Error {
        message: String,
        transient: bool,
    },
    ConfigWarning {
        message: String,
    },
}

/// Current UTC time as `YYYY-MM-DDTHH:MM:SSZ`.
pub fn now_iso() -> String {
    iso_seconds(jiff::Timestamp::now())
}

/// A timestamp as `YYYY-MM-DDTHH:MM:SSZ` (whole seconds, truncated).
pub fn iso_seconds(ts: jiff::Timestamp) -> String {
    ts.strftime("%Y-%m-%dT%H:%M:%SZ").to_string()
}

impl Event {
    pub fn new(body: EventBody) -> Self {
        Event {
            ts: now_iso(),
            body,
        }
    }

    pub fn no_switch(reason: &str, detail: impl Into<String>) -> Self {
        Event::new(EventBody::NoSwitch {
            reason: reason.to_owned(),
            detail: detail.into(),
        })
    }

    pub fn error(message: impl Into<String>, transient: bool) -> Self {
        Event::new(EventBody::Error {
            message: message.into(),
            transient,
        })
    }

    /// The event's `event` discriminator.
    pub fn kind(&self) -> &'static str {
        match &self.body {
            EventBody::Poll { .. } => "poll",
            EventBody::Switch { .. } => "switch",
            EventBody::NoSwitch { .. } => "no-switch",
            EventBody::Quarantined { .. } => "account-quarantined",
            EventBody::Unquarantined { .. } => "account-unquarantined",
            EventBody::AllExhausted { .. } => "all-exhausted",
            EventBody::Sleep { .. } => "sleep",
            EventBody::Error { .. } => "error",
            EventBody::ConfigWarning { .. } => "config-warning",
        }
    }

    pub fn to_json(&self) -> Value {
        let mut out = Map::new();
        out.insert("schemaVersion".into(), json!(SCHEMA_VERSION));
        out.insert("event".into(), json!(self.kind()));
        out.insert("ts".into(), json!(self.ts));
        for (key, value) in self.fields() {
            out.insert(key.to_owned(), value);
        }
        Value::Object(out)
    }

    fn fields(&self) -> Vec<(&'static str, Value)> {
        match &self.body {
            EventBody::Poll {
                active,
                headroom,
                threshold,
                fetch_errors,
                windows,
                weekly_threshold,
            } => {
                let mut fields = vec![
                    ("active", active.clone().unwrap_or(Value::Null)),
                    ("headroomPct", object(headroom, |h| json!(h))),
                    ("threshold", json!(threshold)),
                ];
                if !fetch_errors.is_empty() {
                    fields.push(("fetchErrors", object(fetch_errors, |e| json!(e))));
                }
                if !windows.is_empty() {
                    fields.push(("windowsPct", object(windows, |w| pairs(w))));
                }
                if let Some(weekly) = weekly_threshold {
                    fields.push(("weeklyThreshold", json!(weekly)));
                }
                fields
            }
            EventBody::Switch {
                trigger,
                from,
                to,
                warnings,
                dry_run,
            } => vec![
                ("trigger", json!(trigger)),
                ("from", from.clone().unwrap_or(Value::Null)),
                ("to", to.clone().unwrap_or(Value::Null)),
                ("warnings", json!(warnings)),
                ("dryRun", json!(dry_run)),
            ],
            EventBody::NoSwitch { reason, detail } => {
                vec![("reason", json!(reason)), ("detail", json!(detail))]
            }
            EventBody::Quarantined {
                number,
                email,
                reason,
            }
            | EventBody::Unquarantined {
                number,
                email,
                reason,
            } => vec![
                ("number", json!(number)),
                ("email", json!(email)),
                ("reason", json!(reason)),
            ],
            EventBody::AllExhausted { earliest_reset_at } => {
                vec![("earliestResetAt", json!(earliest_reset_at))]
            }
            EventBody::Sleep { seconds, until } => vec![
                ("seconds", json!((seconds * 10.0).round() / 10.0)),
                ("until", json!(until)),
            ],
            EventBody::Error { message, transient } => {
                vec![("message", json!(message)), ("transient", json!(transient))]
            }
            EventBody::ConfigWarning { message } => vec![("message", json!(message))],
        }
    }

    /// The one-line human rendering (`cswap auto` without `--json`).
    pub fn human(&self) -> String {
        match &self.body {
            EventBody::Poll { .. } => self.poll_human(),
            EventBody::Switch {
                trigger,
                from,
                to,
                dry_run,
                ..
            } => {
                let src = from.as_ref().map_or_else(
                    || "(none)".to_owned(),
                    |r| format!("Account-{}", ref_num(r)),
                );
                let dst = to.as_ref().map_or_else(
                    || "?".to_owned(),
                    |r| format!("Account-{} ({})", ref_num(r), ref_email(r)),
                );
                let prefix = if *dry_run {
                    "[dry-run] would switch"
                } else {
                    "Switched"
                };
                format!("{prefix} {src} -> {dst} ({trigger})")
            }
            EventBody::NoSwitch { reason, detail } if detail.is_empty() => {
                format!("no switch: {reason}")
            }
            EventBody::NoSwitch { reason, detail } => format!("no switch: {reason} ({detail})"),
            EventBody::Quarantined {
                number,
                email,
                reason,
            } => format!(
                "Account-{number} ({email}) quarantined: {reason}. Log in with it and run \
                 '{PROG} --add-account --slot {number}' to recover."
            ),
            EventBody::Unquarantined {
                number,
                email,
                reason,
            } => format!("Account-{number} ({email}) back in rotation ({reason})"),
            EventBody::AllExhausted {
                earliest_reset_at: Some(at),
            } => format!("all accounts exhausted; earliest reset {at}"),
            EventBody::AllExhausted { .. } => "all accounts exhausted; no reset time known".into(),
            EventBody::Sleep { seconds, until } => {
                format!("sleeping {:.0}m (until {until})", seconds / 60.0)
            }
            EventBody::Error { message, transient } => {
                let tail = if *transient { " (will retry)" } else { "" };
                format!("error: {message}{tail}")
            }
            EventBody::ConfigWarning { message } => format!("warning: {message}"),
        }
    }

    fn poll_human(&self) -> String {
        let EventBody::Poll {
            active,
            headroom,
            threshold,
            fetch_errors,
            windows,
            weekly_threshold,
        } = &self.body
        else {
            return String::new();
        };
        let Some(active) = active else {
            return "poll: no active account".into();
        };
        let num = ref_num(active);
        let mut line = *threshold;
        let used = if let Some(wins) = lookup(windows, &num).filter(|w| !w.is_empty()) {
            let windows: Vec<UsageWindow> = wins
                .iter()
                .map(|(label, pct)| UsageWindow {
                    label: label.clone(),
                    pct: *pct,
                    resets_at: None,
                })
                .collect();
            match binding_window(&windows, *threshold, *weekly_threshold, None) {
                Some(binding) => {
                    line = binding.line;
                    format!("{} {:.0}% used", binding.label, binding.pct)
                }
                None => "usage unknown".into(),
            }
        } else if let Some(Some(h)) = lookup(headroom, &num) {
            format!("{:.0}% used", 100.0 - h)
        } else {
            match lookup(fetch_errors, &num) {
                Some(err) => format!("usage unknown ({err})"),
                None => "usage unknown".into(),
            }
        };
        let others: Vec<String> = headroom
            .iter()
            .filter(|(n, _)| *n != num)
            .map(|(n, _)| format!("#{n}: {}", describe(n, headroom, fetch_errors, windows)))
            .collect();
        let tail = if others.is_empty() {
            String::new()
        } else {
            format!(" | others: {}", others.join(", "))
        };
        format!(
            "Account-{num} ({}): {used} (switch at {}%){tail}",
            ref_email(active),
            pct_label(line)
        )
    }
}

fn describe(
    num: &str,
    headroom: &ByAccount<Option<f64>>,
    fetch_errors: &ByAccount<String>,
    windows: &ByAccount<Vec<(String, f64)>>,
) -> String {
    if let Some(wins) = lookup(windows, num).filter(|w| !w.is_empty()) {
        return wins
            .iter()
            .map(|(name, pct)| format!("{name} {pct:.0}%"))
            .collect::<Vec<_>>()
            .join(" · ");
    }
    if let Some(Some(h)) = lookup(headroom, num) {
        return format!("{:.0}%", 100.0 - h);
    }
    match lookup(fetch_errors, num) {
        Some(err) => format!("? ({err})"),
        None => "?".into(),
    }
}

pub(crate) fn lookup<'a, T>(map: &'a ByAccount<T>, num: &str) -> Option<&'a T> {
    map.iter().find(|(n, _)| n == num).map(|(_, v)| v)
}

fn object<T>(map: &ByAccount<T>, f: impl Fn(&T) -> Value) -> Value {
    Value::Object(map.iter().map(|(k, v)| (k.clone(), f(v))).collect())
}

fn pairs(windows: &[(String, f64)]) -> Value {
    Value::Object(windows.iter().map(|(k, v)| (k.clone(), json!(v))).collect())
}

/// An account ref's number as text (`{"number": 3}` → `"3"`).
pub(crate) fn ref_num(r: &Value) -> String {
    match r.get("number") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) | None => "None".into(),
        Some(other) => other.to_string(),
    }
}

fn ref_email(r: &Value) -> String {
    match r.get("email") {
        Some(Value::String(s)) => s.clone(),
        _ => "None".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(body: EventBody) -> Event {
        Event {
            ts: "2026-01-01T00:00:00Z".into(),
            body,
        }
    }

    #[test]
    fn poll_json_omits_empty_optional_fields() {
        let e = ev(EventBody::Poll {
            active: None,
            headroom: vec![],
            threshold: 90.0,
            fetch_errors: vec![],
            windows: vec![],
            weekly_threshold: None,
        });
        assert_eq!(
            e.to_json().to_string(),
            r#"{"schemaVersion":1,"event":"poll","ts":"2026-01-01T00:00:00Z","active":null,"headroomPct":{},"threshold":90.0}"#
        );
        assert_eq!(e.human(), "poll: no active account");
    }

    #[test]
    fn poll_human_names_binding_window_and_others() {
        let e = ev(EventBody::Poll {
            active: Some(json!({"number": 1, "email": "a@x.io"})),
            headroom: vec![("1".into(), Some(20.0)), ("2".into(), None)],
            threshold: 90.0,
            fetch_errors: vec![("2".into(), "HTTP 500".into())],
            windows: vec![("1".into(), vec![("5h".into(), 80.0), ("7d".into(), 40.0)])],
            weekly_threshold: Some(97.0),
        });
        assert_eq!(
            e.human(),
            "Account-1 (a@x.io): 5h 80% used (switch at 90%) | others: #2: ? (HTTP 500)"
        );
        let json = e.to_json();
        assert_eq!(json["windowsPct"]["1"]["5h"], json!(80.0));
        assert_eq!(json["weeklyThreshold"], json!(97.0));
    }

    #[test]
    fn switch_and_misc_human_lines() {
        let s = ev(EventBody::Switch {
            trigger: "proactive".into(),
            from: Some(json!({"number": 1, "email": "a@x.io"})),
            to: Some(json!({"number": 2, "email": "b@x.io"})),
            warnings: vec![],
            dry_run: true,
        });
        assert_eq!(
            s.human(),
            "[dry-run] would switch Account-1 -> Account-2 (b@x.io) (proactive)"
        );
        assert_eq!(s.to_json()["dryRun"], json!(true));
        let sl = ev(EventBody::Sleep {
            seconds: 599.96,
            until: "x".into(),
        });
        assert_eq!(sl.to_json()["seconds"], json!(600.0));
        assert_eq!(sl.human(), "sleeping 10m (until x)");
        assert_eq!(
            ev(EventBody::NoSwitch {
                reason: "cooldown".into(),
                detail: String::new()
            })
            .human(),
            "no switch: cooldown"
        );
        assert_eq!(
            ev(EventBody::Error {
                message: "boom".into(),
                transient: true
            })
            .human(),
            "error: boom (will retry)"
        );
    }
}

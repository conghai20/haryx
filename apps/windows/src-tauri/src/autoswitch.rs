//! Auto-switch as a long-running `haryx auto --json` child: one JSON event per line. The child gets
//! `HARYX_EXIT_WITH_PARENT=1`, so it stops on its own if the app dies without stopping it.

use std::io::{BufRead, BufReader};
use std::process::{Child, Stdio};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use serde_json::Value;

use crate::cli::Cli;

/// How many events the log keeps.
pub const LOG_CAPACITY: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info,
    Notable,
    Problem,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoEvent {
    pub kind: String,
    /// The CLI's timestamp when it sent one.
    pub ts: Option<String>,
    pub summary: String,
    pub severity: Severity,
    /// Title for a user notification, when the event deserves one.
    pub notification_title: Option<String>,
    /// For a switch: the account it left (sessions still running there may need continuing).
    pub from_account: Option<i64>,
}

impl AutoEvent {
    pub fn local(kind: &str, summary: &str, severity: Severity) -> Self {
        AutoEvent {
            kind: kind.into(),
            ts: None,
            summary: summary.into(),
            severity,
            notification_title: None,
            from_account: None,
        }
    }
}

fn account_ref(obj: &Value, key: &str) -> String {
    let Some(r) = obj.get(key).filter(|v| v.is_object()) else {
        return "(none)".into();
    };
    let num = match r.get("number") {
        Some(Value::String(s)) => s.clone(),
        Some(v) if !v.is_null() => v.to_string(),
        _ => "?".into(),
    };
    let email = r
        .get("email")
        .and_then(Value::as_str)
        .map(|e| format!(" ({e})"))
        .unwrap_or_default();
    format!("Account-{num}{email}")
}

/// One stdout line as an event; `None` for non-JSON lines and for noise (`sleep`). Same wording as the
/// macOS app (`AutoEvent.parse`).
pub fn parse_line(line: &str) -> Option<AutoEvent> {
    let obj: Value = serde_json::from_str(line.trim()).ok()?;
    let kind = obj.get("event")?.as_str()?.to_string();
    let s = |k: &str| obj.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let number = || match obj.get("number") {
        Some(Value::String(n)) => n.clone(),
        Some(v) if !v.is_null() => v.to_string(),
        _ => "?".into(),
    };
    let (summary, severity, title) = match kind.as_str() {
        "sleep" => return None,
        "poll" => {
            let headroom = obj
                .get("headroom")
                .and_then(Value::as_object)
                .map_or(0, |m| m.len());
            (
                format!(
                    "Checked {headroom} account(s); active {}",
                    account_ref(&obj, "active")
                ),
                Severity::Info,
                None,
            )
        }
        "switch" => {
            let dry = if obj.get("dryRun").and_then(Value::as_bool) == Some(true) {
                " [dry run]"
            } else {
                ""
            };
            (
                format!(
                    "Switched {} → {} ({}){dry}",
                    account_ref(&obj, "from"),
                    account_ref(&obj, "to"),
                    s("trigger")
                ),
                Severity::Notable,
                None,
            )
        }
        "no-switch" => {
            let detail = s("detail");
            let detail = if detail.is_empty() {
                String::new()
            } else {
                format!(" — {detail}")
            };
            (
                format!("No switch: {}{detail}", s("reason")),
                Severity::Info,
                None,
            )
        }
        "account-quarantined" => (
            format!(
                "Account-{} ({}) quarantined: {}",
                number(),
                s("email"),
                s("reason")
            ),
            Severity::Problem,
            Some("Account quarantined"),
        ),
        "account-unquarantined" => (
            format!("Account-{} ({}) back in rotation", number(), s("email")),
            Severity::Notable,
            Some("Account back in rotation"),
        ),
        "all-exhausted" => {
            let reset = s("earliestResetAt");
            let reset = if reset.is_empty() {
                String::new()
            } else {
                format!("; earliest reset {reset}")
            };
            (
                format!("All accounts exhausted{reset}"),
                Severity::Problem,
                Some("All Claude accounts exhausted"),
            )
        }
        "error" => {
            let transient = obj.get("transient").and_then(Value::as_bool) == Some(true);
            (
                format!(
                    "Error: {}{}",
                    s("message"),
                    if transient { " (will retry)" } else { "" }
                ),
                Severity::Problem,
                (!transient).then_some("Auto-switch error"),
            )
        }
        _ => (kind.clone(), Severity::Info, None),
    };
    Some(AutoEvent {
        kind,
        ts: obj.get("ts").and_then(Value::as_str).map(str::to_owned),
        summary,
        severity,
        notification_title: title.map(str::to_owned),
        from_account: obj.get("from").and_then(|f| f.get("number")).and_then(|n| {
            n.as_i64()
                .or_else(|| n.as_str().and_then(|s| s.parse().ok()))
        }),
    })
}

/// A bounded event log, newest last.
#[derive(Debug, Default)]
pub struct EventLog {
    events: std::collections::VecDeque<AutoEvent>,
}

impl EventLog {
    pub fn push(&mut self, event: AutoEvent) {
        if self.events.len() == LOG_CAPACITY {
            self.events.pop_front();
        }
        self.events.push_back(event);
    }

    pub fn snapshot(&self) -> Vec<AutoEvent> {
        self.events.iter().cloned().collect()
    }
}

/// The running child, if any, and its log.
#[derive(Default)]
pub struct Runner {
    child: Mutex<Option<Child>>,
    pub log: Arc<Mutex<EventLog>>,
}

impl Runner {
    pub fn is_running(&self) -> bool {
        let mut guard = self.child.lock().unwrap_or_else(|e| e.into_inner());
        match guard.as_mut().map(Child::try_wait) {
            Some(Ok(None)) => true,
            Some(_) => {
                *guard = None;
                false
            }
            None => false,
        }
    }

    /// Start `haryx auto --json`; `on_event` sees every parsed event (from a reader thread).
    pub fn start(
        &self,
        cli: &Cli,
        dry_run: bool,
        on_event: impl Fn(&AutoEvent) + Send + 'static,
    ) -> Result<(), String> {
        if self.is_running() {
            return Ok(());
        }
        let mut args = vec!["auto", "--json"];
        if dry_run {
            args.push("--dry-run");
        }
        let mut child = cli
            .command(&args)
            .env("HARYX_EXIT_WITH_PARENT", "1")
            .env("NO_COLOR", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("could not start auto-switch: {e}"))?;
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        *self.child.lock().unwrap_or_else(|e| e.into_inner()) = Some(child);
        let started = AutoEvent::local(
            "started",
            if dry_run {
                "Auto-switch started (dry run)"
            } else {
                "Auto-switch started"
            },
            Severity::Notable,
        );
        self.record(started, &on_event);
        let last_error: Arc<Mutex<Option<String>>> = Arc::default();
        let stderr_thread = stderr.map(|pipe| {
            let last_error = Arc::clone(&last_error);
            std::thread::spawn(move || {
                for line in BufReader::new(pipe).lines().map_while(Result::ok) {
                    if !line.trim().is_empty() {
                        *last_error.lock().unwrap_or_else(|e| e.into_inner()) =
                            Some(crate::cli::strip_ansi(line.trim()));
                    }
                }
            })
        });
        let log = Arc::clone(&self.log);
        std::thread::spawn(move || {
            if let Some(out) = stdout {
                for line in BufReader::new(out).lines().map_while(Result::ok) {
                    if let Some(event) = parse_line(&line) {
                        log.lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .push(event.clone());
                        on_event(&event);
                    }
                }
            }
            if let Some(handle) = stderr_thread {
                let _ = handle.join();
            }
            // The CLI's last words, when it had any, say why it stopped.
            let stopped = match last_error.lock().unwrap_or_else(|e| e.into_inner()).take() {
                Some(message) => AutoEvent {
                    notification_title: Some("Auto-switch stopped".into()),
                    ..AutoEvent::local("error", &message, Severity::Problem)
                },
                None => AutoEvent::local("stopped", "Auto-switch stopped", Severity::Notable),
            };
            log.lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(stopped.clone());
            on_event(&stopped);
        });
        Ok(())
    }

    fn record(&self, event: AutoEvent, on_event: &impl Fn(&AutoEvent)) {
        self.log
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(event.clone());
        on_event(&event);
    }

    pub fn stop(&self) {
        if let Some(mut child) = self.child.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Drop for Runner {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_event_kinds_like_the_macos_app() {
        let switch = parse_line(
            r#"{"event":"switch","ts":"2026-11-24T01:00:00Z","from":{"number":1,"email":"a@x.com"},
                "to":{"number":"2","email":"b@x.com"},"trigger":"threshold","dryRun":true}"#,
        )
        .unwrap();
        assert_eq!(
            switch.summary,
            "Switched Account-1 (a@x.com) → Account-2 (b@x.com) (threshold) [dry run]"
        );
        assert_eq!(switch.ts.as_deref(), Some("2026-11-24T01:00:00Z"));
        assert_eq!(switch.from_account, Some(1));
        let poll = parse_line(r#"{"event":"poll","headroom":{"1":10,"2":20}}"#).unwrap();
        assert_eq!(poll.summary, "Checked 2 account(s); active (none)");
        let error = parse_line(r#"{"event":"error","message":"boom","transient":false}"#).unwrap();
        assert_eq!(error.severity, Severity::Problem);
        assert_eq!(
            error.notification_title.as_deref(),
            Some("Auto-switch error")
        );
        let retry = parse_line(r#"{"event":"error","message":"net","transient":true}"#).unwrap();
        assert_eq!(retry.summary, "Error: net (will retry)");
        assert_eq!(retry.notification_title, None);
        let quarantined = parse_line(
            r#"{"event":"account-quarantined","number":3,"email":"c@x.com","reason":"401"}"#,
        )
        .unwrap();
        assert_eq!(quarantined.summary, "Account-3 (c@x.com) quarantined: 401");
    }

    #[test]
    fn ignores_noise_and_non_json() {
        assert_eq!(parse_line(r#"{"event":"sleep","seconds":60}"#), None);
        assert_eq!(parse_line("Starting auto-switch…"), None);
        assert_eq!(parse_line(r#"{"no":"event"}"#), None);
    }

    #[test]
    fn log_keeps_the_newest_events() {
        let mut log = EventLog::default();
        for i in 0..(LOG_CAPACITY + 5) {
            log.push(AutoEvent::local("x", &i.to_string(), Severity::Info));
        }
        let events = log.snapshot();
        assert_eq!(events.len(), LOG_CAPACITY);
        assert_eq!(events[0].summary, "5");
    }
}

//! HaryX sign-in: `haryx login --provider github --json` as a child that streams one JSON event per
//! line (contract: `packages/contracts/haryx/login-events.json`). The CLI opens the browser and waits
//! up to 5 minutes; Cancel kills it. Each event goes to the UI as `account-login`.

use std::io::{BufRead, BufReader};
use std::process::{Child, Stdio};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

use crate::state::AppState;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct User {
    pub id: String,
    pub email: String,
    pub provider: String,
}

/// One `login --json` line, plus `cancelled`, which the app adds when the user stops the run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum LoginEvent {
    Browser {
        url: String,
        #[serde(default)]
        opened: bool,
    },
    SignedIn {
        user: User,
    },
    Error {
        code: String,
        message: String,
    },
    Cancelled,
}

impl LoginEvent {
    /// The run is over once it sent one of these.
    pub fn is_final(&self) -> bool {
        !matches!(self, LoginEvent::Browser { .. })
    }
}

/// One stdout line as an event; `None` for anything else (a newer CLI's extra events included).
pub fn parse_line(line: &str) -> Option<LoginEvent> {
    serde_json::from_str(line.trim()).ok()
}

/// The running sign-in, if any, and the link it printed.
#[derive(Default)]
pub struct AccountLogin {
    child: Mutex<Option<Child>>,
    url: Mutex<Option<String>>,
    cancelled: Arc<Mutex<bool>>,
}

impl AccountLogin {
    /// Start the sign-in; a run already going is left alone.
    pub fn start(&self, app: &AppHandle) -> Result<(), String> {
        let mut slot = self.child.lock().unwrap_or_else(|e| e.into_inner());
        if slot
            .as_mut()
            .is_some_and(|c| matches!(c.try_wait(), Ok(None)))
        {
            return Ok(());
        }
        if let Some(mut done) = slot.take() {
            let _ = done.wait(); // the last run, already exited
        }
        let cli = app.state::<Arc<AppState>>().cli()?;
        let mut child = cli
            .command(&["login", "--provider", "github", "--json"])
            .env("NO_COLOR", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("could not start the sign-in: {e}"))?;
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        *slot = Some(child);
        *self.url.lock().unwrap_or_else(|e| e.into_inner()) = None;
        *self.cancelled.lock().unwrap_or_else(|e| e.into_inner()) = false;
        let cancelled = Arc::clone(&self.cancelled);
        let app = app.clone();
        std::thread::spawn(move || {
            let stderr = stderr.map(|pipe| {
                std::thread::spawn(move || {
                    BufReader::new(pipe)
                        .lines()
                        .map_while(Result::ok)
                        .filter(|l| !l.trim().is_empty())
                        .last()
                })
            });
            let mut finished = false;
            for event in stdout
                .into_iter()
                .flat_map(|out| BufReader::new(out).lines().map_while(Result::ok))
                .filter_map(|line| parse_line(&line))
            {
                if let LoginEvent::Browser { url, .. } = &event {
                    *app.state::<AccountLogin>()
                        .url
                        .lock()
                        .unwrap_or_else(|e| e.into_inner()) = Some(url.clone());
                }
                finished |= event.is_final();
                let _ = app.emit("account-login", &event);
            }
            let last_words = stderr.and_then(|h| h.join().ok().flatten());
            if !finished {
                let event = if *cancelled.lock().unwrap_or_else(|e| e.into_inner()) {
                    LoginEvent::Cancelled
                } else {
                    LoginEvent::Error {
                        code: "local".into(),
                        message: last_words
                            .map(|l| crate::cli::strip_ansi(l.trim()))
                            .unwrap_or_else(|| "The sign-in stopped without an answer.".into()),
                    }
                };
                let _ = app.emit("account-login", &event);
            }
        });
        Ok(())
    }

    pub fn cancel(&self) {
        *self.cancelled.lock().unwrap_or_else(|e| e.into_inner()) = true;
        if let Some(child) = self
            .child
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_mut()
        {
            let _ = child.kill();
        }
    }

    /// The sign-in link the run printed, when it is one to open.
    pub fn url(&self) -> Option<String> {
        self.url
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .filter(|u| u.starts_with("https://"))
    }
}

impl Drop for AccountLogin {
    fn drop(&mut self) {
        self.cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EVENTS: &str = include_str!("../../../../packages/contracts/haryx/login-events.json");

    #[test]
    fn decodes_every_contract_event_and_round_trips() {
        let lines: Vec<serde_json::Value> = serde_json::from_str(EVENTS).unwrap();
        let events: Vec<LoginEvent> = lines
            .iter()
            .map(|v| parse_line(&v.to_string()).expect("decodes"))
            .collect();
        assert!(
            matches!(&events[0], LoginEvent::Browser { opened: true, url } if url.starts_with("https://"))
        );
        assert!(
            matches!(&events[1], LoginEvent::SignedIn { user } if user.email == "you@example.com" && user.provider == "github")
        );
        assert!(matches!(&events[2], LoginEvent::Error { code, .. } if code == "timeout"));
        assert!(!events[0].is_final() && events[1].is_final() && events[2].is_final());
        for (event, line) in events.iter().zip(&lines) {
            assert_eq!(&serde_json::to_value(event).unwrap(), line);
        }
    }

    #[test]
    fn ignores_other_lines() {
        assert_eq!(parse_line("Opening your browser…"), None);
        assert_eq!(parse_line(r#"{"event":"progress"}"#), None);
        assert_eq!(
            parse_line(r#"{"event":"browser","url":"https://x"}"#),
            Some(LoginEvent::Browser {
                url: "https://x".into(),
                opened: false
            })
        );
        assert_eq!(
            serde_json::to_value(LoginEvent::Cancelled).unwrap(),
            serde_json::json!({"event": "cancelled"})
        );
    }
}

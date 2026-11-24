//! Every live Claude Code session on this machine, by account: `haryx ai sessions`.
//!
//! Built on Claude Code's own records (`<profile>/sessions/{pid}.json`, see [`crate::process`]), so it
//! works the same on every OS. A session in the default profile (`~/.claude`) belongs to the active
//! account; a session in a slot's session profile belongs to that slot.

use std::path::Path;

use serde_json::{Map, Value, json};

use crate::process::ClaudeSession;
use crate::session;

/// A live session and the account it runs as.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunningSession {
    /// The slot number, or `None` for a default-profile session when no account is active.
    pub account: Option<i64>,
    pub email: String,
    /// `"default"` (`~/.claude`) or `"session"` (a slot's own profile, `haryx run`).
    pub profile: &'static str,
    pub session: ClaudeSession,
}

/// Sessions found, and how many records could not be read (not knowing is not "nothing runs").
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Scan {
    pub sessions: Vec<RunningSession>,
    pub unreadable: usize,
}

/// Scan the default profile and every slot's session profile.
pub fn scan(store_root: &Path, default_profile: &Path, roster: &Map<String, Value>) -> Scan {
    scan_with(
        store_root,
        default_profile,
        roster,
        &session::scan_live_sessions,
    )
}

type Scanner = dyn Fn(&Path) -> (Vec<ClaudeSession>, usize);

fn scan_with(
    store_root: &Path,
    default_profile: &Path,
    roster: &Map<String, Value>,
    scanner: &Scanner,
) -> Scan {
    let empty = Map::new();
    let accounts = roster
        .get("accounts")
        .and_then(Value::as_object)
        .unwrap_or(&empty);
    let email_of = |number: i64| {
        accounts
            .get(&number.to_string())
            .and_then(|a| a.get("email"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned()
    };
    let active = roster.get("activeAccountNumber").and_then(Value::as_i64);
    let mut out = Scan::default();
    let (found, unreadable) = scanner(default_profile);
    out.unreadable += unreadable;
    out.sessions
        .extend(found.into_iter().map(|session| RunningSession {
            account: active,
            email: active.map(email_of).unwrap_or_default(),
            profile: "default",
            session,
        }));
    let numbers = roster
        .get("sequence")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_i64);
    for number in numbers {
        let email = email_of(number);
        if email.is_empty() {
            continue;
        }
        let dir = session::session_dir_for(store_root, &number.to_string(), &email);
        let (found, unreadable) = scanner(&dir);
        out.unreadable += unreadable;
        out.sessions
            .extend(found.into_iter().map(|session| RunningSession {
                account: Some(number),
                email: email.clone(),
                profile: "session",
                session,
            }));
    }
    out.sessions
        .sort_by_key(|s| (s.session.started_at, s.session.pid));
    out
}

/// The `--json` payload (contract: `packages/contracts/haryx/sessions.json`).
pub fn to_json(scan: &Scan) -> Value {
    let sessions: Vec<Value> = scan
        .sessions
        .iter()
        .map(|s| {
            json!({
                "pid": s.session.pid,
                "account": s.account,
                "email": s.email,
                "profile": s.profile,
                "sessionId": s.session.session_id,
                "cwd": s.session.cwd,
                "startedAt": s.session.started_at,
                "kind": s.session.kind,
                "entrypoint": s.session.entrypoint,
                "status": s.session.status,
            })
        })
        .collect();
    json!({ "sessions": sessions, "unreadable": scan.unreadable })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn claude(pid: i32, started_at: i64) -> ClaudeSession {
        ClaudeSession {
            pid,
            session_id: format!("s{pid}"),
            cwd: "/work".into(),
            started_at,
            kind: "interactive".into(),
            entrypoint: "cli".into(),
            status: Some("busy".into()),
        }
    }

    fn roster() -> Map<String, Value> {
        json!({
            "activeAccountNumber": 1,
            "sequence": [1, 2, 3],
            "accounts": {
                "1": {"email": "a@x.com"},
                "2": {"email": "b@x.com"},
                "3": {"email": ""}
            }
        })
        .as_object()
        .unwrap()
        .clone()
    }

    #[test]
    fn attributes_default_profile_to_active_and_session_profiles_to_their_slot() {
        let store = PathBuf::from("/store");
        let b_dir = session::session_dir_for(&store, "2", "b@x.com");
        let scanner = move |dir: &Path| -> (Vec<ClaudeSession>, usize) {
            if dir == Path::new("/home/.claude") {
                (vec![claude(20, 200)], 0)
            } else if dir == b_dir {
                (vec![claude(10, 100)], 1)
            } else {
                (vec![], 0)
            }
        };
        let scan = scan_with(&store, Path::new("/home/.claude"), &roster(), &scanner);
        assert_eq!(scan.unreadable, 1);
        let summary: Vec<(i32, Option<i64>, &str, &str)> = scan
            .sessions
            .iter()
            .map(|s| (s.session.pid, s.account, s.email.as_str(), s.profile))
            .collect();
        assert_eq!(
            summary,
            vec![
                (10, Some(2), "b@x.com", "session"),
                (20, Some(1), "a@x.com", "default"),
            ],
            "oldest first; slot 3 without an email is skipped"
        );
    }

    #[test]
    fn no_active_account_leaves_default_sessions_unowned() {
        let mut roster = roster();
        roster.remove("activeAccountNumber");
        let scanner = |dir: &Path| -> (Vec<ClaudeSession>, usize) {
            if dir == Path::new("/d") {
                (vec![claude(1, 1)], 0)
            } else {
                (vec![], 0)
            }
        };
        let scan = scan_with(Path::new("/s"), Path::new("/d"), &roster, &scanner);
        assert_eq!(scan.sessions[0].account, None);
        assert_eq!(scan.sessions[0].email, "");
    }

    #[test]
    fn json_matches_the_contract_fixture() {
        let fixture: Value = serde_json::from_str(include_str!(
            "../../../../packages/contracts/haryx/sessions.json"
        ))
        .unwrap();
        let scan = Scan {
            sessions: vec![RunningSession {
                account: Some(2),
                email: "b@x.com".into(),
                profile: "session",
                session: claude(10, 100),
            }],
            unreadable: 0,
        };
        let ours = to_json(&scan);
        let keys = |v: &Value| -> Vec<String> {
            let mut k: Vec<String> = v.as_object().unwrap().keys().cloned().collect();
            k.sort();
            k
        };
        assert_eq!(keys(&ours), keys(&fixture));
        assert_eq!(
            keys(&ours["sessions"][0]),
            keys(&fixture["sessions"][0]),
            "row fields drift from packages/contracts/haryx/sessions.json"
        );
    }
}

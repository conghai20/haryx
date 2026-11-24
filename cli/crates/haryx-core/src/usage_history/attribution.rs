//! Which account a reply ran as. Transcripts do not say, so in order:
//! 1. the session ledger (`usage/sessions.jsonl`, written by `ai sessions` scans and the apps): exact.
//!    A session continued on another account keeps its id, so the ledger holds one entry per
//!    (session, account) change, and a reply takes the latest entry seen at or before it;
//! 2. the profile folder the transcript was found in (a `haryx run` profile): exact;
//! 3. the account active at that time, from `haryx.log`'s "Switched from account X to Y": approximate;
//! 4. unknown.

use std::collections::HashMap;
use std::io::Write;
use std::path::Path;

use serde_json::{Value, json};

use super::{LEDGER, history_dir};

/// The account and whether it was inferred from the switch log.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Attributed {
    pub account: Option<i64>,
    pub approximate: bool,
}

/// A session's accounts over time: (first seen, account), oldest first.
type Ledger = HashMap<String, Vec<(i64, i64)>>;

#[derive(Default, Debug)]
pub struct Attribution {
    ledger: Ledger,
    /// (unix seconds, account switched to), sorted; plus the account before the first switch.
    switches: Vec<(i64, i64)>,
    first: Option<i64>,
}

impl Attribution {
    pub fn load(store_root: &Path) -> Self {
        let ledger = read_ledger(store_root);
        let mut log = String::new();
        // haryx.log, and an older rotated copy when there is one.
        for name in ["haryx.log.1", "haryx.log"] {
            if let Ok(text) = std::fs::read_to_string(store_root.join(name)) {
                log.push_str(&text);
                log.push('\n');
            }
        }
        let (switches, first) = parse_switches(&log, &jiff::tz::TimeZone::system());
        Attribution {
            ledger,
            switches,
            first,
        }
    }

    /// Who ran `session` at `at`; `profile_account` is the account of a `haryx run` profile the
    /// transcript lives in.
    pub fn account(&self, session: &str, at: i64, profile_account: Option<i64>) -> Attributed {
        if let Some(entries) = self.ledger.get(session).filter(|e| !e.is_empty()) {
            // The latest account seen at or before the reply; a reply older than the first
            // sighting belongs to the first account.
            let idx = entries.partition_point(|(seen, _)| *seen <= at);
            let (_, n) = entries[idx.saturating_sub(1)];
            return Attributed {
                account: Some(n),
                approximate: false,
            };
        }
        if profile_account.is_some() {
            return Attributed {
                account: profile_account,
                approximate: false,
            };
        }
        let idx = self.switches.partition_point(|(t, _)| *t <= at);
        let account = if idx == 0 {
            self.first
        } else {
            Some(self.switches[idx - 1].1)
        };
        Attributed {
            account,
            approximate: account.is_some(),
        }
    }
}

/// "YYYY-MM-DD HH:MM:SS,mmm - INFO - Switched from account X to Y" lines (local time).
pub fn parse_switches(log: &str, tz: &jiff::tz::TimeZone) -> (Vec<(i64, i64)>, Option<i64>) {
    let mut out = Vec::new();
    let mut first = None;
    for line in log.lines() {
        let Some(rest) = line.split(" - ").nth(2) else {
            continue;
        };
        let Some(pair) = rest.strip_prefix("Switched from account ") else {
            continue;
        };
        let Some((from, to)) = pair.split_once(" to ") else {
            continue;
        };
        let Ok(to) = to.trim().parse::<i64>() else {
            continue;
        };
        let Some(stamp) = line.get(..19) else {
            continue;
        };
        let Ok(dt) = jiff::civil::DateTime::strptime("%Y-%m-%d %H:%M:%S", stamp) else {
            continue;
        };
        let Ok(at) = dt.to_zoned(tz.clone()) else {
            continue;
        };
        if out.is_empty() {
            first = from.trim().parse::<i64>().ok();
        }
        out.push((at.timestamp().as_second(), to));
    }
    out.sort_by_key(|(t, _)| *t);
    (out, first)
}

fn read_ledger(store_root: &Path) -> Ledger {
    let Ok(text) = std::fs::read_to_string(history_dir(store_root).join(LEDGER)) else {
        return Ledger::new();
    };
    let mut ledger = Ledger::new();
    for v in text
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
    {
        let (Some(id), Some(n)) = (
            v.get("sessionId").and_then(Value::as_str),
            v.get("account").and_then(Value::as_i64),
        ) else {
            continue;
        };
        let seen = v.get("firstSeen").and_then(Value::as_i64).unwrap_or(0);
        ledger.entry(id.to_owned()).or_default().push((seen, n));
    }
    for entries in ledger.values_mut() {
        entries.sort_by_key(|(seen, _)| *seen);
    }
    ledger
}

/// Record each live session's account when the ledger has none for it yet, or its latest entry
/// names another account (the session was continued elsewhere). Best effort: a failure only
/// loses exactness.
pub fn record_sessions(store_root: &Path, live: &[(String, i64, &str)]) {
    if live.is_empty() || !crate::settings::load_history_settings(store_root).enabled {
        return;
    }
    let known = read_ledger(store_root);
    let fresh: Vec<&(String, i64, &str)> = live
        .iter()
        .filter(|(id, account, _)| {
            !id.is_empty()
                && known
                    .get(id)
                    .and_then(|e| e.last())
                    .is_none_or(|(_, n)| n != account)
        })
        .collect();
    if fresh.is_empty() {
        return;
    }
    let dir = history_dir(store_root);
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join(LEDGER))
    else {
        return;
    };
    let now = jiff::Timestamp::now().as_second();
    for (id, account, profile) in fresh {
        let row =
            json!({"sessionId": id, "account": account, "profile": profile, "firstSeen": now});
        let _ = writeln!(file, "{row}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOG: &str = "\
2026-11-24 10:00:00,100 - INFO - Backed up account 1
2026-11-24 10:00:01,617 - INFO - Switched from account 1 to 2
2026-11-24 12:30:00,000 - INFO - Switched from account 2 to 3
junk line
2026-11-24 13:00:00,000 - INFO - Switched from account 3 to None";

    #[test]
    fn reads_switches_in_local_time() {
        let tz = jiff::tz::TimeZone::fixed(jiff::tz::offset(7));
        let (switches, first) = parse_switches(LOG, &tz);
        assert_eq!(first, Some(1));
        assert_eq!(switches.len(), 2, "a switch to no account is skipped");
        // 10:00:01 at UTC+7 is 03:00:01 UTC.
        let utc = jiff::Timestamp::from_second(switches[0].0)
            .unwrap()
            .to_string();
        assert_eq!(utc, "2026-11-24T03:00:01Z");
        assert_eq!(switches[1].1, 3);
    }

    #[test]
    fn ledger_then_profile_then_log() {
        let tz = jiff::tz::TimeZone::fixed(jiff::tz::offset(0));
        let (switches, first) = parse_switches(LOG, &tz);
        let a = Attribution {
            ledger: HashMap::from([("known".to_owned(), vec![(0, 7)])]),
            switches,
            first,
        };
        let at = |h: i8, m: i8| {
            jiff::civil::date(2026, 11, 24)
                .at(h, m, 0, 0)
                .to_zoned(tz.clone())
                .unwrap()
                .timestamp()
                .as_second()
        };
        assert_eq!(
            a.account("known", at(11, 0), Some(5)),
            Attributed {
                account: Some(7),
                approximate: false
            }
        );
        assert_eq!(
            a.account("x", at(11, 0), Some(5)),
            Attributed {
                account: Some(5),
                approximate: false
            }
        );
        assert_eq!(
            a.account("x", at(9, 0), None),
            Attributed {
                account: Some(1),
                approximate: true
            }
        );
        assert_eq!(
            a.account("x", at(11, 0), None),
            Attributed {
                account: Some(2),
                approximate: true
            }
        );
        assert_eq!(
            a.account("x", at(12, 45), None),
            Attributed {
                account: Some(3),
                approximate: true
            }
        );
        let empty = Attribution::default();
        assert_eq!(
            empty.account("x", 0, None),
            Attributed {
                account: None,
                approximate: false
            }
        );
    }

    #[test]
    fn the_ledger_follows_a_session_across_accounts() {
        let tmp = tempfile::TempDir::new().unwrap();
        record_sessions(tmp.path(), &[("s1".into(), 1, "default")]);
        record_sessions(tmp.path(), &[("s1".into(), 1, "default")]);
        record_sessions(tmp.path(), &[("s2".into(), 3, "session")]);
        let ledger = read_ledger(tmp.path());
        assert_eq!(
            ledger["s1"].len(),
            1,
            "an unchanged account is not recorded again"
        );
        assert_eq!(ledger["s2"][0].1, 3);

        // Continued from account 1 to 2 at t=100: replies before stay on 1, after go to 2.
        let a = Attribution {
            ledger: HashMap::from([("s".to_owned(), vec![(50, 1), (100, 2)])]),
            ..Attribution::default()
        };
        assert_eq!(
            a.account("s", 10, None).account,
            Some(1),
            "before the first sighting"
        );
        assert_eq!(a.account("s", 99, None).account, Some(1));
        assert_eq!(a.account("s", 100, None).account, Some(2));
        assert_eq!(
            a.account("s", 500, Some(9)).account,
            Some(2),
            "the ledger is exact"
        );
    }
}

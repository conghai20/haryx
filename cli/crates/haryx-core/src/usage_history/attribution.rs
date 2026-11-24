//! Which account a reply ran as. Transcripts do not say, so in order:
//! 1. the session ledger (`usage/sessions.jsonl`, written by `ai sessions` scans and the apps): exact;
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

#[derive(Default, Debug)]
pub struct Attribution {
    ledger: HashMap<String, i64>,
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
        if let Some(n) = self.ledger.get(session) {
            return Attributed {
                account: Some(*n),
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

fn read_ledger(store_root: &Path) -> HashMap<String, i64> {
    let Ok(text) = std::fs::read_to_string(history_dir(store_root).join(LEDGER)) else {
        return HashMap::new();
    };
    text.lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter_map(|v| {
            let id = v.get("sessionId")?.as_str()?.to_owned();
            let n = v.get("account")?.as_i64()?;
            Some((id, n))
        })
        .collect()
}

/// Record the account of each live session not in the ledger yet (first sighting wins, which is
/// the account it started on). Best effort: a failure only loses exactness.
pub fn record_sessions(store_root: &Path, live: &[(String, i64, &str)]) {
    if live.is_empty() || !crate::settings::load_history_settings(store_root).enabled {
        return;
    }
    let known = read_ledger(store_root);
    let fresh: Vec<&(String, i64, &str)> = live
        .iter()
        .filter(|(id, _, _)| !id.is_empty() && !known.contains_key(id))
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
            ledger: HashMap::from([("known".to_owned(), 7)]),
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
    fn the_ledger_keeps_the_first_sighting() {
        let tmp = tempfile::TempDir::new().unwrap();
        record_sessions(tmp.path(), &[("s1".into(), 1, "default")]);
        record_sessions(
            tmp.path(),
            &[("s1".into(), 2, "default"), ("s2".into(), 3, "session")],
        );
        let ledger = read_ledger(tmp.path());
        assert_eq!(ledger.get("s1"), Some(&1));
        assert_eq!(ledger.get("s2"), Some(&3));
    }
}

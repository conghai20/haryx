//! Reports over the day files: totals grouped by account, project, session, model or day, the
//! quota readings, and the weekly summary.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use serde_json::{Value, json};

use super::{DAYS, Tokens, history_dir, pricing, quota};

/// What to group by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum By {
    Account,
    Project,
    Session,
    Model,
    Day,
}

impl By {
    pub fn parse(s: &str) -> Option<By> {
        Some(match s {
            "account" => By::Account,
            "project" => By::Project,
            "session" => By::Session,
            "model" => By::Model,
            "day" => By::Day,
            _ => return None,
        })
    }

    fn name(self) -> &'static str {
        match self {
            By::Account => "account",
            By::Project => "project",
            By::Session => "session",
            By::Model => "model",
            By::Day => "day",
        }
    }
}

/// A report's filter.
#[derive(Clone, Debug)]
pub struct Query {
    /// Unix seconds.
    pub since: i64,
    pub by: By,
    pub account: Option<i64>,
    pub project: Option<String>,
}

/// One stored row.
#[derive(Clone, Debug)]
pub struct Row {
    pub hour: i64,
    pub account: Option<i64>,
    pub approximate: bool,
    pub project: String,
    pub session: String,
    pub model: String,
    pub replies: u64,
    pub tokens: Tokens,
}

/// Rows at or after `since`.
pub fn rows(store_root: &Path, since: i64) -> Vec<Row> {
    let since_day = jiff::Timestamp::from_second(since - 86_400)
        .unwrap_or_default()
        .strftime("%Y-%m-%d")
        .to_string();
    let Ok(entries) = std::fs::read_dir(history_dir(store_root).join(DAYS)) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name
            .strip_suffix(".jsonl")
            .is_none_or(|d| d < since_day.as_str())
        {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(entry.path()) else {
            continue;
        };
        out.extend(
            text.lines()
                .filter_map(parse_row)
                .filter(|r| r.hour >= since),
        );
    }
    out
}

fn parse_row(line: &str) -> Option<Row> {
    let v: Value = serde_json::from_str(line).ok()?;
    let n = |k: &str| v.get(k).and_then(Value::as_u64).unwrap_or(0);
    let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
    Some(Row {
        hour: v.get("hour")?.as_i64()?,
        account: v.get("account").and_then(Value::as_i64),
        approximate: v
            .get("approximate")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        project: s("project"),
        session: s("session"),
        model: s("model"),
        replies: n("replies"),
        tokens: Tokens {
            input: n("input"),
            output: n("output"),
            cache_read: n("cacheRead"),
            cache_write_5m: n("cacheWrite5m"),
            cache_write_1h: n("cacheWrite1h"),
        },
    })
}

#[derive(Default)]
struct Group {
    replies: u64,
    tokens: Tokens,
    cost: f64,
    priced: bool,
    unpriced_tokens: u64,
    approximate: bool,
    last: i64,
    accounts: Vec<i64>,
}

fn local_day(hour: i64, tz: &jiff::tz::TimeZone) -> String {
    jiff::Timestamp::from_second(hour)
        .unwrap_or_default()
        .to_zoned(tz.clone())
        .strftime("%Y-%m-%d")
        .to_string()
}

/// Totals grouped by `q.by`, biggest first, as the `ai usage report --json` payload.
pub fn report(rows: &[Row], q: &Query) -> Value {
    let tz = jiff::tz::TimeZone::system();
    let mut groups: BTreeMap<String, Group> = BTreeMap::new();
    let mut total = Group::default();
    for r in rows {
        if q.account.is_some() && r.account != q.account {
            continue;
        }
        if q.project
            .as_ref()
            .is_some_and(|p| !r.project.starts_with(p.as_str()))
        {
            continue;
        }
        let key = match q.by {
            By::Account => r.account.map_or("unknown".to_owned(), |n| n.to_string()),
            By::Project => r.project.clone(),
            By::Session => r.session.clone(),
            By::Model => r.model.clone(),
            By::Day => local_day(r.hour, &tz),
        };
        for g in [groups.entry(key).or_default(), &mut total] {
            g.replies += r.replies;
            g.tokens.add(&r.tokens);
            match pricing::cost(&r.model, &r.tokens) {
                Some(c) => {
                    g.cost += c;
                    g.priced = true;
                }
                None => g.unpriced_tokens += r.tokens.total(),
            }
            g.approximate |= r.approximate;
            g.last = g.last.max(r.hour);
            if let Some(a) = r.account
                && !g.accounts.contains(&a)
            {
                g.accounts.push(a);
            }
        }
    }
    let grand = total.tokens.total().max(1) as f64;
    let mut list: Vec<(String, Group)> = groups.into_iter().collect();
    if q.by == By::Day {
        list.sort_by(|a, b| a.0.cmp(&b.0));
    } else {
        list.sort_by_key(|g| std::cmp::Reverse(g.1.tokens.total()));
    }
    let row = |key: Option<&str>, g: &Group| {
        let mut accounts = g.accounts.clone();
        accounts.sort_unstable();
        json!({
            "key": key,
            "replies": g.replies,
            "input": g.tokens.input,
            "output": g.tokens.output,
            "cacheRead": g.tokens.cache_read,
            "cacheWrite": g.tokens.cache_write_5m + g.tokens.cache_write_1h,
            "tokens": g.tokens.total(),
            "cost": g.priced.then(|| (g.cost * 100.0).round() / 100.0),
            "unpricedTokens": g.unpriced_tokens,
            "share": (g.tokens.total() as f64 / grand * 1000.0).round() / 10.0,
            "approximate": g.approximate,
            "lastActive": (g.last > 0).then_some(g.last),
            "accounts": accounts,
        })
    };
    json!({
        "by": q.by.name(),
        "since": q.since,
        "pricesAsOf": pricing::as_of(),
        "rows": list.iter().map(|(k, g)| row(Some(k), g)).collect::<Vec<_>>(),
        "total": row(None, &total),
    })
}

/// The quota readings since `since`, for one account or all, as `ai usage quota --json`.
pub fn quota_report(store_root: &Path, since: i64, account: Option<i64>) -> Value {
    let readings: Vec<Value> = quota::read(&history_dir(store_root))
        .into_iter()
        .filter(|r| r.at >= since && account.is_none_or(|a| a == r.account))
        .map(|r| json!({"at": r.at, "account": r.account, "fiveHour": r.five_hour, "sevenDay": r.seven_day}))
        .collect();
    json!({"since": since, "readings": readings})
}

/// The weekly digest: per account tokens, cost, times it ran out, and its biggest project.
pub fn summary(store_root: &Path, since: i64) -> Value {
    let rows = rows(store_root, since);
    let by_account = report(
        &rows,
        &Query {
            since,
            by: By::Account,
            account: None,
            project: None,
        },
    );
    let readings = quota::read(&history_dir(store_root));
    let ran_out = quota::exhaustions(&readings, since);
    let mut top: HashMap<Option<i64>, (String, u64, u64)> = HashMap::new();
    let mut per_project: HashMap<(Option<i64>, String), u64> = HashMap::new();
    for r in &rows {
        *per_project
            .entry((r.account, r.project.clone()))
            .or_default() += r.tokens.total();
    }
    for ((account, project), tokens) in per_project {
        let e = top.entry(account).or_insert((String::new(), 0, 0));
        e.2 += tokens;
        if tokens > e.1 {
            e.0 = project;
            e.1 = tokens;
        }
    }
    let accounts: Vec<Value> = by_account["rows"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|row| {
            let account = row["key"].as_str().and_then(|k| k.parse::<i64>().ok());
            let (project, tokens, all) = top.get(&account).cloned().unwrap_or_default();
            json!({
                "account": account,
                "tokens": row["tokens"],
                "cost": row["cost"],
                "share": row["share"],
                "ranOut": account.and_then(|a| ran_out.get(&a)).copied().unwrap_or(0),
                "topProject": (!project.is_empty()).then_some(project),
                "topProjectShare": (all > 0).then(|| (tokens as f64 / all as f64 * 1000.0).round() / 10.0),
            })
        })
        .collect();
    json!({
        "since": since,
        "pricesAsOf": pricing::as_of(),
        "accounts": accounts,
        "total": by_account["total"],
    })
}

/// "7d", "24h", "30d", "all" or a date (YYYY-MM-DD, local) to unix seconds.
pub fn parse_since(text: &str, now: i64) -> Option<i64> {
    let text = text.trim();
    if text == "all" {
        return Some(0);
    }
    if let Some(days) = text.strip_suffix('d').and_then(|n| n.parse::<i64>().ok()) {
        return Some(now - days * 86_400);
    }
    if let Some(hours) = text.strip_suffix('h').and_then(|n| n.parse::<i64>().ok()) {
        return Some(now - hours * 3600);
    }
    let date = jiff::civil::Date::strptime("%Y-%m-%d", text).ok()?;
    let zoned = date.to_zoned(jiff::tz::TimeZone::system()).ok()?;
    Some(zoned.timestamp().as_second())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(
        account: Option<i64>,
        project: &str,
        model: &str,
        output: u64,
        approximate: bool,
    ) -> Row {
        Row {
            hour: 1_795_500_000,
            account,
            approximate,
            project: project.into(),
            session: format!("{project}-s"),
            model: model.into(),
            replies: 1,
            tokens: Tokens {
                output,
                ..Tokens::default()
            },
        }
    }

    fn q(by: By) -> Query {
        Query {
            since: 0,
            by,
            account: None,
            project: None,
        }
    }

    #[test]
    fn groups_biggest_first_with_cost_and_share() {
        let rows = [
            row(Some(1), "/a", "claude-opus-5-5", 1_000_000, false),
            row(Some(2), "/b", "claude-opus-5-5", 3_000_000, true),
            row(Some(2), "/b", "mystery-model", 1_000_000, false),
        ];
        let r = report(&rows, &q(By::Account));
        assert_eq!(r["rows"][0]["key"], "2");
        assert_eq!(r["rows"][0]["tokens"], 4_000_000);
        assert_eq!(r["rows"][0]["cost"], 60.0, "3M output at $20");
        assert_eq!(r["rows"][0]["unpricedTokens"], 1_000_000);
        assert_eq!(r["rows"][0]["approximate"], true);
        assert_eq!(r["rows"][0]["share"], 80.0);
        assert_eq!(r["rows"][1]["cost"], 20.0);
        assert_eq!(r["total"]["tokens"], 5_000_000);
        assert!(r["pricesAsOf"].as_str().is_some());
    }

    #[test]
    fn filters_and_unpriced_groups() {
        let rows = [
            row(Some(1), "/w/api", "claude-opus-5-5", 10, false),
            row(Some(2), "/w/web", "x", 10, false),
            row(None, "/w/api", "x", 10, false),
        ];
        let mut query = q(By::Project);
        query.account = Some(2);
        let r = report(&rows, &query);
        assert_eq!(r["rows"].as_array().unwrap().len(), 1);
        assert_eq!(r["rows"][0]["cost"], Value::Null);
        let mut query = q(By::Account);
        query.project = Some("/w/api".into());
        let r = report(&rows, &query);
        let keys: Vec<&str> = r["rows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x["key"].as_str().unwrap())
            .collect();
        assert_eq!(keys.len(), 2);
        assert!(keys.contains(&"unknown"));
    }

    #[test]
    fn parses_ranges() {
        assert_eq!(parse_since("7d", 1_000_000), Some(1_000_000 - 7 * 86_400));
        assert_eq!(parse_since("24h", 100_000), Some(100_000 - 86_400));
        assert_eq!(parse_since("all", 5), Some(0));
        assert!(parse_since("2026-11-01", 0).is_some());
        assert_eq!(parse_since("soon", 0), None);
        assert_eq!(By::parse("model"), Some(By::Model));
        assert_eq!(By::parse("x"), None);
    }
}

#[cfg(test)]
mod contract_tests {
    use super::*;

    fn keys(v: &Value) -> Vec<String> {
        let mut k: Vec<String> = v.as_object().unwrap().keys().cloned().collect();
        k.sort();
        k
    }

    fn contract(name: &str) -> Value {
        let path = format!(
            "{}/../../../packages/contracts/haryx/{name}",
            env!("CARGO_MANIFEST_DIR")
        );
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    #[test]
    fn report_and_summary_match_their_contracts() {
        let tmp = tempfile::TempDir::new().unwrap();
        let rows = [Row {
            hour: 1,
            account: Some(2),
            approximate: false,
            project: "/p".into(),
            session: "s".into(),
            model: "claude-opus-5-5".into(),
            replies: 1,
            tokens: Tokens {
                output: 5,
                ..Tokens::default()
            },
        }];
        let r = report(
            &rows,
            &Query {
                since: 0,
                by: By::Account,
                account: None,
                project: None,
            },
        );
        let c = contract("usage-report.json");
        let mut want = keys(&c);
        want.retain(|k| k != "history"); // added by the command
        assert_eq!(keys(&r), want);
        assert_eq!(keys(&r["rows"][0]), keys(&c["rows"][0]));
        assert_eq!(keys(&r["total"]), keys(&c["total"]));

        let s = summary(tmp.path(), 0);
        let c = contract("usage-summary.json");
        let mut want = keys(&c);
        want.retain(|k| k != "history");
        assert_eq!(keys(&s), want);
        let q = quota_report(tmp.path(), 0, None);
        let c = contract("usage-quota.json");
        let mut want = keys(&c);
        want.retain(|k| k != "history");
        assert_eq!(keys(&q), want);
    }

    #[test]
    fn summary_rows_match_the_contract() {
        let tmp = tempfile::TempDir::new().unwrap();
        let day = tmp.path().join("usage/days");
        std::fs::create_dir_all(&day).unwrap();
        let now = jiff::Timestamp::now();
        let line = json!({"hour": now.as_second(), "account": 2, "approximate": false, "project": "/p", "session": "s", "model": "claude-opus-5-5", "replies": 1, "input": 1, "output": 1, "cacheRead": 0, "cacheWrite5m": 0, "cacheWrite1h": 0});
        std::fs::write(
            day.join(format!("{}.jsonl", now.strftime("%Y-%m-%d"))),
            format!("{line}\n"),
        )
        .unwrap();
        let s = summary(tmp.path(), 0);
        assert_eq!(
            keys(&s["accounts"][0]),
            keys(&contract("usage-summary.json")["accounts"][0])
        );
    }
}

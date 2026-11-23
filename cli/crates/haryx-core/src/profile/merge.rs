//! Merge rules for applying a profile without losing local data.

use std::cmp::Ordering;
use std::collections::HashSet;

use serde_json::{Map, Value};

use super::secrets;

/// Merge an incoming JSON value over a local one.
///
/// Without `prune`: objects merge key by key (local-only keys stay), arrays become the union (local
/// order first), scalars take the incoming value. With `prune` the incoming value wins as a whole.
/// Either way, an incoming string that still holds an unresolved secret placeholder never replaces a
/// local value.
pub fn merge_value(local: Option<&Value>, incoming: &Value, prune: bool) -> Value {
    let Some(local) = local else {
        return incoming.clone();
    };
    match (local, incoming) {
        (_, Value::String(s)) if secrets::has_placeholder(s) && !local.is_null() => local.clone(),
        (Value::Object(l), Value::Object(i)) => Value::Object(merge_objects(l, i, prune)),
        (Value::Array(l), Value::Array(i)) if !prune => Value::Array(union(l, i)),
        _ => incoming.clone(),
    }
}

fn merge_objects(
    local: &Map<String, Value>,
    incoming: &Map<String, Value>,
    prune: bool,
) -> Map<String, Value> {
    let kept = local
        .iter()
        .filter(|(k, _)| !prune || incoming.contains_key(*k))
        .map(|(k, v)| {
            let merged = match incoming.get(k) {
                Some(i) => merge_value(Some(v), i, prune),
                None => v.clone(),
            };
            (k.clone(), merged)
        });
    let added = incoming
        .iter()
        .filter(|(k, _)| !local.contains_key(*k))
        .map(|(k, v)| (k.clone(), v.clone()));
    kept.chain(added).collect()
}

fn union(local: &[Value], incoming: &[Value]) -> Vec<Value> {
    let extra = incoming.iter().filter(|v| !local.contains(v)).cloned();
    local.iter().cloned().chain(extra).collect()
}

/// `history.jsonl`: the union of both files' lines, de-duplicated, ordered by each entry's `timestamp`
/// (lines without one keep their relative place at the end).
pub fn merge_history_lines(local: &str, incoming: &str) -> String {
    let mut seen = HashSet::new();
    let mut lines: Vec<&str> = local
        .lines()
        .chain(incoming.lines())
        .filter(|l| !l.trim().is_empty())
        .filter(|l| seen.insert(*l))
        .collect();
    lines.sort_by(|a, b| compare_timestamps(timestamp(a), timestamp(b)));
    let mut out = lines.join("\n");
    if !out.is_empty() {
        out.push('\n');
    }
    out
}

fn timestamp(line: &str) -> Option<f64> {
    let value: Value = serde_json::from_str(line).ok()?;
    match value.get("timestamp")? {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s
            .parse::<jiff::Timestamp>()
            .ok()
            .map(|t| t.as_millisecond() as f64),
        _ => None,
    }
}

fn compare_timestamps(a: Option<f64>, b: Option<f64>) -> Ordering {
    match (a, b) {
        (Some(x), Some(y)) => x.partial_cmp(&y).unwrap_or(Ordering::Equal),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

/// A transcript present on both sides: take the incoming one only when it is longer (transcripts only
/// grow), so local progress is never lost.
pub fn take_incoming_transcript(local: &[u8], incoming: &[u8]) -> bool {
    incoming.len() > local.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn merges_objects_and_unions_arrays() {
        let local = json!({"allow": ["a"], "theme": "dark", "keep": 1});
        let incoming = json!({"allow": ["b", "a"], "theme": "light"});
        let out = merge_value(Some(&local), &incoming, false);
        assert_eq!(
            out,
            json!({"allow": ["a", "b"], "theme": "light", "keep": 1})
        );
    }

    #[test]
    fn prune_takes_incoming_exactly() {
        let local = json!({"allow": ["a"], "keep": 1});
        let incoming = json!({"allow": ["b"]});
        assert_eq!(
            merge_value(Some(&local), &incoming, true),
            json!({"allow": ["b"]})
        );
    }

    #[test]
    fn unresolved_placeholder_keeps_local_value() {
        let local = json!({"env": {"TOKEN": "real"}});
        let incoming = json!({"env": {"TOKEN": "${HARYX_SECRET_TOKEN}"}});
        assert_eq!(merge_value(Some(&local), &incoming, true), local);
    }

    #[test]
    fn history_lines_union_sorted_by_timestamp() {
        let local = "{\"display\":\"b\",\"timestamp\":2}\n{\"display\":\"a\",\"timestamp\":1}\n";
        let incoming = "{\"display\":\"c\",\"timestamp\":3}\n{\"display\":\"a\",\"timestamp\":1}\n";
        let out = merge_history_lines(local, incoming);
        let displays: Vec<String> = out
            .lines()
            .map(|l| {
                serde_json::from_str::<Value>(l).unwrap()["display"]
                    .as_str()
                    .unwrap()
                    .to_owned()
            })
            .collect();
        assert_eq!(displays, vec!["a", "b", "c"]);
    }

    #[test]
    fn longer_transcript_wins() {
        assert!(take_incoming_transcript(b"ab", b"abc"));
        assert!(!take_incoming_transcript(b"abc", b"ab"));
    }
}

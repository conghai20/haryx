//! Applying account settings that arrived through sync (`haryx sync`), quietly and under the lock.
//!
//! Only the user-set fields ([`USER_ACCOUNT_SETTINGS`]) are touched, each validated the way its own
//! command validates it. A value this machine cannot take (an alias another local account already
//! uses, a value from a newer version) is skipped and named, never written half-checked.

use serde_json::{Map, Value};

use super::policy::{USER_ACCOUNT_SETTINGS, record_mut};
use super::{JsonObject, Switcher};
use crate::error::Result;
use crate::models::{
    get_timestamp, normalize_account_priority, normalize_account_threshold, normalize_alias,
};

impl Switcher {
    /// Set account `num`'s user fields to `wanted` (absent keys are cleared). Returns the fields
    /// that were skipped, with why.
    pub fn apply_synced_settings(
        &self,
        num: &str,
        wanted: &Map<String, Value>,
    ) -> Result<Vec<(String, String)>> {
        let _lock = self.lock()?;
        let mut data = self.sequence_or_empty()?;
        let aliases_elsewhere = other_aliases(&data, num);
        let Some(rec) = record_mut(&mut data, num) else {
            return Ok(vec![("account".into(), format!("account {num} is gone"))]);
        };
        let mut skipped = Vec::new();
        let mut changed = false;
        for field in USER_ACCOUNT_SETTINGS {
            let next = match wanted.get(field).filter(|v| !v.is_null()) {
                None => None,
                Some(value) => match valid(field, value) {
                    Ok(v) => v,
                    Err(why) => {
                        skipped.push((field.to_string(), why));
                        continue;
                    }
                },
            };
            if field == "alias"
                && let Some(Value::String(alias)) = &next
                && aliases_elsewhere.contains(alias)
            {
                skipped.push((
                    "alias".into(),
                    format!("'{alias}' is used by another account here"),
                ));
                continue;
            }
            if rec.get(field) != next.as_ref() {
                match next {
                    Some(value) => rec.insert(field.to_owned(), value),
                    None => rec.shift_remove(field),
                };
                changed = true;
            }
        }
        if changed {
            data.insert("lastUpdated".into(), Value::String(get_timestamp()));
            self.write_json(&self.sequence_file, &data)?;
        }
        Ok(skipped)
    }
}

/// The value to store for `field`, or `None` for "clear it".
fn valid(field: &str, value: &Value) -> std::result::Result<Option<Value>, String> {
    let bad = || format!("unusable value {value}");
    match field {
        "alias" => {
            let text = value.as_str().ok_or_else(bad)?;
            normalize_alias(text)
                .map(|a| Some(Value::String(a)))
                .map_err(|e| e.to_string())
        }
        "disabled" => Ok(value
            .as_bool()
            .ok_or_else(bad)?
            .then_some(Value::Bool(true))),
        "threshold" => {
            let n = value.as_f64().ok_or_else(bad)?;
            normalize_account_threshold(n)
                .map(|t| Some(Value::from(t)))
                .map_err(|e| e.to_string())
        }
        "priority" => {
            let n = value.as_i64().ok_or_else(bad)?;
            normalize_account_priority(n)
                .map(|p| (p > 0).then(|| Value::from(p)))
                .map_err(|e| e.to_string())
        }
        _ => Err(bad()),
    }
}

fn other_aliases(data: &JsonObject, num: &str) -> Vec<String> {
    data.get("accounts")
        .and_then(Value::as_object)
        .map(|accounts| {
            accounts
                .iter()
                .filter(|(n, _)| n.as_str() != num)
                .filter_map(|(_, rec)| rec.get("alias").and_then(Value::as_str))
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::super::test_support::{TestSwitcher, sample};

    fn wanted(v: serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
        v.as_object().unwrap().clone()
    }

    #[test]
    fn synced_settings_are_written_and_cleared() {
        let sw = TestSwitcher::linux().with_roster(sample());
        let skipped = sw
            .apply_synced_settings(
                "1",
                &wanted(json!({"alias": "work", "threshold": 90, "priority": 2, "disabled": true})),
            )
            .unwrap();
        assert!(skipped.is_empty(), "{skipped:?}");
        let rec = &sw.roster()["accounts"]["1"];
        assert_eq!(rec["alias"], "work");
        assert_eq!(rec["threshold"], 90.0);
        assert_eq!(rec["priority"], 2);
        assert_eq!(rec["disabled"], true);

        sw.apply_synced_settings("1", &wanted(json!({"alias": "work"})))
            .unwrap();
        let rec = &sw.roster()["accounts"]["1"];
        assert_eq!(rec["alias"], "work");
        for gone in ["threshold", "priority", "disabled"] {
            assert!(rec.get(gone).is_none(), "{gone} cleared");
        }
    }

    #[test]
    fn a_taken_alias_and_a_bad_value_are_skipped_not_written() {
        let sw = TestSwitcher::linux().with_roster(sample());
        sw.apply_synced_settings("2", &wanted(json!({"alias": "home"})))
            .unwrap();
        let skipped = sw
            .apply_synced_settings("1", &wanted(json!({"alias": "home", "threshold": "lots"})))
            .unwrap();
        assert_eq!(skipped.len(), 2, "{skipped:?}");
        assert!(sw.roster()["accounts"]["1"].get("alias").is_none());
        assert_eq!(sw.roster()["accounts"]["2"]["alias"], "home");
    }
}

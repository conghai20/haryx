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
    normalize_group,
};

/// The identity fields an `account` row carries beside the settings; not settings, never applied.
const IDENTITY_FIELDS: [&str; 3] = ["email", "organizationUuid", "organizationName"];

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
        // A setting from a newer HaryX: named as skipped so the sync base keeps it, rather than
        // dropped silently and then read as removed here on the next pass.
        skipped.extend(
            wanted
                .keys()
                .filter(|k| {
                    !USER_ACCOUNT_SETTINGS.contains(&k.as_str())
                        && !IDENTITY_FIELDS.contains(&k.as_str())
                })
                .map(|k| {
                    (
                        k.clone(),
                        "this HaryX does not know it; update to use it".into(),
                    )
                }),
        );
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
        "group" => {
            let text = value.as_str().ok_or_else(bad)?;
            normalize_group(text)
                .map(|g| Some(Value::String(g)))
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
    fn a_group_syncs_and_an_unknown_setting_is_skipped_not_dropped() {
        let sw = TestSwitcher::linux().with_roster(sample());
        let skipped = sw
            .apply_synced_settings(
                "1",
                &wanted(json!({"group": " Work ", "email": "account1@example.com",
                    "colour": "red"})),
            )
            .unwrap();
        assert_eq!(sw.roster()["accounts"]["1"]["group"], "Work");
        assert_eq!(skipped.len(), 1, "{skipped:?}");
        assert_eq!(skipped[0].0, "colour");
        sw.apply_synced_settings("1", &wanted(json!({}))).unwrap();
        assert!(sw.roster()["accounts"]["1"].get("group").is_none());
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

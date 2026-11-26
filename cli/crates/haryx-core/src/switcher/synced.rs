//! Applying account settings that arrived through sync (`haryx sync`), quietly and under the lock.
//!
//! Only the user-set fields ([`USER_ACCOUNT_SETTINGS`]) are touched, each validated the way its own
//! command validates it. A value this machine cannot take (an alias another local account already
//! uses, a value from a newer version) is skipped and named, never written half-checked.

use serde_json::{Map, Value};

use super::groups::{groups_of, write_groups};
use super::policy::{USER_ACCOUNT_SETTINGS, record_mut};
use super::{JsonObject, Switcher};
use crate::error::Result;
use crate::models::{
    get_timestamp, normalize_account_priority, normalize_account_threshold, normalize_alias,
    normalize_group, normalize_groups,
};

/// The two group keys, applied together by [`synced_groups`] rather than field by field.
const GROUP_FIELDS: [&str; 2] = ["group", "groups"];

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
        match synced_groups(wanted, &groups_of(rec)) {
            Ok(groups) => changed |= write_groups(rec, &groups),
            Err(why) => skipped.push(why),
        }
        for field in USER_ACCOUNT_SETTINGS {
            if GROUP_FIELDS.contains(&field) {
                continue;
            }
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

/// The groups to store from a synced row, given the record's `current` ones.
///
/// - `groups` (newer machines) is the list; `[]` or absent with no `group` clears.
/// - `group` alone is from a machine that predates `groups`. Its value already in the list: nothing
///   to do. Otherwise it moves to the front, the rest kept: an older machine can rename or set the
///   first group, never drop the others it cannot see.
/// - Both, with `group` not in `groups`: an older machine changed `group` since the last sync
///   (newer machines always write `group` = `groups[0]`), so it goes to the front the same way.
///
/// An unusable value is returned as `(field, why)` and the record's groups stay as they are.
fn synced_groups(
    wanted: &Map<String, Value>,
    current: &[String],
) -> std::result::Result<Vec<String>, (String, String)> {
    let present = |key: &str| wanted.get(key).filter(|v| !v.is_null());
    let skip = |field: &str, why: String| (field.to_owned(), why);
    let bad = |field: &str, v: &Value| skip(field, format!("unusable value {v}"));
    let group = match present("group") {
        None => None,
        Some(v) => {
            let text = v.as_str().ok_or_else(|| bad("group", v))?;
            Some(normalize_group(text).map_err(|e| skip("group", e.to_string()))?)
        }
    };
    let list = match present("groups") {
        None => None,
        Some(v) => {
            let items = v.as_array().ok_or_else(|| bad("groups", v))?;
            let names = items
                .iter()
                .map(|i| i.as_str().ok_or_else(|| bad("groups", v)))
                .collect::<std::result::Result<Vec<&str>, _>>()?;
            Some(normalize_groups(names).map_err(|e| skip("groups", e.to_string()))?)
        }
    };
    let Some(group) = group else {
        return Ok(list.unwrap_or_default());
    };
    let base = list.unwrap_or_else(|| current.to_vec());
    if base
        .iter()
        .any(|g| g.to_lowercase() == group.to_lowercase())
    {
        return Ok(base);
    }
    let merged: Vec<&str> = std::iter::once(group.as_str())
        .chain(base.iter().map(String::as_str))
        .collect();
    normalize_groups(merged).map_err(|e| skip("group", e.to_string()))
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
    fn a_group_list_syncs_whole_and_mirrors_the_first_into_group() {
        let sw = TestSwitcher::linux().with_roster(sample());
        let skipped = sw
            .apply_synced_settings(
                "1",
                &wanted(json!({"groups": ["Work", " home ", "work"], "group": "Work"})),
            )
            .unwrap();
        assert!(skipped.is_empty(), "{skipped:?}");
        let rec = &sw.roster()["accounts"]["1"];
        assert_eq!(rec["groups"], json!(["Work", "home"]));
        assert_eq!(rec["group"], "Work");

        sw.apply_synced_settings("1", &wanted(json!({"groups": []})))
            .unwrap();
        let rec = &sw.roster()["accounts"]["1"];
        assert!(
            rec.get("group").is_none() && rec.get("groups").is_none(),
            "{rec}"
        );
    }

    #[test]
    fn an_older_machines_group_joins_the_front_and_keeps_the_rest() {
        let sw = TestSwitcher::linux().with_roster(sample());
        sw.apply_synced_settings("1", &wanted(json!({"groups": ["Work", "Home"]})))
            .unwrap();
        // An older machine only knows `group`: one already in the list changes nothing.
        sw.apply_synced_settings("1", &wanted(json!({"group": "home"})))
            .unwrap();
        assert_eq!(
            sw.roster()["accounts"]["1"]["groups"],
            json!(["Work", "Home"])
        );
        // A new one goes first, and the groups it cannot see stay.
        sw.apply_synced_settings("1", &wanted(json!({"group": "Client"})))
            .unwrap();
        let rec = &sw.roster()["accounts"]["1"];
        assert_eq!(rec["groups"], json!(["Client", "Work", "Home"]));
        assert_eq!(rec["group"], "Client");
    }

    #[test]
    fn a_group_list_that_is_not_strings_is_skipped_and_the_groups_stay() {
        let sw = TestSwitcher::linux().with_roster(sample());
        sw.apply_synced_settings("1", &wanted(json!({"groups": ["Work"]})))
            .unwrap();
        let skipped = sw
            .apply_synced_settings("1", &wanted(json!({"groups": ["Home", 3]})))
            .unwrap();
        assert_eq!(skipped.len(), 1, "{skipped:?}");
        assert_eq!(skipped[0].0, "groups");
        assert_eq!(sw.roster()["accounts"]["1"]["groups"], json!(["Work"]));
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

//! Account groups: a label on the account record (`group`) that automatic switching honours.
//!
//! Every automatic pick (rotation, `best`, `next-available`, auto-switch) first decides which
//! accounts could take over by its own rule, then prefers those in the active account's group.
//! Ungrouped accounts form a group of their own. Only when no account in that group qualifies does
//! the pick leave it. An explicit `switch <num|email|alias>` ignores groups.

use serde_json::Value;

use super::policy::record_mut;
use super::{JsonObject, Switcher, accounts, record};
use crate::error::{Error, Result};
use crate::models::{get_timestamp, normalize_group};

/// The group of slot `num` as stored, "" when it has none.
pub(crate) fn group_of<'a>(data: &'a JsonObject, num: &str) -> &'a str {
    record(data, num)
        .and_then(|r| r.get("group"))
        .and_then(Value::as_str)
        .unwrap_or("")
}

/// Whether slots `a` and `b` are in the same group (case-insensitive; both ungrouped counts).
pub(crate) fn same_group(data: &JsonObject, a: &str, b: &str) -> bool {
    group_of(data, a).to_lowercase() == group_of(data, b).to_lowercase()
}

fn any_grouped(data: &JsonObject) -> bool {
    accounts(data).is_some_and(|accts| {
        accts.values().any(|r| {
            r.get("group")
                .and_then(Value::as_str)
                .is_some_and(|g| !g.is_empty())
        })
    })
}

/// `nums` reordered so slots in `current`'s group come first, each part keeping its order. Without
/// a current slot, or with no groups set anywhere, the order is unchanged.
pub(crate) fn same_group_first(
    data: &JsonObject,
    current: Option<&str>,
    nums: Vec<String>,
) -> Vec<String> {
    let Some(current) = current.filter(|_| any_grouped(data)) else {
        return nums;
    };
    let (mut mine, rest): (Vec<String>, Vec<String>) =
        nums.into_iter().partition(|n| same_group(data, current, n));
    mine.extend(rest);
    mine
}

fn label(group: &str) -> String {
    if group.is_empty() {
        "Ungrouped".to_owned()
    } else {
        format!("'{group}'")
    }
}

/// The note for an automatic switch from `from` to `to` that left `from`'s group, or `None` when it
/// stayed in it (or no groups are in use).
pub(crate) fn left_group_note(data: &JsonObject, from: &str, to: &str) -> Option<String> {
    if !any_grouped(data) || same_group(data, from, to) {
        return None;
    }
    Some(format!(
        "No account in group {} could take over; switched to Account-{to} in group {}",
        label(group_of(data, from)),
        label(group_of(data, to))
    ))
}

impl Switcher {
    /// Put the account `identifier` names into `group`, joining an existing group whose name matches
    /// it ignoring case (its stored spelling wins). Returns `(account_num, stored_group)`.
    pub fn set_group(&self, identifier: &str, group: &str) -> Result<(String, String)> {
        self.refuse_session_shell()?;
        let wanted = normalize_group(group)?;
        let num = self.resolved_slot(identifier)?;
        let mut data = self.sequence_or_empty()?;
        let stored = existing_spelling(&data, &num, &wanted).unwrap_or(wanted);
        if let Some(rec) = record_mut(&mut data, &num) {
            rec.insert("group".into(), Value::String(stored.clone()));
        }
        data.insert("lastUpdated".into(), Value::String(get_timestamp()));
        self.write_json(&self.sequence_file, &data)?;
        Ok((num, stored))
    }

    /// Take an account out of its group; returns the account number. Idempotent.
    pub fn unset_group(&self, identifier: &str) -> Result<String> {
        self.refuse_session_shell()?;
        let num = self.resolved_slot(identifier)?;
        let mut data = self.sequence_or_empty()?;
        let removed = record_mut(&mut data, &num)
            .and_then(|rec| rec.shift_remove("group"))
            .is_some();
        if removed {
            data.insert("lastUpdated".into(), Value::String(get_timestamp()));
            self.write_json(&self.sequence_file, &data)?;
        }
        Ok(num)
    }

    /// Every group in use with its slot numbers, groups sorted by name ignoring case, slots in
    /// sequence order.
    pub fn list_groups(&self) -> Result<Vec<(String, Vec<String>)>> {
        let data = self.get_sequence_data_migrated()?.unwrap_or_default();
        let mut groups: Vec<(String, Vec<String>)> = Vec::new();
        for num in super::policy::sequence_numbers(&data) {
            let group = group_of(&data, &num);
            if group.is_empty() {
                continue;
            }
            match groups
                .iter_mut()
                .find(|(g, _)| g.to_lowercase() == group.to_lowercase())
            {
                Some((_, nums)) => nums.push(num),
                None => groups.push((group.to_owned(), vec![num])),
            }
        }
        groups.sort_by_key(|(g, _)| g.to_lowercase());
        Ok(groups)
    }

    /// [`same_group_first`] over the roster on disk, for the auto-switch engine.
    pub(crate) fn same_group_first(&self, current: &str, nums: Vec<String>) -> Vec<String> {
        match self.sequence_or_empty() {
            Ok(data) => same_group_first(&data, Some(current), nums),
            Err(_) => nums,
        }
    }

    /// [`left_group_note`] over the roster on disk.
    pub(crate) fn left_group_note(&self, from: &str, to: &str) -> Option<String> {
        left_group_note(&self.sequence_or_empty().ok()?, from, to)
    }

    /// `ai sessions continue --to best`: where a session running as `from` should continue. An
    /// account with headroom left, other than `from`, with the most of it; one in `from`'s group
    /// wins over any outside it, as the automatic switches prefer. Returns the target and, when the
    /// pick left `from`'s group, the note saying so; `None` when no account has headroom.
    pub fn best_handoff_target(
        &self,
        from: Option<&str>,
    ) -> Result<Option<(String, Option<String>)>> {
        let usage = self.usage_by_account();
        let headroom =
            |n: &str| crate::oauth::account_headroom(usage.get(n).and_then(Option::as_ref), &[]);
        let data = self.sequence_or_empty()?;
        let usable: Vec<(f64, String)> = self
            .switchable_account_numbers()?
            .into_iter()
            .filter(|n| Some(n.as_str()) != from)
            .filter_map(|n| headroom(&n).filter(|h| *h > 0.0).map(|h| (h, n)))
            .collect();
        let in_group = |n: &str| from.is_some_and(|f| same_group(&data, f, n));
        let has_mate = usable.iter().any(|(_, n)| in_group(n));
        let best = usable
            .into_iter()
            .filter(|(_, n)| !has_mate || in_group(n))
            .max_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, n)| n);
        Ok(best.map(|target| {
            let note = from.and_then(|f| left_group_note(&data, f, &target));
            (target, note)
        }))
    }

    fn resolved_slot(&self, identifier: &str) -> Result<String> {
        self.get_sequence_data_migrated()?;
        let num = self
            .resolve_account_identifier(identifier)?
            .ok_or_else(|| {
                Error::AccountNotFound(format!("No account found with identifier: {identifier}"))
            })?;
        let data = self.sequence_or_empty()?;
        if record(&data, &num).is_none() {
            return Err(Error::AccountNotFound(format!(
                "Account-{num} does not exist"
            )));
        }
        Ok(num)
    }
}

/// The spelling another account already uses for `wanted`, ignoring case.
fn existing_spelling(data: &JsonObject, num: &str, wanted: &str) -> Option<String> {
    let key = wanted.to_lowercase();
    accounts(data)?
        .iter()
        .filter(|(n, _)| n.as_str() != num)
        .filter_map(|(_, r)| r.get("group").and_then(Value::as_str))
        .find(|g| g.to_lowercase() == key)
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use serde_json::{Value, json};

    use super::super::test_support::{TestSwitcher, creds};
    use super::*;
    use crate::autoswitch::{AutoSwitchEngine, Event, TickOutcome};
    use crate::oauth::tests::stub::{Reply, Stub};
    use crate::settings::AutoSwitchSettings;

    fn roster(groups: &[(&str, Option<&str>)]) -> Value {
        let mut accounts = serde_json::Map::new();
        for (num, group) in groups {
            let mut rec =
                json!({"email": format!("a{num}@example.com"), "uuid": format!("u{num}")});
            if let Some(g) = group {
                rec["group"] = json!(g);
            }
            accounts.insert((*num).to_owned(), rec);
        }
        let sequence: Vec<i64> = groups.iter().map(|(n, _)| n.parse().unwrap()).collect();
        json!({"activeAccountNumber": 1, "sequence": sequence, "accounts": accounts})
    }

    fn obj(v: &Value) -> &JsonObject {
        v.as_object().unwrap()
    }

    fn nums(ns: &[&str]) -> Vec<String> {
        ns.iter().map(|n| (*n).to_owned()).collect()
    }

    #[test]
    fn the_active_group_comes_first_and_each_part_keeps_its_order() {
        let data = roster(&[
            ("1", Some("Work")),
            ("2", None),
            ("3", Some("work")),
            ("4", Some("Home")),
            ("5", Some("Work")),
        ]);
        let order = same_group_first(obj(&data), Some("1"), nums(&["2", "3", "4", "5"]));
        assert_eq!(order, nums(&["3", "5", "2", "4"]));
    }

    #[test]
    fn ungrouped_accounts_are_a_group_of_their_own() {
        let data = roster(&[("1", None), ("2", Some("Work")), ("3", None)]);
        let order = same_group_first(obj(&data), Some("1"), nums(&["2", "3"]));
        assert_eq!(order, nums(&["3", "2"]));
    }

    #[test]
    fn without_groups_or_a_current_slot_the_order_is_unchanged() {
        let plain = roster(&[("1", None), ("2", None), ("3", None)]);
        assert_eq!(
            same_group_first(obj(&plain), Some("1"), nums(&["3", "2"])),
            nums(&["3", "2"])
        );
        let grouped = roster(&[("1", Some("A")), ("2", None), ("3", Some("A"))]);
        assert_eq!(
            same_group_first(obj(&grouped), None, nums(&["2", "3"])),
            nums(&["2", "3"])
        );
    }

    #[test]
    fn leaving_a_group_is_noted_staying_is_not() {
        let data = roster(&[("1", Some("Work")), ("2", Some("work")), ("3", None)]);
        assert_eq!(left_group_note(obj(&data), "1", "2"), None);
        assert_eq!(
            left_group_note(obj(&data), "1", "3").unwrap(),
            "No account in group 'Work' could take over; switched to Account-3 in group Ungrouped"
        );
        let plain = roster(&[("1", None), ("2", None)]);
        assert_eq!(left_group_note(obj(&plain), "1", "2"), None);
    }

    #[test]
    fn set_group_joins_an_existing_spelling_and_unset_clears_it() {
        let sw = TestSwitcher::linux().with_roster(roster(&[("1", Some("Client A")), ("2", None)]));
        assert_eq!(
            sw.set_group("2", "  client a ").unwrap(),
            ("2".into(), "Client A".into())
        );
        assert_eq!(sw.roster()["accounts"]["2"]["group"], "Client A");
        assert_eq!(
            sw.list_groups().unwrap(),
            vec![("Client A".to_owned(), nums(&["1", "2"]))]
        );
        assert_eq!(sw.unset_group("2").unwrap(), "2");
        assert!(sw.roster()["accounts"]["2"].get("group").is_none());
        assert_eq!(sw.unset_group("2").unwrap(), "2", "idempotent");
        assert!(sw.set_group("2", "   ").is_err());
        assert!(sw.set_group("9", "x").is_err());
    }

    // --- automatic picks, end to end against a stub usage endpoint ---------------------------

    const FUTURE_MS: i64 = 9_999_999_999_000;

    /// Three accounts on account 1, `at-N` using `pcts[N-1]`% of its 5-hour window.
    fn fleet(pcts: [f64; 3], groups: [Option<&str>; 3]) -> (TestSwitcher, Stub) {
        let stub = Stub::start(move |req| {
            if req.path.contains("profile") {
                return Reply::json(404, "{}");
            }
            let auth = req.header("Authorization").unwrap_or_default();
            let pct = (1..=3)
                .find(|n| auth.contains(&format!("at-{n}")))
                .map_or(0.0, |n| pcts[n - 1]);
            Reply::json(
                200,
                &json!({"five_hour": {"utilization": pct, "resets_at": "2099-01-01T00:00:00Z"},
                        "seven_day": {"utilization": 10.0, "resets_at": "2099-01-05T00:00:00Z"}})
                .to_string(),
            )
        });
        let rows: Vec<(&str, Option<&str>)> = ["1", "2", "3"].into_iter().zip(groups).collect();
        let mut s = TestSwitcher::linux().with_roster(roster(&rows));
        s.switcher.api = stub.api();
        s.set_live("a1@example.com", "");
        s.write_credentials(&creds("at-1", FUTURE_MS)).unwrap();
        for num in ["1", "2", "3"] {
            let email = format!("a{num}@example.com");
            s.write_account_credentials(num, &email, &creds(&format!("at-{num}"), FUTURE_MS))
                .unwrap();
            let config = json!({"oauthAccount": {"emailAddress": email,
                "accountUuid": format!("u{num}"), "organizationUuid": ""}});
            s.write_account_config(num, &email, &config.to_string())
                .unwrap();
        }
        (s, stub)
    }

    fn auto_tick(s: &TestSwitcher) -> Value {
        let events = Rc::new(RefCell::new(Vec::new()));
        let sink = Rc::clone(&events);
        let mut engine = AutoSwitchEngine::new(
            &s.switcher,
            AutoSwitchSettings::default(),
            move |e: &Event| sink.borrow_mut().push(e.clone()),
            false,
        );
        let outcome = engine.tick();
        drop(engine);
        let events = events.borrow();
        assert_eq!(outcome, TickOutcome::Switched, "{events:?}");
        events
            .iter()
            .rev()
            .find(|e| e.kind() == "switch")
            .unwrap()
            .to_json()
    }

    fn switch_json(s: &TestSwitcher, strategy: Option<&str>) -> Value {
        s.switch(strategy, true, &[], None).unwrap().unwrap()
    }

    const WORK_13: [Option<&str>; 3] = [Some("Work"), None, Some("work")];

    #[test]
    fn auto_switch_takes_a_qualifying_groupmate_over_a_roomier_outsider() {
        let (plain, _stub) = fleet([95.0, 5.0, 40.0], [None; 3]);
        assert_eq!(
            auto_tick(&plain)["to"]["number"],
            2,
            "control: most room wins"
        );

        let (s, _stub) = fleet([95.0, 5.0, 40.0], WORK_13);
        let switch = auto_tick(&s);
        assert_eq!(switch["to"]["number"], 3);
        assert_eq!(switch["warnings"], json!([]));
    }

    #[test]
    fn auto_switch_leaves_the_group_only_when_no_groupmate_qualifies_and_says_so() {
        let (s, _stub) = fleet([95.0, 5.0, 97.0], WORK_13);
        let switch = auto_tick(&s);
        assert_eq!(switch["to"]["number"], 2);
        let warnings = switch["warnings"].to_string();
        assert!(
            warnings.contains("No account in group 'Work' could take over"),
            "{warnings}"
        );
    }

    #[test]
    fn rotation_cycles_inside_the_group() {
        let (s, _stub) = fleet([10.0, 10.0, 10.0], WORK_13);
        assert_eq!(switch_json(&s, None)["to"]["number"], 3);
        assert_eq!(switch_json(&s, None)["to"]["number"], 1);
        assert_eq!(switch_json(&s, None)["to"]["number"], 3);
    }

    #[test]
    fn rotation_from_an_ungrouped_account_prefers_other_ungrouped_ones() {
        let (s, _stub) = fleet([10.0, 10.0, 10.0], [None, Some("Work"), None]);
        assert_eq!(switch_json(&s, None)["to"]["number"], 3);
    }

    #[test]
    fn a_handoff_prefers_the_sources_group_and_leaves_it_only_when_none_there_has_room() {
        let (s, _stub) = fleet([100.0, 5.0, 40.0], WORK_13);
        assert_eq!(
            s.best_handoff_target(Some("1")).unwrap(),
            Some(("3".into(), None))
        );

        let (s, _stub) = fleet([100.0, 5.0, 100.0], WORK_13);
        let (target, note) = s.best_handoff_target(Some("1")).unwrap().unwrap();
        assert_eq!(target, "2");
        assert!(note.unwrap().contains("group 'Work'"));

        let (s, _stub) = fleet([100.0, 5.0, 40.0], [None; 3]);
        assert_eq!(
            s.best_handoff_target(Some("1")).unwrap(),
            Some(("2".into(), None))
        );
        let (s, _stub) = fleet([100.0, 100.0, 100.0], WORK_13);
        assert_eq!(s.best_handoff_target(Some("1")).unwrap(), None);
    }

    #[test]
    fn best_prefers_the_group_and_next_available_falls_back_past_it() {
        let (s, _stub) = fleet([95.0, 5.0, 40.0], WORK_13);
        assert_eq!(switch_json(&s, Some("best"))["to"]["number"], 3);

        let (s, _stub) = fleet([95.0, 5.0, 100.0], WORK_13);
        let out = switch_json(&s, Some("next-available"));
        assert_eq!(out["to"]["number"], 2, "{out}");
        assert!(
            out["warnings"].to_string().contains("in group Ungrouped"),
            "{out}"
        );
    }
}

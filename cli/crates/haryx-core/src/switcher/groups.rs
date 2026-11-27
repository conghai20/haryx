//! Account groups: labels on the account record that automatic switching honours. An account can
//! be in several groups.
//!
//! Storage: `groups` is the canonical field, a JSON array of names (each [`normalize_group`]d,
//! de-duplicated ignoring case, in the order added). For readers that predate it, `group` mirrors
//! the first entry. An account in no group has neither key. A record with only `group` (an older
//! roster) reads as that one group. Read with [`groups_of`], write with [`write_groups`]; nothing
//! else touches the two keys.
//!
//! Every automatic pick (rotation, `best`, `next-available`, auto-switch) first decides which
//! accounts could take over by its own rule, then prefers those sharing a group with the active
//! account. Ungrouped accounts form a group of their own. Only when no such account qualifies does
//! the pick leave the group. An explicit `switch <num|email|alias>` ignores groups.

use serde_json::Value;

use super::policy::record_mut;
use super::{JsonObject, Switcher, accounts, record};
use crate::error::{Error, Result};
use crate::models::{GROUPS_MAX, get_timestamp, normalize_group};

/// The groups of an account record, in order: `groups` when it is an array, else `[group]` when
/// that is a non-empty string (older rosters), else empty.
pub fn groups_of(rec: &JsonObject) -> Vec<String> {
    let clean = |v: &Value| {
        v.as_str()
            .map(str::trim)
            .filter(|g| !g.is_empty())
            .map(str::to_owned)
    };
    match rec.get("groups") {
        Some(Value::Array(items)) => items.iter().filter_map(clean).collect(),
        _ => rec.get("group").and_then(clean).into_iter().collect(),
    }
}

/// Store `groups` on an account record: `groups` = the list and `group` = its first entry, or
/// neither key when the list is empty. Returns whether the record changed.
pub fn write_groups(rec: &mut JsonObject, groups: &[String]) -> bool {
    let before = (rec.get("group").cloned(), rec.get("groups").cloned());
    match groups.first() {
        None => {
            rec.shift_remove("group");
            rec.shift_remove("groups");
        }
        Some(first) => {
            rec.insert("group".into(), Value::String(first.clone()));
            rec.insert(
                "groups".into(),
                Value::Array(groups.iter().cloned().map(Value::String).collect()),
            );
        }
    }
    before != (rec.get("group").cloned(), rec.get("groups").cloned())
}

/// The groups of slot `num`, empty when it has none (or no record).
pub(crate) fn groups_of_slot(data: &JsonObject, num: &str) -> Vec<String> {
    record(data, num).map(groups_of).unwrap_or_default()
}

/// Whether two group names are the same group (case-insensitive).
fn same_name(a: &str, b: &str) -> bool {
    a.to_lowercase() == b.to_lowercase()
}

fn has_group(groups: &[String], name: &str) -> bool {
    groups.iter().any(|g| same_name(g, name))
}

/// Whether slots `a` and `b` are in the same group: they share any group (case-insensitive), or
/// both are ungrouped.
pub(crate) fn same_group(data: &JsonObject, a: &str, b: &str) -> bool {
    let (ga, gb) = (groups_of_slot(data, a), groups_of_slot(data, b));
    if ga.is_empty() || gb.is_empty() {
        return ga.is_empty() && gb.is_empty();
    }
    ga.iter().any(|g| has_group(&gb, g))
}

fn any_grouped(data: &JsonObject) -> bool {
    accounts(data).is_some_and(|accts| {
        accts
            .values()
            .filter_map(Value::as_object)
            .any(|r| !groups_of(r).is_empty())
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

/// "group 'Work'", "groups 'Work', 'Client A'" or "group Ungrouped".
fn label(groups: &[String]) -> String {
    match groups {
        [] => "group Ungrouped".to_owned(),
        [one] => format!("group '{one}'"),
        many => format!(
            "groups {}",
            many.iter()
                .map(|g| format!("'{g}'"))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

/// The note for an automatic switch from `from` to `to` that left `from`'s groups, or `None` when it
/// stayed in one (or no groups are in use).
pub(crate) fn left_group_note(data: &JsonObject, from: &str, to: &str) -> Option<String> {
    if !any_grouped(data) || same_group(data, from, to) {
        return None;
    }
    Some(format!(
        "No account in {} could take over; switched to Account-{to} in {}",
        label(&groups_of_slot(data, from)),
        label(&groups_of_slot(data, to))
    ))
}

/// What [`Switcher::add_group`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupAdded {
    /// The account's slot number.
    pub num: String,
    /// The group's name as stored (an existing spelling wins over the one typed).
    pub group: String,
    /// `false` when the account was already in the group (nothing changed).
    pub added: bool,
}

impl Switcher {
    /// Add the account `identifier` names to `group`, joining an existing group whose name matches
    /// it ignoring case (its stored spelling wins). Already a member: success, nothing written.
    /// Its other groups are kept; a group past [`GROUPS_MAX`] is refused.
    pub fn add_group(&self, identifier: &str, group: &str) -> Result<GroupAdded> {
        self.refuse_session_shell()?;
        let wanted = normalize_group(group)?;
        let num = self.resolved_slot(identifier)?;
        let mut data = self.sequence_or_empty()?;
        let mut current = groups_of_slot(&data, &num);
        if let Some(mine) = current.iter().find(|g| same_name(g, &wanted)) {
            return Ok(GroupAdded {
                num,
                group: mine.clone(),
                added: false,
            });
        }
        if current.len() >= GROUPS_MAX {
            return Err(Error::Validation(format!(
                "Account-{num} is already in {GROUPS_MAX} groups, the most an account can be in; \
                 remove one first (--remove NAME)"
            )));
        }
        let stored = existing_spelling(&data, &num, &wanted).unwrap_or(wanted);
        current.push(stored.clone());
        if let Some(rec) = record_mut(&mut data, &num) {
            write_groups(rec, &current);
        }
        data.insert("lastUpdated".into(), Value::String(get_timestamp()));
        self.write_json(&self.sequence_file, &data)?;
        Ok(GroupAdded {
            num,
            group: stored,
            added: true,
        })
    }

    /// Take the account out of one group (matched ignoring case), keeping its others. Returns
    /// `(account_num, removed_group_as_stored)`; not being in that group is an error.
    pub fn remove_group(&self, identifier: &str, group: &str) -> Result<(String, String)> {
        self.refuse_session_shell()?;
        let wanted = normalize_group(group)?;
        let num = self.resolved_slot(identifier)?;
        let mut data = self.sequence_or_empty()?;
        let current = groups_of_slot(&data, &num);
        let Some(removed) = current.iter().find(|g| same_name(g, &wanted)).cloned() else {
            return Err(Error::Validation(format!(
                "Account-{num} is not in group '{wanted}'"
            )));
        };
        let kept: Vec<String> = current.into_iter().filter(|g| *g != removed).collect();
        if let Some(rec) = record_mut(&mut data, &num) {
            write_groups(rec, &kept);
        }
        data.insert("lastUpdated".into(), Value::String(get_timestamp()));
        self.write_json(&self.sequence_file, &data)?;
        Ok((num, removed))
    }

    /// Take an account out of all its groups; returns the account number. Idempotent.
    pub fn unset_group(&self, identifier: &str) -> Result<String> {
        self.refuse_session_shell()?;
        let num = self.resolved_slot(identifier)?;
        let mut data = self.sequence_or_empty()?;
        let removed = record_mut(&mut data, &num).is_some_and(|rec| write_groups(rec, &[]));
        if removed {
            data.insert("lastUpdated".into(), Value::String(get_timestamp()));
            self.write_json(&self.sequence_file, &data)?;
        }
        Ok(num)
    }

    /// Rename group `old` (matched ignoring case) to `new` on every account in it, each keeping the
    /// group's place in its list. A `new` that names another existing group merges the two, in that
    /// group's spelling; one that differs only in case just respells the group. Returns
    /// [`GroupRenamed`]; no account in `old` is an error.
    pub fn rename_group(&self, old: &str, new: &str) -> Result<GroupRenamed> {
        self.refuse_session_shell()?;
        let old = normalize_group(old)?;
        let new = normalize_group(new)?;
        let mut data = self.get_sequence_data_migrated()?.unwrap_or_default();
        let members = members_of(&data, &old);
        let Some(from) = members.first().and_then(|n| {
            groups_of_slot(&data, n)
                .into_iter()
                .find(|g| same_name(g, &old))
        }) else {
            return Err(no_such_group(&old));
        };
        let to = if same_name(&from, &new) {
            new
        } else {
            all_groups(&data)
                .into_iter()
                .find(|g| same_name(g, &new))
                .unwrap_or(new)
        };
        for num in &members {
            let renamed = groups_of_slot(&data, num)
                .into_iter()
                .fold(Vec::new(), |mut acc, g| {
                    let g = if same_name(&g, &from) { to.clone() } else { g };
                    if !has_group(&acc, &g) {
                        acc.push(g);
                    }
                    acc
                });
            if let Some(rec) = record_mut(&mut data, num) {
                write_groups(rec, &renamed);
            }
        }
        data.insert("lastUpdated".into(), Value::String(get_timestamp()));
        self.write_json(&self.sequence_file, &data)?;
        Ok(GroupRenamed {
            from,
            to,
            accounts: members.len(),
        })
    }

    /// Delete group `name` (matched ignoring case): every account in it leaves it and keeps its
    /// other groups. The accounts themselves stay. Returns `(group_as_stored, accounts_changed)`;
    /// no account in it is an error.
    pub fn delete_group(&self, name: &str) -> Result<(String, usize)> {
        self.refuse_session_shell()?;
        let wanted = normalize_group(name)?;
        let mut data = self.get_sequence_data_migrated()?.unwrap_or_default();
        let members = members_of(&data, &wanted);
        let Some(stored) = members.first().and_then(|n| {
            groups_of_slot(&data, n)
                .into_iter()
                .find(|g| same_name(g, &wanted))
        }) else {
            return Err(no_such_group(&wanted));
        };
        for num in &members {
            let kept: Vec<String> = groups_of_slot(&data, num)
                .into_iter()
                .filter(|g| !same_name(g, &stored))
                .collect();
            if let Some(rec) = record_mut(&mut data, num) {
                write_groups(rec, &kept);
            }
        }
        data.insert("lastUpdated".into(), Value::String(get_timestamp()));
        self.write_json(&self.sequence_file, &data)?;
        Ok((stored, members.len()))
    }

    /// Every group in use with its slot numbers, groups sorted by name ignoring case, slots in
    /// sequence order. An account in several groups is listed under each.
    pub fn list_groups(&self) -> Result<Vec<(String, Vec<String>)>> {
        let data = self.get_sequence_data_migrated()?.unwrap_or_default();
        let mut groups: Vec<(String, Vec<String>)> = Vec::new();
        for num in super::policy::sequence_numbers(&data) {
            for group in groups_of_slot(&data, &num) {
                match groups.iter_mut().find(|(g, _)| same_name(g, &group)) {
                    Some((_, nums)) if nums.contains(&num) => {}
                    Some((_, nums)) => nums.push(num.clone()),
                    None => groups.push((group, vec![num.clone()])),
                }
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
/// The outcome of [`Switcher::rename_group`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupRenamed {
    /// The group's name as it was stored.
    pub from: String,
    /// The name it now has (an existing group's spelling when the rename merged into it).
    pub to: String,
    /// How many accounts were in it.
    pub accounts: usize,
}

fn no_such_group(name: &str) -> Error {
    Error::Validation(format!(
        "No account is in group '{name}' ('haryx ai group' lists the groups)"
    ))
}

/// The slots in group `name` (ignoring case), in sequence order.
fn members_of(data: &JsonObject, name: &str) -> Vec<String> {
    super::policy::sequence_numbers(data)
        .into_iter()
        .filter(|n| has_group(&groups_of_slot(data, n), name))
        .collect()
}

/// Every group name stored on any account, as stored.
fn all_groups(data: &JsonObject) -> Vec<String> {
    accounts(data)
        .map(|accts| {
            accts
                .values()
                .filter_map(|r| r.as_object())
                .flat_map(groups_of)
                .collect()
        })
        .unwrap_or_default()
}

fn existing_spelling(data: &JsonObject, num: &str, wanted: &str) -> Option<String> {
    accounts(data)?
        .iter()
        .filter(|(n, _)| n.as_str() != num)
        .filter_map(|(_, r)| r.as_object())
        .flat_map(groups_of)
        .find(|g| same_name(g, wanted))
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

    fn added(num: &str, group: &str, added: bool) -> GroupAdded {
        GroupAdded {
            num: num.into(),
            group: group.into(),
            added,
        }
    }

    /// A roster whose accounts carry `groups` lists (empty: no group keys).
    fn multi(rows: &[(&str, &[&str])]) -> Value {
        let mut data = roster(&rows.iter().map(|(n, _)| (*n, None)).collect::<Vec<_>>());
        for (num, groups) in rows {
            let list: Vec<String> = groups.iter().map(|g| (*g).to_owned()).collect();
            let rec = data["accounts"][*num].as_object_mut().unwrap();
            write_groups(rec, &list);
        }
        data
    }

    #[test]
    fn add_group_joins_an_existing_spelling_and_unset_clears_both_keys() {
        let sw = TestSwitcher::linux().with_roster(roster(&[("1", Some("Client A")), ("2", None)]));
        assert_eq!(
            sw.add_group("2", "  client a ").unwrap(),
            added("2", "Client A", true)
        );
        assert_eq!(sw.roster()["accounts"]["2"]["group"], "Client A");
        assert_eq!(sw.roster()["accounts"]["2"]["groups"], json!(["Client A"]));
        assert_eq!(
            sw.list_groups().unwrap(),
            vec![("Client A".to_owned(), nums(&["1", "2"]))]
        );
        assert_eq!(sw.add_group("2", "Work").unwrap(), added("2", "Work", true));
        assert_eq!(sw.unset_group("2").unwrap(), "2");
        let rec = &sw.roster()["accounts"]["2"];
        assert!(
            rec.get("group").is_none() && rec.get("groups").is_none(),
            "{rec}"
        );
        assert_eq!(sw.unset_group("2").unwrap(), "2", "idempotent");
        assert!(sw.add_group("2", "   ").is_err());
        assert!(sw.add_group("9", "x").is_err());
    }

    #[test]
    fn adding_a_second_group_keeps_the_first_and_re_adding_changes_nothing() {
        let sw = TestSwitcher::linux().with_roster(roster(&[("1", None), ("2", None)]));
        sw.add_group("1", "Work").unwrap();
        assert_eq!(sw.add_group("1", "Home").unwrap(), added("1", "Home", true));
        let rec = &sw.roster()["accounts"]["1"];
        assert_eq!(rec["groups"], json!(["Work", "Home"]));
        assert_eq!(rec["group"], "Work", "the first group, for older readers");

        let before = sw.roster();
        assert_eq!(
            sw.add_group("1", "home").unwrap(),
            added("1", "Home", false)
        );
        assert_eq!(sw.roster(), before, "already a member: nothing written");

        sw.add_group("2", "home").unwrap();
        assert_eq!(
            sw.list_groups().unwrap(),
            vec![
                ("Home".to_owned(), nums(&["1", "2"])),
                ("Work".to_owned(), nums(&["1"])),
            ],
            "an account in two groups is listed under both"
        );
    }

    #[test]
    fn rename_keeps_each_accounts_order_and_reports_the_count() {
        let sw = TestSwitcher::linux().with_roster(multi(&[
            ("1", &["Work", "Home"]),
            ("2", &["work"]),
            ("3", &["Home"]),
        ]));
        let renamed = sw.rename_group("WORK", "Office").unwrap();
        assert_eq!(
            renamed,
            GroupRenamed {
                from: "Work".into(),
                to: "Office".into(),
                accounts: 2
            }
        );
        let roster = sw.roster();
        assert_eq!(roster["accounts"]["1"]["groups"], json!(["Office", "Home"]));
        assert_eq!(roster["accounts"]["1"]["group"], "Office");
        assert_eq!(roster["accounts"]["2"]["groups"], json!(["Office"]));
        assert_eq!(roster["accounts"]["3"]["groups"], json!(["Home"]));
    }

    #[test]
    fn rename_onto_another_group_merges_in_its_spelling_and_case_only_respells() {
        let sw =
            TestSwitcher::linux().with_roster(multi(&[("1", &["Work", "Home"]), ("2", &["Work"])]));
        let merged = sw.rename_group("Work", "HOME").unwrap();
        assert_eq!(merged.to, "Home", "joins the existing spelling");
        let roster = sw.roster();
        assert_eq!(
            roster["accounts"]["1"]["groups"],
            json!(["Home"]),
            "no duplicate"
        );
        assert_eq!(roster["accounts"]["2"]["groups"], json!(["Home"]));

        let respelled = sw.rename_group("home", "HOME").unwrap();
        assert_eq!(respelled.to, "HOME");
        assert_eq!(sw.roster()["accounts"]["2"]["groups"], json!(["HOME"]));
    }

    #[test]
    fn delete_takes_every_account_out_and_keeps_their_other_groups() {
        let sw =
            TestSwitcher::linux().with_roster(multi(&[("1", &["Work", "Home"]), ("2", &["work"])]));
        assert_eq!(sw.delete_group("WORK").unwrap(), ("Work".into(), 2));
        let roster = sw.roster();
        assert_eq!(roster["accounts"]["1"]["groups"], json!(["Home"]));
        let two = &roster["accounts"]["2"];
        assert!(
            two.get("group").is_none() && two.get("groups").is_none(),
            "{two}"
        );
        assert!(
            roster["accounts"]["2"]["email"].is_string(),
            "the account stays"
        );
    }

    #[test]
    fn renaming_or_deleting_a_group_nobody_is_in_is_refused() {
        let sw = TestSwitcher::linux().with_roster(multi(&[("1", &["Work"])]));
        let err = sw.rename_group("Nope", "X").unwrap_err().to_string();
        assert!(err.contains("No account is in group 'Nope'"), "{err}");
        assert!(sw.delete_group("Nope").is_err());
        assert!(sw.rename_group("Work", "  ").is_err(), "an empty new name");
        assert_eq!(sw.roster()["accounts"]["1"]["groups"], json!(["Work"]));
    }

    #[test]
    fn remove_group_drops_one_keeps_the_rest_and_refuses_a_non_member() {
        let sw = TestSwitcher::linux().with_roster(multi(&[("1", &["Work", "Home"]), ("2", &[])]));
        assert_eq!(
            sw.remove_group("1", "WORK").unwrap(),
            ("1".into(), "Work".into())
        );
        let rec = &sw.roster()["accounts"]["1"];
        assert_eq!(rec["groups"], json!(["Home"]));
        assert_eq!(
            rec["group"], "Home",
            "the mirror follows the new first group"
        );

        let err = sw.remove_group("1", "Work").unwrap_err().to_string();
        assert!(err.contains("not in group 'Work'"), "{err}");
        assert!(sw.remove_group("2", "Home").is_err());

        sw.remove_group("1", "Home").unwrap();
        let rec = &sw.roster()["accounts"]["1"];
        assert!(
            rec.get("group").is_none() && rec.get("groups").is_none(),
            "{rec}"
        );
    }

    #[test]
    fn a_seventeenth_group_is_refused() {
        let sw = TestSwitcher::linux().with_roster(roster(&[("1", None)]));
        for i in 0..GROUPS_MAX {
            sw.add_group("1", &format!("g{i}")).unwrap();
        }
        let err = sw.add_group("1", "one more").unwrap_err().to_string();
        assert!(err.contains("already in 16 groups"), "{err}");
        assert_eq!(
            sw.add_group("1", "G0").unwrap(),
            added("1", "g0", false),
            "re-adding a member is still fine at the cap"
        );
        assert_eq!(
            sw.roster()["accounts"]["1"]["groups"]
                .as_array()
                .unwrap()
                .len(),
            GROUPS_MAX
        );
    }

    #[test]
    fn an_old_roster_with_only_group_still_reads_and_upgrades_on_add() {
        let sw = TestSwitcher::linux().with_roster(roster(&[("1", Some("Work")), ("2", None)]));
        let data = sw.roster();
        assert_eq!(groups_of_slot(&data, "1"), vec!["Work"]);
        assert!(groups_of_slot(&data, "2").is_empty());
        sw.add_group("1", "Home").unwrap();
        assert_eq!(
            sw.roster()["accounts"]["1"]["groups"],
            json!(["Work", "Home"])
        );
    }

    #[test]
    fn sharing_any_group_is_the_same_group() {
        let data = multi(&[
            ("1", &["Work", "Home"]),
            ("2", &["home"]),
            ("3", &[]),
            ("4", &["Client A"]),
            ("5", &[]),
        ]);
        let d = obj(&data);
        assert!(same_group(d, "1", "2"), "share 'Home' (case-insensitive)");
        assert!(!same_group(d, "1", "3"), "ungrouped is not grouped");
        assert!(!same_group(d, "1", "4"), "nothing shared");
        assert!(same_group(d, "3", "5"), "two ungrouped accounts");
        assert_eq!(
            same_group_first(d, Some("1"), nums(&["3", "4", "2"])),
            nums(&["2", "3", "4"])
        );
        assert_eq!(
            left_group_note(d, "1", "4").unwrap(),
            "No account in groups 'Work', 'Home' could take over; switched to Account-4 in \
             group 'Client A'"
        );
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

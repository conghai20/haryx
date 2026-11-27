//! The three-way merge every synced document goes through.
//!
//! Each side is a flat map of values. `base` is what both sides agreed on at the last sync. A key
//! changed on one side only takes that side's value; a key changed on both sides to different values
//! is a conflict, won by the side that changed later (its `changed_at`), and the losing value is
//! recorded so the user can see what was overwritten.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

pub type Values = BTreeMap<String, Value>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    Local,
    Remote,
}

/// A conflicting key: which side's value was kept, and the value that was lost (`None`: unset).
#[derive(Debug, Clone, PartialEq)]
pub struct Overwrite {
    pub key: String,
    pub kept: Side,
    pub lost: Option<Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Merged {
    pub values: Values,
    pub overwritten: Vec<Overwrite>,
}

/// One side of a merge: its values and when they last changed (Unix seconds).
#[derive(Debug, Clone, Copy)]
pub struct Doc<'a> {
    pub values: &'a Values,
    pub changed_at: i64,
}

/// Merge `local` and `remote` against `base`.
///
/// No remote document (`None`) means nothing to merge with: local stands as it is. That holds even
/// when a base exists, because a missing row is a reset or a forget, never "delete every value".
/// No base (first sync on this machine) makes every differing key a conflict.
pub fn merge(local: Doc<'_>, base: Option<&Values>, remote: Option<Doc<'_>>) -> Merged {
    let Some(remote) = remote else {
        return Merged {
            values: local.values.clone(),
            overwritten: Vec::new(),
        };
    };
    let empty = Values::new();
    let base_values = base.unwrap_or(&empty);
    let keys: BTreeSet<&String> = local
        .values
        .keys()
        .chain(remote.values.keys())
        .chain(base_values.keys())
        .collect();
    let mut values = Values::new();
    let mut overwritten = Vec::new();
    for key in keys {
        let l = local.values.get(key);
        let r = remote.values.get(key);
        // With no base (first sync here), a key on one side only is new there, not a removal on
        // the other: the union keeps it.
        let (local_changed, remote_changed) = match base {
            Some(_) => {
                let b = base_values.get(key);
                (l != b, r != b)
            }
            None => (l.is_some(), r.is_some()),
        };
        let pick = match (local_changed, remote_changed) {
            (_, false) => l,
            (false, true) => r,
            (true, true) if l == r => l,
            (true, true) if local.changed_at >= remote.changed_at => {
                overwritten.push(Overwrite {
                    key: key.clone(),
                    kept: Side::Local,
                    lost: r.cloned(),
                });
                l
            }
            (true, true) => {
                overwritten.push(Overwrite {
                    key: key.clone(),
                    kept: Side::Remote,
                    lost: l.cloned(),
                });
                r
            }
        };
        if let Some(value) = pick {
            values.insert(key.clone(), value.clone());
        }
    }
    Merged {
        values,
        overwritten,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn v(pairs: &[(&str, Value)]) -> Values {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), v.clone()))
            .collect()
    }

    fn side(values: &Values, changed_at: i64) -> Doc<'_> {
        Doc { values, changed_at }
    }

    #[test]
    fn changes_on_different_keys_both_survive() {
        let base = v(&[("a", json!(1)), ("b", json!(1))]);
        let local = v(&[("a", json!(2)), ("b", json!(1))]);
        let remote = v(&[("a", json!(1)), ("b", json!(3)), ("c", json!(true))]);
        let m = merge(side(&local, 10), Some(&base), Some(side(&remote, 20)));
        assert_eq!(
            m.values,
            v(&[("a", json!(2)), ("b", json!(3)), ("c", json!(true))])
        );
        assert!(m.overwritten.is_empty());
    }

    #[test]
    fn a_conflict_goes_to_the_later_change_and_records_the_loser() {
        let base = v(&[("a", json!(1))]);
        let local = v(&[("a", json!(2))]);
        let remote = v(&[("a", json!(3))]);
        let later_remote = merge(side(&local, 10), Some(&base), Some(side(&remote, 20)));
        assert_eq!(later_remote.values, v(&[("a", json!(3))]));
        assert_eq!(
            later_remote.overwritten,
            [Overwrite {
                key: "a".into(),
                kept: Side::Remote,
                lost: Some(json!(2))
            }]
        );
        let later_local = merge(side(&local, 30), Some(&base), Some(side(&remote, 20)));
        assert_eq!(later_local.values, v(&[("a", json!(2))]));
        assert_eq!(later_local.overwritten[0].kept, Side::Local);
    }

    #[test]
    fn an_unset_on_one_side_is_a_change_like_any_other() {
        let base = v(&[("a", json!(1)), ("b", json!(1))]);
        let local = v(&[("b", json!(1))]);
        let remote = v(&[("a", json!(1))]);
        let m = merge(side(&local, 10), Some(&base), Some(side(&remote, 20)));
        assert_eq!(m.values, Values::new(), "both removals carry over");
    }

    #[test]
    fn the_same_change_on_both_sides_is_no_conflict() {
        let base = v(&[("a", json!(1))]);
        let both = v(&[("a", json!(5))]);
        let m = merge(side(&both, 10), Some(&base), Some(side(&both, 20)));
        assert_eq!(m.values, both);
        assert!(m.overwritten.is_empty());
    }

    #[test]
    fn a_first_sync_unions_and_resolves_differences_by_time() {
        let local = v(&[("a", json!(1)), ("only_local", json!("x"))]);
        let remote = v(&[("a", json!(2)), ("only_remote", json!("y"))]);
        let m = merge(side(&local, 10), None, Some(side(&remote, 20)));
        assert_eq!(
            m.values,
            v(&[
                ("a", json!(2)),
                ("only_local", json!("x")),
                ("only_remote", json!("y"))
            ])
        );
        assert_eq!(m.overwritten.len(), 1);
    }

    #[test]
    fn a_missing_remote_document_never_deletes_local_values() {
        let base = v(&[("a", json!(1))]);
        let local = v(&[("a", json!(1))]);
        let m = merge(side(&local, 10), Some(&base), None);
        assert_eq!(m.values, local);
    }
}

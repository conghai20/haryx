//! `diff`: compare a saved profile with the live setup or with another saved profile.

use std::collections::{BTreeMap, BTreeSet};

use super::catalog::Category;
use super::items::{self, Target};
use super::plan::{self, Action, ApplyRequest, Ctx};
use super::{Selection, walk};
use crate::error::Result;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    /// Only in the first side.
    OnlyLeft,
    /// Only in the second side.
    OnlyRight,
    Differs,
    Same,
}

impl State {
    pub fn as_str(self) -> &'static str {
        match self {
            State::OnlyLeft => "only-left",
            State::OnlyRight => "only-right",
            State::Differs => "differs",
            State::Same => "same",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub category: Category,
    pub id: String,
    pub state: State,
}

/// A saved profile (left) against the live setup (right).
pub fn against_live(ctx: &Ctx<'_>) -> Result<Vec<Line>> {
    let req = ApplyRequest {
        selection: Selection::everything(),
        prune: true,
    };
    let built = plan::build(ctx, &req)?;
    Ok(built
        .entries
        .into_iter()
        .map(|e| Line {
            category: e.item.category,
            id: e.item.id,
            state: match e.action {
                Action::Add => State::OnlyLeft,
                Action::Remove => State::OnlyRight,
                Action::Same => State::Same,
                Action::Update | Action::Merge | Action::Skip => State::Differs,
            },
        })
        .collect())
}

/// Two saved profiles.
pub fn between(left: &Ctx<'_>, right: &Ctx<'_>) -> Result<Vec<Line>> {
    let all = Selection::everything();
    let l = fingerprints(left, &all)?;
    let r = fingerprints(right, &all)?;
    let ids: BTreeSet<&String> = l.keys().chain(r.keys()).collect();
    Ok(ids
        .into_iter()
        .map(|id| {
            let (category, state) = match (l.get(id), r.get(id)) {
                (Some((c, a)), Some((_, b))) if a == b => (*c, State::Same),
                (Some((c, _)), Some(_)) => (*c, State::Differs),
                (Some((c, _)), None) => (*c, State::OnlyLeft),
                (None, Some((c, _))) => (*c, State::OnlyRight),
                (None, None) => unreachable!("id comes from one side"),
            };
            Line {
                category,
                id: id.clone(),
                state,
            }
        })
        .collect())
}

/// id -> (category, content fingerprint) of a saved profile, in its portable form.
fn fingerprints(ctx: &Ctx<'_>, sel: &Selection) -> Result<BTreeMap<String, (Category, String)>> {
    plan::profile_items(ctx, sel)?
        .into_iter()
        .map(|item| {
            let print = match &item.target {
                Target::Path { rel } => stored_tree_hash(&items::stored_path(ctx.dir, rel))?,
                Target::Extra { stored, .. } => stored_tree_hash(&ctx.dir.join(stored))?,
                Target::Json { doc, key } => ctx
                    .stored_json(*doc)?
                    .as_ref()
                    .and_then(|d| items::json_value(*doc, d, key))
                    .map(|v| v.to_string())
                    .unwrap_or_default(),
                Target::Plugin(p) => p.version.clone().unwrap_or_default(),
                Target::Credentials => "credentials".into(),
            };
            Ok((item.id, (item.category, print)))
        })
        .collect()
}

fn stored_tree_hash(path: &std::path::Path) -> Result<String> {
    plan::live_hash(path).or_else(|_| Ok(walk::sha256_hex(b"")))
}

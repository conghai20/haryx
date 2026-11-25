//! What this machine remembers between syncs: `<store>/sync/state.json` (0600).
//!
//! Per row, the version last seen and the values both sides agreed on (the merge base); the accounts
//! seen only on other machines; and the recent overwrites. It belongs to one HaryX user and project:
//! signing in as someone else starts it afresh.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::SyncError;
use super::merge::{Side, Values};
use crate::account::vault::write_private;

/// Overwrites kept for `sync status`.
const MAX_OVERWRITES: usize = 50;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RowState {
    pub version: i64,
    pub base: Values,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteAccount {
    pub email: String,
    pub organization_name: String,
    pub alias: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OverwriteRecord {
    /// Unix seconds.
    pub at: i64,
    pub kind: String,
    /// The setting or field, for example `autoswitch.threshold` or `you@example.com: alias`.
    pub label: String,
    pub kept: Side,
    /// The value that lost; `None` when it was unset, or when it is a secret.
    pub lost: Option<Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct State {
    pub user: String,
    pub project: String,
    pub last_sync_at: Option<i64>,
    /// `kind/key` → state.
    pub rows: BTreeMap<String, RowState>,
    pub other_machine_accounts: Vec<RemoteAccount>,
    pub overwritten: Vec<OverwriteRecord>,
}

impl State {
    pub fn path(root: &Path) -> PathBuf {
        root.join("sync").join("state.json")
    }

    /// This user's state, or a fresh one (missing, unreadable, or another user's or project's).
    pub fn load(root: &Path, user: &str, project: &str) -> State {
        let loaded = std::fs::read_to_string(Self::path(root))
            .ok()
            .and_then(|t| serde_json::from_str::<State>(&t).ok())
            .filter(|s| s.user == user && s.project == project);
        loaded.unwrap_or_else(|| State {
            user: user.to_owned(),
            project: project.to_owned(),
            ..State::default()
        })
    }

    /// Whatever state is on disk, for `sync status` (no user check).
    pub fn peek(root: &Path) -> Option<State> {
        let text = std::fs::read_to_string(Self::path(root)).ok()?;
        serde_json::from_str(&text).ok()
    }

    pub fn save(&self, root: &Path) -> Result<(), SyncError> {
        let text =
            serde_json::to_string_pretty(self).map_err(|e| SyncError::Local(e.to_string()))?;
        write_private(&root.join("sync"), "state.json", &text).map_err(SyncError::Account)
    }

    pub fn remove(root: &Path) -> Result<(), SyncError> {
        match std::fs::remove_file(Self::path(root)) {
            Err(err) if err.kind() != std::io::ErrorKind::NotFound => {
                Err(SyncError::Local(err.to_string()))
            }
            _ => Ok(()),
        }
    }

    pub fn row_id(kind: &str, key: &str) -> String {
        format!("{kind}/{key}")
    }

    pub fn record(&mut self, mut records: Vec<OverwriteRecord>) {
        self.overwritten.append(&mut records);
        let excess = self.overwritten.len().saturating_sub(MAX_OVERWRITES);
        self.overwritten.drain(..excess);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_round_trips_for_its_user_only() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = State::load(dir.path(), "u1", "p");
        state.last_sync_at = Some(5);
        state.save(dir.path()).unwrap();
        assert_eq!(State::load(dir.path(), "u1", "p").last_sync_at, Some(5));
        assert_eq!(State::load(dir.path(), "u2", "p").last_sync_at, None);
        assert_eq!(State::load(dir.path(), "u1", "other").last_sync_at, None);
        State::remove(dir.path()).unwrap();
        State::remove(dir.path()).unwrap();
        assert!(State::peek(dir.path()).is_none());
    }

    #[test]
    fn only_the_latest_overwrites_are_kept() {
        let mut state = State::default();
        let rec = |at| OverwriteRecord {
            at,
            kind: "settings".into(),
            label: "x".into(),
            kept: Side::Local,
            lost: None,
        };
        state.record((0..60).map(rec).collect());
        assert_eq!(state.overwritten.len(), MAX_OVERWRITES);
        assert_eq!(state.overwritten[0].at, 10);
    }
}

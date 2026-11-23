//! `wake.json` (settings plus per-account state) and the `wake-events.jsonl` log.

use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::config::{ConfigDoc, WakeConfig};
use crate::error::{Error, Result};
use crate::fsutil::{WriteOptions, atomic_write_text};

pub const STATE_FILE: &str = "wake.json";
pub const EVENTS_FILE: &str = "wake-events.jsonl";
pub const LOCK_FILE: &str = "wake.lock";
/// The event log is trimmed to this many lines once it grows past twice that.
const EVENTS_KEEP: usize = 500;

/// What the last tick learned about one account.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_wake_at: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_check_at: Option<f64>,
    /// When the account is next due (its window's reset plus grace).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_due: Option<f64>,
    /// `woke`, `failed:<kind>`, or `skip:<reason>`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_result: Option<String>,
    /// The epoch the last failure notification was sent for, so each failure notifies once.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notified_at: Option<f64>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Doc {
    #[serde(flatten)]
    config: ConfigDoc,
    #[serde(default)]
    state: BTreeMap<String, AccountState>,
}

/// The wake files under the haryx store directory.
#[derive(Debug, Clone)]
pub struct WakeStore {
    dir: PathBuf,
}

impl WakeStore {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        WakeStore { dir: dir.into() }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn lock_path(&self) -> PathBuf {
        self.dir.join(LOCK_FILE)
    }

    pub fn events_path(&self) -> PathBuf {
        self.dir.join(EVENTS_FILE)
    }

    fn state_path(&self) -> PathBuf {
        self.dir.join(STATE_FILE)
    }

    fn read_doc(&self) -> Result<Doc> {
        match std::fs::read_to_string(self.state_path()) {
            Ok(text) => serde_json::from_str(&text).or_else(|e| {
                log::warn!("wake: {STATE_FILE} is unreadable ({e}); starting from defaults");
                Ok(Doc::default())
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Doc::default()),
            Err(e) => Err(Error::General(format!("cannot read {STATE_FILE}: {e}"))),
        }
    }

    fn write_doc(&self, doc: &Doc) -> Result<()> {
        std::fs::create_dir_all(&self.dir)
            .map_err(|e| Error::General(format!("cannot create {}: {e}", self.dir.display())))?;
        let text = serde_json::to_string_pretty(doc)
            .map_err(|e| Error::General(format!("cannot encode {STATE_FILE}: {e}")))?;
        atomic_write_text(&self.state_path(), &text, WriteOptions::default())
            .map_err(|e| Error::General(format!("cannot write {STATE_FILE}: {e}")))
    }

    pub fn load(&self) -> Result<(WakeConfig, BTreeMap<String, AccountState>)> {
        let doc = self.read_doc()?;
        Ok((doc.config.to_config(), doc.state))
    }

    /// Replace the settings, keeping state only for accounts still opted in.
    pub fn save_config(&self, config: &WakeConfig) -> Result<()> {
        let doc = self.read_doc()?;
        let state = doc
            .state
            .into_iter()
            .filter(|(num, _)| config.accounts.contains(num))
            .collect();
        self.write_doc(&Doc {
            config: ConfigDoc::from_config(config),
            state,
        })
    }

    /// Replace the per-account state, keeping the settings as they are on disk.
    pub fn save_state(&self, state: &BTreeMap<String, AccountState>) -> Result<()> {
        let doc = self.read_doc()?;
        self.write_doc(&Doc {
            config: doc.config,
            state: state.clone(),
        })
    }

    /// Append one event line; failures only log (the log is informational).
    pub fn append_event(&self, event: &Value) {
        if let Err(e) = self.try_append(event) {
            log::warn!("wake: cannot append to {EVENTS_FILE}: {e}");
        }
    }

    fn try_append(&self, event: &Value) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        let path = self.events_path();
        let mut file = OpenOptions::new().create(true).append(true).open(&path)?;
        writeln!(file, "{event}")?;
        drop(file);
        let text = std::fs::read_to_string(&path)?;
        let lines: Vec<&str> = text.lines().collect();
        if lines.len() > EVENTS_KEEP * 2 {
            let kept = lines[lines.len() - EVENTS_KEEP..].join("\n") + "\n";
            atomic_write_text(&path, &kept, WriteOptions::default())?;
        }
        Ok(())
    }

    /// The last `limit` events, oldest first; unreadable lines are skipped.
    pub fn recent_events(&self, limit: usize) -> Vec<Value> {
        let Ok(text) = std::fs::read_to_string(self.events_path()) else {
            return Vec::new();
        };
        let events: Vec<Value> = text
            .lines()
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect();
        let skip = events.len().saturating_sub(limit);
        events.into_iter().skip(skip).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn temp_store() -> (tempfile::TempDir, WakeStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = WakeStore::new(dir.path());
        (dir, store)
    }

    #[test]
    fn missing_file_loads_defaults() {
        let (_d, store) = temp_store();
        let (config, state) = store.load().unwrap();
        assert_eq!(config, WakeConfig::default());
        assert!(state.is_empty());
    }

    #[test]
    fn config_and_state_round_trip() {
        let (_d, store) = temp_store();
        let config = WakeConfig {
            accounts: vec!["1".into(), "2".into()],
            ..WakeConfig::default()
        };
        store.save_config(&config).unwrap();
        let state = BTreeMap::from([(
            "1".to_string(),
            AccountState {
                last_wake_at: Some(10.0),
                ..AccountState::default()
            },
        )]);
        store.save_state(&state).unwrap();
        let (c, s) = store.load().unwrap();
        assert_eq!(c, config);
        assert_eq!(s, state);
    }

    #[test]
    fn opting_out_drops_that_state() {
        let (_d, store) = temp_store();
        let both = WakeConfig {
            accounts: vec!["1".into(), "2".into()],
            ..WakeConfig::default()
        };
        store.save_config(&both).unwrap();
        let state: BTreeMap<_, _> = ["1", "2"]
            .into_iter()
            .map(|n| (n.to_string(), AccountState::default()))
            .collect();
        store.save_state(&state).unwrap();
        store
            .save_config(&WakeConfig {
                accounts: vec!["2".into()],
                ..WakeConfig::default()
            })
            .unwrap();
        let (_, s) = store.load().unwrap();
        assert_eq!(s.keys().collect::<Vec<_>>(), vec!["2"]);
    }

    #[test]
    fn corrupt_file_falls_back_to_defaults() {
        let (dir, store) = temp_store();
        std::fs::write(dir.path().join(STATE_FILE), "{not json").unwrap();
        assert_eq!(store.load().unwrap().0, WakeConfig::default());
    }

    #[test]
    fn events_append_and_trim() {
        let (_d, store) = temp_store();
        for i in 0..(EVENTS_KEEP * 2 + 1) {
            store.append_event(&json!({"i": i}));
        }
        let all = store.recent_events(usize::MAX);
        assert_eq!(all.len(), EVENTS_KEEP);
        assert_eq!(all.last().unwrap()["i"], json!(EVENTS_KEEP * 2));
        assert_eq!(store.recent_events(3).len(), 3);
    }
}

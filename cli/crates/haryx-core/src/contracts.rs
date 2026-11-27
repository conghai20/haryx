//! Serde types for the JSON contracts in `packages/contracts/haryx/`: `consents.json`,
//! `machines.json`, `usage-sync.json` and the `consents` part of `whoami.json`. Field names are
//! camelCase, as in every haryx JSON. The consent, machine and usage-sync code (plan tasks 9-12)
//! builds and reads these; the fixture tests below keep them equal to the contracts.

use serde::{Deserialize, Serialize};

/// A HaryX consent purpose.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Purpose {
    Sync,
    Machines,
}

/// `consents` in `whoami.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsentState {
    pub purpose: Purpose,
    pub granted: bool,
    pub version: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConsentAction {
    Grant,
    Withdraw,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConsentEvent {
    pub action: ConsentAction,
    pub version: String,
    pub at: String,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Consent {
    pub purpose: Purpose,
    pub granted: bool,
    pub version: Option<String>,
    pub at: Option<String>,
    pub history: Vec<ConsentEvent>,
}

/// `consents.json` and `GET /api/v1/consents`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Consents {
    pub consents: Vec<Consent>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Machine {
    pub machine_id: String,
    pub name: String,
    pub os: String,
    pub version: String,
    pub last_seen_at: String,
    pub is_this_machine: bool,
    pub signed_in: bool,
}

/// `machines.json` and `GET /api/v1/machines`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Machines {
    pub machines: Vec<Machine>,
}

/// One (day, account, model) row of a usage-sync body. Tokens only: no cost, no project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageDay {
    pub day: String,
    pub account: Option<String>,
    pub model: String,
    pub replies: u64,
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write5m: u64,
    pub cache_write1h: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageQuota {
    pub at: i64,
    pub account: Option<String>,
    pub five_hour_pct: Option<f64>,
    pub seven_day_pct: Option<f64>,
}

/// The plaintext of one `usage` row, before encryption (`usage-sync.json`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageSync {
    pub schema: u32,
    pub machine_id: String,
    pub month: String,
    pub updated_at: String,
    pub days: Vec<UsageDay>,
    pub quota: Vec<UsageQuota>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use std::path::Path;

    fn fixture(name: &str) -> String {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../packages/contracts/haryx")
            .join(name);
        std::fs::read_to_string(path).unwrap()
    }

    /// Decode, re-encode, and compare with the fixture: no field is dropped or renamed.
    fn round_trips<T: Serialize + for<'a> Deserialize<'a>>(name: &str) -> T {
        let text = fixture(name);
        let value: T = serde_json::from_str(&text).unwrap();
        let again = serde_json::to_value(&value).unwrap();
        assert_eq!(
            again,
            serde_json::from_str::<Value>(&text).unwrap(),
            "{name}"
        );
        value
    }

    #[test]
    fn consents_fixture_decodes() {
        let c: Consents = round_trips("consents.json");
        assert_eq!(c.consents[0].purpose, Purpose::Sync);
        assert!(!c.consents[1].granted);
        assert_eq!(c.consents[1].history[1].action, ConsentAction::Withdraw);
    }

    #[test]
    fn machines_fixture_decodes() {
        let m: Machines = round_trips("machines.json");
        assert!(m.machines[0].is_this_machine);
        assert!(!m.machines[1].signed_in);
    }

    #[test]
    fn usage_sync_fixture_decodes_and_has_no_paths() {
        let u: UsageSync = round_trips("usage-sync.json");
        assert_eq!(u.month, "2026-11");
        assert_eq!(u.days[1].account, None);
        let text = fixture("usage-sync.json");
        assert!(!text.contains('/') && !text.contains("project") && !text.contains(".jsonl"));
    }

    #[test]
    fn whoami_consents_decode() {
        let v: Value = serde_json::from_str(&fixture("whoami.json")).unwrap();
        let c: Vec<ConsentState> = serde_json::from_value(v["consents"].clone()).unwrap();
        assert_eq!(c.len(), 2);
        assert_eq!(c[1].version, None);
    }
}

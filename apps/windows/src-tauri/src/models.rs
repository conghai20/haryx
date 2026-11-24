//! The CLI's `--json` shapes the backend reads itself (the UI reads them too, in `src/lib/types.ts`).
//! Contracts: `packages/contracts/`. Unknown fields are ignored and every optional field may be absent,
//! so a newer CLI never breaks an older app.

use serde::Deserialize;

/// `haryx list --json`.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AccountList {
    pub schema_version: i64,
    pub active_account_number: Option<i64>,
    pub accounts: Vec<Account>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub number: i64,
    pub email: String,
    pub organization_uuid: Option<String>,
    #[serde(default)]
    pub active: bool,
    #[serde(default)]
    pub usage_status: String,
    pub usage: Option<Usage>,
    pub alias: Option<String>,
    pub disabled: Option<bool>,
    pub login_expires_at: Option<jiff::Timestamp>,
    pub plan: Option<Plan>,
}

impl Account {
    /// Stable across slot moves: accounts are identified by (email, org).
    pub fn identity_key(&self) -> String {
        format!(
            "{}|{}",
            self.email,
            self.organization_uuid.as_deref().unwrap_or("")
        )
    }

    pub fn display_name(&self) -> &str {
        self.alias
            .as_deref()
            .filter(|a| !a.is_empty())
            .unwrap_or(&self.email)
    }

    pub fn status_note(&self) -> Option<&'static str> {
        match self.usage_status.as_str() {
            "ok" => None,
            "token_expired" => Some("Token expired — refreshes automatically"),
            "api_key" => Some("API key (no quota)"),
            "relogin_required" => Some("Re-login needed: use Sign In Again"),
            "foreign_credential" => Some("Credential belongs to another account"),
            "no_credentials" => Some("No stored credentials: use Sign In Again"),
            _ => Some("Usage unavailable"),
        }
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    pub five_hour: Option<UsageWindow>,
    pub seven_day: Option<UsageWindow>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UsageWindow {
    pub pct: Option<f64>,
    pub resets_at: Option<jiff::Timestamp>,
    pub countdown: Option<String>,
    pub ahead_of_pace: Option<bool>,
    pub projected_exhaustion_at: Option<jiff::Timestamp>,
    pub will_last_to_reset: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct Plan {
    pub label: Option<String>,
    pub expired: Option<bool>,
}

/// `haryx ai sessions --json`.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct SessionList {
    pub sessions: Vec<Session>,
    #[serde(default)]
    pub unreadable: usize,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub pid: i64,
    pub account: Option<i64>,
    #[serde(default)]
    pub email: String,
    #[serde(default)]
    pub profile: String,
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub cwd: String,
    #[serde(default)]
    pub started_at: i64,
    pub status: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONTRACTS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../packages/contracts");

    fn fixture(rel: &str) -> String {
        std::fs::read_to_string(format!("{CONTRACTS}/{rel}")).unwrap()
    }

    #[test]
    fn decodes_the_cswap_list_contract() {
        let list: AccountList = serde_json::from_str(&fixture("cswap/list.json")).unwrap();
        assert_eq!(list.active_account_number, Some(3));
        let active = list.accounts.iter().find(|a| a.active).unwrap();
        assert_eq!(active.display_name(), "work");
        let five = active.usage.as_ref().unwrap().five_hour.as_ref().unwrap();
        assert_eq!(five.pct, Some(88.0));
        assert!(five.resets_at.is_some(), "microsecond offsets parse");
        let relogin = list.accounts.iter().find(|a| a.number == 4).unwrap();
        assert_eq!(relogin.disabled, Some(true));
        assert_eq!(
            relogin.status_note(),
            Some("Re-login needed: use Sign In Again")
        );
    }

    #[test]
    fn decodes_the_haryx_list_contract() {
        let list: AccountList =
            serde_json::from_str(&fixture("haryx/list-unavailable.json")).unwrap();
        assert!(!list.accounts.is_empty());
        assert!(list.accounts.iter().all(|a| a.usage.is_none()));
    }

    #[test]
    fn decodes_the_sessions_contract() {
        let list: SessionList = serde_json::from_str(&fixture("haryx/sessions.json")).unwrap();
        assert_eq!(list.sessions.len(), 2);
        assert_eq!(list.sessions[0].account, Some(1));
        assert_eq!(list.sessions[1].status, None);
    }

    #[test]
    fn identity_survives_slot_moves() {
        let a: Account =
            serde_json::from_str(r#"{"number":1,"email":"a@x.com","organizationUuid":"o"}"#)
                .unwrap();
        let mut b = a.clone();
        b.number = 7;
        assert_eq!(a.identity_key(), b.identity_key());
        assert_eq!(a.identity_key(), "a@x.com|o");
    }
}

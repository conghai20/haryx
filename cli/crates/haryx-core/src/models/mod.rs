//! Data models: alias, threshold and priority validators, platform, account records. Port of
//! `claude_swap/models.py`.
//!
//! A leaf module: it must not depend on `settings` (which sits above it). The threshold bounds are
//! therefore declared here and cross-checked against `settings::SETTING_SPECS` in tests.

pub mod pyfmt;

use std::path::PathBuf;

use serde_json::{Map, Value};

use crate::error::{Error, Result};
use crate::fsutil::{WriteOptions, atomic_write_text};
use crate::usage_store::UsageEntry;

// ---------------------------------------------------------------------------------------------- alias

/// Lowercase and validate a proposed alias.
///
/// Letters/digits/`-`/`_`/`.`, non-empty, not purely digits (so an alias can never collide with a slot
/// number when resolving an account identifier), and not leading with `-` (the argument parser would
/// read it as an option, making the alias impossible to pass back into any command once set).
///
/// Shared by `haryx alias`, `haryx add --alias` and import validation so every path enforces identical
/// rules. Errors are [`Error::Validation`] (upstream `ValueError`).
pub fn normalize_alias(name: &str) -> Result<String> {
    let normalized = name.trim().to_lowercase();
    if normalized.is_empty() {
        return Err(invalid("alias cannot be empty".to_owned()));
    }
    if normalized.chars().all(char::is_numeric) {
        return Err(invalid(format!(
            "alias '{name}' cannot be purely numeric (reserved for slot numbers)"
        )));
    }
    if normalized.starts_with('-') {
        return Err(invalid(format!(
            "alias '{name}' cannot start with '-' (would be read as a command flag)"
        )));
    }
    let allowed = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit() || "_.-".contains(c);
    if !normalized.chars().all(allowed) {
        return Err(invalid(format!(
            "alias '{name}' may only contain letters, digits, '-', '_', and '.'"
        )));
    }
    Ok(normalized)
}

// ---------------------------------------------------------------------------------------------- group

/// Longest group name, in characters.
pub const GROUP_MAX_CHARS: usize = 32;

/// Trim and validate a group name. Unlike an alias it keeps its case and may hold spaces: it is a
/// label, never an identifier. Two names that differ only in case are the same group.
pub fn normalize_group(name: &str) -> Result<String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(invalid("group cannot be empty".to_owned()));
    }
    if trimmed.chars().count() > GROUP_MAX_CHARS {
        return Err(invalid(format!(
            "group '{trimmed}' is longer than {GROUP_MAX_CHARS} characters"
        )));
    }
    if trimmed.chars().any(char::is_control) {
        return Err(invalid(
            "group cannot contain control characters".to_owned(),
        ));
    }
    Ok(trimmed.to_owned())
}

fn invalid(message: String) -> Error {
    Error::Validation(message)
}

// ------------------------------------------------------------------------------------------ threshold

/// Lowest per-account switch-away threshold, in binding-window utilization percent. Mirrors the bounds
/// of `settings` key `autoswitch.threshold` (asserted equal in tests so they cannot drift apart).
pub const ACCOUNT_THRESHOLD_MIN: f64 = 50.0;
/// Highest per-account switch-away threshold.
pub const ACCOUNT_THRESHOLD_MAX: f64 = 99.9;

fn threshold_range_text() -> String {
    format!(
        "must be between {} and {}",
        pyfmt::format_g(ACCOUNT_THRESHOLD_MIN),
        pyfmt::format_g(ACCOUNT_THRESHOLD_MAX)
    )
}

/// Validate a per-account switch-away threshold given as a number.
///
/// Strict, unlike the forgiving clamp applied when *reading* a record: a user who types 150 learns about
/// it at set time rather than by watching the account never switch away. This is the single
/// write-boundary validator for the value. NaN and infinity are rejected by name rather than left to the
/// range test to catch by luck. The message always names the range.
pub fn normalize_account_threshold(value: f64) -> Result<f64> {
    if !value.is_finite() {
        return Err(invalid(format!(
            "threshold {}, got {}",
            threshold_range_text(),
            pyfmt::format_g(value)
        )));
    }
    if !(ACCOUNT_THRESHOLD_MIN..=ACCOUNT_THRESHOLD_MAX).contains(&value) {
        return Err(invalid(format!(
            "threshold {}, got {}",
            threshold_range_text(),
            pyfmt::format_g(value)
        )));
    }
    Ok(value)
}

/// [`normalize_account_threshold`] for text (the CLI hands over argv strings). Surrounding whitespace
/// is ignored; anything that is not a number is rejected with the range in the message.
pub fn normalize_account_threshold_str(value: &str) -> Result<f64> {
    let not_numeric = || {
        invalid(format!(
            "threshold {}, got {}",
            threshold_range_text(),
            pyfmt::repr_str(value)
        ))
    };
    let pct: f64 = value.trim().parse().map_err(|_| not_numeric())?;
    if !pct.is_finite() {
        return Err(not_numeric());
    }
    normalize_account_threshold(pct)
}

// ------------------------------------------------------------------------------------------- priority

/// Highest account priority tier. Higher numbers are preferred; zero is the default.
pub const ACCOUNT_PRIORITY_MAX: u32 = 1000;

fn priority_error() -> Error {
    invalid(format!(
        "priority must be an integer between 0 and {ACCOUNT_PRIORITY_MAX}"
    ))
}

/// Validate a priority tier given as a number.
pub fn normalize_account_priority(value: i64) -> Result<u32> {
    u32::try_from(value)
        .ok()
        .filter(|tier| *tier <= ACCOUNT_PRIORITY_MAX)
        .ok_or_else(priority_error)
}

/// Validate a priority tier given as text: ASCII decimal digits only (no sign), surrounding whitespace
/// ignored. An arbitrarily long hand-edited integer is rejected without being converted.
pub fn normalize_account_priority_str(value: &str) -> Result<u32> {
    let digits = value.trim();
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return Err(priority_error());
    }
    let significant = digits.trim_start_matches('0');
    let significant = if significant.is_empty() {
        "0"
    } else {
        significant
    };
    if significant.len() > ACCOUNT_PRIORITY_MAX.to_string().len() {
        return Err(priority_error());
    }
    let tier: i64 = significant.parse().map_err(|_| priority_error())?;
    normalize_account_priority(tier)
}

/// Per-account auto-switch overrides, stored in the sequence record. Defaults preserve existing
/// behaviour; `priority` is read only by the opt-in `priority-consume-first` strategy.
///
/// `threshold` is this account's own switch-away line in binding-window utilization percent; `None`
/// inherits `autoswitch.threshold`, so reserving headroom on one shared account does not tax the fleet.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AccountPolicy {
    pub threshold: Option<f64>,
    pub priority: u32,
}

// ------------------------------------------------------------------------------------------- platform

/// Supported platforms.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Platform {
    MacOs,
    Linux,
    Wsl,
    Windows,
    Unknown,
}

impl Platform {
    /// Detect the current platform. On Linux, a set `WSL_DISTRO_NAME` means WSL.
    pub fn detect() -> Self {
        let wsl = std::env::var_os("WSL_DISTRO_NAME").is_some_and(|v| !v.is_empty());
        Self::detect_for(std::env::consts::OS, wsl)
    }

    /// [`Platform::detect`] for a given `std::env::consts::OS` value and WSL marker.
    pub fn detect_for(os: &str, wsl_distro_set: bool) -> Self {
        match os {
            "macos" => Self::MacOs,
            "windows" => Self::Windows,
            "linux" if wsl_distro_set => Self::Wsl,
            "linux" => Self::Linux,
            _ => Self::Unknown,
        }
    }
}

// -------------------------------------------------------------------------------------------- account

/// Information about a managed account (one sequence record).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AccountInfo {
    pub email: String,
    pub uuid: String,
    pub organization_uuid: String,
    pub organization_name: String,
    pub added: String,
    pub number: i64,
}

impl AccountInfo {
    /// Whether this is an organization account.
    pub fn is_organization(&self) -> bool {
        !self.organization_uuid.is_empty()
    }

    /// Display label: `email [OrgName]` or `email [personal]`.
    pub fn display_label(&self) -> String {
        let tag = if self.organization_name.is_empty() {
            "personal"
        } else {
            &self.organization_name
        };
        format!("{} [{tag}]", self.email)
    }

    /// Build from a sequence record. Missing (or non-string) fields read as empty.
    pub fn from_dict(number: i64, data: &Map<String, Value>) -> Self {
        let text = |key: &str| {
            data.get(key)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        AccountInfo {
            email: text("email"),
            uuid: text("uuid"),
            organization_uuid: text("organizationUuid"),
            organization_name: text("organizationName"),
            added: text("added"),
            number,
        }
    }

    /// The record's JSON form (the number is the record's key, not a field).
    pub fn to_dict(&self) -> Map<String, Value> {
        [
            ("email", &self.email),
            ("uuid", &self.uuid),
            ("organizationUuid", &self.organization_uuid),
            ("organizationName", &self.organization_name),
            ("added", &self.added),
        ]
        .into_iter()
        .map(|(key, value)| (key.to_owned(), Value::String(value.clone())))
        .collect()
    }
}

/// Whether a per-account listing draws `email`'s org tag under `ui.orgTag`.
///
/// `on` always, `off` never, `auto` only when another managed account (`emails` lists every one,
/// `email` included) has the same email.
pub fn org_tag_shown(mode: &str, email: &str, emails: &[&str]) -> bool {
    mode == "on" || (mode == "auto" && emails.iter().filter(|e| **e == email).count() > 1)
}

// ------------------------------------------------------------------------------------------ snapshot

/// One managed account as seen by interactive UIs (the TUI).
///
/// `usage` is the store-backed [`UsageEntry`] read model; display code reads `usage.last_good` /
/// `age_s` directly (may show old data, annotated with its age), while `usage.sentinel` carries
/// derived states ("api key", "token expired", ...) that replace the bars entirely.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AccountSnapshot {
    pub number: String,
    pub email: String,
    pub org_name: String,
    pub org_uuid: String,
    pub is_active: bool,
    /// `"oauth"` or `"api_key"`.
    pub kind: String,
    pub switchable: bool,
    pub usage: UsageEntry,
    pub alias: String,
    /// Held out of auto-rotation (still a valid explicit target).
    pub disabled: bool,
    /// Per-account auto-switch overrides.
    pub policy: AccountPolicy,
    /// Stored subscription plan record (`subscription::PlanStore` shape), or `None` for a slot whose
    /// plan has never been fetched.
    pub subscription_plan: Option<Map<String, Value>>,
    /// Stored refresh token's expiry (epoch seconds), or `None` when missing/unreadable; feeds the
    /// "re-login in 5d 12h" note every account view shows while the login is due.
    pub login_expires_at: Option<f64>,
}

impl AccountSnapshot {
    /// Org tag for display: the org name, or `personal`.
    pub fn display_tag(&self) -> &str {
        if self.org_name.is_empty() {
            "personal"
        } else {
            &self.org_name
        }
    }
}

/// Coherent one-pass view of every managed account: metadata, active detection and usage entries
/// all come from the same collect pass, so a consumer never sees an account list and usage table
/// that disagree.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AccountsSnapshot {
    pub active_number: Option<String>,
    pub accounts: Vec<AccountSnapshot>,
    pub taken_at: f64,
}

// -------------------------------------------------------------------------------- switch transaction

/// What a [`SwitchTransaction`] needs from the switcher to undo its steps.
pub trait SwitchRollback {
    /// Write the live credential back (step `credentials_written`).
    fn write_credentials(&self, credentials: &str) -> Result<()>;
    /// Point `activeAccountNumber` back at `account_number` and stamp `lastUpdated` (step
    /// `sequence_updated`). A no-op when there is no sequence data.
    fn restore_active_account(&self, account_number: i64) -> Result<()>;
}

/// A switch in progress that can be rolled back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SwitchTransaction {
    pub original_credentials: String,
    pub original_config: String,
    pub original_account_num: String,
    pub original_email: String,
    pub config_path: PathBuf,
    pub completed_steps: Vec<String>,
}

impl SwitchTransaction {
    /// Record a completed step.
    pub fn record_step(&mut self, step: &str) {
        self.completed_steps.push(step.to_owned());
    }

    /// Roll back every completed step in reverse order. Returns `false` if any step failed; a failed
    /// step is logged and the rest are still attempted.
    pub fn rollback(&self, switcher: &dyn SwitchRollback) -> bool {
        let mut success = true;
        for step in self.completed_steps.iter().rev() {
            match self.rollback_step(step, switcher) {
                Ok(()) => log::info!("Rolled back step: {step}"),
                Err(err) => {
                    log::error!("Failed to rollback step {step}: {err}");
                    success = false;
                }
            }
        }
        success
    }

    fn rollback_step(&self, step: &str, switcher: &dyn SwitchRollback) -> Result<()> {
        match step {
            "credentials_written" => switcher.write_credentials(&self.original_credentials),
            // By rename, never in place (upstream #419): see `atomic_write_text`.
            "config_written" => Ok(atomic_write_text(
                &self.config_path,
                &self.original_config,
                WriteOptions::default(),
            )?),
            "sequence_updated" => {
                let number = self.original_account_num.trim().parse().map_err(|_| {
                    Error::General(format!(
                        "invalid literal for account number: {}",
                        pyfmt::repr_str(&self.original_account_num)
                    ))
                })?;
                switcher.restore_active_account(number)
            }
            _ => Ok(()),
        }
    }
}

/// Current UTC timestamp, `YYYY-MM-DDTHH:MM:SSZ`.
pub fn get_timestamp() -> String {
    jiff::Timestamp::now()
        .strftime("%Y-%m-%dT%H:%M:%SZ")
        .to_string()
}

#[cfg(test)]
mod tests;

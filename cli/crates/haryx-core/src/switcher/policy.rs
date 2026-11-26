//! Per-account auto-switch policy: rotation eligibility, disable/enable, threshold and priority
//! (upstream `switcher.py` ~2120-2400).

use serde_json::Value;

use super::{JsonObject, Switcher, accounts, record, text, truthy};
use crate::error::{Error, Result};
use crate::models::pyfmt::format_g;
use crate::models::{
    AccountPolicy, get_timestamp, normalize_account_priority, normalize_account_priority_str,
    normalize_account_threshold, normalize_account_threshold_str,
};
use crate::printer::{self, accent, dimmed};

/// The fields a user puts on a managed slot, as opposed to the identity fields a re-login rewrites.
/// `add --slot` rebuilds a record wholesale and carries exactly these across.
pub const USER_ACCOUNT_SETTINGS: [&str; 6] = [
    "alias",
    "disabled",
    "threshold",
    "priority",
    "group",
    "groups",
];

/// `data["sequence"]` as slot-number strings, in order (non-integers skipped).
pub(crate) fn sequence_numbers(data: &JsonObject) -> Vec<String> {
    data.get("sequence")
        .and_then(Value::as_array)
        .map(|seq| seq.iter().filter_map(slot_string).collect())
        .unwrap_or_default()
}

/// `str(n)` of a sequence entry.
fn slot_string(value: &Value) -> Option<String> {
    match value {
        Value::Number(n) => n
            .as_i64()
            .map(|i| i.to_string())
            .or_else(|| Some(n.to_string())),
        Value::String(s) => Some(s.clone()),
        _ => None,
    }
}

/// A mutable view of `data["accounts"][num]`.
pub(crate) fn record_mut<'a>(data: &'a mut JsonObject, num: &str) -> Option<&'a mut JsonObject> {
    data.get_mut("accounts")?
        .as_object_mut()?
        .get_mut(num)?
        .as_object_mut()
}

impl Switcher {
    /// Slots, in rotation order, eligible for AUTOMATIC selection: backed up and not disabled. Disabled
    /// slots stay valid explicit `switch <num|email>` targets.
    pub fn switchable_account_numbers(&self) -> Result<Vec<String>> {
        let data = self.sequence_or_empty()?;
        Ok(sequence_numbers(&data)
            .into_iter()
            .filter(|num| self.switchable_in(&data, num) && !Self::disabled_from_data(&data, num))
            .collect())
    }

    /// Whether a slot is flagged out of rotation in already-loaded data.
    pub fn disabled_from_data(data: &JsonObject, account_num: &str) -> bool {
        record(data, account_num).is_some_and(|r| truthy(r.get("disabled")))
    }

    /// The user-set fields of `record`, to reattach to a rebuilt one (absent or null keys stay absent).
    pub fn carried_settings(record: Option<&JsonObject>) -> JsonObject {
        let Some(record) = record.filter(|r| !r.is_empty()) else {
            return JsonObject::new();
        };
        USER_ACCOUNT_SETTINGS
            .iter()
            .filter_map(|key| {
                record
                    .get(*key)
                    .filter(|v| !v.is_null())
                    .map(|v| ((*key).to_owned(), v.clone()))
            })
            .collect()
    }

    /// A slot's policy from already-loaded data. Degrades, never fails: an unusable hand-edited value
    /// reads as the default (threshold `None` = inherit, priority 0).
    pub fn policy_from_data(data: &JsonObject, account_num: &str) -> AccountPolicy {
        let Some(rec) = record(data, account_num) else {
            return AccountPolicy::default();
        };
        let threshold = match rec.get("threshold") {
            Some(Value::Number(n)) => n.as_f64().and_then(|f| normalize_account_threshold(f).ok()),
            Some(Value::String(s)) => normalize_account_threshold_str(s).ok(),
            _ => None,
        };
        let priority = match rec.get("priority") {
            None => 0,
            Some(Value::Number(n)) => n
                .as_i64()
                .and_then(|i| normalize_account_priority(i).ok())
                .unwrap_or(0),
            Some(Value::String(s)) => normalize_account_priority_str(s).unwrap_or(0),
            Some(_) => 0,
        };
        AccountPolicy {
            threshold,
            priority,
        }
    }

    /// Every slot's policy from ONE roster read, keyed by slot number in sequence order.
    pub fn account_policies(&self) -> Result<Vec<(String, AccountPolicy)>> {
        let data = self.sequence_or_empty()?;
        Ok(sequence_numbers(&data)
            .into_iter()
            .map(|num| {
                let policy = Self::policy_from_data(&data, &num);
                (num, policy)
            })
            .collect())
    }

    pub fn is_account_disabled(&self, account_num: &str) -> Result<bool> {
        Ok(Self::disabled_from_data(
            &self.sequence_or_empty()?,
            account_num,
        ))
    }

    /// Disabled slots, in sequence order.
    pub fn disabled_account_numbers(&self) -> Result<Vec<String>> {
        let data = self.sequence_or_empty()?;
        Ok(sequence_numbers(&data)
            .into_iter()
            .filter(|num| Self::disabled_from_data(&data, num))
            .collect())
    }

    /// Resolve an identifier for a policy write: `(account_num, email, data)`. Every failure happens
    /// before any write.
    fn resolve_for_policy_write(&self, identifier: &str) -> Result<(String, String, JsonObject)> {
        if !self.sequence_file.exists() {
            return Err(Error::Config("No accounts are managed yet".into()));
        }
        let (num, email, _) = self.resolve_account(identifier)?;
        let data = self.sequence_or_empty()?;
        if record(&data, &num).is_none() {
            return Err(Error::AccountNotFound(format!(
                "Account-{num} does not exist"
            )));
        }
        Ok((num, email, data))
    }

    /// Set (`Some`) or clear (`None`) one account's own switch-away threshold. The value is validated
    /// before anything is resolved or written; clearing pops the key (never writes `null`).
    pub fn set_account_threshold(&self, identifier: &str, threshold: Option<f64>) -> Result<()> {
        let normalized = threshold.map(normalize_account_threshold).transpose()?;
        let (num, email, mut data) = self.resolve_for_policy_write(identifier)?;
        let Some(rec) = record_mut(&mut data, &num) else {
            return Err(Error::AccountNotFound(format!(
                "Account-{num} does not exist"
            )));
        };
        let current = rec.get("threshold").and_then(Value::as_f64);
        match normalized {
            None if !rec.contains_key("threshold") => {
                println!(
                    "{}",
                    dimmed(&format!(
                        "Account-{num} ({email}) has no threshold override — already using the global \
                         default."
                    ))
                );
                return Ok(());
            }
            Some(value) if current == Some(value) => {
                println!(
                    "{}",
                    dimmed(&format!(
                        "Account-{num} ({email}) is already at {}%.",
                        format_g(value)
                    ))
                );
                return Ok(());
            }
            None => {
                rec.shift_remove("threshold");
            }
            Some(value) => {
                rec.insert("threshold".into(), Value::from(value));
            }
        }
        data.insert("lastUpdated".into(), Value::String(get_timestamp()));
        self.write_json(&self.sequence_file, &data)?;
        match normalized {
            None => {
                log::info!("Cleared threshold for account {num}: {email}");
                println!(
                    "{} threshold for Account-{num} ({email}) — now using the global default.",
                    accent("Cleared")
                );
            }
            Some(value) => {
                log::info!(
                    "Set threshold {} for account {num}: {email}",
                    format_g(value)
                );
                println!(
                    "{} Account-{num} ({email}) threshold to {}%.",
                    accent("Set"),
                    format_g(value)
                );
            }
        }
        Ok(())
    }

    /// Set a priority tier, or remove it with `None`/0. Validated before writing.
    pub fn set_account_priority(&self, identifier: &str, priority: Option<i64>) -> Result<()> {
        let normalized = priority
            .map(normalize_account_priority)
            .transpose()?
            .unwrap_or(0);
        let (num, email, mut data) = self.resolve_for_policy_write(identifier)?;
        let Some(rec) = record_mut(&mut data, &num) else {
            return Err(Error::AccountNotFound(format!(
                "Account-{num} does not exist"
            )));
        };
        let changed = if normalized > 0 {
            let changed =
                rec.get("priority").and_then(Value::as_i64) != Some(i64::from(normalized));
            rec.insert("priority".into(), Value::from(normalized));
            changed
        } else {
            rec.shift_remove("priority").is_some()
        };
        if changed {
            data.insert("lastUpdated".into(), Value::String(get_timestamp()));
            self.write_json(&self.sequence_file, &data)?;
        }
        println!(
            "Account-{num} ({email}) priority: {normalized} (used by priority-consume-first)."
        );
        Ok(())
    }

    /// Hold an account out of AUTOMATIC selection (`disabled = true`) or return it. The account stays
    /// managed and switchable by name; re-enabling restores its original rotation position.
    pub fn set_account_disabled(&self, identifier: &str, disabled: bool) -> Result<()> {
        if !self.sequence_file.exists() {
            return Err(Error::Config("No accounts are managed yet".into()));
        }
        let (num, email, _) = self.resolve_account(identifier)?;
        let mut data = self.sequence_or_empty()?;
        let Some(rec) = record_mut(&mut data, &num).filter(|r| !r.is_empty()) else {
            return Err(Error::AccountNotFound(format!(
                "Account-{num} does not exist"
            )));
        };
        let verb = if disabled { "disabled" } else { "enabled" };
        if truthy(rec.get("disabled")) == disabled {
            println!(
                "{}",
                dimmed(&format!("Account-{num} ({email}) is already {verb}."))
            );
            return Ok(());
        }
        if disabled {
            rec.insert("disabled".into(), Value::Bool(true));
        } else {
            rec.shift_remove("disabled");
        }
        data.insert("lastUpdated".into(), Value::String(get_timestamp()));
        self.write_json(&self.sequence_file, &data)?;
        let capitalized = if disabled { "Disabled" } else { "Enabled" };
        log::info!("{capitalized} account {num}: {email}");
        println!("{} Account-{num} ({email}).", accent(capitalized));
        if !disabled {
            println!("{}", dimmed("  It is back in the rotation."));
            return Ok(());
        }
        if active_number(&data).as_deref() == Some(num.as_str()) {
            println!(
                "{}",
                dimmed(
                    "  It is the active account — it stays live until you switch away; it just won't \
                     be an automatic switch target."
                )
            );
        }
        if self.switchable_account_numbers()?.is_empty() {
            printer::warning(&format!(
                "  No accounts remain in rotation — auto-switch and bare switch have nothing to pick. \
                 Re-enable one with {} enable <num|email>.",
                crate::PROG
            ));
        }
        Ok(())
    }

    /// Stored email for a slot; `""` when unknown.
    pub fn account_email(&self, account_num: &str) -> Result<String> {
        let data = self.sequence_or_empty()?;
        Ok(accounts(&data)
            .and_then(|a| a.get(account_num))
            .and_then(Value::as_object)
            .map(|r| text(r, "email").to_owned())
            .unwrap_or_default())
    }
}

/// `str(data["activeAccountNumber"])`, `None` when null or missing.
pub(crate) fn active_number(data: &JsonObject) -> Option<String> {
    data.get("activeAccountNumber").and_then(slot_string)
}

//! Account lookup and the light roster edits: `resolve_account`, aliases, directory mappings, the live
//! slot, and the uuid backfill (upstream `switcher.py` ~1068-1172, ~1812-1858, ~2397-2490).

use std::path::Path;

use serde_json::Value;

use super::policy::record_mut;
use super::{Switcher, accounts, record, slot_key, text};
use crate::error::{Error, Result};
use crate::mappings::{MappingEntry, MappingStore};
use crate::models::{get_timestamp, normalize_alias};
use crate::printer::{bolded, dimmed, muted};
use crate::settings::load_ui_settings;

/// One set alias: `(account_num, alias, email)`.
pub type AliasRow = (String, String, String);

impl Switcher {
    fn not_found(identifier: &str) -> Error {
        Error::AccountNotFound(format!("No account found with identifier: {identifier}"))
    }

    fn missing(num: &str) -> Error {
        Error::AccountNotFound(format!("Account-{num} does not exist"))
    }

    /// Resolve NUM|ALIAS|EMAIL to `(account_num, email, organization_uuid)`. Ambiguity is a hard error
    /// ([`Error::Config`]), not a prompt: callers such as session mode need a deterministic answer.
    pub fn resolve_account(&self, identifier: &str) -> Result<(String, String, String)> {
        self.get_sequence_data_migrated()?;
        let num = self
            .resolve_account_identifier(identifier)?
            .ok_or_else(|| Self::not_found(identifier))?;
        let data = self.sequence_or_empty()?;
        let rec = record(&data, &num).ok_or_else(|| Self::missing(&num))?;
        Ok((
            num.clone(),
            text(rec, "email").to_owned(),
            text(rec, "organizationUuid").to_owned(),
        ))
    }

    /// Set (or rename) the alias of the account `identifier` names (a number, email, or existing alias).
    /// Returns `(account_num, normalized_alias)`.
    pub fn set_alias(&self, identifier: &str, alias: &str) -> Result<(String, String)> {
        self.refuse_session_shell()?;
        let normalized = normalize_alias(alias)?;
        self.get_sequence_data_migrated()?;
        let num = self
            .resolve_account_identifier(identifier)?
            .ok_or_else(|| Self::not_found(identifier))?;
        let mut data = self.sequence_or_empty()?;
        if record(&data, &num).is_none() {
            return Err(Self::missing(&num));
        }
        if let Some(conflict) = self.alias_in_use(&normalized, Some(&num))? {
            return Err(Error::Config(format!(
                "Alias '{normalized}' is already used by account {conflict}"
            )));
        }
        if let Some(rec) = record_mut(&mut data, &num) {
            rec.insert("alias".into(), Value::String(normalized.clone()));
        }
        data.insert("lastUpdated".into(), Value::String(get_timestamp()));
        self.write_json(&self.sequence_file, &data)?;
        Ok((num, normalized))
    }

    /// Clear an account's alias; returns the account number. Idempotent.
    pub fn unset_alias(&self, identifier: &str) -> Result<String> {
        self.refuse_session_shell()?;
        self.get_sequence_data_migrated()?;
        let num = self
            .resolve_account_identifier(identifier)?
            .ok_or_else(|| Self::not_found(identifier))?;
        let mut data = self.sequence_or_empty()?;
        if record(&data, &num).is_none() {
            return Err(Self::missing(&num));
        }
        let removed = record_mut(&mut data, &num)
            .and_then(|rec| rec.shift_remove("alias"))
            .is_some();
        if removed {
            data.insert("lastUpdated".into(), Value::String(get_timestamp()));
            self.write_json(&self.sequence_file, &data)?;
        }
        Ok(num)
    }

    /// Every set alias, in slot-number order.
    pub fn list_aliases(&self) -> Result<Vec<AliasRow>> {
        let data = self.get_sequence_data_migrated()?.unwrap_or_default();
        let mut rows: Vec<AliasRow> = accounts(&data)
            .map(|accts| {
                accts
                    .iter()
                    .filter_map(|(num, acc)| {
                        let acc = acc.as_object()?;
                        let alias = text(acc, "alias");
                        (!alias.is_empty())
                            .then(|| (num.clone(), alias.to_owned(), text(acc, "email").to_owned()))
                    })
                    .collect()
            })
            .unwrap_or_default();
        rows.sort_by_key(|row| slot_key(&row.0));
        Ok(rows)
    }

    // --- directory mappings ------------------------------------------------------------------------

    /// Resolve a directory to its mapped slot for `haryx run`: `(None, None)` when no mapping covers it,
    /// `(None, Some(email))` when the mapped account was removed, `(Some(slot), Some(email))` otherwise.
    pub fn slot_for_directory(&self, directory: &Path) -> Result<(Option<String>, Option<String>)> {
        let Some((_, entry)) = MappingStore::new(&self.backup_dir).resolve(directory)? else {
            return Ok((None, None));
        };
        let entry = MappingEntry::from_value(&entry);
        let seq = self.get_sequence_data_migrated()?.unwrap_or_default();
        let slot = Self::find_account_slot(&seq, &entry.email, &entry.organization_uuid);
        Ok((slot, Some(entry.email)))
    }

    /// Print every directory → account mapping (`haryx map` with no account).
    pub fn list_mappings(&self) -> Result<()> {
        let mappings = MappingStore::new(&self.backup_dir).all();
        if mappings.is_empty() {
            println!("{}", dimmed("No directory mappings yet."));
            println!(
                "{}",
                muted(&format!(
                    "Map one with: {} map <NUM|EMAIL> [PATH]",
                    crate::PROG
                ))
            );
            return Ok(());
        }
        let seq = self.get_sequence_data_migrated()?.unwrap_or_default();
        let tag_mode = load_ui_settings(&self.backup_dir).org_tag;
        let emails: Vec<&str> = accounts(&seq)
            .map(|a| {
                a.values()
                    .map(|acc| {
                        acc.as_object()
                            .map(|o| text(o, "email"))
                            .unwrap_or_default()
                    })
                    .collect()
            })
            .unwrap_or_default();
        println!("{}", bolded("Directory mappings:"));
        let mut paths: Vec<&String> = mappings.keys().collect();
        paths.sort();
        for path in paths {
            let entry = MappingEntry::from_value(&mappings[path]);
            let arrow = dimmed("→");
            match Self::find_account_slot(&seq, &entry.email, &entry.organization_uuid) {
                Some(slot) => {
                    let org_name = record(&seq, &slot)
                        .map(|r| text(r, "organizationName"))
                        .unwrap_or_default();
                    let tag = Self::tag_suffix(
                        &tag_mode,
                        &emails,
                        &entry.email,
                        org_name,
                        &entry.organization_uuid,
                    );
                    println!("  {path} {arrow} {slot}: {}{tag}", entry.email);
                }
                None => println!(
                    "  {path} {arrow} {} {}",
                    entry.email,
                    muted("(account removed)")
                ),
            }
        }
        Ok(())
    }

    // --- the live slot -----------------------------------------------------------------------------

    /// Slot of the live login; `None` when there is none OR it is unmanaged. Deliberately no fallback to
    /// the recorded `activeAccountNumber`: a guessed slot could have the wrong account's usage evaluated
    /// and an unowned login overwritten. [`Self::has_live_login`] tells the two `None`s apart.
    pub fn current_account_number(&self) -> Result<Option<String>> {
        let Some((email, org)) = self.get_current_account() else {
            return Ok(None);
        };
        let data = self.sequence_or_empty()?;
        Ok(Self::find_account_slot(&data, &email, &org))
    }

    /// Whether `.claude.json` carries any live account identity.
    pub fn has_live_login(&self) -> bool {
        self.get_current_account().is_some()
    }

    /// Record a resolved uuid on a slot that has none (never rewrites one). With `expected_email` /
    /// `expected_org`, the slot must still hold that identity under the lock. Takes the account lock:
    /// the caller must not hold it.
    pub fn backfill_account_uuid(
        &self,
        account_num: &str,
        uuid: &str,
        expected_email: Option<&str>,
        expected_org: Option<&str>,
    ) -> Result<()> {
        if uuid.is_empty() {
            return Ok(());
        }
        let _lock = self.lock()?;
        let mut data = self.sequence_or_empty()?;
        let Some(acct) = record_mut(&mut data, account_num) else {
            return Ok(());
        };
        let fill = text(acct, "uuid").trim().is_empty()
            && expected_email.is_none_or(|e| acct.get("email").and_then(Value::as_str) == Some(e))
            && expected_org.is_none_or(|o| text(acct, "organizationUuid") == o);
        if !fill {
            return Ok(());
        }
        acct.insert("uuid".into(), Value::String(uuid.to_owned()));
        data.insert("lastUpdated".into(), Value::String(get_timestamp()));
        self.write_json(&self.sequence_file, &data)
    }
}

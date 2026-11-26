//! Choosing a switch target: upstream `_switch_result_from_op`, `_switch_noop`, `_usage_by_account`,
//! `_warn_inert_models`, `_select_best_switchable`, `switch` and `switch_to`.

use std::collections::{BTreeMap, HashSet};

use serde_json::{Value, json};

use super::perform::SwitchOp;
use super::{JsonObject, Switcher, groups};
use crate::error::{Error, Result};
use crate::json_output::{SCHEMA_VERSION, account_ref};
use crate::oauth;
use crate::printer::{accent, dimmed, muted, warning};
use crate::usage_store::Decision;
use crate::{AI_PROG, PROG};

/// A no-op switch result.
struct Noop<'a> {
    strategy: &'a str,
    reason: &'a str,
    message: String,
    from: Option<Value>,
    to: Option<Value>,
    warnings: Vec<String>,
}

fn noop(n: Noop<'_>) -> Value {
    let to = n.to.unwrap_or(Value::Null);
    json!({
        "schemaVersion": SCHEMA_VERSION,
        "switched": false,
        "from": n.from.unwrap_or_else(|| to.clone()),
        "to": to,
        "strategy": n.strategy,
        "reason": n.reason,
        "message": n.message,
        "warnings": n.warnings,
    })
}

fn result_from_op(op: SwitchOp, strategy: &str, mut extra: Vec<String>) -> Value {
    let switched = op.from != op.to;
    let (reason, verb) = if switched {
        ("switched", "Switched to")
    } else {
        ("already-active", "Already on")
    };
    let message = format!(
        "{verb} Account-{} ({})",
        op.to["number"],
        op.to["email"].as_str().unwrap_or_default()
    );
    extra.extend(op.warnings);
    json!({
        "schemaVersion": SCHEMA_VERSION,
        "switched": switched,
        "from": op.from,
        "to": op.to,
        "strategy": strategy,
        "reason": reason,
        "message": message,
        "warnings": extra,
    })
}

fn sequence_nums(data: &JsonObject) -> Vec<String> {
    data.get("sequence")
        .and_then(Value::as_array)
        .map(|s| {
            s.iter()
                .map(|v| v.as_i64().map_or_else(|| v.to_string(), |n| n.to_string()))
                .collect()
        })
        .unwrap_or_default()
}

fn active_number(data: &JsonObject) -> Option<String> {
    data.get("activeAccountNumber")
        .filter(|v| !v.is_null())
        .map(|v| v.as_i64().map_or_else(|| v.to_string(), |n| n.to_string()))
}

/// Ask for a slot when one email is managed under several orgs. `Ok(None)`: cancelled.
fn choose_slot(switcher: &Switcher, identifier: &str) -> Result<Option<String>> {
    let data = switcher.get_sequence_data()?.unwrap_or_default();
    let mut matches: Vec<String> = data
        .get("accounts")
        .and_then(Value::as_object)
        .map(|a| {
            a.iter()
                .filter(|(_, acc)| acc.get("email").and_then(Value::as_str) == Some(identifier))
                .map(|(num, _)| num.clone())
                .collect()
        })
        .unwrap_or_default();
    if matches.len() <= 1 {
        return Ok(Some(identifier.to_owned()));
    }
    matches.sort_by_key(|n| n.parse::<i64>().unwrap_or(0));
    println!("Multiple accounts found for '{identifier}':");
    for num in &matches {
        let acc = data.get("accounts").and_then(|a| a.get(num));
        let text = |k: &str| {
            acc.and_then(|a| a.get(k))
                .and_then(Value::as_str)
                .unwrap_or_default()
        };
        let tag = Switcher::get_display_tag(
            text("email"),
            text("organizationName"),
            text("organizationUuid"),
        );
        println!("  {num}: {identifier} {}", muted(&format!("[{tag}]")));
    }
    let choice = super::add::prompt("Enter account number to switch to: ").unwrap_or_default();
    let choice = choice.trim().to_owned();
    if !choice.is_empty() && choice.bytes().all(|b| b.is_ascii_digit()) && matches.contains(&choice)
    {
        return Ok(Some(choice));
    }
    println!("{}", dimmed("Cancelled"));
    Ok(None)
}

impl Switcher {
    /// Each account's decision value as JSON: a reading, a sentinel string, or `None`.
    pub fn usage_by_account(&self) -> BTreeMap<String, Option<Value>> {
        let infos = self.build_accounts_info();
        self.collect_usage_entries(&infos, None, false)
            .into_iter()
            .map(|(num, entry)| {
                let value = match entry.decision_value() {
                    Some(Decision::Usage(u)) => Some(u.clone()),
                    Some(Decision::Sentinel(s)) => Some(Value::String(s.to_owned())),
                    None => None,
                };
                (num, value)
            })
            .collect()
    }

    /// Warn about `--model` names that match no account's scoped windows (likely a typo).
    fn warn_inert_models(
        usage: &BTreeMap<String, Option<Value>>,
        models: &[String],
        json_output: bool,
        warnings: &mut Vec<String>,
    ) {
        let wanted: Vec<&String> = models
            .iter()
            .filter(|m| m.to_lowercase() != "all")
            .collect();
        if wanted.is_empty()
            || usage.is_empty()
            || usage
                .values()
                .any(|v| !v.as_ref().is_some_and(Value::is_object))
        {
            return;
        }
        let seen: HashSet<String> = usage
            .values()
            .flatten()
            .flat_map(|v| {
                v.get("scoped")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default()
            })
            .filter_map(|s| s.get("name").and_then(Value::as_str).map(str::to_lowercase))
            .collect();
        let missing: Vec<&str> = wanted
            .iter()
            .filter(|m| !seen.contains(&m.to_lowercase()))
            .map(|m| m.as_str())
            .collect();
        if missing.is_empty() {
            return;
        }
        let msg = format!(
            "model(s) {} match no account's usage windows (typo?)",
            missing.join(", ")
        );
        if json_output {
            warnings.push(msg);
        } else {
            warning(&msg);
        }
    }

    /// The best other account by headroom, or a note saying why there is none: `none`,
    /// `current-unavailable`, `no-comparison`, `incomplete-comparison`, `exhausted`, `stay`.
    pub fn select_best_switchable(
        &self,
        current_num: Option<&str>,
        models: &[String],
        usage: &BTreeMap<String, Option<Value>>,
    ) -> (Option<String>, &'static str) {
        let data = self.get_sequence_data().ok().flatten().unwrap_or_default();
        let others: Vec<String> = sequence_nums(&data)
            .into_iter()
            .filter(|n| Some(n.as_str()) != current_num)
            .filter(|n| {
                self.account_is_switchable(n).unwrap_or(false)
                    && !Self::disabled_from_data(&data, n)
            })
            .collect();
        if others.is_empty() {
            return (None, "none");
        }
        let headroom =
            |num: &str| oauth::account_headroom(usage.get(num).and_then(Option::as_ref), models);
        let Some(current) = current_num.and_then(headroom) else {
            return (None, "current-unavailable");
        };
        let scored: Vec<(Option<f64>, String)> =
            others.iter().map(|n| (headroom(n), n.clone())).collect();
        let best_where = |keep: &dyn Fn(&str) -> bool| {
            scored
                .iter()
                .filter(|(_, n)| keep(n))
                .filter_map(|(h, n)| h.map(|h| (h, n)))
                .fold(None::<(f64, &String)>, |acc, (h, n)| match acc {
                    Some((bh, _)) if bh >= h => acc,
                    _ => Some((h, n)),
                })
        };
        // The active account's group first: its best is taken whenever it beats the current one.
        let in_group = |n: &str| current_num.is_none_or(|c| groups::same_group(&data, c, n));
        if let Some((h, n)) = best_where(&in_group)
            && h > current
        {
            return (Some(n.clone()), "");
        }
        let Some((best_h, best_num)) = best_where(&|_| true) else {
            return (None, "no-comparison");
        };
        if best_h > current {
            return (Some(best_num.clone()), "");
        }
        if scored.iter().any(|(h, _)| h.is_none()) {
            return (None, "incomplete-comparison");
        }
        if current <= 0.0 {
            return (None, "exhausted");
        }
        (None, "stay")
    }

    /// `switch` (rotate) or `switch --strategy best|next-available`.
    pub fn switch(
        &self,
        strategy: Option<&str>,
        json_output: bool,
        models: &[String],
        model_source: Option<&str>,
    ) -> Result<Option<Value>> {
        let label = strategy
            .filter(|s| matches!(*s, "best" | "next-available"))
            .unwrap_or("rotation");
        let models: &[String] = if label == "rotation" { &[] } else { models };
        let mut warnings = Vec::new();
        if !models.is_empty() && !json_output {
            let source = match model_source {
                Some("cli") => Some("--model"),
                other => other,
            };
            let suffix = source.map(|s| format!(" (from {s})")).unwrap_or_default();
            println!(
                "{}",
                dimmed(&format!(
                    "Using configured model limits: {}{suffix}",
                    models.join(", ")
                ))
            );
        }
        if !self.sequence_file.exists() {
            return Err(Error::Config("No accounts are managed yet".into()));
        }
        let identity = self.get_current_account();
        self.get_sequence_data_migrated()?;
        let Some((email, org)) = identity else {
            return self.switch_without_login(label, json_output, warnings);
        };
        if !self.account_exists(&email, &org)? {
            if json_output {
                let r = account_ref(None, &email);
                return Ok(Some(noop(Noop {
                    strategy: label,
                    reason: "unmanaged-account",
                    message: format!("Active account is not managed; run {PROG} --add-account"),
                    from: Some(r.clone()),
                    to: Some(r),
                    warnings: Vec::new(),
                })));
            }
            println!(
                "{} Active account '{email}' was not managed.",
                accent("Notice:")
            );
            self.add_account(None, None, false)?;
            let data = self.get_sequence_data()?.unwrap_or_default();
            println!(
                "It has been automatically added as Account-{}.",
                active_number(&data).unwrap_or_else(|| "None".into())
            );
            println!(
                "{}",
                dimmed("Please run the switch command again to switch to the next account.")
            );
            return Ok(None);
        }
        let data = self.get_sequence_data()?.unwrap_or_default();
        let sequence = sequence_nums(&data);
        if sequence.len() < 2 {
            let msg = "Only one account is managed. Add more accounts to switch between.";
            if json_output {
                let to = Self::find_account_slot(&data, &email, &org)
                    .map(|n| account_ref(n.parse().ok(), &email));
                return Ok(Some(noop(Noop {
                    strategy: label,
                    reason: "only-one-account",
                    message: msg.into(),
                    from: None,
                    to,
                    warnings: Vec::new(),
                })));
            }
            println!("{}", dimmed(msg));
            return Ok(None);
        }
        let active = active_number(&data);
        let current = Self::find_account_slot(&data, &email, &org).or_else(|| active.clone());
        let current_ref = current
            .as_ref()
            .map(|n| account_ref(n.parse().ok(), &email));
        let ctx = Ctx {
            label,
            json_output,
            current: current.clone(),
            current_ref: current_ref.clone(),
            email: email.clone(),
        };

        if strategy == Some("best") {
            let usage = self.usage_by_account();
            Self::warn_inert_models(&usage, models, json_output, &mut warnings);
            let (target, note) = self.select_best_switchable(current.as_deref(), models, &usage);
            if let Some(target) = target {
                ctx.note_left_group(&data, &target, &mut warnings);
                let op = self.perform_switch(&target, !json_output, false, None)?;
                return Ok(json_output.then(|| result_from_op(op, label, warnings)));
            }
            if note != "none" {
                return Ok(ctx.best_note(note, !models.is_empty(), warnings));
            }
            // No other switchable account: fall through, so rotation reports it.
        }

        let anchor = if strategy == Some("next-available") {
            current.clone()
        } else {
            active.clone()
        };
        let index = anchor
            .and_then(|a| sequence.iter().position(|n| *n == a))
            .or_else(|| active.and_then(|a| sequence.iter().position(|n| *n == a)))
            .unwrap_or(0);
        let usage = if strategy == Some("next-available") {
            let usage = self.usage_by_account();
            Self::warn_inert_models(&usage, models, json_output, &mut warnings);
            usage
        } else {
            BTreeMap::new()
        };
        let mut next: Option<String> = None;
        let mut skipped_exhausted = false;
        let order: Vec<String> = (1..sequence.len())
            .map(|offset| sequence[(index + offset) % sequence.len()].clone())
            .collect();
        for candidate in groups::same_group_first(&data, current.as_deref(), order) {
            if Self::disabled_from_data(&data, &candidate) {
                ctx.skip(&mut warnings, &candidate, "(disabled)", "(disabled)".into());
                continue;
            }
            if !self.account_is_switchable(&candidate)? {
                ctx.skip(
                    &mut warnings,
                    &candidate,
                    "(no stored credentials/config)",
                    format!("(no stored credentials/config, re-add with {PROG} --add-account --slot {candidate})"),
                );
                continue;
            }
            if strategy == Some("next-available") {
                let value = usage.get(&candidate).and_then(Option::as_ref);
                if oauth::account_headroom(value, models).is_some_and(|h| h <= 0.0) {
                    skipped_exhausted = true;
                    let at: Vec<String> = oauth::relevant_windows(value, models)
                        .into_iter()
                        .filter(|w| w.pct >= 100.0)
                        .map(|w| w.label)
                        .collect();
                    let limit = if models.is_empty() || at.is_empty() {
                        "5h/7d".to_owned()
                    } else {
                        at.join("/")
                    };
                    let reason = format!("(at {limit} limit)");
                    ctx.skip(&mut warnings, &candidate, &reason, reason.clone());
                    continue;
                }
            }
            next = Some(candidate);
            break;
        }
        let Some(next) = next else {
            if skipped_exhausted {
                let limits = if models.is_empty() {
                    "5h/7d limit"
                } else {
                    "usage limits"
                };
                let msg = format!(
                    "All other accounts are at their {limits} — staying on Account-{}.",
                    ctx.current_label()
                );
                return Ok(ctx.stay_warning("candidates-exhausted", msg, warnings));
            }
            if json_output {
                return Ok(Some(noop(Noop {
                    strategy: label,
                    reason: "no-valid-target",
                    message: "No other accounts have valid stored credentials/config.".into(),
                    from: None,
                    to: current_ref,
                    warnings,
                })));
            }
            println!(
                "{}",
                dimmed(&format!(
                    "No other accounts have valid stored credentials/config.\nRe-add a skipped slot with: {PROG} \
                     --add-account --slot <number>"
                ))
            );
            return Ok(None);
        };
        let mut provenance = None;
        if Some(&next) == current.as_ref() {
            let (action, prov) = self.self_switch_action(&next, &email);
            if action != "reconcile" {
                let msg = format!("Already on Account-{next} ({email})");
                if json_output {
                    return Ok(Some(noop(Noop {
                        strategy: label,
                        reason: "already-active",
                        message: msg,
                        from: current_ref.clone(),
                        to: current_ref,
                        warnings,
                    })));
                }
                println!("{} Account-{next} ({email})", accent("Already on"));
                return Ok(None);
            }
            provenance = prov;
        }
        ctx.note_left_group(&data, &next, &mut warnings);
        let op = self.perform_switch(&next, !json_output, false, provenance)?;
        Ok(json_output.then(|| result_from_op(op, label, warnings)))
    }

    /// No live login: activate the preferred (or first usable) slot.
    fn switch_without_login(
        &self,
        label: &str,
        json_output: bool,
        mut warnings: Vec<String>,
    ) -> Result<Option<Value>> {
        let data = self.get_sequence_data()?.unwrap_or_default();
        let sequence = sequence_nums(&data);
        let Some(mut target) = active_number(&data)
            .filter(|n| n != "0")
            .or_else(|| sequence.first().cloned())
        else {
            return Err(Error::Config("No accounts are managed yet".into()));
        };
        let disabled = Self::disabled_from_data(&data, &target);
        if disabled || !self.account_is_switchable(&target)? {
            let (reason, console) = if disabled {
                ("(disabled)".to_owned(), "(disabled)".to_owned())
            } else {
                (
                    "(no stored credentials/config)".to_owned(),
                    format!(
                        "(no stored credentials/config, re-add with {PROG} --add-account --slot {target})"
                    ),
                )
            };
            if json_output {
                warnings.push(format!("Skipped Account-{target} {reason}"));
            } else {
                println!("{} Account-{target} {console}", accent("Skipping"));
            }
            let fallback = sequence.iter().find(|n| {
                **n != target
                    && !Self::disabled_from_data(&data, n)
                    && self.account_is_switchable(n).unwrap_or(false)
            });
            let Some(fallback) = fallback else {
                if sequence
                    .iter()
                    .any(|n| self.account_is_switchable(n).unwrap_or(false))
                {
                    return Err(Error::Config(format!(
                        "No accounts remain in rotation. Re-enable one with: {AI_PROG} enable <num|email>"
                    )));
                }
                return Err(Error::Config(format!(
                    "No managed accounts have valid stored credentials/config. Re-add a slot with: {PROG} \
                     --add-account --slot <number>"
                )));
            };
            target = fallback.clone();
        }
        let op = self.perform_switch(&target, !json_output, false, None)?;
        Ok(json_output.then(|| result_from_op(op, label, warnings)))
    }

    /// `switch <num|email|alias>` (`--switch-to`). `force` rewrites the live login from the backup.
    pub fn switch_to(
        &self,
        identifier: &str,
        json_output: bool,
        force: bool,
    ) -> Result<Option<Value>> {
        if !self.sequence_file.exists() {
            return Err(Error::Config("No accounts are managed yet".into()));
        }
        self.get_sequence_data_migrated()?;
        let mut identifier = identifier.to_owned();
        if identifier.is_empty() || !identifier.bytes().all(|b| b.is_ascii_digit()) {
            let is_alias = self.find_account_by_alias(&identifier)?.is_some();
            if !is_alias && !Self::validate_email(&identifier) {
                return Err(Error::Validation(format!(
                    "Invalid account identifier: {identifier}"
                )));
            }
            if !json_output && !is_alias {
                match choose_slot(self, &identifier)? {
                    Some(chosen) => identifier = chosen,
                    None => return Ok(None),
                }
            }
        }
        let target = self
            .resolve_account_identifier(&identifier)?
            .ok_or_else(|| {
                Error::AccountNotFound(format!("No account found with identifier: {identifier}"))
            })?;
        let data = self.get_sequence_data()?.unwrap_or_default();
        let target_email = data
            .get("accounts")
            .and_then(|a| a.get(&target))
            .ok_or_else(|| Error::AccountNotFound(format!("Account-{target} does not exist")))?
            .get("email")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let mut provenance = None;
        if !force
            && let Some((email, org)) = self.get_current_account()
            && Self::find_account_slot(&data, &email, &org).as_deref() == Some(target.as_str())
        {
            let (action, prov) = self.self_switch_action(&target, &email);
            if action != "reconcile" {
                let r = account_ref(target.parse().ok(), &target_email);
                if !json_output {
                    println!("{} Account-{target} ({target_email})", accent("Already on"));
                    println!(
                        "{}",
                        dimmed(&format!(
                            "To rewrite the live login from the stored backup (e.g. after --import), run: {PROG} \
                             --switch-to {target} --force"
                        ))
                    );
                    return Ok(None);
                }
                return Ok(Some(noop(Noop {
                    strategy: "direct",
                    reason: "already-active",
                    message: format!("Already on Account-{target} ({target_email})"),
                    from: Some(r.clone()),
                    to: Some(r),
                    warnings: Vec::new(),
                })));
            }
            provenance = prov;
        }
        let op = self.perform_switch(&target, !json_output, force, provenance)?;
        if !json_output {
            return Ok(None);
        }
        let mut result = result_from_op(op, "direct", Vec::new());
        if force && result["switched"] == false {
            let to = result["to"].clone();
            result["reason"] = json!("activated");
            result["message"] = json!(format!(
                "Activated Account-{} ({}) from stored backup",
                to["number"],
                to["email"].as_str().unwrap_or_default()
            ));
        }
        Ok(Some(result))
    }
}

/// Shared rendering for one `switch` run.
struct Ctx<'a> {
    label: &'a str,
    json_output: bool,
    current: Option<String>,
    current_ref: Option<Value>,
    #[allow(dead_code)]
    email: String,
}

impl Ctx<'_> {
    /// Say so when an automatic pick left the active account's group.
    fn note_left_group(&self, data: &JsonObject, target: &str, warnings: &mut Vec<String>) {
        let Some(note) = self
            .current
            .as_deref()
            .and_then(|from| groups::left_group_note(data, from, target))
        else {
            return;
        };
        if self.json_output {
            warnings.push(note);
        } else {
            warning(&note);
        }
    }

    fn current_label(&self) -> String {
        self.current.clone().unwrap_or_else(|| "None".into())
    }

    fn skip(
        &self,
        warnings: &mut Vec<String>,
        candidate: &str,
        json_reason: &str,
        console_reason: String,
    ) {
        if self.json_output {
            warnings.push(format!("Skipped Account-{candidate} {json_reason}"));
        } else {
            println!(
                "{} Account-{candidate} {console_reason}",
                accent("Skipping")
            );
        }
    }

    fn stay(&self, reason: &str, json_msg: String, warnings: Vec<String>) -> Value {
        noop(Noop {
            strategy: self.label,
            reason,
            message: json_msg,
            from: None,
            to: self.current_ref.clone(),
            warnings,
        })
    }

    fn stay_warning(&self, reason: &str, msg: String, warnings: Vec<String>) -> Option<Value> {
        if self.json_output {
            return Some(self.stay(reason, msg, warnings));
        }
        warning(&msg);
        None
    }

    /// The `--strategy best` outcome when it found no better account.
    fn best_note(&self, note: &str, has_models: bool, warnings: Vec<String>) -> Option<Value> {
        let cur = self.current_label();
        let (reason, json_msg, console) = match note {
            "current-unavailable" => (
                "usage-unavailable",
                format!("Current account usage is unavailable — staying on Account-{cur}."),
                Some(format!(
                    "Current account usage is unavailable — staying on Account-{cur}. Run {PROG} --switch to rotate."
                )),
            ),
            "no-comparison" => (
                "usage-unavailable",
                format!("No other account has usage data to compare — staying on Account-{cur}."),
                Some(format!(
                    "No other account has usage data to compare — staying on Account-{cur}. Run {PROG} --switch \
                     to rotate."
                )),
            ),
            "incomplete-comparison" => {
                let msg = format!(
                    "No account with known usage has more remaining quota; some usage is unavailable — staying on \
                     Account-{cur}."
                );
                ("usage-unavailable", msg.clone(), Some(msg))
            }
            "stay" => {
                if self.json_output {
                    return Some(self.stay(
                        "already-best",
                        format!(
                            "Already on the account with the most remaining quota (Account-{cur})."
                        ),
                        warnings,
                    ));
                }
                println!(
                    "{} (Account-{cur}).",
                    accent("Already on the account with the most remaining quota")
                );
                return None;
            }
            "exhausted" => {
                let limits = if has_models {
                    "usage limits"
                } else {
                    "5h/7d limit"
                };
                let msg = format!("All accounts are at their {limits} — staying on Account-{cur}.");
                return self.stay_warning("candidates-exhausted", msg, warnings);
            }
            // "none": no other switchable account; upstream falls through to rotation, which then reports it.
            _ => return None,
        };
        if self.json_output {
            return Some(self.stay(reason, json_msg, warnings));
        }
        if let Some(text) = console {
            println!("{}", dimmed(&text));
        }
        None
    }
}

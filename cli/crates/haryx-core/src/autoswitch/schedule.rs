//! Usage collection on the shared poll schedule, the loop's pacing and the run loop.

use std::collections::{BTreeMap, HashSet};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::error::Result;
use crate::lines::window_line;
use crate::oauth;
use crate::poll_policy::{self, ESCALATION_MARGIN_PCT, binding_pct, limiting_reset_ts};
use crate::settings::log_unrecognized_keys;
use crate::usage_store::{UsageEntry, due_candidate, plan_oversleeps_interval};

use super::engine::AutoSwitchEngine;
use super::events::{EventBody, iso_seconds};
use super::rank::Headroom;
use super::{
    MAX_SLEEP_S, NO_RESET_FALLBACK_S, TickOutcome, UsageMap, dict_of, headroom_by_account,
    sentinel_of, usage_from_entries,
};
use crate::json_output::USAGE_TOKEN_EXPIRED;

pub(super) type Entries = BTreeMap<String, UsageEntry>;

impl AutoSwitchEngine<'_> {
    pub(super) fn entries(&self, fetch: &HashSet<String>, scheduled: bool) -> Entries {
        self.switcher
            .usage_entries_by_account(Some(fetch), scheduled)
    }

    /// Usage for this tick: the active account when its plan is due, one due candidate, and every
    /// candidate when the active account nears a line or reads unknown (escalation).
    pub(super) fn collect_scheduled_usage(
        &self,
        current: &str,
        quarantined: &HashSet<String>,
        threshold: f64,
    ) -> Result<(Entries, UsageMap, Headroom)> {
        let now = self.now();
        let candidates: Vec<String> = self
            .switcher
            .switchable_account_numbers()?
            .into_iter()
            .filter(|n| n != current && !quarantined.contains(n))
            .collect();
        let pre = self.entries(&HashSet::new(), true);
        let mut plan = HashSet::new();
        let active_pre = pre.get(current);
        let stale_candidate_plan = active_pre.is_some_and(|e| {
            e.age_s
                .is_some_and(|age| age >= poll_policy::ACTIVE_MAX_INTERVAL_S)
                && e.poll_interval_s.unwrap_or(0.0) > poll_policy::ACTIVE_MAX_INTERVAL_S
                && binding_pct(e.last_good.as_ref(), &self.models).unwrap_or(0.0) < 100.0
        });
        let overslept = active_pre.is_some_and(|e| plan_oversleeps_interval(e, now));
        let active_due = match active_pre {
            None => true,
            Some(e) => match (e.age_s, e.next_poll_at) {
                (None, _) => true,
                (Some(_), Some(next)) => now >= next,
                (Some(age), None) => age >= poll_policy::MIN_INTERVAL_S,
            },
        };
        if active_due || stale_candidate_plan || overslept {
            plan.insert(current.to_owned());
        }
        if self.idle_hold_since.is_none() {
            let mut pick = due_candidate(&candidates, &pre, now);
            if pick.is_none() {
                let disabled: Vec<String> = self
                    .switcher
                    .disabled_account_numbers()?
                    .into_iter()
                    .filter(|n| n != current && !quarantined.contains(n))
                    .collect();
                pick = due_candidate(&disabled, &pre, now);
            }
            plan.extend(pick);
        }
        let mut entries = self.entries(&plan, !stale_candidate_plan);
        let mut usage = usage_from_entries(&entries);
        let active_value = dict_of(&usage, current);
        let active_headroom = oauth::account_headroom(active_value, &self.models);
        let weekly = self.settings.weekly_threshold;
        let near_a_line = oauth::relevant_windows(active_value, &self.models)
            .iter()
            .any(|w| w.pct >= window_line(&w.label, threshold, weekly) - ESCALATION_MARGIN_PCT);
        let expired = sentinel_of(&usage, current) == Some(USAGE_TOKEN_EXPIRED);
        let escalate = !candidates.is_empty()
            && ((active_headroom.is_none() && !expired)
                || (active_headroom.is_some() && near_a_line));
        if escalate {
            let fetch: HashSet<String> = std::iter::once(current.to_owned())
                .chain(candidates.iter().cloned())
                .filter(|num| {
                    let Some(entry) = entries.get(num) else {
                        return true;
                    };
                    let planned = oauth::account_headroom(dict_of(&usage, num), &self.models);
                    let parked = entry.next_poll_at.is_some_and(|next| now < next)
                        && entry.poll_interval_s.unwrap_or(0.0) > poll_policy::EXHAUSTED_INTERVAL_S
                        && planned.is_some_and(|h| h <= 0.0);
                    !parked
                })
                .collect();
            entries = self.entries(&fetch, true);
            usage = usage_from_entries(&entries);
        }
        let headroom = headroom_by_account(&usage, &self.models);
        Ok((entries, usage, headroom))
    }

    /// Warn once when a configured model name matches no account's scoped windows.
    pub(super) fn check_model_names(
        &mut self,
        quarantined: &HashSet<String>,
        usage: &UsageMap,
    ) -> Result<()> {
        let wanted: Vec<&String> = self
            .models
            .iter()
            .filter(|m| !m.eq_ignore_ascii_case("all"))
            .collect();
        if wanted.is_empty() {
            self.model_check_done = true;
            return Ok(());
        }
        let mut readable = Vec::new();
        for num in self.switcher.switchable_account_numbers()? {
            if quarantined.contains(&num) || self.switcher.account_kind_for(&num)? == "api_key" {
                continue;
            }
            match dict_of(usage, &num) {
                Some(v) => readable.push(v),
                None => return Ok(()),
            }
        }
        if readable.is_empty() {
            return Ok(());
        }
        let seen: HashSet<String> = readable
            .iter()
            .filter_map(|v| v.get("scoped").and_then(Value::as_array))
            .flatten()
            .filter_map(|s| s.get("name").and_then(Value::as_str))
            .map(str::to_lowercase)
            .collect();
        self.model_check_done = true;
        let missing: Vec<String> = wanted
            .into_iter()
            .filter(|name| !seen.contains(&name.to_lowercase()))
            .cloned()
            .collect();
        if !missing.is_empty() {
            self.emit_body(EventBody::ConfigWarning {
                message: format!(
                    "autoswitch.model: {} matches no account's usage windows — only the 5h/7d \
                     limits are being watched for it (typo?)",
                    missing.join(", ")
                ),
            });
        }
        Ok(())
    }

    /// The soonest moment an exhausted account becomes usable; `None` when one has no provable
    /// recovery (so the loop never oversleeps).
    pub(super) fn earliest_recovery(&self, usage: &UsageMap) -> Option<f64> {
        let now = self.now();
        let mut earliest: Option<f64> = None;
        for (num, _) in usage {
            let Some(value) = dict_of(usage, num) else {
                continue;
            };
            let blocked = oauth::relevant_windows(Some(value), &self.models)
                .iter()
                .any(|w| w.pct >= 100.0);
            if !blocked {
                continue;
            }
            let usable_at = limiting_reset_ts(Some(value), &self.models).filter(|ts| *ts > now)?;
            earliest = Some(earliest.map_or(usable_at, |e: f64| e.min(usable_at)));
        }
        earliest
    }

    pub(super) fn next_delay(&self, outcome: TickOutcome) -> f64 {
        let interval = self.settings.interval_seconds;
        match outcome {
            TickOutcome::Blocked => {
                if let Some(until) = self.sleep_until_ts {
                    return (until - self.now()).max(interval).min(MAX_SLEEP_S);
                }
                if self.blocked_wait_long {
                    return interval.max(NO_RESET_FALLBACK_S);
                }
            }
            TickOutcome::NoAction if self.idle_hold_slow => {
                return interval.max(NO_RESET_FALLBACK_S);
            }
            _ => {}
        }
        self.respect_poll_plan(interval * (0.9 + 0.2 * rand::random::<f64>()))
    }

    /// Wake no later than the active account's planned poll (never sooner than the urgent floor).
    fn respect_poll_plan(&self, delay: f64) -> f64 {
        let Ok(Some(current)) = self.switcher.current_account_number() else {
            return delay;
        };
        let entries = self.entries(&HashSet::new(), true);
        match entries.get(&current).and_then(|e| e.next_poll_at) {
            Some(next) => delay.min((next - self.now()).max(poll_policy::URGENT_INTERVAL_S)),
            None => delay,
        }
    }

    /// Tick until stopped. Returns the process exit code (0).
    pub fn run_loop(&mut self) -> i32 {
        log_unrecognized_keys(self.switcher.backup_dir());
        loop {
            if self.stop.load(Ordering::SeqCst) {
                return 0;
            }
            let outcome = self.tick();
            let delay = self.next_delay(outcome);
            if delay > self.settings.interval_seconds * 1.5 {
                let until = jiff::Timestamp::now()
                    .checked_add(jiff::SignedDuration::from_secs_f64(delay))
                    .unwrap_or_else(|_| jiff::Timestamp::now());
                self.emit_body(EventBody::Sleep {
                    seconds: delay,
                    until: iso_seconds(until),
                });
            }
            self.sleep(delay);
        }
    }

    /// Sleep up to `seconds`, returning early when stopped.
    fn sleep(&self, seconds: f64) {
        let deadline = Instant::now() + Duration::from_secs_f64(seconds.max(0.0));
        while !self.stop.load(Ordering::SeqCst) {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return;
            }
            std::thread::sleep(left.min(Duration::from_millis(200)));
        }
    }
}

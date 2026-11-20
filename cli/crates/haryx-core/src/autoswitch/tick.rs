//! One decision pass (`_tick_inner`).

use std::collections::{HashMap, HashSet};

use log::warn;
use serde_json::{Map, Value};

use crate::PROG;
use crate::error::Result;
use crate::json_output::USAGE_TOKEN_EXPIRED;
use crate::models::AccountPolicy;
use crate::poll_policy::RESET_SLACK_S;

use super::engine::AutoSwitchEngine;
use super::events::{Event, EventBody, iso_seconds};
use super::rank::{Headroom, RankInput, Ranked};
use super::{
    IDLE_HOLD_MAX_S, ThresholdMap, TickOutcome, UsageMap, account_ref, below_line_detail,
    binding_recovery_ts, dict_of, headroom_by_account, headroom_of, sentinel_of, systemic_message,
    systemic_rank, usage_from_entries, window_pcts,
};

/// The per-tick context `_rank` closes over.
struct RankCtx<'s> {
    trigger: &'s str,
    priority_mode: bool,
    consume_first: bool,
    oauth_candidates: &'s [String],
    current: &'s str,
    active_enabled: bool,
    thresholds: &'s ThresholdMap,
    policies: &'s HashMap<String, AccountPolicy>,
    state: &'s Map<String, Value>,
}

impl AutoSwitchEngine<'_> {
    fn rank(&self, ctx: &RankCtx<'_>, usage: &UsageMap, headroom: &Headroom, now: f64) -> Ranked {
        let input = RankInput {
            trigger: ctx.trigger,
            consume_first: ctx.consume_first,
            oauth_candidates: ctx.oauth_candidates,
            usage,
            headroom,
            current: ctx.current,
            active_headroom: headroom_of(headroom, ctx.current),
            now,
            thresholds: ctx.thresholds,
        };
        if ctx.priority_mode {
            return self.rank_priority_candidates(&input, ctx.active_enabled, ctx.policies);
        }
        let recovered = self.left_account_recovered(&input, ctx.state);
        let no_return = self.no_return_account(&input, ctx.state, recovered);
        let ranked = self.rank_candidates(&input, no_return.as_deref());
        if no_return.is_some() && ranked.0.is_empty() && recovered {
            let unbarred = self.rank_candidates(&input, None);
            if !unbarred.0.is_empty() {
                return unbarred;
            }
        }
        ranked
    }

    fn no_active_account(&mut self) -> TickOutcome {
        self.emit_body(EventBody::Poll {
            active: None,
            headroom: Vec::new(),
            threshold: self.settings.threshold,
            fetch_errors: Vec::new(),
            windows: Vec::new(),
            weekly_threshold: self.settings.weekly_threshold,
        });
        let event = if self.switcher.has_live_login() {
            Event::no_switch(
                "unmanaged-active-account",
                format!("run '{PROG} --add-account' to include it in rotation"),
            )
        } else {
            Event::no_switch(
                "no-active-account",
                format!("log in and run '{PROG} --add-account' first"),
            )
        };
        self.emit(event);
        TickOutcome::NoAction
    }

    pub(super) fn tick_inner(&mut self) -> Result<TickOutcome> {
        self.sleep_until_ts = None;
        self.blocked_wait_long = false;
        self.idle_hold_slow = false;
        let strategy = self.settings.strategy.clone();
        let priority_mode = strategy == "priority-consume-first";
        let mut state = self.read_state();
        if !self.dry_run {
            state = self.release_recovered_quarantines(state)?;
        }
        let quarantined = Self::quarantined(&state);
        let current = self.switcher.current_account_number()?;
        if let Some(c) = &current
            && self.health_account.as_ref() != Some(c)
        {
            self.load_health_counters(&state, c);
            self.health_account = Some(c.clone());
        }
        let Some(current) = current else {
            return Ok(self.no_active_account());
        };
        let current_email = self.switcher.account_email(&current)?;
        let active_ref = account_ref(&current, &current_email);
        let policies = self.policies()?;
        let thresholds = self.resolve_thresholds(&policies);
        self.refresh_poll_policy_inputs(&thresholds);
        let line = thresholds.get(&current);
        let (entries, mut usage, mut headroom) =
            self.collect_scheduled_usage(&current, &quarantined, line)?;
        // Slot order (as `usage`), not the entries map's text order.
        let fetch_errors = usage
            .iter()
            .filter(|(_, v)| v.is_none())
            .filter_map(|(num, _)| {
                let err = entries.get(num)?.last_error.clone()?;
                Some((num.clone(), err))
            })
            .collect();
        let windows = usage
            .iter()
            .map(|(num, _)| (num.clone(), window_pcts(dict_of(&usage, num), &self.models)))
            .filter(|(_, w)| !w.is_empty())
            .collect();
        self.emit_body(EventBody::Poll {
            active: Some(active_ref),
            headroom: headroom.clone(),
            threshold: line,
            fetch_errors,
            windows,
            weekly_threshold: self.settings.weekly_threshold,
        });
        if !self.model_check_done {
            self.check_model_names(&quarantined, &usage)?;
        }
        let include_api = self.settings.include_api_key_accounts;
        if self.switcher.account_kind_for(&current)? == "api_key" && !include_api {
            self.emit(Event::no_switch(
                "active-api-key",
                "API-key accounts have no quota to watch",
            ));
            return Ok(TickOutcome::NoAction);
        }

        // ---- trigger
        let mut active_headroom = headroom_of(&headroom, &current);
        let mut active_binding = None;
        let trigger: &str = if let Some(ah) = active_headroom {
            self.unhealthy_ticks = 0;
            self.idle_hold_since = None;
            active_binding = self.binding(dict_of(&usage, &current), line);
            let below = match &active_binding {
                Some(b) => !b.over(),
                None => (100.0 - ah) < line,
            };
            if below {
                if strategy != "consume-first" && !priority_mode {
                    let detail = below_line_detail(active_binding.as_ref(), ah, line);
                    self.emit(Event::no_switch("below-threshold", detail));
                    return Ok(TickOutcome::NoAction);
                }
                if priority_mode {
                    "priority"
                } else {
                    "consume-first"
                }
            } else if ah <= 0.0 {
                "at-limit"
            } else {
                "proactive"
            }
        } else {
            if sentinel_of(&usage, &current) == Some(USAGE_TOKEN_EXPIRED) {
                let now = self.now();
                let since = *self.idle_hold_since.get_or_insert(now);
                if now - since <= IDLE_HOLD_MAX_S {
                    self.unhealthy_ticks = 0;
                    self.idle_hold_slow = true;
                    self.emit(Event::no_switch(
                        "active-idle",
                        "token expired while Claude Code is idle; resumes on next use",
                    ));
                    return Ok(TickOutcome::NoAction);
                }
                warn!(
                    "Active token expired and owned for over {:.0} minutes; resuming unhealthy \
                     counting (dead refresh token?)",
                    IDLE_HOLD_MAX_S / 60.0
                );
            } else {
                self.idle_hold_since = None;
            }
            self.unhealthy_ticks += 1;
            if self.unhealthy_ticks < self.settings.unhealthy_ticks {
                let detail = format!(
                    "{}/{} before failover",
                    self.unhealthy_ticks, self.settings.unhealthy_ticks
                );
                self.emit(Event::no_switch("active-usage-unknown", detail));
                return Ok(TickOutcome::NoAction);
            }
            "failover"
        };
        if matches!(trigger, "proactive" | "consume-first" | "priority") && self.in_cooldown(&state)
        {
            self.emit(Event::no_switch("cooldown", ""));
            return Ok(TickOutcome::NoAction);
        }

        // ---- candidates
        let switchable = self.switcher.switchable_account_numbers()?;
        let candidates: Vec<String> = switchable
            .iter()
            .filter(|n| **n != current && !quarantined.contains(*n))
            .cloned()
            .collect();
        let mut oauth_candidates = Vec::new();
        let mut api_key_candidates = Vec::new();
        for n in &candidates {
            if self.switcher.account_kind_for(n)? == "api_key" {
                if include_api {
                    api_key_candidates.push(n.clone());
                }
            } else {
                oauth_candidates.push(n.clone());
            }
        }
        if matches!(trigger, "consume-first" | "priority")
            && oauth_candidates.is_empty()
            && let Some(ah) = active_headroom
        {
            let detail = below_line_detail(active_binding.as_ref(), ah, line);
            self.emit(Event::no_switch("below-threshold", detail));
            return Ok(TickOutcome::NoAction);
        }
        if oauth_candidates.is_empty() && api_key_candidates.is_empty() {
            self.blocked_wait_long = true;
            self.emit(Event::no_switch("no-candidates", ""));
            return Ok(TickOutcome::Blocked);
        }

        // ---- rank
        let ctx = RankCtx {
            trigger,
            priority_mode,
            consume_first: strategy == "consume-first",
            oauth_candidates: &oauth_candidates,
            current: &current,
            active_enabled: switchable.contains(&current),
            thresholds: &thresholds,
            policies: &policies,
            state: &state,
        };
        let mut entries = entries;
        let mut decided_now = self.now();
        let (mut ordered, mut any_known, mut active_reset_ts) =
            self.rank(&ctx, &usage, &headroom, decided_now);
        if (trigger == "consume-first" || priority_mode) && !ordered.is_empty() {
            let fetch: HashSet<String> = std::iter::once(current.clone())
                .chain(candidates.iter().cloned())
                .collect();
            entries = self.entries(&fetch, true);
            usage = usage_from_entries(&entries);
            headroom = headroom_by_account(&usage, &self.models);
            active_headroom = headroom_of(&headroom, &current);
            decided_now = self.now();
            (ordered, any_known, active_reset_ts) = self.rank(&ctx, &usage, &headroom, decided_now);
        }
        let priority_active_ready = priority_mode
            && ctx.active_enabled
            && self.priority_ready(dict_of(&usage, &current), active_headroom, line);
        if ordered.is_empty()
            && !api_key_candidates.is_empty()
            && !matches!(trigger, "consume-first" | "priority")
            && !priority_active_ready
        {
            ordered = api_key_candidates;
        }
        if ordered.is_empty() {
            return Ok(self.nothing_qualifies(
                trigger,
                priority_mode,
                priority_active_ready,
                any_known,
                active_reset_ts,
                &oauth_candidates,
                &usage,
                &headroom,
            ));
        }

        // ---- switch
        let left = (
            active_headroom,
            binding_recovery_ts(
                dict_of(&usage, &current),
                &self.models,
                decided_now,
                Some((
                    line,
                    self.settings.weekly_threshold,
                    self.settings.threshold,
                )),
            ),
        );
        let mut transient_failure = false;
        let mut systemic = String::new();
        for num in &ordered {
            let email = self.switcher.account_email(num)?;
            if trigger == "consume-first" || (priority_mode && oauth_candidates.contains(num)) {
                let fresh = entries.get(num).is_some_and(|e| e.fresh(self.now()));
                if !fresh {
                    self.emit(Event::no_switch(
                        "stale-usage",
                        format!(
                            "account {num} usage could not be refreshed this tick (backoff or a \
                             concurrent poller); retrying"
                        ),
                    ));
                    if priority_mode && !priority_active_ready {
                        continue;
                    }
                    return Ok(TickOutcome::NoAction);
                }
            }
            if self.dry_run {
                return self.perform(num, &email, trigger, left);
            }
            let status = self.freshen_target(num, &email);
            match status.as_str() {
                "identity-conflict" | "invalid_grant" => {
                    self.quarantine(num, &email, &status)?;
                    continue;
                }
                "transient" => {
                    transient_failure = true;
                    continue;
                }
                "skip-live-session" | "skip-live-login" => continue,
                s if systemic_rank(s).is_some() => {
                    if systemic.is_empty() || systemic_rank(s) < systemic_rank(&systemic) {
                        systemic = s.to_owned();
                    }
                    continue;
                }
                _ => return self.perform(num, &email, trigger, left),
            }
        }
        if !systemic.is_empty() || transient_failure {
            let message = if systemic.is_empty() {
                "could not freshen any candidate (network?)".to_owned()
            } else {
                format!("could not freshen: {}", systemic_message(&systemic))
            };
            self.emit(Event::error(message, true));
            return Ok(TickOutcome::Error);
        }
        self.emit(Event::no_switch("no-viable-target", ""));
        Ok(TickOutcome::Blocked)
    }

    #[allow(clippy::too_many_arguments)]
    fn nothing_qualifies(
        &mut self,
        trigger: &str,
        priority_mode: bool,
        priority_active_ready: bool,
        any_known: bool,
        active_reset_ts: Option<f64>,
        oauth_candidates: &[String],
        usage: &UsageMap,
        headroom: &Headroom,
    ) -> TickOutcome {
        if priority_active_ready {
            self.emit(Event::no_switch(
                "already-consuming-priority",
                "no usable account has higher priority or an earlier weekly reset in this tier",
            ));
            return TickOutcome::NoAction;
        }
        if priority_mode {
            self.emit(Event::no_switch(
                "no-qualifying-candidate",
                "no candidate is below every switch line with readable usage; waiting for recovery",
            ));
            return TickOutcome::Blocked;
        }
        if !any_known {
            self.emit(Event::no_switch(
                "no-comparison",
                "no candidate has readable usage",
            ));
            return TickOutcome::Blocked;
        }
        if trigger == "consume-first" {
            if active_reset_ts.is_none() {
                self.emit(Event::no_switch(
                    "reset-unknown",
                    "active account's weekly reset time is unknown; consume-first is idle until \
                     it is reported",
                ));
            } else {
                self.emit(Event::no_switch(
                    "already-consuming-soonest",
                    "no sooner-resetting account with room to spare",
                ));
            }
            return TickOutcome::NoAction;
        }
        let truly_exhausted = oauth_candidates
            .iter()
            .all(|n| headroom_of(headroom, n).is_some_and(|h| h <= 0.0));
        if !truly_exhausted {
            self.emit(Event::no_switch(
                "no-qualifying-candidate",
                "no candidate is below its switch lines and better than the active account by \
                 the hysteresis margin, or usage is unreadable this tick",
            ));
            return TickOutcome::Blocked;
        }
        self.blocked_wait_long = true;
        let earliest = self.earliest_recovery(usage);
        if let Some(ts) = earliest {
            self.sleep_until_ts = Some(ts + RESET_SLACK_S);
        }
        let earliest_reset_at = earliest.map(iso_utc);
        self.emit_body(EventBody::AllExhausted { earliest_reset_at });
        TickOutcome::Blocked
    }
}

/// `datetime.fromtimestamp(ts, utc).isoformat().replace("+00:00", "Z")`: whole seconds unless
/// the timestamp carries a fraction (then microseconds, as Python prints them).
pub(super) fn iso_utc(ts: f64) -> String {
    // Round to whole microseconds first so a fraction that rounds up carries into the second.
    let total_micros = (ts * 1e6).round() as i64;
    let (secs, micros) = (
        total_micros.div_euclid(1_000_000),
        total_micros.rem_euclid(1_000_000),
    );
    let Ok(base) = jiff::Timestamp::from_second(secs) else {
        return ts.to_string();
    };
    let text = iso_seconds(base);
    if micros == 0 {
        return text;
    }
    format!("{}.{micros:06}Z", text.trim_end_matches('Z'))
}

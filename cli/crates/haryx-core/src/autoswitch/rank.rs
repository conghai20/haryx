//! Target ranking: the no-return bar, recovery evidence and the per-strategy candidate order.

use std::cmp::Ordering;
use std::collections::HashMap;

use serde_json::{Map, Value};

use crate::models::AccountPolicy;

use super::engine::{AutoSwitchEngine, number};
use super::{
    HORIZON_HEADROOM_RATIO, RECOVERY_HORIZON_S, RECOVERY_HYSTERESIS_S, SPENT_HEADROOM_PCT,
    ThresholdMap, UsageMap, all_above_margin_met, binding_recovery_ts, dict_of,
    every_account_above_threshold, every_candidate_above_threshold, headroom_of,
    seven_day_reset_ts,
};

pub(super) type Headroom = Vec<(String, Option<f64>)>;
pub(super) type Ranked = (Vec<String>, bool, Option<f64>);

/// Inputs shared by every ranking pass of one tick.
pub(super) struct RankInput<'u> {
    pub trigger: &'u str,
    pub consume_first: bool,
    pub oauth_candidates: &'u [String],
    pub usage: &'u UsageMap,
    pub headroom: &'u Headroom,
    pub current: &'u str,
    pub active_headroom: Option<f64>,
    pub now: f64,
    pub thresholds: &'u ThresholdMap,
}

fn state_num(state: &Map<String, Value>, key: &str) -> Option<String> {
    match state.get(key) {
        Some(Value::String(s)) => Some(s.clone()),
        Some(Value::Number(n)) => Some(n.to_string()),
        _ => None,
    }
}

fn cmp_keys(a: &[f64], b: &[f64]) -> Ordering {
    a.partial_cmp(b).unwrap_or(Ordering::Equal)
}

impl AutoSwitchEngine<'_> {
    /// The scale-aware recovery timestamp of `num`'s binding window.
    fn recovery_ts(&self, usage: &UsageMap, num: &str, now: f64, thresholds: &ThresholdMap) -> f64 {
        binding_recovery_ts(
            dict_of(usage, num),
            &self.models,
            now,
            Some((
                thresholds.get(num),
                self.settings.weekly_threshold,
                self.settings.threshold,
            )),
        )
    }

    /// The account a proactive/consume-first switch must not flip straight back to, if any.
    pub(super) fn no_return_account(
        &self,
        input: &RankInput<'_>,
        state: &Map<String, Value>,
        recovered: bool,
    ) -> Option<String> {
        if !matches!(input.trigger, "proactive" | "consume-first") {
            return None;
        }
        let barred = state_num(state, "lastSwitchFrom")?;
        if let Some(landed_on) = state_num(state, "lastSwitchTo")
            && landed_on != input.current
        {
            return None;
        }
        if !recovered {
            return Some(barred);
        }
        if let Some(left_headroom) = headroom_of(input.headroom, &barred) {
            match input.active_headroom {
                Some(active) => {
                    if all_above_margin_met(left_headroom, active) {
                        return None;
                    }
                }
                None => {
                    let left =
                        self.binding(dict_of(input.usage, &barred), input.thresholds.get(&barred));
                    if left.is_some_and(|b| !b.over()) {
                        return None;
                    }
                }
            }
        }
        Some(barred)
    }

    /// Whether the account we last left has recovered enough to be eligible again.
    pub(super) fn left_account_recovered(
        &self,
        input: &RankInput<'_>,
        state: &Map<String, Value>,
    ) -> bool {
        let Some(barred) = state_num(state, "lastSwitchFrom") else {
            return true;
        };
        if !state.contains_key("leftHeadroom") {
            return true;
        }
        let (usage, now, thresholds) = (input.usage, input.now, input.thresholds);
        let h = headroom_of(input.headroom, &barred);
        let left_headroom = state.get("leftHeadroom").and_then(number);
        let left_recovery = state.get("leftRecoveryAt").and_then(number);
        let is_failover_snapshot = match state.get("leftTrigger") {
            Some(Value::Null) | None => left_headroom.is_none() && left_recovery.is_none(),
            Some(t) => t.as_str() == Some("failover"),
        };
        if is_failover_snapshot {
            let peer = self.binding(dict_of(usage, &barred), thresholds.get(&barred));
            if peer.is_some_and(|b| !b.over()) {
                return true;
            }
            let peer_ts = self.recovery_ts(usage, &barred, now, thresholds);
            let active_ts = self.recovery_ts(usage, input.current, now, thresholds);
            return (active_ts.is_finite() || peer_ts - now <= RECOVERY_HORIZON_S)
                && peer_ts < active_ts - RECOVERY_HYSTERESIS_S;
        }
        if let Some(h) = h {
            match input.active_headroom {
                Some(active) => {
                    if h > active * HORIZON_HEADROOM_RATIO + SPENT_HEADROOM_PCT {
                        return true;
                    }
                }
                None => {
                    let peer = self.binding(dict_of(usage, &barred), thresholds.get(&barred));
                    if peer.is_some_and(|b| !b.over()) {
                        return true;
                    }
                }
            }
            if let Some(left) = left_headroom
                && h >= (left + SPENT_HEADROOM_PCT).min(100.0)
            {
                return true;
            }
        }
        let was = left_recovery.unwrap_or(f64::INFINITY);
        self.recovery_ts(usage, &barred, now, thresholds) < was - RECOVERY_HYSTERESIS_S
    }

    /// Below every switch line with headroom left (priority strategy's eligibility).
    pub(super) fn priority_ready(
        &self,
        value: Option<&Value>,
        headroom: Option<f64>,
        line: f64,
    ) -> bool {
        headroom.is_some_and(|h| h > 0.0) && self.binding(value, line).is_some_and(|b| !b.over())
    }

    pub(super) fn rank_priority_candidates(
        &self,
        input: &RankInput<'_>,
        active_enabled: bool,
        policies: &HashMap<String, AccountPolicy>,
    ) -> Ranked {
        let (usage, now) = (input.usage, input.now);
        let key = |num: &str| -> Vec<f64> {
            let priority = policies.get(num).map_or(0, |p| p.priority);
            let reset = seven_day_reset_ts(dict_of(usage, num), now).unwrap_or(f64::INFINITY);
            vec![-f64::from(priority), reset]
        };
        let ready = |num: &str| {
            self.priority_ready(
                dict_of(usage, num),
                headroom_of(input.headroom, num),
                input.thresholds.get(num),
            )
        };
        let active_key = (active_enabled && ready(input.current)).then(|| key(input.current));
        let mut ordered: Vec<String> = input
            .oauth_candidates
            .iter()
            .filter(|num| {
                ready(num)
                    && active_key
                        .as_ref()
                        .is_none_or(|ak| cmp_keys(&key(num), ak) == Ordering::Less)
            })
            .cloned()
            .collect();
        ordered.sort_by(|a, b| cmp_keys(&key(a), &key(b)));
        let any_known = input
            .oauth_candidates
            .iter()
            .any(|n| headroom_of(input.headroom, n).is_some());
        (
            ordered,
            any_known,
            seven_day_reset_ts(dict_of(usage, input.current), now),
        )
    }

    pub(super) fn rank_candidates(&self, input: &RankInput<'_>, no_return: Option<&str>) -> Ranked {
        let settings = &self.settings;
        let (usage, now, thresholds) = (input.usage, input.now, input.thresholds);
        let (trigger, consume_first) = (input.trigger, input.consume_first);
        let weekly_first = settings.strategy == "weekly-first";
        let active_reset_ts = if consume_first {
            seven_day_reset_ts(dict_of(usage, input.current), now)
        } else {
            None
        };
        let bindings: HashMap<&str, _> = input
            .oauth_candidates
            .iter()
            .map(String::as_str)
            .chain(std::iter::once(input.current))
            .map(|num| (num, self.binding(dict_of(usage, num), thresholds.get(num))))
            .collect();
        let active_binding = bindings.get(input.current).cloned().flatten();
        let active_lh = active_binding
            .as_ref()
            .map(|b| b.line_headroom)
            .or(input.active_headroom);
        let over_by_account: HashMap<String, Option<bool>> = bindings
            .iter()
            .map(|(num, b)| ((*num).to_owned(), b.as_ref().map(|b| b.over())))
            .collect();
        let all_above = if matches!(trigger, "at-limit" | "failover") {
            every_candidate_above_threshold(input.oauth_candidates, &over_by_account)
        } else {
            every_account_above_threshold(
                input.oauth_candidates,
                &over_by_account,
                active_binding.as_ref().map(|b| b.over()),
            )
        };
        let most_room = |num: &str, h: f64| -> Vec<f64> {
            vec![
                -h,
                binding_recovery_ts(dict_of(usage, num), &self.models, now, None),
            ]
        };
        let mut qualifying: Vec<(Vec<f64>, String)> = Vec::new();
        let mut any_known = false;
        for num in input.oauth_candidates {
            let Some(h) = headroom_of(input.headroom, num) else {
                continue;
            };
            any_known = true;
            if h <= 0.0 || no_return == Some(num.as_str()) {
                continue;
            }
            let line = thresholds.get(num);
            let binding = bindings.get(num.as_str()).cloned().flatten();
            let lh = binding.as_ref().map_or(h, |b| b.line_headroom);
            let room = self
                .room(dict_of(usage, num), line)
                .unwrap_or(line - (100.0 - h));
            let over = binding.as_ref().map_or((100.0 - h) >= line, |b| b.over());
            let reset_ts = if consume_first || weekly_first {
                seven_day_reset_ts(dict_of(usage, num), now)
            } else {
                None
            };
            if matches!(trigger, "proactive" | "consume-first") {
                if over && !all_above {
                    continue;
                }
                if all_above {
                    if !all_above_margin_met(h, input.active_headroom.unwrap_or(0.0)) {
                        continue;
                    }
                } else if consume_first {
                    if trigger == "consume-first" {
                        let sooner =
                            matches!((reset_ts, active_reset_ts), (Some(r), Some(a)) if r < a);
                        if !sooner || room < settings.hysteresis_pct {
                            continue;
                        }
                    }
                } else if input.active_headroom.is_some()
                    && lh - active_lh.unwrap_or(0.0) < settings.hysteresis_pct
                {
                    continue;
                }
            }
            let mut key: Vec<f64> = if all_above {
                most_room(num, h)
            } else if consume_first || (weekly_first && trigger == "proactive") {
                let tight = trigger == "proactive" && room < settings.hysteresis_pct;
                vec![flag(tight), reset_ts.unwrap_or(f64::INFINITY), -lh]
            } else {
                vec![-lh]
            };
            if matches!(trigger, "at-limit" | "failover") && !all_above {
                key = if !consume_first {
                    prepend(flag(over), key)
                } else if !over && room >= settings.hysteresis_pct {
                    prepend(0.0, key)
                } else {
                    let mut k = vec![1.0, flag(over)];
                    k.extend(most_room(num, h));
                    k
                };
            }
            qualifying.push((key, num.clone()));
        }
        let mut qualifying = if settings.strategy == "best" && !all_above {
            self.sooner_reset_breaks_near_ties(qualifying, usage, now)
        } else {
            qualifying
        };
        qualifying.sort_by(|a, b| cmp_keys(&a.0, &b.0));
        (
            qualifying.into_iter().map(|(_, n)| n).collect(),
            any_known,
            active_reset_ts,
        )
    }

    /// `best`'s keys end in `-line_headroom`. Candidates within `hysteresis_pct` of the leader's
    /// headroom are a wash on room, so among them the one whose 7-day window resets first wins:
    /// its quota is the first to be lost unused. Candidates further back keep their headroom order.
    fn sooner_reset_breaks_near_ties(
        &self,
        qualifying: Vec<(Vec<f64>, String)>,
        usage: &UsageMap,
        now: f64,
    ) -> Vec<(Vec<f64>, String)> {
        let leader = qualifying
            .iter()
            .filter_map(|(key, _)| key.last().copied())
            .fold(f64::INFINITY, f64::min);
        qualifying
            .into_iter()
            .map(|(key, num)| {
                let Some((&neg_lh, head)) = key.split_last() else {
                    return (key, num);
                };
                let near = neg_lh - leader < self.settings.hysteresis_pct;
                let reset = seven_day_reset_ts(dict_of(usage, &num), now).unwrap_or(f64::INFINITY);
                let key = head
                    .iter()
                    .copied()
                    .chain([flag(!near), reset, neg_lh])
                    .collect();
                (key, num)
            })
            .collect()
    }
}

fn flag(b: bool) -> f64 {
    if b { 1.0 } else { 0.0 }
}

fn prepend(head: f64, tail: Vec<f64>) -> Vec<f64> {
    std::iter::once(head).chain(tail).collect()
}

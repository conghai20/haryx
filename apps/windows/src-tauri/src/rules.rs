//! Pure diffing of two `list --json` snapshots into notices: the same rules and thresholds as the macOS
//! app (`apps/macos/Sources/HaryXCore/NotificationRules.swift`).

use jiff::Timestamp;

use crate::models::{Account, UsageWindow};

pub const USAGE_LEVELS: [f64; 2] = [80.0, 95.0];
pub const EXHAUSTED_PCT: f64 = 95.0;
pub const RECOVERED_PCT: f64 = 50.0;
/// Warn this long before a stored login expires.
pub const EXPIRY_WARNING_S: i64 = 3 * 86_400;
/// Notices whose keys carry no time; the notifier re-arms them once the condition clears.
pub const REARMING_PREFIXES: [&str; 2] = ["relogin:", "plan:"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Category {
    Switched,
    Usage,
    Login,
    Reset,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    /// Dedupe key: the same condition in the same reset window fires once.
    pub key: String,
    pub category: Category,
    pub title: String,
    pub body: String,
}

/// Minute precision: reset times jitter by sub-seconds between polls.
fn window_key(at: Option<Timestamp>) -> String {
    at.map_or_else(
        || "-".to_string(),
        |t| ((t.as_second() as f64) / 60.0).round().to_string(),
    )
}

pub fn evaluate(previous: &[Account], current: &[Account], now: Timestamp) -> Vec<Notice> {
    let mut notices = Vec::new();
    if let (Some(active), Some(before)) = (
        current.iter().find(|a| a.active),
        previous.iter().find(|a| a.active),
    ) && before.identity_key() != active.identity_key()
    {
        notices.push(Notice {
            key: format!("switch:{}:{}", active.identity_key(), now.as_second()),
            category: Category::Switched,
            title: format!("Switched to {}", active.display_name()),
            body: format!("From {}. {}", before.display_name(), usage_line(active)),
        });
    }
    for account in current {
        let name = account.display_name();
        let id = account.identity_key();
        if account.active {
            notices.extend(usage_notices(account));
        }
        if account.usage_status == "relogin_required" {
            notices.push(Notice {
                key: format!("relogin:{id}"),
                category: Category::Login,
                title: format!("{name} needs a re-login"),
                body: "Open HaryX and choose Sign In Again for this account.".into(),
            });
        }
        if let Some(exp) = account.login_expires_at {
            let left = exp.as_second() - now.as_second();
            if left > 0 && left < EXPIRY_WARNING_S {
                notices.push(Notice {
                    key: format!("expiry:{id}:{}", exp.as_second()),
                    category: Category::Login,
                    title: format!("{name} login expires soon"),
                    body: format!(
                        "Expires in {}. Sign in again to renew it.",
                        days_hours(left)
                    ),
                });
            }
        }
        if account.plan.as_ref().and_then(|p| p.expired) == Some(true) {
            notices.push(Notice {
                key: format!("plan:{id}"),
                category: Category::Login,
                title: format!("{name} plan expired"),
                body: "Its subscription is no longer active.".into(),
            });
        }
        if let Some(before) = previous.iter().find(|p| p.identity_key() == id) {
            notices.extend(reset_notices(before, account));
        }
    }
    notices
}

fn windows(a: &Account) -> [(&'static str, Option<&UsageWindow>); 2] {
    let usage = a.usage.as_ref();
    [
        ("5-hour", usage.and_then(|u| u.five_hour.as_ref())),
        ("weekly", usage.and_then(|u| u.seven_day.as_ref())),
    ]
}

fn usage_notices(a: &Account) -> Vec<Notice> {
    let mut out = Vec::new();
    for (label, window) in windows(a) {
        let Some(window) = window else { continue };
        let Some(pct) = window.pct else { continue };
        let reset_key = window_key(window.resets_at);
        if let Some(level) = USAGE_LEVELS.iter().rev().find(|l| pct >= **l) {
            let reset = window
                .countdown
                .as_ref()
                .map(|c| format!(" Resets in {c}."))
                .unwrap_or_default();
            out.push(Notice {
                key: format!("usage:{}:{label}:{level}:{reset_key}", a.identity_key()),
                category: Category::Usage,
                title: format!("{} at {}% of {label} limit", a.display_name(), pct as i64),
                body: format!("Crossed {level}%.{reset}"),
            });
        }
        if window.ahead_of_pace == Some(true)
            && window.will_last_to_reset == Some(false)
            && let Some(run_out) = window.projected_exhaustion_at
        {
            out.push(Notice {
                key: format!("pace:{}:{label}:{reset_key}", a.identity_key()),
                category: Category::Usage,
                title: format!("{} may run out early", a.display_name()),
                body: format!(
                    "At this pace the {label} quota runs out {}, before it resets.",
                    run_out.strftime("%b %-d %H:%M UTC")
                ),
            });
        }
    }
    out
}

fn reset_notices(before: &Account, after: &Account) -> Vec<Notice> {
    windows(before)
        .into_iter()
        .zip(windows(after))
        .filter_map(|((label, old), (_, new))| {
            let (o, n) = (old?.pct?, new?.pct?);
            (o >= EXHAUSTED_PCT && n < RECOVERED_PCT).then(|| Notice {
                key: format!(
                    "reset:{}:{label}:{}",
                    after.identity_key(),
                    window_key(old.and_then(|w| w.resets_at))
                ),
                category: Category::Reset,
                title: format!("{} is ready again", after.display_name()),
                body: format!("Its {label} quota has reset ({}% used).", n as i64),
            })
        })
        .collect()
}

pub fn usage_line(a: &Account) -> String {
    let parts: Vec<String> = [("5h", windows(a)[0].1), ("7d", windows(a)[1].1)]
        .into_iter()
        .filter_map(|(label, w)| {
            w.and_then(|w| w.pct)
                .map(|p| format!("{label} {}%", p as i64))
        })
        .collect();
    if parts.is_empty() {
        a.status_note().unwrap_or("").to_string()
    } else {
        parts.join(" · ")
    }
}

fn days_hours(seconds: i64) -> String {
    let days = seconds / 86_400;
    let hours = (seconds % 86_400) / 3_600;
    match (days, hours) {
        (0, h) => format!("{}h", h.max(1)),
        (d, 0) => format!("{d}d"),
        (d, h) => format!("{d}d {h}h"),
    }
}

/// Remembers which notices already fired, so each condition notifies once.
#[derive(Debug, Default)]
pub struct Dedupe {
    seen: std::collections::HashSet<String>,
}

impl Dedupe {
    /// The notices not shown before. Re-arming notices that are no longer present are forgotten, so
    /// they fire again if the condition comes back.
    pub fn fresh(&mut self, notices: Vec<Notice>) -> Vec<Notice> {
        let present: std::collections::HashSet<&str> =
            notices.iter().map(|n| n.key.as_str()).collect();
        self.seen.retain(|k| {
            !REARMING_PREFIXES.iter().any(|p| k.starts_with(p)) || present.contains(k.as_str())
        });
        notices
            .into_iter()
            .filter(|n| self.seen.insert(n.key.clone()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account(json: &str) -> Account {
        serde_json::from_str(json).unwrap()
    }

    fn now() -> Timestamp {
        "2026-11-24T00:00:00Z".parse().unwrap()
    }

    #[test]
    fn switch_is_announced_with_the_new_usage() {
        let a = account(r#"{"number":1,"email":"a@x.com","active":true,"usageStatus":"ok"}"#);
        let b = account(
            r#"{"number":2,"email":"b@x.com","alias":"work","active":true,"usageStatus":"ok",
               "usage":{"fiveHour":{"pct":12.0},"sevenDay":{"pct":40.0}}}"#,
        );
        let n = evaluate(&[a], &[b], now());
        assert_eq!(n[0].category, Category::Switched);
        assert_eq!(n[0].title, "Switched to work");
        assert_eq!(n[0].body, "From a@x.com. 5h 12% · 7d 40%");
    }

    #[test]
    fn usage_fires_at_the_highest_crossed_level_for_the_active_account_only() {
        let hot = r#"{"number":1,"email":"a@x.com","active":ACTIVE,"usageStatus":"ok",
            "usage":{"fiveHour":{"pct":96.0,"countdown":"40m","resetsAt":"2026-11-24T01:00:00Z"}}}"#;
        let active = account(&hot.replace("ACTIVE", "true"));
        let notices = evaluate(&[], &[active], now());
        assert_eq!(notices.len(), 1);
        assert_eq!(notices[0].title, "a@x.com at 96% of 5-hour limit");
        assert_eq!(notices[0].body, "Crossed 95%. Resets in 40m.");
        let idle = account(&hot.replace("ACTIVE", "false"));
        assert!(evaluate(&[], &[idle], now()).is_empty());
    }

    #[test]
    fn reset_fires_when_an_exhausted_window_recovers() {
        let before = account(
            r#"{"number":1,"email":"a@x.com","usageStatus":"ok","usage":{"fiveHour":{"pct":99.0}}}"#,
        );
        let after = account(
            r#"{"number":1,"email":"a@x.com","usageStatus":"ok","usage":{"fiveHour":{"pct":3.0}}}"#,
        );
        let n = evaluate(&[before], &[after], now());
        assert_eq!(n.len(), 1);
        assert_eq!(n[0].category, Category::Reset);
        assert_eq!(n[0].body, "Its 5-hour quota has reset (3% used).");
    }

    #[test]
    fn login_expiry_warns_inside_three_days() {
        let soon = account(
            r#"{"number":1,"email":"a@x.com","usageStatus":"ok","loginExpiresAt":"2026-11-25T06:00:00Z"}"#,
        );
        let n = evaluate(&[], &[soon], now());
        assert_eq!(n[0].title, "a@x.com login expires soon");
        assert_eq!(n[0].body, "Expires in 1d 6h. Sign in again to renew it.");
        let later = account(
            r#"{"number":1,"email":"a@x.com","usageStatus":"ok","loginExpiresAt":"2026-12-24T00:00:00Z"}"#,
        );
        assert!(evaluate(&[], &[later], now()).is_empty());
    }

    #[test]
    fn dedupe_fires_once_and_rearms_relogin_when_it_clears() {
        let relogin = account(r#"{"number":1,"email":"a@x.com","usageStatus":"relogin_required"}"#);
        let fine = account(r#"{"number":1,"email":"a@x.com","usageStatus":"ok"}"#);
        let mut dedupe = Dedupe::default();
        assert_eq!(
            dedupe
                .fresh(evaluate(&[], std::slice::from_ref(&relogin), now()))
                .len(),
            1
        );
        assert!(
            dedupe
                .fresh(evaluate(&[], std::slice::from_ref(&relogin), now()))
                .is_empty()
        );
        assert!(dedupe.fresh(evaluate(&[], &[fine], now())).is_empty());
        assert_eq!(dedupe.fresh(evaluate(&[], &[relogin], now())).len(), 1);
    }
}

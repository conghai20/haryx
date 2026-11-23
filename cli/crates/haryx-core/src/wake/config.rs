//! Wake settings: which accounts, quiet hours and the weekly cap.

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// Default 7-day utilization at or above which a wake is skipped.
pub const DEFAULT_WEEKLY_CAP_PCT: f64 = 90.0;

/// A local time range in minutes since midnight; `start > end` wraps past midnight.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuietHours {
    pub start: u16,
    pub end: u16,
}

impl QuietHours {
    /// Parse `HH:MM-HH:MM`.
    pub fn parse(text: &str) -> Result<Self> {
        let invalid =
            || Error::Validation(format!("quiet hours must be HH:MM-HH:MM, got '{text}'"));
        let (a, b) = text.trim().split_once('-').ok_or_else(invalid)?;
        let start = parse_clock(a).ok_or_else(invalid)?;
        let end = parse_clock(b).ok_or_else(invalid)?;
        if start == end {
            return Err(Error::Validation(
                "quiet hours must not start and end at the same time".into(),
            ));
        }
        Ok(QuietHours { start, end })
    }

    pub fn format(self) -> String {
        format!(
            "{:02}:{:02}-{:02}:{:02}",
            self.start / 60,
            self.start % 60,
            self.end / 60,
            self.end % 60
        )
    }
}

fn parse_clock(text: &str) -> Option<u16> {
    let (h, m) = text.trim().split_once(':')?;
    if h.is_empty() || h.len() > 2 || m.len() != 2 {
        return None;
    }
    let (h, m): (u16, u16) = (h.parse().ok()?, m.parse().ok()?);
    (h < 24 && m < 60).then_some(h * 60 + m)
}

#[derive(Debug, Clone, PartialEq)]
pub struct WakeConfig {
    /// Opted-in account numbers, sorted and unique.
    pub accounts: Vec<String>,
    pub quiet_hours: Option<QuietHours>,
    pub weekly_cap_pct: f64,
}

impl Default for WakeConfig {
    fn default() -> Self {
        WakeConfig {
            accounts: Vec::new(),
            quiet_hours: None,
            weekly_cap_pct: DEFAULT_WEEKLY_CAP_PCT,
        }
    }
}

/// Validate a weekly cap in percent.
pub fn parse_weekly_cap(value: f64) -> Result<f64> {
    if value.is_finite() && (1.0..=100.0).contains(&value) {
        Ok(value)
    } else {
        Err(Error::Validation(
            "weekly cap must be between 1 and 100".into(),
        ))
    }
}

/// On-disk shape of the settings part of `wake.json`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ConfigDoc {
    #[serde(default)]
    pub accounts: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quiet_hours: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weekly_cap_pct: Option<f64>,
}

impl ConfigDoc {
    /// Read leniently: a malformed value falls back to its default with a warning.
    pub fn to_config(&self) -> WakeConfig {
        let quiet_hours = self.quiet_hours.as_deref().and_then(|q| {
            QuietHours::parse(q)
                .map_err(|e| log::warn!("wake: ignoring quiet hours: {e}"))
                .ok()
        });
        let weekly_cap_pct = self
            .weekly_cap_pct
            .and_then(|v| parse_weekly_cap(v).ok())
            .unwrap_or(DEFAULT_WEEKLY_CAP_PCT);
        let mut accounts = self.accounts.clone();
        accounts.sort_by_key(|n| n.parse::<i64>().unwrap_or(i64::MAX));
        accounts.dedup();
        WakeConfig {
            accounts,
            quiet_hours,
            weekly_cap_pct,
        }
    }

    pub fn from_config(config: &WakeConfig) -> Self {
        ConfigDoc {
            accounts: config.accounts.clone(),
            quiet_hours: config.quiet_hours.map(QuietHours::format),
            weekly_cap_pct: (config.weekly_cap_pct != DEFAULT_WEEKLY_CAP_PCT)
                .then_some(config.weekly_cap_pct),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_quiet_hours() {
        let q = QuietHours::parse("01:00-06:30").unwrap();
        assert_eq!((q.start, q.end), (60, 390));
        assert_eq!(q.format(), "01:00-06:30");
    }

    #[test]
    fn rejects_bad_quiet_hours() {
        for bad in [
            "",
            "1-6",
            "25:00-06:00",
            "01:60-02:00",
            "01:00",
            "03:00-03:00",
            "1:0-2:00",
        ] {
            assert!(QuietHours::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn weekly_cap_bounds() {
        assert!(parse_weekly_cap(90.0).is_ok());
        assert!(parse_weekly_cap(0.0).is_err());
        assert!(parse_weekly_cap(101.0).is_err());
        assert!(parse_weekly_cap(f64::NAN).is_err());
    }

    #[test]
    fn doc_round_trip_sorts_and_drops_bad_values() {
        let doc = ConfigDoc {
            accounts: vec!["3".into(), "1".into(), "3".into()],
            quiet_hours: Some("nope".into()),
            weekly_cap_pct: Some(500.0),
        };
        let config = doc.to_config();
        assert_eq!(config.accounts, vec!["1", "3"]);
        assert_eq!(config.quiet_hours, None);
        assert_eq!(config.weekly_cap_pct, DEFAULT_WEEKLY_CAP_PCT);
        let back = ConfigDoc::from_config(&config);
        assert_eq!(back.weekly_cap_pct, None);
    }
}

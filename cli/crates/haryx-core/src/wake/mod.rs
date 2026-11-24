//! `haryx ai wake`: start the next 5-hour window as soon as the last one resets, with one
//! 1-output-token request, for the accounts the user opted in (plan: `docs/ai-wake-plan.md`).

pub mod agent;
pub mod config;
pub mod decide;
pub mod state;
pub mod task;
pub mod tick;

pub use config::{DEFAULT_WEEKLY_CAP_PCT, QuietHours, WakeConfig, parse_weekly_cap};
pub use decide::{Decision, GRACE_S, Now, SkipReason, WindowRead, decide};
pub use state::{AccountState, WakeStore};
pub use tick::{Outcome, run_tick, send_probe};

/// The word that names every account in `on`/`off`.
pub const ALL: &str = "all";

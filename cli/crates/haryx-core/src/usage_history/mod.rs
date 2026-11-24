//! Usage history: tokens and API-price cost by account, project, session and model, from Claude
//! Code's own transcripts, plus 5h/7d quota readings over time. Local only; no conversation text
//! is stored. Plan: `docs/usage-history-plan.md`.
//!
//! Files under `<store>/usage/`: `index.json` (how far each transcript was read), `days/*.jsonl`
//! (aggregated rows), `sessions.jsonl` (session → account ledger), `quota.jsonl` (readings).

pub mod attribution;
pub mod index;
pub mod pricing;
pub mod quota;
pub mod report;
pub mod transcript;

use std::path::{Path, PathBuf};

pub const DIR: &str = "usage";
pub const LEDGER: &str = "sessions.jsonl";
pub const QUOTA: &str = "quota.jsonl";
pub const INDEX: &str = "index.json";
pub const DAYS: &str = "days";

/// `<store>/usage`.
pub fn history_dir(store_root: &Path) -> PathBuf {
    store_root.join(DIR)
}

/// Token counts by kind.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tokens {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write_5m: u64,
    pub cache_write_1h: u64,
}

impl Tokens {
    pub fn add(&mut self, other: &Tokens) {
        self.input += other.input;
        self.output += other.output;
        self.cache_read += other.cache_read;
        self.cache_write_5m += other.cache_write_5m;
        self.cache_write_1h += other.cache_write_1h;
    }

    pub fn total(&self) -> u64 {
        self.input + self.output + self.cache_read + self.cache_write_5m + self.cache_write_1h
    }
}

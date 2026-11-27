//! haryx-core: the account store, credentials, usage, switching, sessions and auto-switch engine behind the
//! `haryx` CLI. A Rust port of claude-swap 1.2.0b1 (MIT); each module names the upstream module it ports.
//!
//! Surfaces (the CLI, TUI) live in the `haryx` crate and hold no account logic.

pub mod error;

// Storage and platform (leaves).
pub mod claude_locks;
pub mod fsutil;
pub mod keychain;
pub mod locking;
pub mod paths;
pub mod process;
pub mod spawn;

// Formatting and policy (leaves).
pub mod appearance;
pub mod lines;
pub mod pace;
pub mod poll_policy;
pub mod printer;

// Core.
pub mod account;
pub mod autoswitch;
pub mod claude_setup;
pub mod contracts;
pub mod credentials;
pub mod desktop;
pub mod handoff;
pub mod json_output;
pub mod mappings;
pub mod models;
pub mod oauth;
pub mod profile;
pub mod release;
pub mod rotation;
pub mod running;
pub mod self_update;
pub mod service;
pub mod session;
pub mod session_history;
pub mod session_manager;
pub mod settings;
pub mod snapshot;
pub mod subscription;
pub mod switcher;
pub mod sync;
pub mod transfer;
pub mod update_check;
pub mod usage_history;
pub mod usage_share;
pub mod usage_store;
pub mod wake;

pub use error::{Error, Result};

/// Program name shown in messages and help.
pub const PROG: &str = "haryx";
/// The AI-accounts section every account, usage and session command lives under (`haryx ai list`).
pub const AI_PROG: &str = "haryx ai";
/// Version of haryx itself.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// The claude-swap release whose behaviour this build reproduces.
pub const UPSTREAM_VERSION: &str = "1.2.0b1";

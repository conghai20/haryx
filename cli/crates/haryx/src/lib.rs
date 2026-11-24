//! `haryx`: the command-line surface (argument parsing and dispatch). Account logic lives in
//! `haryx-core`; this crate only turns argv into a [`cli::Command`] and routes it.

pub mod cli;
pub mod commands;
pub mod entry;
pub mod logging;
pub mod tui;

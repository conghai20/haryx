//! `haryxw`: haryx without a console window, for the Windows wake task (a console program started by
//! Task Scheduler would flash a window every minute). Same commands as `haryx`; leading
//! `--env NAME=VALUE` pairs set the store variables the task cannot carry. Output goes nowhere unless
//! redirected, so the wake tick logs to its file as usual.
#![cfg_attr(windows, windows_subsystem = "windows")]

use std::process::ExitCode;

fn main() -> ExitCode {
    let (env, argv) = match haryx::entry::split_env_args(&haryx::entry::args()) {
        Ok(split) => split,
        Err(message) => {
            eprintln!("haryxw: {message}");
            return ExitCode::from(2);
        }
    };
    for (name, value) in env {
        // SAFETY: at startup, before any other thread exists; only haryx's own store variables.
        unsafe { std::env::set_var(name, value) };
    }
    haryx::entry::run(&argv)
}

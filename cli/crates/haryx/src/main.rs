//! `haryx`: command-line entry point. Argument parsing and rendering only; logic lives in `haryx-core`.

use std::io::IsTerminal;
use std::process::ExitCode;

use haryx::{cli, commands};

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args_os()
        .skip(1)
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    // Bare `haryx` opens the TUI only when a person is at both ends; scripts get the usage error.
    let interactive = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
    let invocation = match cli::parse(&argv, interactive) {
        Ok(invocation) => invocation,
        // Help and version go to stdout (exit 0); usage errors to stderr (exit 2).
        Err(err) => err.exit(),
    };
    init_logging(invocation.debug);
    match commands::dispatch(invocation.command) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("{err}");
            ExitCode::from(err.exit_code)
        }
    }
}

/// `--debug` turns on debug-level logging; otherwise only warnings. Logs go to stderr.
fn init_logging(debug: bool) {
    let level = if debug {
        log::LevelFilter::Debug
    } else {
        log::LevelFilter::Warn
    };
    env_logger::Builder::new()
        .filter_level(level)
        .target(env_logger::Target::Stderr)
        .init();
}

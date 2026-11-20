//! `haryx`: command-line entry point. Argument parsing and rendering only; logic lives in `haryx-core`.

use std::io::IsTerminal;
use std::process::ExitCode;

use haryx::{cli, commands, logging};

fn main() -> ExitCode {
    restore_sigpipe();
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
    logging::init(invocation.debug);
    match commands::dispatch(invocation.command, invocation.debug) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            if !err.message.is_empty() {
                eprintln!("{err}");
            }
            ExitCode::from(err.exit_code)
        }
    }
}

/// Die quietly on a closed pipe (`haryx list | head`) instead of panicking on the failed write.
fn restore_sigpipe() {
    #[cfg(unix)]
    // SAFETY: resetting SIGPIPE to its default disposition at startup, before any thread exists.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
}

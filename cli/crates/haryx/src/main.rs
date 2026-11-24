//! `haryx`: command-line entry point. Argument parsing and rendering only; logic lives in `haryx-core`.

use std::process::ExitCode;

fn main() -> ExitCode {
    restore_sigpipe();
    haryx::entry::run(&haryx::entry::args())
}

/// Die quietly on a closed pipe (`haryx list | head`) instead of panicking on the failed write.
fn restore_sigpipe() {
    #[cfg(unix)]
    // SAFETY: resetting SIGPIPE to its default disposition at startup, before any thread exists.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
}

//! Open a URL in the user's browser.
//!
//! `$BROWSER`, when set, is run with the URL as its only argument (a program path, not a template).
//! Otherwise: `open` on macOS, `rundll32 url.dll,FileProtocolHandler` on Windows (which, unlike
//! `cmd /c start`, does not split the URL at `&`), and `xdg-open` elsewhere.

use std::process::{Command, Stdio};

pub const BROWSER_ENV: &str = "BROWSER";

/// Start the browser on `url` without waiting for it. `false` when nothing could be started.
pub fn open(url: &str) -> bool {
    let mut command = match std::env::var_os(BROWSER_ENV).filter(|v| !v.is_empty()) {
        Some(program) => Command::new(program),
        None => platform_opener(),
    };
    let spawned = command
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    match spawned {
        Ok(mut child) => {
            // Reap it whenever it exits; the sign-in does not wait on the browser.
            std::thread::spawn(move || child.wait());
            true
        }
        Err(err) => {
            log::debug!("could not start a browser: {err}");
            false
        }
    }
}

fn platform_opener() -> Command {
    if cfg!(target_os = "macos") {
        Command::new("/usr/bin/open")
    } else if cfg!(windows) {
        // By full path: a `rundll32.exe` earlier in the search path must not get the URL.
        let root = std::env::var_os("SystemRoot").unwrap_or_else(|| r"C:\Windows".into());
        let mut command = Command::new(std::path::Path::new(&root).join(r"System32\rundll32.exe"));
        command.arg("url.dll,FileProtocolHandler");
        command
    } else {
        Command::new("xdg-open")
    }
}

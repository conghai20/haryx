//! Runs the `haryx` CLI. Arguments go straight to the process (never through a shell), secrets go on
//! stdin, and on Windows no console window opens. The CLI owns accounts and credentials; the app only
//! asks it.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Bound on a normal command. `list` takes about 0.25 s; a slow network read stays well under this.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);
/// `CREATE_NO_WINDOW`: a console program started from the tray app must not flash a window.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Output {
    pub status: i32,
    pub stdout: String,
    pub stderr: String,
}

impl Output {
    pub fn success(&self) -> bool {
        self.status == 0
    }

    /// The message to show for a failed run: the CLI's own words (the `error.message` of a JSON error
    /// envelope when it printed one), else the exit code.
    pub fn error_message(&self) -> String {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(self.stdout.trim())
            && let Some(message) = value
                .get("error")
                .and_then(|e| e.get("message").or(Some(e)))
                .and_then(serde_json::Value::as_str)
        {
            return message.to_string();
        }
        let text = self.stderr.trim();
        let text = if text.is_empty() {
            self.stdout.trim()
        } else {
            text
        };
        if text.is_empty() {
            format!("haryx exited with code {}", self.status)
        } else {
            strip_ansi(text)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CliError {
    NotFound,
    Spawn(String),
    TimedOut(Duration),
}

impl std::fmt::Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CliError::NotFound => write!(
                f,
                "haryx not found. Install it with `winget install HaryX.haryx`, `npm i -g haryx` or \
                 `cargo install haryx`, or set its path in Settings."
            ),
            CliError::Spawn(e) => write!(f, "could not start haryx: {e}"),
            CliError::TimedOut(d) => write!(f, "haryx did not answer within {}s", d.as_secs()),
        }
    }
}

/// The program file name on this OS.
pub fn program_name() -> &'static str {
    if cfg!(windows) { "haryx.exe" } else { "haryx" }
}

/// Where to look, most preferred first: an explicit override, then the user's own installs (so an
/// upgradable copy wins), then the copy bundled next to the app.
pub fn candidates(
    override_path: Option<&Path>,
    path_var: Option<&std::ffi::OsStr>,
    home: Option<&Path>,
    bundled: Option<&Path>,
) -> Vec<PathBuf> {
    let name = program_name();
    let mut out = Vec::new();
    if let Some(p) = override_path {
        out.push(p.to_path_buf());
    }
    if let Some(path_var) = path_var {
        out.extend(std::env::split_paths(path_var).map(|dir| dir.join(name)));
    }
    if let Some(home) = home {
        out.push(home.join(".cargo").join("bin").join(name));
        out.push(home.join(".local").join("bin").join(name));
    }
    if let Some(dir) = bundled {
        out.push(dir.join(name));
    }
    out
}

/// The first candidate that is a file.
pub fn locate(candidates: &[PathBuf]) -> Option<PathBuf> {
    candidates.iter().find(|p| p.is_file()).cloned()
}

/// One CLI, found once.
#[derive(Debug, Clone)]
pub struct Cli {
    pub exe: PathBuf,
}

impl Cli {
    pub fn new(exe: PathBuf) -> Self {
        Cli { exe }
    }

    /// Run `haryx <args>` with `stdin` text, `env` additions, and a timeout.
    pub fn run(
        &self,
        args: &[&str],
        stdin: Option<&str>,
        env: &[(&str, &str)],
        timeout: Duration,
    ) -> Result<Output, CliError> {
        let mut command = self.command(args);
        command
            .envs(env.iter().copied())
            .env("NO_COLOR", "1")
            .stdin(if stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn().map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => CliError::NotFound,
            _ => CliError::Spawn(e.to_string()),
        })?;
        if let (Some(text), Some(mut pipe)) = (stdin, child.stdin.take()) {
            let _ = pipe.write_all(text.as_bytes());
        }
        let stdout = drain(child.stdout.take());
        let stderr = drain(child.stderr.take());
        let started = Instant::now();
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if started.elapsed() >= timeout => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(CliError::TimedOut(timeout));
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(10)),
                Err(e) => return Err(CliError::Spawn(e.to_string())),
            }
        };
        Ok(Output {
            status: status.code().unwrap_or(-1),
            stdout: stdout.join().unwrap_or_default(),
            stderr: stderr.join().unwrap_or_default(),
        })
    }

    /// A `Command` for `haryx <args>` with no console window, for callers that stream its output.
    pub fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(&self.exe);
        command.args(args);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(CREATE_NO_WINDOW);
        }
        command
    }

    /// Run and require success; the error is the CLI's message.
    pub fn ok(&self, args: &[&str]) -> Result<String, String> {
        let out = self
            .run(args, None, &[], DEFAULT_TIMEOUT)
            .map_err(|e| e.to_string())?;
        if out.success() {
            Ok(out.stdout)
        } else {
            Err(out.error_message())
        }
    }

    /// Run a `--json` command and parse its stdout.
    pub fn json(&self, args: &[&str]) -> Result<serde_json::Value, String> {
        let out = self
            .run(args, None, &[], DEFAULT_TIMEOUT)
            .map_err(|e| e.to_string())?;
        // Some commands print a JSON error envelope with a non-zero code; prefer it when it parses.
        match serde_json::from_str::<serde_json::Value>(out.stdout.trim()) {
            Ok(value) if out.success() => Ok(value),
            Ok(value) => Err(value
                .get("error")
                .and_then(|e| e.get("message").or(Some(e)))
                .and_then(|m| m.as_str())
                .map(str::to_owned)
                .unwrap_or_else(|| out.error_message())),
            Err(_) if out.success() => {
                Err(format!("unexpected output from `haryx {}`", args.join(" ")))
            }
            Err(_) => Err(out.error_message()),
        }
    }
}

fn drain<R: Read + Send + 'static>(pipe: Option<R>) -> std::thread::JoinHandle<String> {
    std::thread::spawn(move || {
        let mut text = String::new();
        if let Some(mut pipe) = pipe {
            let mut bytes = Vec::new();
            let _ = pipe.read_to_end(&mut bytes);
            text = String::from_utf8_lossy(&bytes).into_owned();
        }
        text
    })
}

/// Drop ANSI escape sequences (`ESC [ ... letter`) from CLI text shown in the UI.
pub fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next();
            for d in chars.by_ref() {
                if d.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidates_prefer_override_then_path_then_home_then_bundle() {
        let path = std::env::join_paths([PathBuf::from("/p1"), PathBuf::from("/p2")]).unwrap();
        let list = candidates(
            Some(Path::new("/custom/haryx")),
            Some(&path),
            Some(Path::new("/home/u")),
            Some(Path::new("/app/bin")),
        );
        let name = program_name();
        assert_eq!(list[0], PathBuf::from("/custom/haryx"));
        assert_eq!(list[1], PathBuf::from("/p1").join(name));
        assert_eq!(list[2], PathBuf::from("/p2").join(name));
        assert_eq!(
            list[3],
            PathBuf::from("/home/u")
                .join(".cargo")
                .join("bin")
                .join(name)
        );
        assert_eq!(list.last().unwrap(), &PathBuf::from("/app/bin").join(name));
    }

    #[test]
    fn locate_takes_the_first_existing_file() {
        let tmp = tempfile::tempdir().unwrap();
        let real = tmp.path().join("b");
        std::fs::write(&real, "").unwrap();
        let found = locate(&[tmp.path().join("a"), real.clone(), tmp.path().join("c")]);
        assert_eq!(found, Some(real));
        assert_eq!(locate(&[tmp.path().join("nope")]), None);
    }

    #[test]
    fn error_message_prefers_stderr_and_strips_colour() {
        let out = Output {
            status: 1,
            stdout: "ignored".into(),
            stderr: "\u{1b}[31mError:\u{1b}[0m no such account\n".into(),
        };
        assert_eq!(out.error_message(), "Error: no such account");
        let silent = Output {
            status: 3,
            stdout: String::new(),
            stderr: String::new(),
        };
        assert_eq!(silent.error_message(), "haryx exited with code 3");
    }

    #[test]
    fn error_message_reads_a_json_error_envelope() {
        let out = Output {
            status: 1,
            stdout: r#"{"success": false, "error": {"code": "not_found", "message": "Account-9 does not exist"}}"#.into(),
            stderr: "noise".into(),
        };
        assert_eq!(out.error_message(), "Account-9 does not exist");
    }

    #[test]
    fn missing_program_is_not_found() {
        let cli = Cli::new(PathBuf::from("/definitely/not/here/haryx"));
        let err = cli
            .run(&["list"], None, &[], Duration::from_secs(5))
            .unwrap_err();
        assert_eq!(err, CliError::NotFound);
    }

    #[cfg(unix)]
    #[test]
    fn runs_with_stdin_and_times_out() {
        let tmp = tempfile::tempdir().unwrap();
        let script = tmp.path().join("fake");
        std::fs::write(&script, "#!/bin/sh\nread x\necho \"got $x $1\"\n").unwrap();
        let slow = tmp.path().join("slow");
        std::fs::write(&slow, "#!/bin/sh\nsleep 5\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        for p in [&script, &slow] {
            std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let out = Cli::new(script)
            .run(&["arg"], Some("secret\n"), &[], Duration::from_secs(5))
            .unwrap();
        assert_eq!(out.stdout.trim(), "got secret arg");
        let err = Cli::new(slow)
            .run(&[], None, &[], Duration::from_millis(200))
            .unwrap_err();
        assert_eq!(err, CliError::TimedOut(Duration::from_millis(200)));
    }
}

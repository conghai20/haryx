//! Port of `claude_swap/spawn.py`: run short-lived helpers (`ps`, `security`, `claude`) with captured output,
//! optional stdin and a time limit.
//!
//! Upstream exists to keep helpers off `fork()` on macOS: once the kernel has sent a process a critical
//! memory-pressure note, libmalloc runs MallocStackLogging's fork hook in every forked child, which prints
//! `MallocStackLogging: can't turn off malloc stack logging ...` onto the inherited stderr (upstream #517).
//! Python's `subprocess` forks unless the executable is directory-qualified and `close_fds` is false, so
//! upstream resolves bare names itself and checks for inheritable descriptors.
//!
//! In Rust, `std::process::Command` already launches through `posix_spawn` on macOS, and every descriptor
//! std creates is close-on-exec, so the descriptor scan has no counterpart. One case still sends std back to
//! `fork`: a bare program name combined with a changed `PATH` in the child's environment. [`run`] therefore
//! keeps upstream's resolution step: a directory-qualified program is never looked up (so
//! `/usr/bin/security` stays pinned), and a bare name is searched on the PATH the child would get and made
//! absolute.

use std::ffi::{OsStr, OsString};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// How often a timed run polls its child.
const POLL_INTERVAL: Duration = Duration::from_millis(5);
/// Upstream's fallback search path for an environment without `PATH` (CPython's `os.defpath`).
const DEFAULT_PATH: &str = "/bin:/usr/bin";

/// What to run besides the argv.
#[derive(Clone, Debug, Default)]
pub struct RunOptions {
    /// Variables set (on top of the inherited environment) in the child.
    pub env: Vec<(OsString, OsString)>,
    /// Bytes written to the child's stdin, which is then closed. `None` gives the child an empty stdin.
    pub input: Option<Vec<u8>>,
    /// Kill the child and fail with [`SpawnError::Timeout`] when it runs longer.
    pub timeout: Option<Duration>,
}

/// A finished run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunOutput {
    /// Exit code; `None` when the child was ended by a signal.
    pub code: Option<i32>,
    /// Captured stdout, decoded as UTF-8 (lossily).
    pub stdout: String,
    /// Captured stderr, decoded as UTF-8 (lossily).
    pub stderr: String,
}

impl RunOutput {
    pub fn success(&self) -> bool {
        self.code == Some(0)
    }
}

/// Why a run did not finish.
#[derive(Debug, thiserror::Error)]
pub enum SpawnError {
    /// The child outlived its time limit and was killed (upstream `subprocess.TimeoutExpired`).
    #[error("{program} timed out after {}s", .timeout.as_secs_f64())]
    Timeout { program: String, timeout: Duration },
    /// The program could not be started or waited on (upstream `OSError`).
    #[error("{0}")]
    Io(#[from] io::Error),
}

/// Run `argv` to completion, capturing stdout and stderr. See the module docs.
pub fn run<S: AsRef<OsStr>>(argv: &[S], opts: &RunOptions) -> Result<RunOutput, SpawnError> {
    let Some((program, args)) = argv.split_first() else {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "empty command").into());
    };
    let program = program.as_ref();
    let child_path = child_path_var(&opts.env);
    let executable =
        resolve(program, child_path.as_deref()).unwrap_or_else(|| PathBuf::from(program));

    let mut command = Command::new(&executable);
    command
        .args(args)
        .envs(opts.env.iter().map(|(k, v)| (k, v)))
        .stdin(if opts.input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn()?;

    let writer = opts.input.clone().and_then(|input| {
        let mut stdin = child.stdin.take()?;
        // A child that exits without reading its stdin breaks the pipe; that is not our failure.
        Some(std::thread::spawn(move || {
            let _ = stdin.write_all(&input);
        }))
    });
    let stdout = child.stdout.take().map(drain);
    let stderr = child.stderr.take().map(drain);

    let status = match wait(&mut child, opts.timeout) {
        Ok(Some(status)) => status,
        Ok(None) => {
            let _ = child.kill();
            let _ = child.wait();
            // Reader threads are left behind: a grandchild may still hold the pipes open.
            return Err(SpawnError::Timeout {
                program: executable.display().to_string(),
                timeout: opts.timeout.unwrap_or_default(),
            });
        }
        Err(err) => return Err(err.into()),
    };
    if let Some(writer) = writer {
        let _ = writer.join();
    }
    Ok(RunOutput {
        code: status.code(),
        stdout: collect(stdout),
        stderr: collect(stderr),
    })
}

/// Wait for `child`; `Ok(None)` when `timeout` passed first.
fn wait(child: &mut Child, timeout: Option<Duration>) -> io::Result<Option<ExitStatus>> {
    let Some(timeout) = timeout else {
        return child.wait().map(Some);
    };
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(Some(status));
        }
        if start.elapsed() >= timeout {
            return Ok(None);
        }
        std::thread::sleep(POLL_INTERVAL.min(timeout.saturating_sub(start.elapsed())));
    }
}

fn drain<R: Read + Send + 'static>(mut pipe: R) -> JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = pipe.read_to_end(&mut buf);
        buf
    })
}

fn collect(reader: Option<JoinHandle<Vec<u8>>>) -> String {
    let bytes = reader.and_then(|r| r.join().ok()).unwrap_or_default();
    String::from_utf8_lossy(&bytes).into_owned()
}

/// The `PATH` the child gets: the last override in `env`, else the parent's.
fn child_path_var(env: &[(OsString, OsString)]) -> Option<OsString> {
    env.iter()
        .rev()
        .find(|(k, _)| k == "PATH")
        .map(|(_, v)| v.clone())
        .or_else(|| std::env::var_os("PATH"))
}

/// `name` when directory-qualified, else its absolute hit on `path_var` (upstream's default path when
/// `None`), else `None`.
///
/// An empty `PATH` entry means the current directory, as in a shell; such a hit is made absolute.
pub fn resolve(name: &OsStr, path_var: Option<&OsStr>) -> Option<PathBuf> {
    let candidate = Path::new(name);
    if candidate.components().count() > 1 || candidate.is_absolute() {
        return Some(candidate.to_path_buf());
    }
    if name.is_empty() {
        return None;
    }
    let search = path_var.unwrap_or(OsStr::new(DEFAULT_PATH));
    std::env::split_paths(search).find_map(|dir| {
        let dir = if dir.as_os_str().is_empty() {
            PathBuf::from(".")
        } else {
            dir
        };
        let hit = dir.join(name);
        is_executable_file(&hit).then(|| std::path::absolute(&hit).unwrap_or(hit))
    })
}

#[cfg(unix)]
fn is_executable_file(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable_file(path: &Path) -> bool {
    path.is_file()
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use tempfile::TempDir;

    fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[test]
    fn captures_stdout_stderr_and_exit_code() {
        let out = run(
            &["/bin/sh", "-c", "echo out; echo err >&2; exit 3"],
            &RunOptions::default(),
        )
        .unwrap();
        assert_eq!(out.code, Some(3));
        assert_eq!(out.stdout, "out\n");
        assert_eq!(out.stderr, "err\n");
        assert!(!out.success());
    }

    #[test]
    fn stdin_input_reaches_the_child() {
        let opts = RunOptions {
            input: Some(b"hello\n".to_vec()),
            ..Default::default()
        };
        let out = run(&["/bin/cat"], &opts).unwrap();
        assert_eq!(out.stdout, "hello\n");
        assert!(out.success());
    }

    #[test]
    fn env_overrides_reach_the_child() {
        let opts = RunOptions {
            env: vec![("HARYX_SPAWN_TEST".into(), "yes".into())],
            ..Default::default()
        };
        let out = run(&["/bin/sh", "-c", "printf %s \"$HARYX_SPAWN_TEST\""], &opts).unwrap();
        assert_eq!(out.stdout, "yes");
    }

    #[test]
    fn timeout_kills_the_child() {
        let opts = RunOptions {
            timeout: Some(Duration::from_millis(100)),
            ..Default::default()
        };
        let started = Instant::now();
        let err = run(&["/bin/sleep", "5"], &opts).unwrap_err();
        assert!(matches!(err, SpawnError::Timeout { .. }));
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn missing_program_is_an_io_error() {
        let err = run(
            &["/nonexistent/haryx-no-such-binary"],
            &RunOptions::default(),
        )
        .unwrap_err();
        assert!(matches!(err, SpawnError::Io(_)));
        let empty: [&str; 0] = [];
        assert!(matches!(
            run(&empty, &RunOptions::default()),
            Err(SpawnError::Io(_))
        ));
    }

    #[test]
    fn directory_qualified_executable_is_never_looked_up() {
        let tmp = TempDir::new().unwrap();
        script(tmp.path(), "security", "echo planted");
        let path = tmp.path().as_os_str();
        assert_eq!(
            resolve(OsStr::new("/usr/bin/security"), Some(path)),
            Some(PathBuf::from("/usr/bin/security"))
        );
        assert_eq!(
            resolve(OsStr::new("./x"), Some(path)),
            Some(PathBuf::from("./x"))
        );
    }

    #[test]
    fn bare_names_are_resolved_on_the_given_path_only() {
        let tmp = TempDir::new().unwrap();
        let tool = script(tmp.path(), "haryx-tool", "echo hi");
        std::fs::write(tmp.path().join("not-exec"), "").unwrap();
        assert_eq!(
            resolve(OsStr::new("haryx-tool"), Some(tmp.path().as_os_str())),
            Some(tool)
        );
        assert_eq!(
            resolve(OsStr::new("not-exec"), Some(tmp.path().as_os_str())),
            None
        );
        assert_eq!(
            resolve(OsStr::new("haryx-tool"), Some(OsStr::new("/nonexistent"))),
            None
        );
    }

    #[test]
    fn lookup_uses_the_path_the_child_would_get() {
        let tmp = TempDir::new().unwrap();
        script(tmp.path(), "haryx-tool", "echo from-child-path");
        let opts = RunOptions {
            env: vec![("PATH".into(), tmp.path().into())],
            ..Default::default()
        };
        let out = run(&["haryx-tool"], &opts).unwrap();
        assert_eq!(out.stdout, "from-child-path\n");
    }
}

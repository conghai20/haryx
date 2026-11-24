//! Continue a live Claude Code session on another account (`haryx ai sessions continue`). Plan:
//! `docs/session-handoff-plan.md`.
//!
//! The steps before the launch live here, each testable on its own: find the session, choose the
//! account, end the old process (a normal termination request, never a hard kill), and make the
//! conversation file reachable from the target profile. The launch itself is `run` (the session
//! manager), with `--resume <id>` in the session's folder.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::running::{RunningSession, Scan};

/// How long the old process gets to exit after the termination request. Windows waits for the
/// user to close the window (no graceful signal reaches a console app from outside), so longer.
#[cfg(unix)]
pub const STOP_TIMEOUT: Duration = Duration::from_secs(10);
#[cfg(not(unix))]
pub const STOP_TIMEOUT: Duration = Duration::from_secs(300);
const POLL: Duration = Duration::from_millis(100);

/// Find a live session by pid or session id (a unique prefix of 8+ characters works too).
pub fn find<'a>(scan: &'a Scan, key: &str) -> Result<&'a RunningSession, String> {
    let key = key.trim();
    if let Ok(pid) = key.parse::<i32>()
        && let Some(s) = scan.sessions.iter().find(|s| s.session.pid == pid)
    {
        return Ok(s);
    }
    if let Some(s) = scan.sessions.iter().find(|s| s.session.session_id == key) {
        return Ok(s);
    }
    let by_prefix: Vec<&RunningSession> = if key.len() >= 8 {
        scan.sessions
            .iter()
            .filter(|s| s.session.session_id.starts_with(key))
            .collect()
    } else {
        Vec::new()
    };
    match by_prefix.as_slice() {
        [one] => Ok(one),
        [] => Err(format!(
            "no running Claude Code session matches {key:?}; see `haryx ai sessions`"
        )),
        _ => Err(format!(
            "{key:?} matches several sessions; use the full id or the pid"
        )),
    }
}

/// Whether an account has no headroom left (its 5-hour or weekly window is at the limit).
pub fn is_exhausted(headroom: Option<f64>) -> bool {
    headroom.is_some_and(|h| h <= 0.0)
}

/// Ask `pid` to exit and wait until it has, up to `timeout`.
pub fn stop(pid: i32, timeout: Duration) -> Result<(), String> {
    stop_with(pid, timeout, &request_exit, &|p| {
        crate::process::is_pid_alive(i64::from(p))
    })
}

/// [`stop`] with the signal and the liveness check supplied (tests).
pub fn stop_with(
    pid: i32,
    timeout: Duration,
    request: &dyn Fn(i32) -> Result<(), String>,
    alive: &dyn Fn(i32) -> bool,
) -> Result<(), String> {
    if !alive(pid) {
        return Ok(());
    }
    request(pid)?;
    let started = Instant::now();
    while alive(pid) {
        if started.elapsed() >= timeout {
            return Err(format!(
                "Claude Code (pid {pid}) is still running after {}s; close it, then run this again. \
                 Nothing was changed.",
                timeout.as_secs()
            ));
        }
        std::thread::sleep(POLL);
    }
    Ok(())
}

/// SIGTERM: Claude Code saves and exits, as on Ctrl-C.
#[cfg(unix)]
fn request_exit(pid: i32) -> Result<(), String> {
    // SAFETY: kill with a plain pid and signal; no memory is involved.
    let rc = unsafe { libc::kill(pid, libc::SIGTERM) };
    if rc == 0 {
        Ok(())
    } else {
        Err(format!(
            "cannot stop Claude Code (pid {pid}): {}",
            std::io::Error::last_os_error()
        ))
    }
}

/// Windows has no graceful signal another process can send to a console app (see the plan's
/// spike): ask the user to close it, and wait.
#[cfg(not(unix))]
fn request_exit(pid: i32) -> Result<(), String> {
    eprintln!(
        "Close the Claude Code window that is out of quota (pid {pid}); the conversation continues \
         here as soon as it has closed."
    );
    Ok(())
}

/// The conversation file `<profile>/projects/<slug>/<session_id>.jsonl`, if it exists.
pub fn transcript(profile: &Path, session_id: &str) -> Option<PathBuf> {
    let name = format!("{session_id}.jsonl");
    std::fs::read_dir(profile.join("projects"))
        .ok()?
        .flatten()
        .map(|slug| slug.path().join(&name))
        .find(|p| p.is_file())
}

/// Make the conversation readable from `target_profile`: when that profile's `projects/` is another
/// folder (history not shared) and has no copy yet, copy the file into the same slug folder. Never
/// moves or overwrites. Returns the copy made, if any.
pub fn ensure_transcript(source: &Path, target_profile: &Path) -> Result<Option<PathBuf>, String> {
    let slug = source
        .parent()
        .and_then(Path::file_name)
        .ok_or_else(|| format!("{} is not in a projects folder", source.display()))?;
    let name = source.file_name().unwrap_or_default();
    let dest_dir = target_profile.join("projects").join(slug);
    let dest = dest_dir.join(name);
    let same = |a: &Path, b: &Path| match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    };
    if dest.exists() || same(source, &dest) {
        return Ok(None);
    }
    std::fs::create_dir_all(&dest_dir)
        .map_err(|e| format!("cannot create {}: {e}", dest_dir.display()))?;
    std::fs::copy(source, &dest)
        .map_err(|e| format!("cannot copy the conversation to {}: {e}", dest.display()))?;
    Ok(Some(dest))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::ClaudeSession;
    use std::cell::Cell;
    use tempfile::TempDir;

    fn session(pid: i32, id: &str, account: i64) -> RunningSession {
        RunningSession {
            account: Some(account),
            email: format!("a{account}@x.com"),
            profile: "default",
            session: ClaudeSession {
                pid,
                session_id: id.into(),
                cwd: "/w".into(),
                started_at: 0,
                kind: "interactive".into(),
                entrypoint: "cli".into(),
                status: Some("idle".into()),
            },
        }
    }

    fn scan() -> Scan {
        Scan {
            sessions: vec![
                session(100, "aaaaaaaa-1111", 1),
                session(200, "aaaaaaaa-2222", 2),
                session(300, "bbbbbbbb-3333", 1),
            ],
            unreadable: 0,
        }
    }

    #[test]
    fn finds_by_pid_id_or_unique_prefix() {
        let scan = scan();
        assert_eq!(
            find(&scan, "200").unwrap().session.session_id,
            "aaaaaaaa-2222"
        );
        assert_eq!(find(&scan, "bbbbbbbb-3333").unwrap().session.pid, 300);
        assert_eq!(find(&scan, "bbbbbbbb").unwrap().session.pid, 300);
        assert!(find(&scan, "aaaaaaaa").unwrap_err().contains("several"));
        assert!(find(&scan, "999").unwrap_err().contains("no running"));
        assert!(
            find(&scan, "bbb").is_err(),
            "short prefixes are not guessed"
        );
    }

    #[test]
    fn exhausted_means_no_headroom() {
        assert!(is_exhausted(Some(0.0)));
        assert!(is_exhausted(Some(-3.0)));
        assert!(!is_exhausted(Some(12.0)));
        assert!(!is_exhausted(None), "unknown is not exhausted");
    }

    #[test]
    fn stop_requests_once_and_waits_for_exit() {
        let requests = Cell::new(0);
        let checks = Cell::new(0);
        let result = stop_with(
            42,
            Duration::from_secs(5),
            &|_| {
                requests.set(requests.get() + 1);
                Ok(())
            },
            &|_| {
                checks.set(checks.get() + 1);
                checks.get() < 4
            },
        );
        assert_eq!(result, Ok(()));
        assert_eq!(requests.get(), 1);
    }

    #[test]
    fn stop_gives_up_when_the_process_stays() {
        let err = stop_with(42, Duration::from_millis(250), &|_| Ok(()), &|_| true).unwrap_err();
        assert!(err.contains("still running"), "{err}");
    }

    #[test]
    fn a_gone_process_needs_no_signal() {
        let result = stop_with(
            42,
            Duration::from_secs(1),
            &|_| panic!("no signal"),
            &|_| false,
        );
        assert_eq!(result, Ok(()));
    }

    #[cfg(unix)]
    #[test]
    fn stops_a_real_child() {
        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .unwrap();
        let pid = i32::try_from(child.id()).unwrap();
        let reaped = std::thread::spawn(move || child.wait());
        stop(pid, Duration::from_secs(5)).unwrap();
        assert!(reaped.join().unwrap().is_ok());
    }

    #[test]
    fn finds_and_copies_a_transcript_only_when_needed() {
        let tmp = TempDir::new().unwrap();
        let source_profile = tmp.path().join("default");
        let slug = source_profile.join("projects").join("-w");
        std::fs::create_dir_all(&slug).unwrap();
        std::fs::write(slug.join("abc.jsonl"), "{}\n").unwrap();
        let found = transcript(&source_profile, "abc").unwrap();
        assert_eq!(found, slug.join("abc.jsonl"));
        assert_eq!(transcript(&source_profile, "nope"), None);

        // Same profile: nothing to copy.
        assert_eq!(ensure_transcript(&found, &source_profile).unwrap(), None);
        // A profile with its own history gets a copy; the source stays.
        let other = tmp.path().join("other");
        let copy = ensure_transcript(&found, &other).unwrap().unwrap();
        assert_eq!(copy, other.join("projects").join("-w").join("abc.jsonl"));
        assert!(found.exists());
        // A second call does not overwrite.
        std::fs::write(&copy, "edited").unwrap();
        assert_eq!(ensure_transcript(&found, &other).unwrap(), None);
        assert_eq!(std::fs::read_to_string(&copy).unwrap(), "edited");
    }

    #[cfg(unix)]
    #[test]
    fn a_shared_projects_folder_counts_as_the_same() {
        let tmp = TempDir::new().unwrap();
        let shared = tmp.path().join("default");
        std::fs::create_dir_all(shared.join("projects").join("-w")).unwrap();
        std::fs::write(shared.join("projects/-w/abc.jsonl"), "{}").unwrap();
        let profile = tmp.path().join("profile");
        std::fs::create_dir_all(&profile).unwrap();
        std::os::unix::fs::symlink(shared.join("projects"), profile.join("projects")).unwrap();
        let src = transcript(&shared, "abc").unwrap();
        assert_eq!(ensure_transcript(&src, &profile).unwrap(), None);
    }
}

//! Port of `claude_swap/claude_locks.py`: cooperate with Claude Code's own advisory locks while mutating its
//! files.
//!
//! Claude Code guards its OAuth token refresh with the npm `proper-lockfile` package, and its
//! `~/.claude.json` writes with the same mechanism on the config file. The protocol (verified upstream
//! against the 2.1.218 bundle):
//!
//! - The lock artifact is a **directory**; `mkdir` atomicity is the mutex.
//! - The refresh path takes **two** locks, in order: the primary `<config-home>/.oauth_refresh.lock`, then
//!   the legacy `<config-home>.lock` (`~/.claude.lock`) kept for compatibility with external tools. Both run
//!   `stale: 60000, update: 5000`: a credential lock is stale only past **60s**, and live holders touch every
//!   5s. On a contended legacy lock Claude Code releases the primary and retries.
//! - The config lock (`~/.claude.json.lock`) keeps the older defaults: stale after 10s, touched every 5s.
//! - Claude Code retries a held credentials lock 5 times with 1-2s jittered sleeps before giving up, so
//!   briefly holding it is fully cooperative.
//!
//! Holding these locks while swapping credentials closes the one real race with a running Claude Code: its
//! refresh reads credentials, refreshes over the network, and saves, all under both credential locks, so a
//! swap landing inside that window would be overwritten by the refreshed old-account token. Under the lock,
//! Claude Code's own double-checked re-read sees the swapped (non-expired) credential and aborts the refresh.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime};

use crate::error::{Error, Result};
use crate::paths::{PathEnv, claude_config_home_in, global_config_path_in};

/// Claude Code's credential-refresh locks run `stale: 60000`: a lock younger than 60s belongs to a live
/// holder and must never be stolen, since the holder's toucher may stall well past 10s (suspend, blocked
/// event loop) while it still legitimately owns the lock.
pub const CREDENTIALS_STALENESS: Duration = Duration::from_secs(60);
/// The config lock keeps the older proper-lockfile defaults: stale after 10s, touched every 5s.
pub const CONFIG_STALENESS: Duration = Duration::from_secs(10);
/// We touch a little faster than Claude Code's 5s for margin.
pub const TOUCH_INTERVAL: Duration = Duration::from_secs(3);
/// Per-lock wait budget. Claude Code holds the credentials lock for one token-endpoint round trip and its
/// config lock for a local read-modify-write; 9s outlasts both without stalling the CLI forever. Note
/// [`claude_credentials_lock`] acquires two locks sequentially, so its worst case is about twice this.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(9);
/// A short back-off after an arm declines, so the retry loop cannot spin hot.
const DECLINE_BACKOFF: Duration = Duration::from_millis(50);
/// Jittered wait on a fresh held lock: 250ms plus up to 250ms more.
const JITTER_BASE: Duration = Duration::from_millis(250);
/// Two waiters that both saw the same dead holder could each run the stat + rmdir takeover, the second rmdir
/// hitting the first waiter's fresh lock (upstream #420). Takeovers are serialized through this sibling
/// directory and re-check staleness inside it. Claude Code never looks at it.
pub const TAKEOVER_SUFFIX: &str = ".takeover";
/// The takeover directory is held for one stat + rmdir; older than this, its holder died mid-takeover.
pub const TAKEOVER_STALENESS: Duration = Duration::from_secs(5);
/// How long a release waits for the toucher thread before leaving it behind.
const TOUCHER_JOIN_WAIT: Duration = Duration::from_secs(1);

/// Legacy credential lock (`~/.claude.lock`): Claude Code still takes it for compatibility; external
/// exclusion today rests on this one.
pub fn credentials_lock_dir(env: &PathEnv) -> PathBuf {
    sibling_with_suffix(&claude_config_home_in(env), ".lock")
}

/// Claude Code's primary OAuth refresh lock (`<config-home>/.oauth_refresh.lock`, 2.1.218+).
pub fn oauth_refresh_lock_dir(env: &PathEnv) -> PathBuf {
    claude_config_home_in(env).join(".oauth_refresh.lock")
}

/// Lock directory guarding the global config file (`~/.claude.json.lock`).
pub fn config_lock_dir(env: &PathEnv) -> PathBuf {
    sibling_with_suffix(&global_config_path_in(env), ".lock")
}

fn sibling_with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_default();
    name.push(suffix);
    path.with_file_name(name)
}

/// Seconds since `time`, negative when it lies in the future.
fn age(time: SystemTime) -> f64 {
    match SystemTime::now().duration_since(time) {
        Ok(d) => d.as_secs_f64(),
        Err(e) => -e.duration().as_secs_f64(),
    }
}

fn mtime(path: &Path, follow: bool) -> std::io::Result<SystemTime> {
    let meta = if follow {
        std::fs::metadata(path)?
    } else {
        std::fs::symlink_metadata(path)?
    };
    meta.modified()
}

/// Sleep at most what is LEFT of the budget, never a full jitter draw: the deadline is checked at the top
/// of the loop, so a longer sleep would make the timeout land late. Never negative.
fn nap(want: Duration, start: Instant, timeout: Duration) {
    let left = timeout.saturating_sub(start.elapsed());
    if !left.is_zero() {
        std::thread::sleep(want.min(left));
    }
}

/// Remove `lock_dir` if it is still stale, under the takeover directory.
///
/// Returns true if this call removed it. False means another waiter is mid-takeover, the lock was already
/// gone or retaken, or it cannot be removed.
fn remove_if_still_stale(lock_dir: &Path, staleness: Duration) -> bool {
    let takeover = sibling_with_suffix(lock_dir, TAKEOVER_SUFFIX);
    match std::fs::create_dir(&takeover) {
        Ok(()) => {}
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
            sweep_dead_takeover(&takeover);
            return false;
        }
        Err(_) => return false,
    }
    // Re-check inside: the waiter before us may already have removed the dead lock and taken a fresh one
    // at the same path.
    let removed = match mtime(lock_dir, true) {
        Ok(held) if age(held) > staleness.as_secs_f64() => std::fs::remove_dir(lock_dir).is_ok(),
        _ => false,
    };
    let _ = std::fs::remove_dir(&takeover);
    removed
}

/// A takeover directory older than [`TAKEOVER_STALENESS`] belongs to a waiter that died mid-takeover.
fn sweep_dead_takeover(takeover: &Path) {
    let Ok(held) = mtime(takeover, false) else {
        return;
    };
    if age(held) <= TAKEOVER_STALENESS.as_secs_f64() {
        return;
    }
    if std::fs::remove_dir(takeover).is_err() {
        // A file or symlink at the guard path: not ours to keep.
        let _ = std::fs::remove_file(takeover);
    }
}

/// A held proper-lockfile-compatible directory lock. Dropping it stops the toucher and removes the
/// directory.
#[derive(Debug)]
pub struct ProperLock {
    lock_dir: PathBuf,
    stop: Option<Sender<()>>,
    toucher: Option<JoinHandle<()>>,
}

impl ProperLock {
    pub fn path(&self) -> &Path {
        &self.lock_dir
    }
}

/// Acquire a proper-lockfile-compatible directory lock.
///
/// Blocks up to `timeout` (default [`DEFAULT_TIMEOUT`]), taking over locks whose mtime is older than
/// `staleness`, touches the directory mtime every [`TOUCH_INTERVAL`] while held so other holders don't deem
/// it stale, and removes it when the returned guard drops.
///
/// Fails with [`Error::ClaudeCodeLockTimeout`] when the lock stayed held past `timeout`.
pub fn proper_lockfile(
    lock_dir: &Path,
    timeout: Option<Duration>,
    staleness: Duration,
) -> Result<ProperLock> {
    let timeout = timeout.unwrap_or(DEFAULT_TIMEOUT);
    if let Some(parent) = lock_dir.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let start = Instant::now();
    loop {
        match std::fs::create_dir(lock_dir) {
            Ok(()) => break,
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(err) => return Err(err.into()),
        }
        if start.elapsed() > timeout {
            let name = lock_dir
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            return Err(Error::ClaudeCodeLockTimeout(format!(
                "Could not acquire {name} \u{2014} Claude Code appears to be refreshing credentials. \
                 Retry in a few seconds."
            )));
        }
        let held = match mtime(lock_dir, true) {
            Ok(held) => held,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                // BACK OFF HERE TOO. A dangling symlink at the lock path answers "exists" to mkdir and
                // "not found" to stat, so this arm can repeat for the whole budget.
                nap(DECLINE_BACKOFF, start, timeout);
                continue;
            }
            Err(err) => return Err(err.into()),
        };
        if age(held) > staleness.as_secs_f64() {
            // Dead holder per the protocol: remove and retake. Declining must not spin hot, and must not
            // sleep past the deadline.
            if !remove_if_still_stale(lock_dir, staleness) {
                nap(DECLINE_BACKOFF, start, timeout);
            }
            continue;
        }
        nap(
            JITTER_BASE.mul_f64(1.0 + rand::random::<f64>()),
            start,
            timeout,
        );
    }

    let (stop, stopped) = mpsc::channel();
    let toucher_dir = lock_dir.to_path_buf();
    let toucher = std::thread::Builder::new()
        .name("haryx-lock-toucher".into())
        .spawn(move || touch_until_stopped(&toucher_dir, staleness, &stopped));
    let toucher = match toucher {
        Ok(handle) => Some(handle),
        Err(err) => {
            let _ = std::fs::remove_dir(lock_dir);
            return Err(err.into());
        }
    };
    Ok(ProperLock {
        lock_dir: lock_dir.to_path_buf(),
        stop: Some(stop),
        toucher,
    })
}

/// The toucher loop: refresh the lock's mtime every [`TOUCH_INTERVAL`] until told to stop or the lock is
/// gone.
fn touch_until_stopped(lock_dir: &Path, staleness: Duration, stopped: &mpsc::Receiver<()>) {
    let mut warned = false;
    let mut last_ok = Instant::now();
    loop {
        match stopped.recv_timeout(TOUCH_INTERVAL) {
            Err(RecvTimeoutError::Timeout) => {}
            Ok(()) | Err(RecvTimeoutError::Disconnected) => return,
        }
        match touch(lock_dir) {
            Ok(()) => {
                // THE LATCH IS PER FREEZE, NOT PER HOLD: a refresh that lands ends the episode the warning
                // describes, so the next freeze is a new fact.
                last_ok = Instant::now();
                warned = false;
            }
            // Gone; nothing left to keep alive.
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return,
            Err(err) => {
                // Transient, so stay armed. Warn only once the sentence is true: a freeze that outlives
                // `staleness`.
                if !warned && last_ok.elapsed() > staleness {
                    warned = true;
                    log::warn!(
                        "Could not refresh {} ({err}); its mtime stops advancing, so a waiter may take it over \
                         as stale",
                        lock_dir.display()
                    );
                }
            }
        }
    }
}

/// Set the mtime (and atime) of `path` to now, like Python's `os.utime(path)`.
#[cfg(unix)]
fn touch(path: &Path) -> std::io::Result<()> {
    use std::os::unix::ffi::OsStrExt;
    let c_path = std::ffi::CString::new(path.as_os_str().as_bytes()).map_err(|_| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "path contains a NUL byte")
    })?;
    // SAFETY: `c_path` is a valid NUL-terminated string that outlives the call; a null `times` pointer asks
    // for the current time, as documented for utimes(2).
    let rc = unsafe { libc::utimes(c_path.as_ptr(), std::ptr::null()) };
    if rc == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

// TODO(windows): touch the lock directory's mtime (upstream uses os.utime).
#[cfg(not(unix))]
fn touch(path: &Path) -> std::io::Result<()> {
    std::fs::metadata(path).map(|_| ())
}

impl Drop for ProperLock {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(handle) = self.toucher.take() {
            // The toucher wakes on the stop message at once; a utime stuck in the kernel is waited out for
            // at most TOUCHER_JOIN_WAIT, then left behind (it exits once the call returns).
            let deadline = Instant::now() + TOUCHER_JOIN_WAIT;
            while !handle.is_finished() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(5));
            }
            if handle.is_finished() {
                let _ = handle.join();
            }
        }
        match std::fs::remove_dir(&self.lock_dir) {
            Ok(()) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                log::warn!(
                    "Lock {} vanished while held (taken over as stale?)",
                    self.lock_dir.display()
                );
            }
            Err(err) => log::warn!("Failed to release lock {}: {err}", self.lock_dir.display()),
        }
    }
}

/// Both of Claude Code's credential-refresh locks, held. Dropping releases the legacy lock first, then the
/// primary (reverse acquisition order).
#[derive(Debug)]
pub struct CredentialsLock {
    // Field order is drop order: legacy first, then primary.
    _legacy: ProperLock,
    _primary: ProperLock,
}

/// Hold Claude Code's credential-refresh locks, in Claude Code's own order.
///
/// 2.1.218 takes `<config-home>/.oauth_refresh.lock` first, then the legacy `~/.claude.lock`; on legacy
/// contention it releases the primary before retrying. Mirroring both the pair and the order means a waiting
/// haryx and a waiting Claude Code can never deadlock, and exclusion holds even after Claude Code drops the
/// legacy lock. Both use the 60s staleness: never steal a lock a live Claude Code may still hold.
pub fn claude_credentials_lock(
    env: &PathEnv,
    timeout: Option<Duration>,
) -> Result<CredentialsLock> {
    let primary = proper_lockfile(&oauth_refresh_lock_dir(env), timeout, CREDENTIALS_STALENESS)?;
    // On failure here `primary` drops, releasing the primary lock.
    let legacy = proper_lockfile(&credentials_lock_dir(env), timeout, CREDENTIALS_STALENESS)?;
    Ok(CredentialsLock {
        _legacy: legacy,
        _primary: primary,
    })
}

/// Hold Claude Code's global-config write lock (`~/.claude.json.lock`).
pub fn claude_config_lock(env: &PathEnv, timeout: Option<Duration>) -> Result<ProperLock> {
    proper_lockfile(&config_lock_dir(env), timeout, CONFIG_STALENESS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    const SHORT: Option<Duration> = Some(Duration::from_millis(300));

    #[cfg(unix)]
    fn set_age(path: &Path, seconds_ago: u64) {
        let when = SystemTime::now() - Duration::from_secs(seconds_ago);
        let file = std::fs::File::open(path).unwrap();
        file.set_modified(when).unwrap();
    }

    #[test]
    fn acquire_creates_and_release_removes() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("x.lock");
        let lock = proper_lockfile(&dir, None, CONFIG_STALENESS).unwrap();
        assert!(dir.is_dir());
        assert_eq!(lock.path(), dir);
        drop(lock);
        assert!(!dir.exists());
        drop(proper_lockfile(&dir, None, CONFIG_STALENESS).unwrap());
    }

    #[test]
    fn creates_missing_parent() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("a").join("b").join("x.lock");
        drop(proper_lockfile(&dir, None, CONFIG_STALENESS).unwrap());
        assert!(tmp.path().join("a").join("b").is_dir());
    }

    #[test]
    fn contention_times_out_with_a_claude_code_lock_error() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("x.lock");
        std::fs::create_dir(&dir).unwrap();
        let started = Instant::now();
        let err = proper_lockfile(&dir, SHORT, CONFIG_STALENESS).unwrap_err();
        assert!(matches!(err, Error::ClaudeCodeLockTimeout(_)));
        assert!(err.is_lock());
        assert!(err.to_string().contains("Could not acquire x.lock"));
        assert!(
            started.elapsed() < Duration::from_millis(800),
            "naps never outlive the budget"
        );
        assert!(dir.is_dir(), "a fresh lock is never stolen");
    }

    #[cfg(unix)]
    #[test]
    fn stale_lock_is_taken_over_and_leaves_no_takeover_dir() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("x.lock");
        std::fs::create_dir(&dir).unwrap();
        set_age(&dir, 30);
        let lock = proper_lockfile(&dir, SHORT, CONFIG_STALENESS).unwrap();
        assert!(!tmp.path().join("x.lock.takeover").exists());
        drop(lock);
    }

    #[cfg(unix)]
    #[test]
    fn credentials_staleness_is_60s_not_10s() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("x.lock");
        std::fs::create_dir(&dir).unwrap();
        set_age(&dir, 30);
        assert!(proper_lockfile(&dir, SHORT, CREDENTIALS_STALENESS).is_err());
        set_age(&dir, 61);
        assert!(proper_lockfile(&dir, SHORT, CREDENTIALS_STALENESS).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn a_live_takeover_dir_blocks_takeover_and_a_dead_one_is_swept() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("x.lock");
        let guard = tmp.path().join("x.lock.takeover");
        std::fs::create_dir(&dir).unwrap();
        set_age(&dir, 30);
        std::fs::create_dir(&guard).unwrap();
        assert!(
            proper_lockfile(&dir, SHORT, CONFIG_STALENESS).is_err(),
            "a peer is mid-takeover"
        );
        set_age(&guard, 10);
        drop(proper_lockfile(&dir, Some(Duration::from_secs(2)), CONFIG_STALENESS).unwrap());
        assert!(!guard.exists());
    }

    #[cfg(unix)]
    #[test]
    fn a_non_directory_at_the_guard_path_is_swept_only_once_dead() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("x.lock");
        let guard = tmp.path().join("x.lock.takeover");
        std::fs::create_dir(&dir).unwrap();
        set_age(&dir, 30);
        std::fs::write(&guard, "").unwrap();
        assert!(proper_lockfile(&dir, SHORT, CONFIG_STALENESS).is_err());
        assert!(guard.exists());
        set_age(&guard, 10);
        drop(proper_lockfile(&dir, Some(Duration::from_secs(2)), CONFIG_STALENESS).unwrap());
        assert!(!guard.exists());
    }

    #[cfg(unix)]
    #[test]
    fn two_waiters_cannot_both_take_over_one_stale_lock() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("x.lock");
        std::fs::create_dir(&dir).unwrap();
        set_age(&dir, 30);
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let handles: Vec<_> = (0..2)
            .map(|_| {
                let dir = dir.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    let got =
                        proper_lockfile(&dir, Some(Duration::from_millis(500)), CONFIG_STALENESS);
                    // Hold past the other waiter's budget so a double takeover would show.
                    std::thread::sleep(Duration::from_millis(700));
                    got.is_ok()
                })
            })
            .collect();
        let wins = handles
            .into_iter()
            .map(|h| h.join().unwrap())
            .filter(|ok| *ok)
            .count();
        assert_eq!(wins, 1);
    }

    #[test]
    fn release_tolerates_a_stolen_lock() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("x.lock");
        let lock = proper_lockfile(&dir, None, CONFIG_STALENESS).unwrap();
        std::fs::remove_dir(&dir).unwrap();
        drop(lock);
        assert!(!dir.exists());
    }

    #[cfg(unix)]
    #[test]
    fn touch_refreshes_mtime() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("d");
        std::fs::create_dir(&dir).unwrap();
        set_age(&dir, 100);
        touch(&dir).unwrap();
        assert!(age(mtime(&dir, true).unwrap()) < 5.0);
        assert_eq!(
            touch(&tmp.path().join("missing")).unwrap_err().kind(),
            std::io::ErrorKind::NotFound
        );
    }

    fn env_at(home: &Path) -> PathEnv {
        PathEnv::with_home(home)
    }

    #[test]
    fn default_lock_paths() {
        let tmp = TempDir::new().unwrap();
        let env = env_at(tmp.path());
        assert_eq!(credentials_lock_dir(&env), tmp.path().join(".claude.lock"));
        assert_eq!(
            oauth_refresh_lock_dir(&env),
            tmp.path().join(".claude/.oauth_refresh.lock")
        );
        assert_eq!(config_lock_dir(&env), tmp.path().join(".claude.json.lock"));
    }

    #[test]
    fn claude_config_dir_is_honored() {
        let tmp = TempDir::new().unwrap();
        let ccd = tmp.path().join("profile");
        let env = PathEnv {
            claude_config_dir: Some(ccd.clone().into()),
            ..env_at(tmp.path())
        };
        assert_eq!(credentials_lock_dir(&env), tmp.path().join("profile.lock"));
        assert_eq!(
            oauth_refresh_lock_dir(&env),
            ccd.join(".oauth_refresh.lock")
        );
        assert_eq!(config_lock_dir(&env), ccd.join(".claude.json.lock"));
    }

    #[test]
    fn credentials_lock_takes_both_and_releases_both() {
        let tmp = TempDir::new().unwrap();
        let env = env_at(tmp.path());
        let lock = claude_credentials_lock(&env, None).unwrap();
        assert!(oauth_refresh_lock_dir(&env).is_dir());
        assert!(credentials_lock_dir(&env).is_dir());
        drop(lock);
        assert!(!oauth_refresh_lock_dir(&env).exists());
        assert!(!credentials_lock_dir(&env).exists());
    }

    #[test]
    fn primary_contention_never_touches_legacy() {
        let tmp = TempDir::new().unwrap();
        let env = env_at(tmp.path());
        std::fs::create_dir_all(oauth_refresh_lock_dir(&env)).unwrap();
        assert!(claude_credentials_lock(&env, SHORT).is_err());
        assert!(!credentials_lock_dir(&env).exists());
    }

    #[test]
    fn legacy_contention_releases_primary() {
        let tmp = TempDir::new().unwrap();
        let env = env_at(tmp.path());
        std::fs::create_dir_all(credentials_lock_dir(&env)).unwrap();
        let err = claude_credentials_lock(&env, SHORT).unwrap_err();
        assert!(matches!(err, Error::ClaudeCodeLockTimeout(_)));
        assert!(
            !oauth_refresh_lock_dir(&env).exists(),
            "primary released on legacy contention"
        );
        assert!(
            credentials_lock_dir(&env).is_dir(),
            "the other holder's lock is untouched"
        );
    }

    #[cfg(unix)]
    #[test]
    fn config_lock_staleness_stays_10s() {
        let tmp = TempDir::new().unwrap();
        let env = env_at(tmp.path());
        let dir = config_lock_dir(&env);
        std::fs::create_dir_all(&dir).unwrap();
        set_age(&dir, 15);
        drop(claude_config_lock(&env, SHORT).unwrap());
    }

    #[test]
    fn nap_sleeps_only_what_is_left() {
        let start = Instant::now();
        nap(Duration::from_secs(5), start, Duration::from_millis(50));
        assert!(start.elapsed() < Duration::from_millis(500));
        let expired = Instant::now();
        nap(Duration::from_secs(5), expired, Duration::ZERO);
        assert!(expired.elapsed() < Duration::from_millis(50));
    }
}

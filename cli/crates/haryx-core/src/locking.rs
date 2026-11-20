//! Port of `claude_swap/locking.py`: haryx's own cross-process file lock.
//!
//! An exclusive advisory lock (`flock` on Unix, via `fs4`) on a file under the store root. Locks belong to
//! the open file, so two [`FileLock`]s on one path exclude each other even inside one process.

use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use fs4::fs_std::FileExt;

use crate::error::{Error, Result};

/// Default wait of [`FileLock::new`].
pub const DEFAULT_LOCK_TIMEOUT: Duration = Duration::from_secs(10);
/// Longest single sleep between attempts; clamped to what is left of the budget.
const RETRY_SLEEP: Duration = Duration::from_millis(100);
/// Message of [`FileLock::lock`] when the wait runs out.
pub const LOCK_FAILED_MESSAGE: &str = "Failed to acquire lock - another instance may be running";

/// Cross-process exclusive file lock. Released by [`FileLock::release`] or on drop.
#[derive(Debug)]
pub struct FileLock {
    lock_path: PathBuf,
    timeout: Duration,
    file: Option<File>,
}

impl FileLock {
    /// A lock on `lock_path`, not yet acquired.
    pub fn new(lock_path: impl Into<PathBuf>, timeout: Duration) -> Self {
        FileLock {
            lock_path: lock_path.into(),
            timeout,
            file: None,
        }
    }

    /// Acquire and hold a lock on `lock_path`, or fail with [`Error::Lock`] after `timeout` (the upstream
    /// context-manager form).
    pub fn lock(lock_path: impl Into<PathBuf>, timeout: Duration) -> Result<Self> {
        let mut lock = FileLock::new(lock_path, timeout);
        if lock.acquire(None)? {
            Ok(lock)
        } else {
            Err(Error::Lock(LOCK_FAILED_MESSAGE.into()))
        }
    }

    pub fn path(&self) -> &Path {
        &self.lock_path
    }

    pub fn is_locked(&self) -> bool {
        self.file.is_some()
    }

    /// Acquire the exclusive lock, waiting up to `timeout` (default: the one given at construction).
    ///
    /// Creates missing parent directories. Returns `Ok(false)` when the wait runs out; I/O errors creating
    /// or opening the lock file are returned. Acquiring a lock already held by this value is a no-op.
    pub fn acquire(&mut self, timeout: Option<Duration>) -> Result<bool> {
        if self.file.is_some() {
            return Ok(true);
        }
        let timeout = timeout.unwrap_or(self.timeout);
        if let Some(parent) = self.lock_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&self.lock_path)?;

        let start = Instant::now();
        loop {
            // Any error (contention or otherwise) is retried until the budget runs out, as upstream does.
            if matches!(FileExt::try_lock_exclusive(&file), Ok(true)) {
                self.file = Some(file);
                return Ok(true);
            }
            let elapsed = start.elapsed();
            if elapsed > timeout {
                return Ok(false);
            }
            // Clamped to the remaining budget, so `timeout` bounds the call.
            std::thread::sleep(RETRY_SLEEP.min(timeout.saturating_sub(elapsed)));
        }
    }

    /// Release the lock. Safe to call when not held, and more than once.
    pub fn release(&mut self) {
        if let Some(file) = self.file.take() {
            if let Err(err) = FileExt::unlock(&file) {
                log::debug!("unlock of {} failed: {err}", self.lock_path.display());
            }
        }
    }
}

impl Drop for FileLock {
    fn drop(&mut self) {
        self.release();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn acquire_and_release() {
        let tmp = TempDir::new().unwrap();
        let mut lock = FileLock::new(tmp.path().join("x.lock"), DEFAULT_LOCK_TIMEOUT);
        assert!(lock.acquire(None).unwrap());
        assert!(lock.is_locked());
        lock.release();
        assert!(!lock.is_locked());
    }

    #[test]
    fn lock_creates_parent_dirs() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("a").join("b").join("x.lock");
        let lock = FileLock::lock(&path, DEFAULT_LOCK_TIMEOUT).unwrap();
        assert!(path.exists());
        assert_eq!(lock.path(), path);
    }

    #[test]
    fn contention_times_out_and_lock_raises() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("x.lock");
        let _held = FileLock::lock(&path, DEFAULT_LOCK_TIMEOUT).unwrap();

        let mut other = FileLock::new(&path, Duration::from_millis(200));
        let started = Instant::now();
        assert!(!other.acquire(None).unwrap());
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "the budget bounds the call"
        );

        let err = FileLock::lock(&path, Duration::from_millis(50)).unwrap_err();
        assert!(err.is_lock());
        assert_eq!(err.to_string(), LOCK_FAILED_MESSAGE);
    }

    #[test]
    fn acquired_after_release_and_after_drop() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("x.lock");
        let mut first = FileLock::lock(&path, DEFAULT_LOCK_TIMEOUT).unwrap();
        first.release();
        let second = FileLock::lock(&path, Duration::from_millis(100)).unwrap();
        drop(second);
        assert!(FileLock::lock(&path, Duration::from_millis(100)).is_ok());
    }

    #[test]
    fn double_release_is_safe() {
        let tmp = TempDir::new().unwrap();
        let mut lock = FileLock::lock(tmp.path().join("x.lock"), DEFAULT_LOCK_TIMEOUT).unwrap();
        lock.release();
        lock.release();
        let mut never = FileLock::new(tmp.path().join("y.lock"), DEFAULT_LOCK_TIMEOUT);
        never.release();
    }

    #[test]
    fn waiter_gets_the_lock_once_the_holder_lets_go() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("x.lock");
        let held = FileLock::lock(&path, DEFAULT_LOCK_TIMEOUT).unwrap();
        let waiter_path = path.clone();
        let waiter = std::thread::spawn(move || {
            FileLock::lock(&waiter_path, Duration::from_secs(5)).is_ok()
        });
        std::thread::sleep(Duration::from_millis(150));
        drop(held);
        assert!(waiter.join().unwrap());
    }

    #[test]
    fn a_budget_smaller_than_the_flat_sleep_is_not_overshot() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("x.lock");
        let _held = FileLock::lock(&path, DEFAULT_LOCK_TIMEOUT).unwrap();
        let started = Instant::now();
        assert!(
            !FileLock::new(&path, Duration::from_millis(10))
                .acquire(None)
                .unwrap()
        );
        assert!(started.elapsed() < RETRY_SLEEP + Duration::from_millis(50));
    }
}

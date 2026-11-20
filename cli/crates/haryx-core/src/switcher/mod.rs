//! Port of `claude_swap/switcher.py`: [`Switcher`], the Rust `ClaudeAccountSwitcher`.
//!
//! Upstream is one 8.5k-line class. Here the struct and its construction live in this file and every
//! method group adds an `impl Switcher` block in its own file; each file names the upstream methods it ports.
//!
//! Shared rules (upstream `docs/architecture.md`): lock order haryx lock → Claude credential lock →
//! Claude config lock; no network under a lock except the documented token refresh; a live credential
//! enters a slot only when it is known to belong there; paths only from [`crate::paths`].

mod accounts;
mod add;
mod grant;
mod listing;
mod perform;
mod policy;
mod relocate;
mod sequence;
mod session_profile;
mod storage;
mod switch;
mod usage;
mod usage_active;

#[cfg(test)]
mod test_support;
#[cfg(test)]
mod tests_grant;
#[cfg(test)]
mod tests_relocate;
#[cfg(test)]
mod tests_sequence;
#[cfg(test)]
mod tests_usage;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Map, Value};

use crate::credentials::{
    CredentialStore, KeychainBackend, StoreHost, Tunables, haryx_backup_namespace,
};
use crate::error::{Error, Result};
use crate::fsutil::{self, WriteOptions};
use crate::keychain::Security;
use crate::locking::{DEFAULT_LOCK_TIMEOUT, FileLock};
use crate::models::pyfmt::json_dumps_pretty;
use crate::oauth::Api;
use crate::paths::{self, PathEnv};
use crate::printer;
use crate::session::{self, ProfileKeychain};
use crate::usage_store::UsageStore;

pub use perform::{Provenance, SwitchOp};
pub use sequence::LineageKey;
pub use usage::AccountInfo;

/// A parsed `sequence.json` (or any JSON object file).
pub type JsonObject = Map<String, Value>;

/// The multi-account switcher: slots, aliases, policies, swap/move and (in later parts) add, remove,
/// usage and switching. Port of upstream `ClaudeAccountSwitcher`.
///
/// Holds what upstream's `__init__` sets up that this port needs: the path environment, the store root
/// and its well-known files, and the [`CredentialStore`]. Upstream's logging setup is the binary's
/// (`env_logger`); its legacy-dir and data migrations are not ported (haryx starts with the current
/// layout). Caches owned by unported parts (usage store, poll-policy inputs, probe verdicts, provenance
/// warnings, the per-thread active verdict) are added by those parts.
pub struct Switcher {
    /// Where Claude Code's files are and where the store is.
    pub(crate) env: PathEnv,
    /// The store root (upstream `backup_dir`).
    pub(crate) backup_dir: PathBuf,
    pub(crate) sequence_file: PathBuf,
    pub(crate) configs_dir: PathBuf,
    pub(crate) credentials_dir: PathBuf,
    /// haryx's own account lock (upstream `lock_file`, `.lock` in the store root).
    pub(crate) lock_file: PathBuf,
    /// The active and per-slot backup credential stores.
    pub(crate) store: CredentialStore,
    /// The Keychain backend the store uses, shared with the session-profile helpers.
    pub(crate) keychain: Arc<dyn KeychainBackend>,
    /// Keychain account name of Claude Code's items.
    pub(crate) keychain_account: String,
    /// Pause between strict session Keychain read attempts.
    pub(crate) session_retry_delay: Duration,
    /// The OAuth/usage endpoints (`HARYX_API_BASE` redirects them in tests).
    pub(crate) api: Api,
    /// Per-account usage table (`cache/usage.json`).
    pub(crate) usage_store: UsageStore,
    /// Mutable per-process state of the collector (upstream's instance caches).
    pub(crate) state: Mutex<usage::CollectState>,
    debug: bool,
    #[cfg(test)]
    pub(crate) faults: test_support::Faults,
}

impl std::fmt::Debug for Switcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Switcher")
            .field("backup_dir", &self.backup_dir)
            .field("debug", &self.debug)
            .finish_non_exhaustive()
    }
}

impl Switcher {
    /// A switcher over an explicit store host and Keychain backend. The store root is the directory of
    /// `host.sequence_file`.
    pub fn new(
        host: StoreHost,
        keychain: Arc<dyn KeychainBackend>,
        tunables: Tunables,
        debug: bool,
    ) -> Self {
        let backup_dir = host
            .sequence_file
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default();
        let env = host.env.clone();
        let keychain_account = host.keychain_account.clone();
        let store = CredentialStore::with_keychain(host.clone(), keychain.clone(), tunables);
        let usage_store = UsageStore::new(&backup_dir.join("cache"));
        Switcher {
            env,
            sequence_file: host.sequence_file.clone(),
            configs_dir: backup_dir.join("configs"),
            credentials_dir: host.credentials_dir.clone(),
            lock_file: backup_dir.join(".lock"),
            backup_dir,
            store,
            keychain,
            keychain_account,
            session_retry_delay: session::STRICT_KEYCHAIN_RETRY_DELAY,
            api: Api::from_env(),
            usage_store,
            state: Mutex::new(usage::CollectState::default()),
            debug,
            #[cfg(test)]
            faults: test_support::Faults::default(),
        }
    }

    /// haryx's own store for a path environment, driving the system `security` binary
    /// (`HARYX_SECURITY_BIN` overrides it).
    pub fn from_env(env: PathEnv, debug: bool) -> Result<Self> {
        let host = StoreHost::new(&haryx_backup_namespace(&env)?, env);
        Ok(Self::new(
            host,
            Arc::new(Security::system()),
            Tunables::default(),
            debug,
        ))
    }

    /// haryx's own store for this process's environment.
    pub fn from_process(debug: bool) -> Result<Self> {
        Self::from_env(PathEnv::from_process()?, debug)
    }

    /// Point every endpoint at another base URL (tests, a local stub).
    pub fn with_api(mut self, api: Api) -> Self {
        self.api = api;
        self
    }

    pub fn debug(&self) -> bool {
        self.debug
    }

    /// The store root.
    pub fn backup_dir(&self) -> &Path {
        &self.backup_dir
    }

    pub fn sequence_file(&self) -> &Path {
        &self.sequence_file
    }

    pub fn credential_store(&self) -> &CredentialStore {
        &self.store
    }

    /// The Keychain view the session-profile helpers use.
    pub(crate) fn profile_keychain(&self) -> ProfileKeychain<'_> {
        ProfileKeychain {
            backend: self.keychain.as_ref(),
            platform: self.env.os,
            account: &self.keychain_account,
            retry_delay: self.session_retry_delay,
        }
    }

    /// Take haryx's account lock with upstream's default timeout (non-reentrant).
    pub(crate) fn lock(&self) -> Result<FileLock> {
        FileLock::lock(&self.lock_file, DEFAULT_LOCK_TIMEOUT)
    }

    /// Whether this process runs inside a container (where root is the normal user).
    pub fn is_running_in_container(&self) -> bool {
        is_running_in_container()
    }

    /// The live Claude configuration file (`.claude.json`), mirroring Claude Code.
    pub fn claude_config_path(&self) -> PathBuf {
        paths::global_config_path_in(&self.env)
    }

    /// Validate an email's shape.
    pub fn validate_email(email: &str) -> bool {
        let Some((local, domain)) = email.split_once('@') else {
            return false;
        };
        let local_ok = !local.is_empty()
            && local
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "._%+-".contains(c));
        let Some((host, tld)) = domain.rsplit_once('.') else {
            return false;
        };
        let host_ok = !host.is_empty()
            && host
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || ".-".contains(c));
        let tld_ok = tld.len() >= 2 && tld.chars().all(|c| c.is_ascii_alphabetic());
        local_ok && host_ok && tld_ok
    }

    /// Create the store directories, 0700.
    pub fn setup_directories(&self) -> Result<()> {
        for dir in [&self.backup_dir, &self.configs_dir, &self.credentials_dir] {
            std::fs::create_dir_all(dir)?;
            set_private_dir(dir)?;
        }
        Ok(())
    }

    /// Read and parse a JSON object file. `Ok(None)` when the file is ABSENT.
    ///
    /// With `strict`, a file that is THERE but unreadable, unparseable or not an object is an
    /// [`Error::Config`] rather than `None`: absent is a genuine empty start, unreadable is a file that must
    /// not be overwritten unread (upstream measured a torn `~/.claude.json` replaced by a one-key backup).
    pub fn read_json(&self, path: &Path, strict: bool) -> Result<Option<JsonObject>> {
        read_json(path, strict)
    }

    /// Write a JSON file atomically (0600 temp beside the target, then rename) in `json.dumps(indent=2)`
    /// layout. NaN cannot occur: `serde_json` values hold finite numbers only.
    pub fn write_json(&self, path: &Path, data: &JsonObject) -> Result<()> {
        #[cfg(test)]
        self.faults.check_write_json()?;
        let text = json_dumps_pretty(&Value::Object(data.clone()));
        fsutil::atomic_write_text(path, &text, WriteOptions::default())?;
        Ok(())
    }

    /// Copy an unreadable file aside before it is replaced; returns the copy.
    ///
    /// The bytes survive and the user knows: written 0600 from creation (the file can hold
    /// `primaryApiKey`), a counter suffix so two failures in one second keep two copies, a name free of
    /// `:`, and a warning both into `warnings_out` and (with `emit_output`) on stdout.
    pub fn salvage_unreadable(
        &self,
        path: &Path,
        emit_output: bool,
        warnings_out: &mut Vec<String>,
    ) -> Result<PathBuf> {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let stem = format!("{name}.unreadable-{}", jiff::Timestamp::now().as_second());
        let mut salvage = path.with_file_name(&stem);
        let mut n = 1;
        while salvage.exists() {
            salvage = path.with_file_name(format!("{stem}.{n}"));
            n += 1;
        }
        std::fs::read(path)
            .and_then(|bytes| fsutil::atomic_write_bytes(&salvage, &bytes, WriteOptions::default()))
            .map_err(|err| {
                Error::Switch(format!(
                    "{} could not be parsed and the salvage copy failed ({err}); aborting rather \
                     than destroying it",
                    path.display()
                ))
            })?;
        let salvage_name = salvage
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let msg = format!("{name} could not be parsed — a copy was kept at {salvage_name}");
        log::warn!(
            "{} could not be parsed; a copy was kept at {} before it was replaced",
            path.display(),
            salvage.display()
        );
        warnings_out.push(msg.clone());
        if emit_output {
            printer::warning(&msg);
        }
        Ok(salvage)
    }

    /// Refuse live-store or roster mutation from inside a `haryx run` shell.
    ///
    /// A `CLAUDE_CONFIG_DIR` inside `<store>/sessions` means this shell IS a session: its "live store" is
    /// the profile, so a mutation here would operate on the wrong store. Called by every entry point that
    /// mutates the live store or the roster (there is no single chokepoint).
    pub fn refuse_session_shell(&self) -> Result<()> {
        let Some(cfg_dir) = self
            .env
            .claude_config_dir
            .as_ref()
            .filter(|d| !d.is_empty())
        else {
            return Ok(());
        };
        let inside = paths::resolve_lenient(Path::new(cfg_dir))
            .starts_with(paths::resolve_lenient(&self.backup_dir.join("sessions")));
        if !inside {
            return Ok(());
        }
        Err(Error::Switch(format!(
            "This shell is inside a {} run session profile (CLAUDE_CONFIG_DIR points at it). Mutating \
             accounts here would operate on the wrong live store — unset CLAUDE_CONFIG_DIR or run from a \
             normal shell.",
            crate::PROG
        )))
    }
}

/// [`Switcher::read_json`] as a free function.
pub(crate) fn read_json(path: &Path, strict: bool) -> Result<Option<JsonObject>> {
    let text = match fsutil::read_text_with_retry(
        path,
        fsutil::DEFAULT_RETRY_ATTEMPTS,
        fsutil::INITIAL_RETRY_DELAY,
    ) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) if err.kind() == std::io::ErrorKind::InvalidData => {
            log::warn!("Invalid JSON in {}", path.display());
            return strict_or_none(strict, || {
                format!(
                    "{} exists but could not be parsed ({err}). Repair or move it, then retry — \
                     refusing to overwrite it unread.",
                    path.display()
                )
            });
        }
        Err(err) => {
            log::warn!("Could not read {}: {err}", path.display());
            return strict_or_none(strict, || {
                format!(
                    "{} exists but could not be read ({err}). Fix what is blocking the read, then \
                     retry.",
                    path.display()
                )
            });
        }
    };
    match serde_json::from_str::<Value>(&text) {
        Ok(Value::Object(map)) => Ok(Some(map)),
        Ok(other) => {
            let kind = python_type_name(&other);
            log::warn!("{} holds {kind}, not a JSON object", path.display());
            strict_or_none(strict, || {
                format!(
                    "{} holds {kind}, not a JSON object. Repair or move it, then retry.",
                    path.display()
                )
            })
        }
        Err(err) => {
            log::warn!("Invalid JSON in {}", path.display());
            strict_or_none(strict, || {
                format!(
                    "{} exists but could not be parsed ({err}). Repair or move it, then retry — \
                     refusing to overwrite it unread.",
                    path.display()
                )
            })
        }
    }
}

fn strict_or_none(strict: bool, message: impl FnOnce() -> String) -> Result<Option<JsonObject>> {
    if strict {
        Err(Error::Config(message()))
    } else {
        Ok(None)
    }
}

/// Python's `type(x).__name__` for a parsed JSON value.
fn python_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "NoneType",
        Value::Bool(_) => "bool",
        Value::Number(n) if n.is_f64() => "float",
        Value::Number(_) => "int",
        Value::String(_) => "str",
        Value::Array(_) => "list",
        Value::Object(_) => "dict",
    }
}

/// `CONTAINER`/`container` set, `/.dockerenv` present, PID 1's cgroup naming a container runtime, or
/// this process's mount info naming docker/overlay.
pub fn is_running_in_container() -> bool {
    let env_set = |name: &str| std::env::var_os(name).is_some_and(|v| !v.is_empty());
    if env_set("CONTAINER") || env_set("container") {
        return true;
    }
    if cfg!(windows) {
        return false;
    }
    if Path::new("/.dockerenv").exists() {
        return true;
    }
    let file_mentions = |path: &str, markers: &[&str]| {
        std::fs::read_to_string(path)
            .is_ok_and(|content| markers.iter().any(|marker| content.contains(marker)))
    };
    file_mentions(
        "/proc/1/cgroup",
        &["docker", "lxc", "containerd", "kubepods"],
    ) || file_mentions("/proc/self/mountinfo", &["docker", "overlay"])
}

#[cfg(unix)]
pub(crate) fn set_private_dir(dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
}

#[cfg(not(unix))]
pub(crate) fn set_private_dir(_dir: &Path) -> std::io::Result<()> {
    Ok(())
}

// --- small JSON accessors shared by the method groups -------------------------------------------------

/// `data["accounts"]` as an object (an empty view when missing or not an object).
pub(crate) fn accounts(data: &JsonObject) -> Option<&JsonObject> {
    data.get("accounts").and_then(Value::as_object)
}

/// `data["accounts"][num]` when it is a non-empty object (upstream's truthiness test on a record).
pub(crate) fn record<'a>(data: &'a JsonObject, num: &str) -> Option<&'a JsonObject> {
    accounts(data)
        .and_then(|a| a.get(num))
        .and_then(Value::as_object)
        .filter(|r| !r.is_empty())
}

/// A string field of a record, `""` when missing, null or not a string (upstream `.get(k, "") or ""`).
pub(crate) fn text<'a>(record: &'a JsonObject, key: &str) -> &'a str {
    record.get(key).and_then(Value::as_str).unwrap_or_default()
}

/// Python truthiness of a JSON value.
pub(crate) fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|f| f != 0.0),
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Object(o)) => !o.is_empty(),
    }
}

/// Sort key of a slot number string: numeric slots by value, anything else after them.
pub(crate) fn slot_key(num: &str) -> (bool, i64) {
    match num.parse::<i64>() {
        Ok(n) if num.bytes().all(|b| b.is_ascii_digit()) => (false, n),
        _ => (true, 0),
    }
}

//! Port of `claude_swap/credentials.py`: the credential storage layer.
//!
//! Owns *where* credentials live and *how* they are read/written: the macOS Keychain-vs-file routing,
//! per-process capability detection and sticky fallback, and the `.enc`-wins backup reconciliation.
//!
//! [`CredentialStore`] is a leaf collaborator: it reads its configuration from a [`StoreHost`] (data only)
//! and never calls into the switcher. It owns only its capability state (sticky, process-local) and the
//! backend of the last active-credential write (for the post-switch follow-up message).
//!
//! Two kinds of credential:
//!
//! - The LIVE credential, shared with Claude Code: Keychain `Claude Code-credentials` (or the hashed name
//!   of a custom profile, see [`claude_keychain`]) on macOS, `.credentials.json` everywhere, plus the
//!   managed API key (`Claude Code` item / `primaryApiKey`). Named and placed exactly as upstream.
//! - Each slot's BACKUP, haryx's own namespace (see [`namespace`]): Keychain service `haryx`, account
//!   `account-{num}-{email}`, or base64 `.creds-{num}-{email}.enc` files under the store root's
//!   `credentials/`.
//!
//! Module map: `active` (live credential), `backup` (slot backups and `.prev`), `stash` (unclaimed
//! credentials), `kind` (key-vs-OAuth detection, shared fields).

mod active;
mod backup;
pub mod claude_keychain;
mod kind;
pub mod namespace;
mod stash;

#[cfg(test)]
pub(crate) mod test_support;
#[cfg(test)]
mod tests_active;
#[cfg(test)]
mod tests_backup;
#[cfg(test)]
mod tests_capability;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

pub use claude_keychain::{
    CLAUDE_CODE_KEYCHAIN_SERVICE, CLAUDE_CODE_MANAGED_KEYCHAIN_SERVICE,
    active_oauth_keychain_services, active_profile_is_default,
    active_store_is_the_default_secure_store, delete_macos_keychain_entry, keychain_service_name,
};
pub use kind::{
    ACCOUNT_CREDENTIAL_KEYS, SHARED_CREDENTIAL_KEYS, approved_form, looks_like_api_key,
    merge_shared_credential_fields, shared_credential_fields,
};
pub use namespace::{
    BackupNamespace, CSWAP_BACKUP_SERVICE, CSWAP_LEGACY_KEYRING_SERVICE, HARYX_BACKUP_SERVICE,
    cswap_backup_namespaces, haryx_backup_namespace,
};
pub use stash::ManifestVerdict;

use crate::keychain::{self, KeychainError, Security};
use crate::paths::{HostOs, PathEnv};

/// Attempts of the active OAuth-credential Keychain read. A locked/contended login Keychain can fail a single
/// `security` call transiently (just after wake, or under contention with Claude Code's own statusline);
/// a second attempt a moment later usually succeeds. An I/O backoff, not a sleep over an internal race.
pub const ACTIVE_READ_ATTEMPTS: u32 = 2;
/// Pause between those attempts.
pub const ACTIVE_READ_RETRY_DELAY: Duration = Duration::from_millis(300);
/// After a Keychain failure the store drops to file mode so one CLI invocation cannot split-brain between
/// backends. A long-running process re-probes this long after the last failure: far longer than any CLI
/// command runs, short enough that a transient `security` timeout self-heals within a minute.
pub const KEYCHAIN_RECHECK_COOLDOWN: Duration = Duration::from_secs(60);
/// TTL of the per-slot cache for DISPLAY reads of backup Keychain items (upstream #496: a dashboard
/// repainting every few seconds must not spawn `security` for every slot on every pass). Below the usage
/// store's fetch floor, so a slot's next fetch always re-reads after an external rotation.
pub const BACKUP_READ_CACHE_TTL: Duration = Duration::from_secs(60);

/// The Keychain operations the store needs. Production uses [`Security`] (the `security` CLI); tests use an
/// in-memory fake.
pub trait KeychainBackend: Send + Sync {
    /// The stored password, `None` when absent (rc 44); an error for anything else.
    fn get_password(&self, service: &str, account: &str) -> Result<Option<String>, KeychainError>;
    /// Create or update an item.
    fn set_password(
        &self,
        service: &str,
        account: &str,
        password: &str,
    ) -> Result<(), KeychainError>;
    /// Delete an item; already absent counts as success.
    fn delete_password(&self, service: &str, account: &str) -> Result<(), KeychainError>;
}

impl KeychainBackend for Security {
    fn get_password(&self, service: &str, account: &str) -> Result<Option<String>, KeychainError> {
        Security::get_password(self, service, account, None)
    }

    fn set_password(
        &self,
        service: &str,
        account: &str,
        password: &str,
    ) -> Result<(), KeychainError> {
        Security::set_password(self, service, account, password)
    }

    fn delete_password(&self, service: &str, account: &str) -> Result<(), KeychainError> {
        Security::delete_password(self, service, account)
    }
}

/// Outcome of reading Claude Code's active credential.
///
/// `value` is the credential string (OAuth JSON or a raw managed key), `Some("")` when none exists in any
/// backend, or `None` on a plaintext-file read error. `keychain_unavailable` is true only when the macOS
/// OAuth Keychain read failed (locked / denied / timeout) and nothing else covered it, so callers can tell a
/// transiently unreadable Keychain from a genuinely empty slot. `degraded` is true whenever the OAuth
/// Keychain read failed, even when a fallback covered it: the bytes served may then be a stale generation
/// (on macOS Claude Code rotates keychain-only, so the plaintext file can lag). A degraded credential may be
/// adopted or served, but its refresh token must never be consumed: POSTing a superseded one-time refresh
/// token yields `invalid_grant` and a false dead-token strike on a live account.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActiveCredentials {
    pub value: Option<String>,
    pub keychain_unavailable: bool,
    pub degraded: bool,
}

impl ActiveCredentials {
    fn new(value: Option<String>, keychain_unavailable: bool, degraded: bool) -> Self {
        ActiveCredentials {
            value,
            keychain_unavailable,
            degraded,
        }
    }
}

/// Where the most recent active-credential write landed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CredentialBackend {
    Keychain,
    File,
}

impl CredentialBackend {
    pub fn as_str(self) -> &'static str {
        match self {
            CredentialBackend::Keychain => "keychain",
            CredentialBackend::File => "file",
        }
    }
}

/// The configuration the store reads (upstream `_StoreHost`, plus the path environment upstream reads from
/// the process).
#[derive(Clone, Debug)]
pub struct StoreHost {
    /// Keychain routing is macOS-only; every other OS uses files.
    pub platform: HostOs,
    /// Directory of `.enc` backups and the unclaimed stash.
    pub credentials_dir: PathBuf,
    /// The account registry; its stat keys the backup read cache.
    pub sequence_file: PathBuf,
    /// Keychain service of the slot backups ([`HARYX_BACKUP_SERVICE`]; [`CSWAP_BACKUP_SERVICE`] to read a cswap
    /// store).
    pub backup_service: String,
    /// Keychain account of Claude Code's live items ([`keychain::keychain_account_name`]).
    pub keychain_account: String,
    /// Where Claude Code's files are (`CLAUDE_CONFIG_DIR`, `CLAUDE_SECURESTORAGE_CONFIG_DIR`, home).
    pub env: PathEnv,
}

impl StoreHost {
    /// The host for a backup namespace, under the given path environment.
    pub fn new(namespace: &BackupNamespace, env: PathEnv) -> Self {
        StoreHost {
            platform: env.os,
            credentials_dir: namespace.credentials_dir.clone(),
            sequence_file: namespace.sequence_file.clone(),
            backup_service: namespace.keychain_service.to_string(),
            keychain_account: keychain::keychain_account_name(),
            env,
        }
    }

    /// haryx's own store for this process's environment.
    pub fn from_process() -> crate::Result<Self> {
        let env = PathEnv::from_process()?;
        Ok(StoreHost::new(&haryx_backup_namespace(&env)?, env))
    }
}

/// Timing knobs; the defaults are upstream's constants. Tests shorten them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tunables {
    pub active_read_retry_delay: Duration,
    pub keychain_recheck_cooldown: Duration,
    pub backup_read_cache_ttl: Duration,
}

impl Default for Tunables {
    fn default() -> Self {
        Tunables {
            active_read_retry_delay: ACTIVE_READ_RETRY_DELAY,
            keychain_recheck_cooldown: KEYCHAIN_RECHECK_COOLDOWN,
            backup_read_cache_ttl: BACKUP_READ_CACHE_TTL,
        }
    }
}

/// Keychain capability state, learned per process. One mutex guards it all (upstream's attributes plus
/// `_pin_lock`); it is never held across a `security` call.
#[derive(Debug, Default)]
struct KcState {
    /// Where ops should be ROUTED: `None` not yet probed, then learned from real calls.
    usable_cache: Option<bool>,
    /// When file mode was entered by a real failure, when to re-probe. `None`: no pending re-probe.
    disabled_until: Option<Instant>,
    /// Whether file mode was CHOSEN (a write fell back and the item was deleted) rather than forced.
    file_mode_is_ours: bool,
    /// Whether a Keychain op FAILED and has not since succeeded. An observation, unlike the routing cache.
    op_failed: bool,
    /// Set by the ACTIVE OAuth read when that read could not reach the Keychain. Per item: a success on some
    /// other item (an idle slot's backup) is not evidence about this one.
    active_read_failed: bool,
    /// What the last pin OBSERVED about the residual active item: `None` never pinned, `Some(true)` the
    /// delete returned (nothing can shadow the file), `Some(false)` it could not run.
    residual_verdict: Option<bool>,
    /// Whether the current pin came from the OAuth write fallback, i.e. points at `.credentials.json`.
    pinned_to_oauth_file: bool,
    last_backend: Option<CredentialBackend>,
    /// The cooldown deadline already announced in the log: one line per cooldown.
    backoff_announced_until: Option<Instant>,
}

/// Owns the active and per-account backup credential stores.
///
/// One store per switcher: the capability state is per process, learned from real `security` calls, and a
/// fresh process re-evaluates from scratch. `Sync`: one store may serve several worker threads.
pub struct CredentialStore {
    host: StoreHost,
    keychain: Arc<dyn KeychainBackend>,
    tunables: Tunables,
    state: Mutex<KcState>,
    cache: Mutex<backup::BackupReadCache>,
}

impl std::fmt::Debug for CredentialStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CredentialStore")
            .field("host", &self.host)
            .finish_non_exhaustive()
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // A poisoned lock only means another thread panicked mid-update of plain flags; they stay usable.
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl CredentialStore {
    /// A store over `host`, driving the system `security` binary (`HARYX_SECURITY_BIN` overrides it).
    pub fn new(host: StoreHost) -> Self {
        Self::with_keychain(host, Arc::new(Security::system()), Tunables::default())
    }

    /// A store with an explicit Keychain backend and timing.
    pub fn with_keychain(
        host: StoreHost,
        keychain: Arc<dyn KeychainBackend>,
        tunables: Tunables,
    ) -> Self {
        CredentialStore {
            host,
            keychain,
            tunables,
            state: Mutex::new(KcState::default()),
            cache: Mutex::new(backup::BackupReadCache::default()),
        }
    }

    pub fn host(&self) -> &StoreHost {
        &self.host
    }

    pub fn credentials_dir(&self) -> &Path {
        &self.host.credentials_dir
    }

    fn is_macos(&self) -> bool {
        self.host.platform == HostOs::MacOs
    }

    /// Where the most recent active-credential write landed (`None` before any).
    pub fn last_active_credentials_backend(&self) -> Option<CredentialBackend> {
        lock(&self.state).last_backend
    }

    fn set_last_backend(&self, backend: CredentialBackend) {
        lock(&self.state).last_backend = Some(backend);
    }

    /// Run a Keychain op, learning Keychain usability (upstream `_kc_call`).
    ///
    /// A success (including "absent") marks the Keychain usable, but only flips the routing cache
    /// `None -> true`, never `false -> true`: once a call has failed this run the store stays in file mode so
    /// one invocation cannot split-brain. A failure marks it unusable, arms the re-probe deadline and is
    /// returned so the caller can fall back. A success also clears `op_failed` unconditionally: the cache is
    /// a routing decision, deliberately sticky; `op_failed` is a fact about whether the Keychain answers, and
    /// a call that just returned is proof it does.
    fn kc_call<T>(
        &self,
        op: impl FnOnce(&dyn KeychainBackend) -> Result<T, KeychainError>,
    ) -> Result<T, KeychainError> {
        let result = op(self.keychain.as_ref());
        let mut state = lock(&self.state);
        match &result {
            Err(_) => {
                state.op_failed = true;
                state.usable_cache = Some(false);
                state.disabled_until =
                    Some(Instant::now() + self.tunables.keychain_recheck_cooldown);
            }
            Ok(_) => {
                state.op_failed = false;
                if state.usable_cache.is_none() {
                    state.usable_cache = Some(true);
                }
            }
        }
        result
    }

    /// Whether credential ops should target the macOS Keychain right now.
    ///
    /// `false` off macOS. On macOS, `true` until a Keychain op fails, which drops to file mode with a
    /// re-probe deadline: within one CLI invocation the deadline never passes, but a long-running process
    /// re-probes once the cooldown elapses. A pinned file mode (no deadline) stays sticky; see
    /// [`Self::pin_file_mode`].
    pub fn use_keychain(&self) -> bool {
        if !self.is_macos() {
            return false;
        }
        let mut state = lock(&self.state);
        Self::use_keychain_locked(&mut state)
    }

    fn use_keychain_locked(state: &mut KcState) -> bool {
        if state.usable_cache == Some(false)
            && state.disabled_until.is_some_and(|d| Instant::now() >= d)
        {
            state.usable_cache = None; // cooldown elapsed -> re-probe
            state.disabled_until = None;
        }
        state.usable_cache != Some(false)
    }

    /// Pin file mode for the rest of the process, with no Keychain re-probe.
    ///
    /// A read timeout is safe to recover from (re-probe on cooldown), but an active-credential *write* that
    /// falls back to the file is not: its best-effort delete of the old Keychain item may have failed,
    /// leaving a stale entry, and re-probing later could read that residual and show the wrong account.
    ///
    /// `residual_cleared` is the caller's OBSERVATION of that delete. True: nothing can shadow the file, so
    /// it is the authority and the two failure flags are settled (a true verdict settles the PAST, not the
    /// future). False: a residual may survive, however many unrelated items answer afterwards.
    pub fn pin_file_mode(&self, residual_cleared: bool) {
        Self::pin_file_mode_locked(&mut lock(&self.state), residual_cleared);
    }

    fn pin_file_mode_locked(state: &mut KcState, residual_cleared: bool) {
        state.usable_cache = Some(false);
        state.disabled_until = None;
        state.file_mode_is_ours = true;
        state.residual_verdict = Some(residual_cleared);
        // Only the OAuth write fallback marks a pin as liftable, right after this call; every other pin resets
        // it, so a stale true can never lift a pin it does not describe.
        state.pinned_to_oauth_file = false;
        if residual_cleared {
            state.op_failed = false;
            state.active_read_failed = false;
        }
    }

    /// End a pinned file mode once the file it points at no longer exists.
    ///
    /// The pin is a claim about `.credentials.json`. Claude Code removes the plaintext file when it stores a
    /// rotation in the Keychain, so on a long-running process the pin outlives its file within hours, and from
    /// then on every active read would return "" for the life of the process. With the file gone, a
    /// surviving Keychain item is what Claude Code itself is using. This only RE-ASKS the Keychain; the read
    /// that follows reports its own outcome.
    fn lift_pin_if_oauth_file_gone(&self) {
        let cred_file = crate::paths::credentials_path_in(&self.host.env);
        {
            let mut state = lock(&self.state);
            if !state.pinned_to_oauth_file || Self::use_keychain_locked(&mut state) {
                return;
            }
            if state.disabled_until.is_some() {
                return; // a later failure re-armed the cooldown; let it run
            }
            match std::fs::metadata(&cred_file) {
                Ok(meta) if meta.len() > 0 => return,
                Ok(_) => {}
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return, // cannot tell; the pin is the conservative answer
            }
            state.usable_cache = None;
            state.file_mode_is_ours = false;
            state.pinned_to_oauth_file = false;
            state.residual_verdict = None;
        }
        log::info!(
            "Active credentials file is gone; reading the Keychain again \
             (file mode had been pinned by a failed Keychain write)."
        );
    }

    /// The Keychain cannot be asked, so an empty read proves nothing.
    ///
    /// True only when a Keychain op FAILED and has not since succeeded, and the cooldown has not lapsed.
    /// Off macOS there is no Keychain to be unreadable. A pinned file mode is not part of the answer: on
    /// macOS the pin is reachable only through a Keychain op that just failed, so it cannot mean "nothing
    /// failed".
    pub fn keychain_unreadable(&self) -> bool {
        if !self.is_macos() {
            return false;
        }
        let mut state = lock(&self.state);
        if Self::use_keychain_locked(&mut state) {
            return false;
        }
        state.op_failed
    }

    /// Whether per-account backup *writes* go to `.enc` files rather than the Keychain. Backup *reads* are
    /// `.enc`-wins regardless.
    pub fn uses_file_backup_backend(&self) -> bool {
        !self.use_keychain()
    }
}

/// Read-only views of the capability state, for tests that assert on it.
#[cfg(test)]
impl CredentialStore {
    fn set_usable_cache(&self, value: Option<bool>) {
        lock(&self.state).usable_cache = value;
    }

    fn expire_cooldown(&self) {
        lock(&self.state).disabled_until = Some(Instant::now() - Duration::from_millis(1));
    }

    fn cooldown_armed(&self) -> bool {
        lock(&self.state).disabled_until.is_some()
    }

    fn residual_verdict(&self) -> Option<bool> {
        lock(&self.state).residual_verdict
    }

    fn file_mode_is_ours(&self) -> bool {
        lock(&self.state).file_mode_is_ours
    }

    fn active_read_failed(&self) -> bool {
        lock(&self.state).active_read_failed
    }
}

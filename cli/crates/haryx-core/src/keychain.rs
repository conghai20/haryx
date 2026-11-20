//! Port of `claude_swap/macos_keychain.py`: macOS Keychain access via the `security` CLI.
//!
//! A small wrapper around the system `security` tool for generic passwords. Items are created and read by
//! the same stable `security` binary, so reads stay silent across haryx upgrades: an in-process
//! Security.framework call would anchor the item's access to the haryx binary, which every upgrade replaces,
//! and macOS would then prompt. It also keeps item ACLs compatible with Claude Code, which drives `security`
//! the same way.
//!
//! The read/write/delete shapes mirror Claude Code's own implementation
//! (`utils/secureStorage/macOsKeychainStorage.ts`):
//!
//! - [`Security::set_password`] hex-encodes the value (`-X`) and pipes the command through `security -i`
//!   (stdin) so the secret never appears in process argv. It falls back to argv only when the command would
//!   overflow `security -i`'s 4096-byte stdin line buffer, which would otherwise truncate mid-argument and
//!   silently corrupt the entry.
//! - [`Security::get_password`] uses `find-generic-password ... -w` and treats exit code 44 as "not found"
//!   (`None`); any *other* non-zero exit is an error so callers can tell a genuine miss from a
//!   locked/denied/unavailable Keychain.
//!
//! Caveat: values must be printable text. `find-generic-password -w` prints printable data raw and anything
//! else hex-encoded, so a round trip of binary data is not identity. Fine for credentials (ASCII JSON).
//!
//! The binary is `/usr/bin/security`, pinned rather than looked up on `PATH`: this is a credential tool, so an
//! attacker-controlled `security` earlier on `PATH` must not intercept secrets. `HARYX_SECURITY_BIN` replaces
//! it for tests (a fake `security` script); anyone able to set the environment of haryx already controls it.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::error::Error;
use crate::spawn::{self, RunOptions, RunOutput, SpawnError};

/// `security -i` reads stdin with a 4096-byte `fgets()` buffer (BUFSIZ on Darwin). A longer command line is
/// truncated mid-argument: it fails to write while leaving any previous entry intact (Claude Code #30337).
/// 64 bytes of headroom guards against line-terminator accounting differences.
pub const SECURITY_STDIN_LINE_LIMIT: usize = 4096 - 64;
/// errSecItemNotFound, surfaced by find/delete-generic-password.
pub const NOT_FOUND_RC: i32 = 44;
/// Bound on every `security` spawn, so a wedged Keychain (a locked login keychain prompting for an unlock
/// that never comes on a headless/SSH host) cannot hang the CLI. A healthy Keychain answers in well under
/// 100ms.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);
/// Apple's system binary.
pub const SYSTEM_SECURITY_BIN: &str = "/usr/bin/security";
/// Environment variable naming a replacement `security` binary (tests).
pub const SECURITY_BIN_ENV: &str = "HARYX_SECURITY_BIN";
/// Last-resort Keychain account name, as Claude Code uses.
pub const FALLBACK_ACCOUNT_NAME: &str = "claude-code-user";

/// A Keychain operation failed. Every variant means "Keychain unusable" to callers (upstream's
/// `KEYCHAIN_ERRORS`: `KeychainError`, `KeychainTimeout`, `TimeoutExpired`, `OSError`), never a bug.
#[derive(Debug, thiserror::Error)]
pub enum KeychainError {
    /// `security` exited non-zero for a reason other than "not found".
    #[error("{0}")]
    Failed(String),
    /// `security` did not finish within its time limit.
    #[error("{0}")]
    Timeout(String),
    /// `security` could not be started.
    #[error("{0}")]
    Io(#[from] std::io::Error),
}

impl KeychainError {
    pub fn is_timeout(&self) -> bool {
        matches!(self, KeychainError::Timeout(_))
    }
}

impl From<KeychainError> for Error {
    fn from(err: KeychainError) -> Self {
        Error::Credential(err.to_string())
    }
}

/// The account name for the active-credential Keychain item, mirroring Claude Code's `getUsername()`:
/// `$USER` first, then the OS username, then [`FALLBACK_ACCOUNT_NAME`]. Matching this exactly matters on
/// headless/launchd/cron hosts where `$USER` is unset: a divergent default would key a *different* Keychain
/// item than Claude Code.
pub fn keychain_account_name() -> String {
    account_name_from(std::env::var("USER").ok().as_deref(), os_username)
}

fn account_name_from(user_env: Option<&str>, os_user: impl FnOnce() -> Option<String>) -> String {
    match user_env {
        Some(user) if !user.is_empty() => user.to_string(),
        _ => os_user().unwrap_or_else(|| FALLBACK_ACCOUNT_NAME.to_string()),
    }
}

#[cfg(unix)]
fn os_username() -> Option<String> {
    // SAFETY: getpwuid returns NULL or a pointer to static storage valid until the next getpw* call on this
    // thread; the name is copied out immediately.
    unsafe {
        let pw = libc::getpwuid(libc::geteuid());
        if pw.is_null() || (*pw).pw_name.is_null() {
            return None;
        }
        Some(
            std::ffi::CStr::from_ptr((*pw).pw_name)
                .to_string_lossy()
                .into_owned(),
        )
    }
}

#[cfg(not(unix))]
fn os_username() -> Option<String> {
    None
}

/// Quote a value for a `security -i` stdin command line. `security -i` re-parses each line shell-style, so
/// wrap the value in double quotes and backslash-escape any embedded `"`/`\` (the active-credential service
/// name contains a space).
fn quote(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// A handle on a `security` binary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Security {
    bin: PathBuf,
    timeout: Duration,
}

impl Security {
    /// `/usr/bin/security`, or `$HARYX_SECURITY_BIN` when set and non-empty.
    pub fn system() -> Self {
        let bin = std::env::var_os(SECURITY_BIN_ENV)
            .filter(|v| !v.is_empty())
            .map_or_else(|| PathBuf::from(SYSTEM_SECURITY_BIN), PathBuf::from);
        Security::with_binary(bin)
    }

    /// A specific `security` binary, with the default time limit.
    pub fn with_binary(bin: impl Into<PathBuf>) -> Self {
        Security {
            bin: bin.into(),
            timeout: DEFAULT_TIMEOUT,
        }
    }

    /// The same binary with another default time limit.
    pub fn with_timeout(self, timeout: Duration) -> Self {
        Security { timeout, ..self }
    }

    pub fn binary(&self) -> &Path {
        &self.bin
    }

    fn run(
        &self,
        args: &[&str],
        input: Option<String>,
        timeout: Duration,
    ) -> Result<RunOutput, SpawnError> {
        let argv: Vec<&std::ffi::OsStr> = std::iter::once(self.bin.as_os_str())
            .chain(args.iter().map(std::ffi::OsStr::new))
            .collect();
        let opts = RunOptions {
            env: Vec::new(),
            input: input.map(String::into_bytes),
            timeout: Some(timeout),
            ..Default::default()
        };
        spawn::run(&argv, &opts)
    }

    fn checked(
        &self,
        op: &str,
        args: &[&str],
        input: Option<String>,
        timeout: Duration,
    ) -> Result<RunOutput, KeychainError> {
        match self.run(args, input, timeout) {
            Ok(out) => Ok(out),
            Err(SpawnError::Timeout { .. }) => Err(KeychainError::Timeout(format!(
                "security {op} timed out after {}s",
                timeout.as_secs_f64()
            ))),
            Err(SpawnError::Io(err)) => Err(KeychainError::Io(err)),
        }
    }

    fn failure(op: &str, out: &RunOutput) -> KeychainError {
        KeychainError::Failed(format!(
            "security {op} failed (rc={}): {}",
            out.code
                .map_or_else(|| "signal".to_string(), |c| c.to_string()),
            out.stderr.trim()
        ))
    }

    /// The stored password, or `None` if no such item exists (rc 44).
    ///
    /// Any other non-zero exit is [`KeychainError::Failed`] and a timeout [`KeychainError::Timeout`], so a
    /// genuine miss is not confused with a transient failure. `timeout` replaces the default bound for a read
    /// that waits on a person.
    pub fn get_password(
        &self,
        service: &str,
        account: &str,
        timeout: Option<Duration>,
    ) -> Result<Option<String>, KeychainError> {
        let op = "find-generic-password";
        let out = self.checked(
            op,
            &[op, "-a", account, "-w", "-s", service],
            None,
            timeout.unwrap_or(self.timeout),
        )?;
        match out.code {
            // `-w` prints the value followed by one newline; strip exactly that so values with meaningful
            // leading/trailing whitespace survive intact.
            Some(0) => Ok(Some(
                out.stdout
                    .strip_suffix('\n')
                    .unwrap_or(&out.stdout)
                    .to_string(),
            )),
            Some(NOT_FOUND_RC) => Ok(None),
            _ => Err(Self::failure(op, &out)),
        }
    }

    /// Whether a generic-password item exists, without touching its secret.
    ///
    /// Attribute-only lookup (no `-w`): nothing is decrypted, so this never triggers a Keychain prompt.
    /// `true` only on rc 0; "not found", error exits, a timeout and a missing binary are all `false`.
    /// Deliberately non-failing: callers use it for cleanup verification, not access decisions.
    pub fn item_exists(&self, service: &str, account: &str) -> bool {
        self.run(
            &["find-generic-password", "-a", account, "-s", service],
            None,
            self.timeout,
        )
        .is_ok_and(|out| out.success())
    }

    /// Create or update a generic-password item (`-U`).
    ///
    /// Prefers `security -i` stdin so the secret stays out of argv; falls back to argv only for payloads that
    /// would overflow the stdin line buffer (hex in argv is recoverable by a determined observer, but silent
    /// corruption is strictly worse).
    pub fn set_password(
        &self,
        service: &str,
        account: &str,
        password: &str,
    ) -> Result<(), KeychainError> {
        let op = "add-generic-password";
        let hex_value = hex(password.as_bytes());
        let command = format!(
            "{op} -U -a {} -s {} -X {hex_value}\n",
            quote(account),
            quote(service)
        );
        let out = if command.len() <= SECURITY_STDIN_LINE_LIMIT {
            self.checked(op, &["-i"], Some(command), self.timeout)?
        } else {
            self.checked(
                op,
                &[op, "-U", "-a", account, "-s", service, "-X", &hex_value],
                None,
                self.timeout,
            )?
        };
        if out.success() {
            Ok(())
        } else {
            Err(Self::failure(op, &out))
        }
    }

    /// Delete a generic-password item. rc 44 (already absent) counts as success.
    pub fn delete_password(&self, service: &str, account: &str) -> Result<(), KeychainError> {
        let op = "delete-generic-password";
        let out = self.checked(op, &[op, "-a", account, "-s", service], None, self.timeout)?;
        match out.code {
            Some(0 | NOT_FOUND_RC) => Ok(()),
            _ => Err(Self::failure(op, &out)),
        }
    }
}

/// [`Security::get_password`] on [`Security::system`].
pub fn get_password(
    service: &str,
    account: &str,
    timeout: Option<Duration>,
) -> Result<Option<String>, KeychainError> {
    Security::system().get_password(service, account, timeout)
}

/// [`Security::item_exists`] on [`Security::system`].
pub fn item_exists(service: &str, account: &str) -> bool {
    Security::system().item_exists(service, account)
}

/// [`Security::set_password`] on [`Security::system`].
pub fn set_password(service: &str, account: &str, password: &str) -> Result<(), KeychainError> {
    Security::system().set_password(service, account, password)
}

/// [`Security::delete_password`] on [`Security::system`].
pub fn delete_password(service: &str, account: &str) -> Result<(), KeychainError> {
    Security::system().delete_password(service, account)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use tempfile::TempDir;

    /// A fake `security`: logs its argv (one per line) to `argv.log` and stdin to `stdin.log` in its
    /// directory, then runs `body`.
    struct Fake {
        dir: TempDir,
        security: Security,
    }

    impl Fake {
        fn new(body: &str) -> Self {
            let dir = TempDir::new().unwrap();
            let bin = dir.path().join("security");
            let log = dir.path().display().to_string();
            std::fs::write(
                &bin,
                format!(
                    "#!/bin/sh\nfor a in \"$@\"; do printf '%s\\n' \"$a\"; done > '{log}/argv.log'\n\
                     cat > '{log}/stdin.log'\n{body}\n"
                ),
            )
            .unwrap();
            std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
            Fake {
                security: Security::with_binary(&bin),
                dir,
            }
        }

        fn argv(&self) -> Vec<String> {
            std::fs::read_to_string(self.dir.path().join("argv.log"))
                .unwrap()
                .lines()
                .map(String::from)
                .collect()
        }

        fn stdin(&self) -> String {
            std::fs::read_to_string(self.dir.path().join("stdin.log")).unwrap()
        }
    }

    #[test]
    fn get_password_returns_value_on_rc0_stripping_one_newline() {
        let fake = Fake::new("printf ' secret \\n\\n'");
        assert_eq!(
            fake.security.get_password("svc", "acct", None).unwrap(),
            Some(" secret \n".into())
        );
        assert_eq!(
            fake.argv(),
            ["find-generic-password", "-a", "acct", "-w", "-s", "svc"]
        );
    }

    #[test]
    fn get_password_returns_none_only_on_rc44() {
        let fake = Fake::new("exit 44");
        assert_eq!(
            fake.security.get_password("svc", "acct", None).unwrap(),
            None
        );
    }

    #[test]
    fn get_password_raises_on_other_nonzero() {
        let fake = Fake::new("echo 'User interaction is not allowed.' >&2; exit 36");
        let err = fake.security.get_password("svc", "acct", None).unwrap_err();
        assert!(matches!(err, KeychainError::Failed(_)));
        assert_eq!(
            err.to_string(),
            "security find-generic-password failed (rc=36): User interaction is not allowed."
        );
    }

    #[test]
    fn timeout_becomes_keychain_timeout() {
        let fake = Fake::new("sleep 5");
        let err = fake
            .security
            .get_password("svc", "acct", Some(Duration::from_millis(200)))
            .unwrap_err();
        assert!(err.is_timeout());
        assert!(err.to_string().contains("timed out after 0.2s"));
        let quick = fake
            .security
            .clone()
            .with_timeout(Duration::from_millis(200));
        assert!(
            quick
                .set_password("svc", "acct", "pw")
                .unwrap_err()
                .is_timeout()
        );
        assert!(
            quick
                .delete_password("svc", "acct")
                .unwrap_err()
                .is_timeout()
        );
        assert!(
            !quick.item_exists("svc", "acct"),
            "item_exists stays false on timeout"
        );
    }

    #[test]
    fn item_exists_true_on_rc0_and_never_requests_secret() {
        let fake = Fake::new("exit 0");
        assert!(fake.security.item_exists("svc", "acct"));
        assert!(!fake.argv().contains(&"-w".to_string()));
        assert!(!Fake::new("exit 44").security.item_exists("svc", "acct"));
        assert!(!Fake::new("exit 1").security.item_exists("svc", "acct"));
        assert!(!Security::with_binary("/nonexistent/security").item_exists("svc", "acct"));
    }

    #[test]
    fn set_password_small_payload_uses_security_i_stdin() {
        let fake = Fake::new("exit 0");
        fake.security
            .set_password("Claude Code-credentials", "me", "{\"t\":1}")
            .unwrap();
        assert_eq!(fake.argv(), ["-i"]);
        let line = fake.stdin();
        assert_eq!(
            line,
            format!(
                "add-generic-password -U -a \"me\" -s \"Claude Code-credentials\" -X {}\n",
                hex(b"{\"t\":1}")
            )
        );
        assert!(
            !line.contains("{\"t\":1}"),
            "the secret never appears in clear"
        );
    }

    #[test]
    fn set_password_large_payload_falls_back_to_argv() {
        let fake = Fake::new("exit 0");
        let big = "x".repeat(SECURITY_STDIN_LINE_LIMIT);
        fake.security.set_password("svc", "acct", &big).unwrap();
        let argv = fake.argv();
        assert_eq!(
            argv[..7],
            [
                "add-generic-password",
                "-U",
                "-a",
                "acct",
                "-s",
                "svc",
                "-X"
            ]
        );
        assert_eq!(argv[7], hex(big.as_bytes()));
    }

    #[test]
    fn set_password_raises_on_nonzero() {
        let fake = Fake::new("echo denied >&2; exit 1");
        let err = fake.security.set_password("svc", "acct", "pw").unwrap_err();
        assert_eq!(
            err.to_string(),
            "security add-generic-password failed (rc=1): denied"
        );
    }

    #[test]
    fn set_get_roundtrip_hex_is_decodable() {
        let fake = Fake::new("exit 0");
        let secret = "{\"accessToken\":\"sk-ant-\\\"quoted\\\" é\"}";
        fake.security.set_password("svc", "acct", secret).unwrap();
        let line = fake.stdin();
        let hex_value = line.trim_end().rsplit(' ').next().unwrap();
        let decoded: Vec<u8> = (0..hex_value.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex_value[i..i + 2], 16).unwrap())
            .collect();
        assert_eq!(String::from_utf8(decoded).unwrap(), secret);
    }

    #[test]
    fn delete_password_rc0_and_rc44_are_success_others_raise() {
        assert!(
            Fake::new("exit 0")
                .security
                .delete_password("svc", "acct")
                .is_ok()
        );
        assert!(
            Fake::new("exit 44")
                .security
                .delete_password("svc", "acct")
                .is_ok()
        );
        let err = Fake::new("exit 2")
            .security
            .delete_password("svc", "acct")
            .unwrap_err();
        assert!(matches!(err, KeychainError::Failed(_)));
    }

    #[test]
    fn missing_binary_is_an_io_error() {
        let security = Security::with_binary("/nonexistent/security");
        assert!(matches!(
            security.get_password("s", "a", None),
            Err(KeychainError::Io(_))
        ));
        let as_error: Error = security.set_password("s", "a", "p").unwrap_err().into();
        assert!(as_error.is_credential());
    }

    #[test]
    fn quote_escapes_backslash_and_quote() {
        assert_eq!(quote("a b"), "\"a b\"");
        assert_eq!(quote(r#"a"b\c"#), r#""a\"b\\c""#);
    }

    #[test]
    fn account_name_prefers_user_env_then_os_then_fallback() {
        assert_eq!(
            account_name_from(Some("alice"), || Some("os".into())),
            "alice"
        );
        assert_eq!(account_name_from(Some(""), || Some("os".into())), "os");
        assert_eq!(account_name_from(None, || Some("os".into())), "os");
        assert_eq!(account_name_from(None, || None), FALLBACK_ACCOUNT_NAME);
        assert!(os_username().is_some_and(|n| !n.is_empty()));
    }
}

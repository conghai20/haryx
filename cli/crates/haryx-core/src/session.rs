//! Port of `claude_swap/session.py` (partial): the session-profile helpers the switcher needs.
//!
//! `haryx run NUM|EMAIL` launches Claude Code with `CLAUDE_CONFIG_DIR` pointing at a persistent per-account
//! profile under `<store>/sessions/<num>-<email-slug>/`, leaving the default login untouched. On macOS Claude
//! hashes the (NFC-normalized) env var value into its Keychain service name, so each profile gets its own
//! Keychain entry ([`crate::credentials::keychain_service_name`]).
//!
//! Ported so far: profile naming, the stale marker, credential and identity reads, and the live-session
//! scan. TODO(session): `SessionManager` (`run`, bootstrap, sharing, history migration) and the rest of
//! `session.py`.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::Value;
use unicode_normalization::UnicodeNormalization;

use crate::credentials::{KeychainBackend, keychain_service_name};
use crate::error::{Error, Result};
use crate::paths::HostOs;
use crate::process::{self, ClaudeSession};

/// Name of a profile's stale marker (see [`stale_marker_for`]).
pub const STALE_MARKER: &str = ".cswap-stale-credentials";
/// Attempts of the strict (capture) Keychain read, mirroring the active store's.
pub const STRICT_KEYCHAIN_ATTEMPTS: u32 = 2;
/// Pause between those attempts.
pub const STRICT_KEYCHAIN_RETRY_DELAY: Duration = Duration::from_millis(300);

/// How profile helpers reach the Keychain: the backend, the OS (Keychain reads are macOS-only), the item
/// account name, and the strict-read retry pause (tests shorten it).
#[derive(Clone, Copy)]
pub struct ProfileKeychain<'a> {
    pub backend: &'a dyn KeychainBackend,
    pub platform: HostOs,
    pub account: &'a str,
    pub retry_delay: Duration,
}

impl ProfileKeychain<'_> {
    fn is_macos(&self) -> bool {
        self.platform == HostOs::MacOs
    }

    /// Best-effort delete of a profile's hashed Keychain entry (upstream `delete_macos_keychain_entry`).
    pub fn delete_entry(&self, session_dir: &Path) {
        crate::credentials::delete_macos_keychain_entry(
            self.backend,
            self.platform,
            session_dir,
            self.account,
        );
    }
}

/// Where a profile's stale marker is WRITTEN: a SIBLING of the profile dir.
///
/// Not a child: the marker is the fallback for an invalidation that just failed, and the faults it exists
/// for (EACCES on the session dir, a read-only mount) are faults on that same directory. The parent is
/// ours. A sibling also keeps `purge`'s dir scan and the profile `rm -r` seeing what they expect.
pub fn stale_marker_for(session_dir: &Path) -> PathBuf {
    let name = session_dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    session_dir
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .join(format!(".{name}{STALE_MARKER}"))
}

/// Whether a profile is flagged for re-bootstrap. Both locations: the child path is where the marker used
/// to live (read, never written).
pub fn is_session_stale(session_dir: &Path) -> bool {
    stale_marker_for(session_dir).exists() || session_dir.join(STALE_MARKER).exists()
}

/// Drop a profile's stale flag from both marker locations; whether every marker is gone.
///
/// Each unlink tolerates its own failure (EACCES/EROFS as well as ENOENT), but a removal that did not happen
/// is reported: a surviving marker re-fires the next run's stale arm forever.
pub fn clear_session_stale(session_dir: &Path) -> bool {
    let mut cleared = true;
    for marker in [
        stale_marker_for(session_dir),
        session_dir.join(STALE_MARKER),
    ] {
        match std::fs::remove_file(&marker) {
            Ok(()) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => cleared = false,
        }
    }
    cleared
}

/// Flag a live session profile for re-bootstrap once it exits; whether the marker landed.
pub fn mark_session_stale(session_dir: &Path) -> bool {
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(stale_marker_for(session_dir))
        .is_ok()
}

/// Filesystem-safe slug for an email address (NFC first; anything but ASCII alphanumerics and `._-`
/// becomes `_`). Uniqueness comes from the `<num>-` prefix, so this only needs to be safe.
pub fn slugify_email(email: &str) -> String {
    email
        .nfc()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || "._-".contains(ch) {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

/// Session profile directory for an account: `<store>/sessions/<num>-<slug>`.
pub fn session_dir_for(store_root: &Path, account_num: &str, email: &str) -> PathBuf {
    store_root
        .join("sessions")
        .join(format!("{account_num}-{}", slugify_email(email)))
}

/// A profile's *current* credential JSON, best effort: the hashed Keychain entry (macOS), else the
/// profile's `.credentials.json`. `None` when nothing readable is there.
pub fn read_session_credentials(kc: &ProfileKeychain<'_>, session_dir: &Path) -> Option<String> {
    read_config_dir_credentials(kc, session_dir.as_os_str(), false, None)
        .ok()
        .flatten()
}

/// Lexical `os.path.normpath` for POSIX paths: collapse `.`, `..` and repeated separators.
fn normpath(path: &str) -> String {
    let absolute = path.starts_with('/');
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if parts.last().is_some_and(|p| *p != "..") {
                    parts.pop();
                } else if !absolute {
                    parts.push("..");
                }
            }
            other => parts.push(other),
        }
    }
    let joined = parts.join("/");
    match (absolute, joined.is_empty()) {
        (true, _) => format!("/{joined}"),
        (false, true) => ".".to_owned(),
        (false, false) => joined,
    }
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (
        crate::fsutil::canonicalize(a),
        crate::fsutil::canonicalize(b),
    ) {
        (Ok(x), Ok(y)) => x == y,
        _ => false,
    }
}

/// Keychain service names that may hold a profile's credential, in lookup order (upstream
/// `_keychain_services`).
///
/// Claude hashes the exported `CLAUDE_CONFIG_DIR` verbatim, so a profile that is a symlink was launched
/// under its own path or the target's. The profile's own name first, then the target's; a relative target
/// is tried as the lexical form (only when it names the same directory) and then the raw join. `override`
/// names the unsuffixed default-profile item and stands alone.
pub fn keychain_services(config_dir: &OsStr, override_service: Option<&str>) -> Vec<String> {
    if let Some(service) = override_service {
        return vec![service.to_owned()];
    }
    let mut services = vec![keychain_service_name(config_dir)];
    let Ok(target) = std::fs::read_link(config_dir) else {
        return services;
    };
    if target.is_absolute() {
        services.push(keychain_service_name(target.as_os_str()));
        return services;
    }
    let parent = Path::new(config_dir)
        .parent()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    let target = target.to_string_lossy().into_owned();
    let raw = if parent.is_empty() {
        target
    } else if parent.ends_with('/') {
        format!("{parent}{target}")
    } else {
        format!("{parent}/{target}")
    };
    let lexical = normpath(&raw);
    if lexical != raw && same_file(Path::new(&lexical), Path::new(&raw)) {
        services.push(keychain_service_name(OsStr::new(&lexical)));
    }
    services.push(keychain_service_name(OsStr::new(&raw)));
    services
}

fn keychain_entry(
    kc: &ProfileKeychain<'_>,
    service: &str,
    strict: bool,
) -> std::result::Result<Option<String>, crate::keychain::KeychainError> {
    let mut attempts = if strict { STRICT_KEYCHAIN_ATTEMPTS } else { 1 };
    loop {
        match kc.backend.get_password(service, kc.account) {
            Ok(value) => return Ok(value),
            Err(err) => {
                attempts -= 1;
                if attempts == 0 {
                    return Err(err);
                }
                std::thread::sleep(kc.retry_delay);
            }
        }
    }
}

/// The credential of an arbitrary `CLAUDE_CONFIG_DIR` value (raw: a path round-trip would drop a trailing
/// slash and look up a service name claude never wrote).
///
/// `keychain_service` overrides the derived hashed name for the default profile's unsuffixed item.
/// `strict_keychain` is for capture: an UNREADABLE Keychain retries once and then fails with
/// [`Error::CredentialRead`] instead of falling back to a plaintext seed that may predate an in-profile
/// `/login`. An ABSENT entry (rc 44) falls back either way. `Ok(None)`: not a directory, or no readable
/// `.credentials.json`.
pub fn read_config_dir_credentials(
    kc: &ProfileKeychain<'_>,
    config_dir: &OsStr,
    strict_keychain: bool,
    keychain_service: Option<&str>,
) -> Result<Option<String>> {
    let directory = Path::new(config_dir);
    if !directory.is_dir() {
        return Ok(None);
    }
    if kc.is_macos() {
        for service in keychain_services(config_dir, keychain_service) {
            match keychain_entry(kc, &service, strict_keychain) {
                Ok(Some(creds)) if !creds.is_empty() => return Ok(Some(creds)),
                Ok(_) => {} // absent (rc 44): claude's own signal to look further
                Err(err) if strict_keychain => {
                    return Err(Error::CredentialRead(format!(
                        "Keychain entry for profile {} is unreadable (locked or busy) — unlock the \
                         keychain and retry: {err}",
                        config_dir.to_string_lossy()
                    )));
                }
                Err(_) => break, // best effort: the plaintext seed is the next-best truth
            }
        }
    }
    Ok(std::fs::read_to_string(directory.join(".credentials.json")).ok())
}

/// The account identity a profile is logged in as: `(email, organization_uuid)` from its `.claude.json`
/// `oauthAccount` (`""` for a missing org), or `None` when unreadable or absent.
pub fn read_session_identity(session_dir: &Path) -> Option<(String, String)> {
    let text = std::fs::read_to_string(session_dir.join(".claude.json")).ok()?;
    let config: Value = serde_json::from_str(&text).ok()?;
    let account = config.as_object()?.get("oauthAccount");
    let account = match account {
        None | Some(Value::Null) => return None,
        Some(Value::Object(map)) => map,
        Some(_) => return None,
    };
    let email = account
        .get("emailAddress")
        .and_then(Value::as_str)
        .filter(|e| !e.is_empty())?;
    let org = account
        .get("organizationUuid")
        .and_then(Value::as_str)
        .unwrap_or_default();
    Some((email.to_owned(), org.to_owned()))
}

/// Whether the profile is logged in as a *different* account than its slot. The email must match; the
/// org only when both sides have one. An unreadable identity is NOT drift (a read-side policy: the
/// adopting write checks [`read_session_identity`] itself).
pub fn session_identity_drifted(session_dir: &Path, email: &str, org_uuid: &str) -> bool {
    let Some((profile_email, profile_org)) = read_session_identity(session_dir) else {
        return false;
    };
    if profile_email != email {
        return true;
    }
    !profile_org.is_empty() && !org_uuid.is_empty() && profile_org != org_uuid
}

/// Live Claude instances for a profile, and the number of records that could not be read: a record that
/// could not be read is not evidence that nothing is running.
pub fn scan_live_sessions(session_dir: &Path) -> (Vec<ClaudeSession>, usize) {
    if !session_dir.exists() {
        return (Vec::new(), 0);
    }
    process::scan_sessions(session_dir)
}

/// Nothing is running against this profile AND every record could be read.
pub fn profile_is_quiescent(session_dir: &Path) -> bool {
    let (sessions, unreadable) = scan_live_sessions(session_dir);
    sessions.is_empty() && unreadable == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_replaces_unsafe_characters() {
        assert_eq!(slugify_email("a.b+c@x.com"), "a.b_c_x.com");
        assert_eq!(slugify_email("ü@x.com"), "__x.com");
    }

    #[test]
    fn session_dir_layout() {
        let dir = session_dir_for(Path::new("/store"), "2", "u@x.com");
        assert_eq!(dir, Path::new("/store/sessions/2-u_x.com"));
        assert_eq!(
            stale_marker_for(&dir),
            Path::new("/store/sessions/.2-u_x.com.cswap-stale-credentials")
        );
    }

    #[test]
    fn normpath_matches_python() {
        assert_eq!(normpath("/x/sessions/../space"), "/x/space");
        assert_eq!(normpath("a/./b//c/.."), "a/b");
        assert_eq!(normpath("../a"), "../a");
        assert_eq!(normpath("/.."), "/");
    }

    #[test]
    fn stale_marker_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("1-a");
        std::fs::create_dir_all(&dir).unwrap();
        assert!(!is_session_stale(&dir));
        assert!(mark_session_stale(&dir));
        assert!(is_session_stale(&dir));
        assert!(clear_session_stale(&dir));
        assert!(!is_session_stale(&dir));
        // The legacy child location is read and cleared too.
        std::fs::write(dir.join(STALE_MARKER), "").unwrap();
        assert!(is_session_stale(&dir));
        assert!(clear_session_stale(&dir));
        assert!(!is_session_stale(&dir));
    }

    #[test]
    fn identity_reads_and_drift() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        assert_eq!(read_session_identity(dir), None);
        assert!(!session_identity_drifted(dir, "a@x.com", "org"));
        std::fs::write(
            dir.join(".claude.json"),
            r#"{"oauthAccount": {"emailAddress": "a@x.com", "organizationUuid": null}}"#,
        )
        .unwrap();
        assert_eq!(
            read_session_identity(dir),
            Some(("a@x.com".into(), String::new()))
        );
        assert!(!session_identity_drifted(dir, "a@x.com", "org"));
        assert!(session_identity_drifted(dir, "b@x.com", ""));
        std::fs::write(
            dir.join(".claude.json"),
            r#"{"oauthAccount": {"emailAddress": "a@x.com", "organizationUuid": "o1"}}"#,
        )
        .unwrap();
        assert!(session_identity_drifted(dir, "a@x.com", "o2"));
        assert!(!session_identity_drifted(dir, "a@x.com", ""));
        std::fs::write(dir.join(".claude.json"), "not json{").unwrap();
        assert_eq!(read_session_identity(dir), None);
    }
}

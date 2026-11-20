//! Claude Code's own Keychain naming, and which item is the active one for an environment.
//!
//! Upstream keeps [`keychain_service_name`] and [`delete_macos_keychain_entry`] in `session.py`; they live
//! here until `session.rs` is ported, because the credential store needs them first.

use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};

use sha2::{Digest, Sha256};
use unicode_normalization::UnicodeNormalization;

use super::KeychainBackend;
use crate::paths::{self, HostOs, PathEnv};

/// Service name of Claude Code's *active* OAuth credential in the macOS Keychain (read by Claude Code itself;
/// haryx reads/writes it when switching accounts). Only the DEFAULT profile's item has this exact name.
pub const CLAUDE_CODE_KEYCHAIN_SERVICE: &str = "Claude Code-credentials";

/// Service name of Claude Code's *active* managed API key (`/login` with an `sk-ant-…` key) in the macOS
/// Keychain. Distinct from the OAuth service above (no `-credentials` suffix); Claude Code resolves it on a
/// separate auth axis (`getApiKeyFromConfigOrMacOSKeychain`). Off macOS the managed key lives in
/// `~/.claude.json` as `primaryApiKey`.
pub const CLAUDE_CODE_MANAGED_KEYCHAIN_SERVICE: &str = "Claude Code";

/// Keychain service name Claude Code derives for a config dir.
///
/// Claude hashes the raw `CLAUDE_CONFIG_DIR` env var value, NFC-normalized and unresolved (claude src
/// `envUtils.ts`/`macOsKeychainHelpers.ts`). Hash exactly the string exported, never a resolved variant: a
/// trailing slash or `./` changes the name.
///
/// Deliberate gap: no NFC normalization (no Unicode-normalization crate in the workspace). ASCII and
/// already-NFC values, which is what a shell hands over, hash identically; a decomposed (NFD) value would
/// name a different item than Claude Code's.
pub fn keychain_service_name(config_dir: &OsStr) -> String {
    // Claude Code hashes the NFC-normalized value (macOS file names often arrive decomposed). A value that is
    // not valid UTF-8 cannot be normalized; hash its raw bytes.
    let bytes = match config_dir.to_str() {
        Some(text) => text.nfc().collect::<String>().into_bytes(),
        None => os_bytes(config_dir),
    };
    let digest = Sha256::digest(bytes);
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    format!("{CLAUDE_CODE_KEYCHAIN_SERVICE}-{}", &hex[..8])
}

#[cfg(unix)]
fn os_bytes(value: &OsStr) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    value.as_bytes().to_vec()
}

#[cfg(not(unix))]
fn os_bytes(value: &OsStr) -> Vec<u8> {
    value.to_string_lossy().into_owned().into_bytes()
}

/// Best-effort delete of a session profile's hashed Keychain entry (upstream `session.delete_macos_keychain_entry`).
///
/// No-op off macOS. Needed before seeding (Claude reads the Keychain before the plaintext file, so a stale
/// entry would shadow a fresh seed) and on profile removal (once the dir is gone the hashed name is
/// unrecoverable). An absent entry is already success (rc 44); any failure is ignored.
pub fn delete_macos_keychain_entry(
    keychain: &dyn KeychainBackend,
    platform: HostOs,
    session_dir: &Path,
    account: &str,
) {
    if platform != HostOs::MacOs {
        return;
    }
    let service = keychain_service_name(session_dir.as_os_str());
    if let Err(err) = keychain.delete_password(&service, account) {
        log::debug!("best-effort delete of a session Keychain entry failed: {err}");
    }
}

/// Whether the active config home is the default profile's.
///
/// `CLAUDE_CONFIG_DIR` pointed at the default profile is still the default profile, so this keys on where
/// the path resolves rather than on the variable being set. Resolution also collapses symlinks, which is how
/// a profile reached through one shows up as itself.
pub fn active_profile_is_default(env: &PathEnv) -> bool {
    resolve_lenient(&paths::claude_config_home_in(env))
        == resolve_lenient(&paths::default_claude_config_home_in(env))
}

/// Python's non-strict `Path.resolve()`: walk the components, resolving symlinks in every prefix that
/// exists and applying `..` to the resolved prefix; a missing tail is kept as written.
fn resolve_lenient(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => {
                out.push(other);
                if let Ok(canonical) = std::fs::canonicalize(&out) {
                    out = canonical;
                }
            }
        }
    }
    out
}

/// Keychain services holding the OAuth credential for the active environment, in try-order.
///
/// Claude (2.1.220 `getMacOsKeychainStorageServiceName`) sources secure storage from
/// `CLAUDE_SECURESTORAGE_CONFIG_DIR` when that is *defined*, else `CLAUDE_CONFIG_DIR`; defined-but-empty
/// selects the *default* secure store, whose item is the unsuffixed one.
///
/// More than one name only for an explicit `CLAUDE_CONFIG_DIR` naming the default profile: Claude hashes
/// the exported string, so it writes a *suffixed* item there, but a user who has always used the default
/// profile may only have the unsuffixed one. A defined `CLAUDE_SECURESTORAGE_CONFIG_DIR` gets no fallback:
/// it names the only store claude will read for this environment.
///
/// The first name is the item claude reads for this environment, and the only one haryx deletes. A second
/// name is a read-only fallback.
pub fn active_oauth_keychain_services(env: &PathEnv) -> Vec<String> {
    if let Some(secure) = &env.secure_storage_dir {
        if secure.is_empty() {
            return vec![CLAUDE_CODE_KEYCHAIN_SERVICE.to_string()];
        }
        return vec![keychain_service_name(secure)];
    }
    let Some(config_dir) = env.claude_config_dir.as_ref().filter(|d| !d.is_empty()) else {
        return vec![CLAUDE_CODE_KEYCHAIN_SERVICE.to_string()];
    };
    let mut services = vec![keychain_service_name(config_dir)];
    if active_profile_is_default(env) {
        services.push(CLAUDE_CODE_KEYCHAIN_SERVICE.to_string());
    }
    services
}

/// Whether the unsuffixed Keychain item IS this environment's OAuth store.
///
/// A write may only touch [`CLAUDE_CODE_KEYCHAIN_SERVICE`] when claude resolves that same item for this
/// environment. Under any other profile that item holds a login haryx was not asked to touch, and the
/// switch's outgoing backup reads the ACTIVE profile, so overwriting it destroys a refresh token that has
/// no copy anywhere (upstream #206). Keyed on the FIRST resolved name, not on membership.
pub fn active_store_is_the_default_secure_store(env: &PathEnv) -> bool {
    active_oauth_keychain_services(env)
        .first()
        .is_some_and(|s| s == CLAUDE_CODE_KEYCHAIN_SERVICE)
}

#[cfg(test)]
mod tests {

    #[test]
    fn service_name_is_nfc_normalized() {
        let composed = keychain_service_name(OsStr::new("/Users/caf\u{e9}/.claude-x"));
        let decomposed = keychain_service_name(OsStr::new("/Users/cafe\u{301}/.claude-x"));
        assert_eq!(composed, decomposed);
        // Python: hashlib.sha256("/Users/café/.claude-x".encode()).hexdigest()[:8]
        assert_eq!(composed, "Claude Code-credentials-29f5bd81");
    }
    use super::*;
    use std::ffi::OsString;
    use tempfile::TempDir;

    #[test]
    fn service_name_is_sha256_prefix_of_the_raw_value() {
        // sha256("/tmp/x") = 9b1b2d0b...; check shape and stability rather than a hand-computed digest.
        let a = keychain_service_name(OsStr::new("/tmp/x"));
        let b = keychain_service_name(OsStr::new("/tmp/x/"));
        assert!(a.starts_with("Claude Code-credentials-"));
        assert_eq!(a.len(), "Claude Code-credentials-".len() + 8);
        assert_ne!(a, b, "the raw string is hashed, unresolved");
        let digest = Sha256::digest(b"/tmp/x");
        assert_eq!(
            &a[24..],
            &format!(
                "{:02x}{:02x}{:02x}{:02x}",
                digest[0], digest[1], digest[2], digest[3]
            )
        );
    }

    #[test]
    fn services_follow_secure_storage_then_config_dir() {
        let tmp = TempDir::new().unwrap();
        let mut env = PathEnv::with_home(tmp.path());
        assert_eq!(
            active_oauth_keychain_services(&env),
            vec![CLAUDE_CODE_KEYCHAIN_SERVICE]
        );
        env.claude_config_dir = Some(OsString::from("/elsewhere"));
        assert_eq!(
            active_oauth_keychain_services(&env),
            vec![keychain_service_name(OsStr::new("/elsewhere"))]
        );
        env.secure_storage_dir = Some(OsString::new());
        assert_eq!(
            active_oauth_keychain_services(&env),
            vec![CLAUDE_CODE_KEYCHAIN_SERVICE]
        );
        env.secure_storage_dir = Some(OsString::from("/secure"));
        assert_eq!(
            active_oauth_keychain_services(&env),
            vec![keychain_service_name(OsStr::new("/secure"))]
        );
    }

    #[test]
    fn explicit_default_profile_gets_the_unsuffixed_fallback() {
        let tmp = TempDir::new().unwrap();
        let default = tmp.path().join(".claude");
        std::fs::create_dir_all(&default).unwrap();
        let mut env = PathEnv::with_home(tmp.path());
        env.claude_config_dir = Some(default.clone().into_os_string());
        assert!(active_profile_is_default(&env));
        assert_eq!(
            active_oauth_keychain_services(&env),
            vec![
                keychain_service_name(default.as_os_str()),
                CLAUDE_CODE_KEYCHAIN_SERVICE.to_string()
            ]
        );
        assert!(!active_store_is_the_default_secure_store(&env));
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_to_the_default_profile_is_the_default_profile() {
        let tmp = TempDir::new().unwrap();
        std::fs::create_dir_all(tmp.path().join(".claude")).unwrap();
        std::os::unix::fs::symlink(tmp.path().join(".claude"), tmp.path().join("link")).unwrap();
        let mut env = PathEnv::with_home(tmp.path());
        env.claude_config_dir = Some(tmp.path().join("link").into_os_string());
        assert!(active_profile_is_default(&env));
        env.claude_config_dir = Some(tmp.path().join("missing/../.claude").into_os_string());
        assert!(active_profile_is_default(&env));
    }
}

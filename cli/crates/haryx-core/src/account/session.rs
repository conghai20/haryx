//! The HaryX session on this machine: where it is kept ([`Vault`], name `session`) and how it stays
//! fresh.
//!
//! Renewal happens on use, under `<store>/account/session.lock`, re-reading after the lock so two
//! processes never spend the same refresh token. Supabase tolerates a reuse inside 10 s anyway
//! (spike), so the lock is for renewals further apart.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::AccountError;
use super::client::Backend;
use super::vault::{Vault, storage};
use crate::keychain::{SECURITY_STDIN_LINE_LIMIT, Security};
use crate::locking::FileLock;

const NAME: &str = "session";
const LOCK_NAME: &str = "session.lock";
/// Renew when less than this is left on the access token.
const RENEW_MARGIN_S: i64 = 60;
/// Above this, the access token is not stored, so `security -i` (its line limit, with the value
/// hex-encoded at twice its size, after the ~60-byte command) never falls back to putting the value
/// in argv. The next use renews instead. A real GitHub session measured 1,359 bytes (spike).
const MAX_KEYCHAIN_JSON: usize = (SECURITY_STDIN_LINE_LIMIT - 96) / 2;
const LOCK_TIMEOUT: Duration = Duration::from_secs(30);

/// Who is signed in. Never carries a token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct User {
    pub id: String,
    pub email: String,
    /// `github` or `google`.
    pub provider: String,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    pub access_token: String,
    pub refresh_token: String,
    /// Unix seconds when the access token expires.
    pub expires_at: i64,
    pub user: User,
    /// The Supabase project URL that issued it. A session is only ever sent back to that project.
    #[serde(default)]
    pub project: String,
}

/// Tokens stay out of debug output and logs.
impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("expires_at", &self.expires_at)
            .field("user", &self.user)
            .field("project", &self.project)
            .finish_non_exhaustive()
    }
}

impl Session {
    fn is_fresh(&self, now: i64) -> bool {
        !self.access_token.is_empty() && self.expires_at - now > RENEW_MARGIN_S
    }

    /// The form small enough for the Keychain.
    fn compact(&self) -> Session {
        Session {
            access_token: String::new(),
            expires_at: 0,
            ..self.clone()
        }
    }
}

#[derive(Debug, Clone)]
pub struct SessionStore {
    vault: Vault,
}

impl SessionStore {
    /// The store for this OS, under the haryx store root.
    pub fn for_store(store_root: &Path) -> Self {
        SessionStore {
            vault: Vault::for_store(store_root),
        }
    }

    pub fn file(dir: PathBuf) -> Self {
        SessionStore {
            vault: Vault::file(dir),
        }
    }

    pub fn keychain(dir: PathBuf, security: Security) -> Self {
        SessionStore {
            vault: Vault::keychain(dir, security),
        }
    }

    /// Where the session lives; the sync key lives beside it.
    pub fn vault(&self) -> &Vault {
        &self.vault
    }

    pub fn load(&self) -> Result<Option<Session>, AccountError> {
        // A value this build cannot read is treated as signed out, not as a hard failure.
        Ok(self
            .vault
            .read(NAME)?
            .and_then(|t| serde_json::from_str(&t).ok()))
    }

    pub fn save(&self, session: &Session) -> Result<(), AccountError> {
        let full = serde_json::to_string(session).map_err(storage)?;
        let text = if self.vault.is_keychain() && full.len() > MAX_KEYCHAIN_JSON {
            serde_json::to_string(&session.compact()).map_err(storage)?
        } else {
            full
        };
        self.vault.write(NAME, &text)
    }

    /// Forget the session. Already gone counts as done.
    pub fn clear(&self) -> Result<(), AccountError> {
        self.vault.remove(NAME)
    }

    pub fn lock(&self) -> Result<FileLock, AccountError> {
        FileLock::lock(self.vault.dir().join(LOCK_NAME), LOCK_TIMEOUT).map_err(storage)
    }

    /// The session with an access token good for at least a minute, renewing it if needed.
    ///
    /// `Ok(None)`: not signed in to `backend`'s project (a session from another project is left
    /// alone and never sent there). A refused renewal (revoked, or the refresh token was spent
    /// elsewhere) forgets the session and is [`AccountError::SignedOut`]. A network failure keeps it.
    pub fn current(&self, backend: &Backend, now: i64) -> Result<Option<Session>, AccountError> {
        let ours = |s: &Session| s.project == backend.base();
        match self.load()? {
            Some(session) if !ours(&session) => return Ok(None),
            Some(session) if session.is_fresh(now) => return Ok(Some(session)),
            None => return Ok(None),
            Some(_) => {}
        }
        let _lock = self.lock()?;
        // Another process may have renewed while this one waited.
        let Some(session) = self.load()?.filter(ours) else {
            return Ok(None);
        };
        if session.is_fresh(now) {
            return Ok(Some(session));
        }
        match backend.refresh(&session.refresh_token) {
            Ok(renewed) => {
                // The old refresh token is spent: losing the new one would sign out on next use.
                if let Err(first) = self.save(&renewed) {
                    log::warn!("saving the renewed HaryX session failed, retrying: {first}");
                    self.save(&renewed)?;
                }
                Ok(Some(renewed))
            }
            Err(AccountError::Refused(reason)) => {
                self.clear()?;
                Err(AccountError::SignedOut(reason))
            }
            Err(other) => Err(other),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oauth::tests::stub::{Reply, Stub, closed_base};

    const NOW: i64 = 1_800_000_000;

    fn session(access: &str, refresh: &str, expires_at: i64) -> Session {
        on("", access, refresh, expires_at)
    }

    /// A session issued by `backend`'s project.
    fn on(project: &str, access: &str, refresh: &str, expires_at: i64) -> Session {
        Session {
            access_token: access.into(),
            refresh_token: refresh.into(),
            expires_at,
            user: User {
                id: "u-1".into(),
                email: "you@example.com".into(),
                provider: "github".into(),
            },
            project: project.into(),
        }
    }

    fn renewed_reply() -> Reply {
        Reply::json(
            200,
            r#"{"access_token":"at-2","refresh_token":"rt-2","expires_at":1800003600,
               "user":{"id":"u-1","email":"you@example.com","app_metadata":{"provider":"github"}}}"#,
        )
    }

    fn store() -> (tempfile::TempDir, SessionStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::file(dir.path().join("account"));
        (dir, store)
    }

    #[test]
    fn session_round_trips_through_the_vault() {
        let (_dir, store) = store();
        assert_eq!(store.load().unwrap(), None);
        let s = session("at", "rt", NOW + 3600);
        store.save(&s).unwrap();
        assert_eq!(store.load().unwrap(), Some(s));
        store.clear().unwrap();
        store.clear().unwrap();
        assert_eq!(store.load().unwrap(), None);
    }

    #[test]
    fn an_unreadable_value_counts_as_signed_out() {
        let (_dir, store) = store();
        store.vault().write(NAME, "not json").unwrap();
        assert_eq!(store.load().unwrap(), None);
    }

    #[test]
    fn a_fresh_session_is_used_without_a_request() {
        let (_dir, store) = store();
        let backend = Backend::new(&closed_base(), "k");
        let s = on(backend.base(), "at", "rt", NOW + 600);
        store.save(&s).unwrap();
        assert_eq!(store.current(&backend, NOW).unwrap(), Some(s));
    }

    #[test]
    fn a_session_from_another_project_is_never_sent_there() {
        let (_dir, store) = store();
        let stub = Stub::start(|_| renewed_reply());
        store
            .save(&on("https://prod.example", "at", "rt", NOW - 10))
            .unwrap();
        let other = Backend::new(stub.base(), "k");
        assert_eq!(store.current(&other, NOW).unwrap(), None);
        assert!(stub.requests().is_empty());
        assert!(store.load().unwrap().is_some(), "left alone, not cleared");
    }

    #[test]
    fn a_session_about_to_expire_is_renewed_and_saved() {
        let (_dir, store) = store();
        let stub = Stub::start(|_| renewed_reply());
        let backend = Backend::new(stub.base(), "k");
        store
            .save(&on(backend.base(), "at-1", "rt-1", NOW + 30))
            .unwrap();
        let renewed = store.current(&backend, NOW).unwrap().unwrap();
        assert_eq!(renewed.access_token, "at-2");
        assert_eq!(store.load().unwrap().unwrap().refresh_token, "rt-2");
        assert!(stub.requests()[0].body.contains("rt-1"));
    }

    #[test]
    fn a_compact_session_from_the_keychain_is_renewed() {
        let (_dir, store) = store();
        let stub = Stub::start(|_| renewed_reply());
        let backend = Backend::new(stub.base(), "k");
        store
            .save(&on(backend.base(), "at", "rt-1", NOW + 3600).compact())
            .unwrap();
        assert_eq!(
            store.current(&backend, NOW).unwrap().unwrap().access_token,
            "at-2"
        );
    }

    #[test]
    fn a_refused_renewal_signs_out() {
        let (_dir, store) = store();
        let stub =
            Stub::start(|_| Reply::json(400, r#"{"msg":"Invalid Refresh Token: Already Used"}"#));
        let backend = Backend::new(stub.base(), "k");
        store
            .save(&on(backend.base(), "at", "rt-dead", NOW - 10))
            .unwrap();
        assert_eq!(
            store.current(&backend, NOW).unwrap_err(),
            AccountError::SignedOut("Invalid Refresh Token: Already Used".into())
        );
        assert_eq!(store.load().unwrap(), None);
    }

    #[test]
    fn a_network_failure_keeps_the_session() {
        let (_dir, store) = store();
        let backend = Backend::new(&closed_base(), "k");
        let s = on(backend.base(), "at", "rt", NOW - 10);
        store.save(&s).unwrap();
        assert!(matches!(
            store.current(&backend, NOW).unwrap_err(),
            AccountError::Network(_)
        ));
        assert_eq!(store.load().unwrap(), Some(s));
    }

    #[test]
    fn two_processes_renewing_spend_the_refresh_token_once() {
        let (_dir, store) = store();
        let stub = Stub::start(|_| renewed_reply());
        let backend = Backend::new(stub.base(), "k");
        store
            .save(&on(backend.base(), "at-1", "rt-1", NOW - 10))
            .unwrap();
        let threads: Vec<_> = (0..4)
            .map(|_| {
                let (store, backend) = (store.clone(), backend.clone());
                std::thread::spawn(move || store.current(&backend, NOW).unwrap().unwrap())
            })
            .collect();
        for t in threads {
            assert_eq!(t.join().unwrap().access_token, "at-2");
        }
        assert_eq!(stub.requests().len(), 1);
    }

    #[test]
    fn compact_drops_only_the_access_token() {
        let full = session(&"x".repeat(1500), "rt", NOW + 3600);
        let compact = full.compact();
        assert!(compact.access_token.is_empty() && compact.expires_at == 0);
        assert!(
            !compact.is_fresh(NOW),
            "a compact session renews on first use"
        );
        assert_eq!(
            (compact.refresh_token, compact.user),
            ("rt".into(), full.user)
        );
    }

    #[test]
    fn debug_output_never_shows_a_token() {
        let shown = format!("{:?}", session("secret-access", "secret-refresh", NOW));
        assert!(!shown.contains("secret"), "{shown}");
        assert!(shown.contains("you@example.com"));
    }
}

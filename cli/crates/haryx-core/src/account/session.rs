//! The HaryX session on this machine: where it is kept ([`Vault`], name `session`) and how it stays
//! fresh.
//!
//! Renewal happens on use, under `<store>/account/session.lock`, re-reading after the lock so two
//! processes never spend the same refresh token (Keycloak rotates it on each renewal).

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
/// in argv. The next use renews instead. A Keycloak session measured 2,396 bytes in full and 1,013
/// compact (plan Phase 0, item 2), so on macOS the access token is in practice never stored.
const MAX_KEYCHAIN_JSON: usize = (SECURITY_STDIN_LINE_LIMIT - 96) / 2;
const LOCK_TIMEOUT: Duration = Duration::from_secs(30);

/// Who is signed in. Never carries a token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct User {
    pub id: String,
    pub email: String,
    /// `github`, `google`, `apple` or `email` (Keycloak's `identity_provider` claim, absent for a
    /// password sign-in).
    pub provider: String,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    pub access_token: String,
    pub refresh_token: String,
    /// Unix seconds when the access token expires.
    pub expires_at: i64,
    pub user: User,
    /// The SSO issuer that signed it in. A session is only ever sent back to that issuer.
    pub issuer: String,
    /// Keycloak's session id (the access token's `sid`): this machine's sign-in, as the web's
    /// machine list and remote sign-out name it.
    #[serde(default)]
    pub sid: String,
}

/// Tokens stay out of debug output and logs.
impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("expires_at", &self.expires_at)
            .field("user", &self.user)
            .field("issuer", &self.issuer)
            .field("sid", &self.sid)
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

/// The Supabase-era session: `project` (its URL) where an SSO session has `issuer`.
fn is_supabase_session(text: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(text)
        .is_ok_and(|v| v.get("project").is_some() && v.get("issuer").is_none())
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

    /// The stored session. A value this build cannot read is treated as signed out, not as a hard
    /// failure. A session from the Supabase sign-in this replaced is [`AccountError::SignedOut`],
    /// with what to do; its tokens are never migrated. It stays until the next login or logout, so
    /// every command says it, not only the first.
    pub fn load(&self) -> Result<Option<Session>, AccountError> {
        let Some(text) = self.vault.read(NAME)? else {
            return Ok(None);
        };
        if is_supabase_session(&text) {
            return Err(AccountError::SignedOut(format!(
                "HaryX now signs in with a WeCoAp ID; run '{} login' to sign in again",
                crate::PROG
            )));
        }
        Ok(serde_json::from_str(&text).ok())
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
    /// `Ok(None)`: not signed in to `backend`'s issuer (a session from another issuer is left
    /// alone and never sent there). A refused renewal (revoked, or the refresh token was spent
    /// elsewhere) forgets the session and is [`AccountError::SignedOut`]. A network failure keeps it.
    pub fn current(&self, backend: &Backend, now: i64) -> Result<Option<Session>, AccountError> {
        self.usable(backend, now, "")
    }

    /// The HaryX API refused `rejected`, an access token [`SessionStore::current`] handed out (it
    /// may have been revoked before it expired): a session with another one. That is the stored
    /// session when another process already renewed it, else a renewal, with the same outcomes as
    /// [`SessionStore::current`].
    pub fn renew(
        &self,
        backend: &Backend,
        rejected: &str,
        now: i64,
    ) -> Result<Option<Session>, AccountError> {
        self.usable(backend, now, rejected)
    }

    fn usable(
        &self,
        backend: &Backend,
        now: i64,
        rejected: &str,
    ) -> Result<Option<Session>, AccountError> {
        let ours = |s: &Session| s.issuer == backend.issuer();
        let good = |s: &Session| s.is_fresh(now) && s.access_token != rejected;
        match self.load()? {
            Some(session) if !ours(&session) => return Ok(None),
            Some(session) if good(&session) => return Ok(Some(session)),
            None => return Ok(None),
            Some(_) => {}
        }
        let _lock = self.lock()?;
        // Another process may have renewed while this one waited.
        let Some(session) = self.load()?.filter(ours) else {
            return Ok(None);
        };
        if good(&session) {
            return Ok(Some(session));
        }
        match backend.refresh(&session.refresh_token) {
            Ok(renewed) => {
                // Keep the method this machine signed in with, should a renewal not carry it.
                let renewed = Session {
                    user: User {
                        provider: session.user.provider,
                        ..renewed.user
                    },
                    ..renewed
                };
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
    use super::super::client::fake::{self, TOKEN, at};
    use super::*;
    use crate::oauth::tests::stub::{Reply, Stub, closed_base};

    const NOW: i64 = 1_800_000_000;

    fn session(access: &str, refresh: &str, expires_at: i64) -> Session {
        on("", access, refresh, expires_at)
    }

    /// A session signed in by `issuer`.
    fn on(issuer: &str, access: &str, refresh: &str, expires_at: i64) -> Session {
        Session {
            access_token: access.into(),
            refresh_token: refresh.into(),
            expires_at,
            user: User {
                id: "u-1".into(),
                email: "you@example.com".into(),
                provider: "github".into(),
            },
            issuer: issuer.into(),
            sid: "sid-1".into(),
        }
    }

    /// A fake Keycloak whose renewals answer `rt-2`, as a password session (no provider claim).
    fn renewing() -> (Stub, Backend) {
        let stub = fake::keycloak(|_| Reply::json(200, &fake::tokens(None, "rt-2")));
        let backend = fake::backend(&stub);
        (stub, backend)
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
    fn a_supabase_session_is_signed_out_with_what_to_do_and_never_sent() {
        let (_dir, store) = store();
        let (stub, backend) = renewing();
        let old = r#"{"access_token":"at","refresh_token":"rt","expires_at":1,
            "user":{"id":"u","email":"you@example.com","provider":"github"},
            "project":"https://ygvcznwlugaxqqmlsssh.supabase.co"}"#;
        store.vault().write(NAME, old).unwrap();
        let AccountError::SignedOut(why) = store.load().unwrap_err() else {
            panic!("not signed out");
        };
        assert!(
            why.contains("WeCoAp ID") && why.contains("haryx login"),
            "{why}"
        );
        assert!(matches!(
            store.current(&backend, NOW).unwrap_err(),
            AccountError::SignedOut(_)
        ));
        assert!(stub.requests().is_empty(), "its tokens go nowhere");
    }

    #[test]
    fn a_fresh_session_is_used_without_a_request() {
        let (_dir, store) = store();
        let backend = Backend::new(&closed_base(), "api");
        let s = on(backend.issuer(), "at", "rt", NOW + 600);
        store.save(&s).unwrap();
        assert_eq!(store.current(&backend, NOW).unwrap(), Some(s));
    }

    #[test]
    fn a_session_from_another_issuer_is_never_sent_there() {
        let (_dir, store) = store();
        let (stub, other) = renewing();
        store
            .save(&on(
                "https://sso.wecoap.com/realms/wecoap",
                "at",
                "rt",
                NOW - 10,
            ))
            .unwrap();
        assert_eq!(store.current(&other, NOW).unwrap(), None);
        assert!(stub.requests().is_empty());
        assert!(store.load().unwrap().is_some(), "left alone, not cleared");
    }

    #[test]
    fn a_session_about_to_expire_is_renewed_and_saved() {
        let (_dir, store) = store();
        let (stub, backend) = renewing();
        store
            .save(&on(backend.issuer(), "at-1", "rt-1", NOW + 30))
            .unwrap();
        let renewed = store.current(&backend, NOW).unwrap().unwrap();
        assert_ne!(renewed.access_token, "at-1");
        assert_eq!(store.load().unwrap().unwrap().refresh_token, "rt-2");
        assert!(at(&stub, TOKEN)[0].body.contains("refresh_token=rt-1"));
    }

    #[test]
    fn a_renewal_keeps_the_provider_this_machine_signed_in_with() {
        let (_dir, store) = store();
        let (_stub, backend) = renewing(); // its tokens carry no provider claim
        store
            .save(&on(backend.issuer(), "at-1", "rt-1", NOW + 30))
            .unwrap();
        assert_eq!(
            store.current(&backend, NOW).unwrap().unwrap().user.provider,
            "github"
        );
        assert_eq!(store.load().unwrap().unwrap().user.provider, "github");
    }

    #[test]
    fn a_compact_session_from_the_keychain_is_renewed() {
        let (_dir, store) = store();
        let (_stub, backend) = renewing();
        store
            .save(&on(backend.issuer(), "at", "rt-1", NOW + 3600).compact())
            .unwrap();
        let renewed = store.current(&backend, NOW).unwrap().unwrap();
        assert!(!renewed.access_token.is_empty());
        assert_eq!(renewed.refresh_token, "rt-2");
    }

    #[test]
    fn a_rejected_access_token_is_renewed_even_while_it_looks_fresh() {
        let (_dir, store) = store();
        let (stub, backend) = renewing();
        // Real time: a renewal sets its expiry from the clock.
        let now = jiff::Timestamp::now().as_second();
        store
            .save(&on(backend.issuer(), "at-1", "rt-1", now + 3600))
            .unwrap();
        let renewed = store.renew(&backend, "at-1", now).unwrap().unwrap();
        assert_ne!(renewed.access_token, "at-1");
        assert_eq!(at(&stub, TOKEN).len(), 1);
        // A token other than the rejected one is used as it is.
        let again = store.renew(&backend, "at-1", now).unwrap().unwrap();
        assert_eq!(again.access_token, renewed.access_token);
        assert_eq!(at(&stub, TOKEN).len(), 1);
    }

    #[test]
    fn an_invalid_grant_renewal_signs_out() {
        let (_dir, store) = store();
        let stub = fake::keycloak(|_| {
            Reply::json(
                400,
                r#"{"error":"invalid_grant","error_description":"Offline user session not found"}"#,
            )
        });
        let backend = fake::backend(&stub);
        store
            .save(&on(backend.issuer(), "at", "rt-dead", NOW - 10))
            .unwrap();
        assert_eq!(
            store.current(&backend, NOW).unwrap_err(),
            AccountError::SignedOut("Offline user session not found".into())
        );
        assert_eq!(store.load().unwrap(), None);
    }

    #[test]
    fn a_failed_renewal_of_any_other_kind_keeps_the_session() {
        let replies = [
            Reply::json(503, r#"{"error":"invalid_grant"}"#),
            Reply::json(429, r#"{"error":"invalid_grant"}"#),
            Reply::json(400, r#"{"error":"invalid_request"}"#),
            Reply::json(200, "garbage"),
        ];
        for reply in replies {
            let (_dir, store) = store();
            let stub = fake::keycloak(move |_| reply.clone());
            let backend = fake::backend(&stub);
            let s = on(backend.issuer(), "at", "rt", NOW - 10);
            store.save(&s).unwrap();
            assert!(matches!(
                store.current(&backend, NOW).unwrap_err(),
                AccountError::Network(_)
            ));
            assert_eq!(store.load().unwrap(), Some(s));
        }
        let (_dir, store) = store();
        let backend = Backend::new(&closed_base(), "api");
        store
            .save(&on(backend.issuer(), "at", "rt", NOW - 10))
            .unwrap();
        assert!(matches!(
            store.current(&backend, NOW).unwrap_err(),
            AccountError::Network(_)
        ));
        assert!(store.load().unwrap().is_some());
    }

    #[test]
    fn two_processes_renewing_spend_the_refresh_token_once() {
        let (_dir, store) = store();
        let (stub, backend) = renewing();
        // The real clock: a renewal's expiry is `expires_in` from now, and the later threads must
        // find it fresh.
        let now = jiff::Timestamp::now().as_second();
        store
            .save(&on(backend.issuer(), "at-1", "rt-1", now - 10))
            .unwrap();
        let threads: Vec<_> = (0..4)
            .map(|_| {
                let (store, backend) = (store.clone(), backend.clone());
                std::thread::spawn(move || store.current(&backend, now).unwrap().unwrap())
            })
            .collect();
        for t in threads {
            assert_eq!(t.join().unwrap().refresh_token, "rt-2");
        }
        assert_eq!(at(&stub, TOKEN).len(), 1);
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
            (
                compact.refresh_token,
                compact.user,
                compact.issuer,
                compact.sid
            ),
            ("rt".into(), full.user, full.issuer, full.sid)
        );
    }

    #[test]
    fn a_keycloak_session_fits_the_keychain_only_compact() {
        // Phase 0 sizes: access token 1,374 bytes, offline refresh token 730, a 63-character email.
        let email = format!("{}@example.com", "a".repeat(51));
        assert_eq!(email.len(), 63);
        let s = Session {
            access_token: "a".repeat(1374),
            refresh_token: "r".repeat(730),
            expires_at: NOW,
            user: User {
                id: "3f0c9b1e-7d2a-4c55-9e41-0b6f2d8a1c77".into(),
                email,
                provider: "github".into(),
            },
            issuer: crate::account::client::SSO_ISSUER.into(),
            sid: "9b3c1e2a-5d4f-4a6b-8c7d-0e1f2a3b4c5d".into(),
        };
        let full = serde_json::to_string(&s).unwrap().len();
        let compact = serde_json::to_string(&s.compact()).unwrap().len();
        assert!(full > MAX_KEYCHAIN_JSON, "{full}");
        assert!(
            compact <= MAX_KEYCHAIN_JSON,
            "{compact} > {MAX_KEYCHAIN_JSON}"
        );
        eprintln!("full {full} bytes, compact {compact}, limit {MAX_KEYCHAIN_JSON}");
    }

    #[test]
    fn debug_output_never_shows_a_token() {
        let shown = format!("{:?}", session("secret-access", "secret-refresh", NOW));
        assert!(!shown.contains("secret"), "{shown}");
        assert!(shown.contains("you@example.com"));
    }
}

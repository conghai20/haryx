//! The HaryX account: signing this machine in to HaryX itself (not a Claude account).
//!
//! Sign-in is a WeCoAp ID through WeCoAp SSO (Keycloak, OpenID Connect), as a native app does it
//! (RFC 8252): PKCE, the browser, and a one-shot listener on `127.0.0.1`. Without a browser here
//! (`--no-browser`, over SSH) it is the device authorization grant (RFC 8628) instead. haryx keeps
//! only the tokens and who signed in. Plan: `docs/sso-platform-plan.md`.

pub mod browser;
pub mod client;
mod loopback;
mod page;
mod pkce;
pub mod session;
pub mod vault;

use std::time::{Duration, Instant};

pub use client::{Backend, DeviceCode};
pub use session::{Session, SessionStore, User};
pub use vault::Vault;

use client::DevicePoll;
use loopback::{Callback, Loopback};
use pkce::Pkce;

/// How long `login` waits for the browser to come back.
pub const LOGIN_TIMEOUT: Duration = Duration::from_secs(5 * 60);
/// The fewest seconds between device polls, whatever the server says.
const MIN_POLL: Duration = Duration::from_secs(1);
/// RFC 8628 §3.5: a `slow_down` adds five seconds to the interval.
const SLOW_DOWN: Duration = Duration::from_secs(5);

/// A way to sign in to a WeCoAp ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    Github,
    Google,
    Apple,
    /// Email and password, on the WeCoAp page itself.
    Email,
}

impl Provider {
    /// The `--provider` value, and Keycloak's `identity_provider` claim.
    pub fn id(self) -> &'static str {
        match self {
            Provider::Github => "github",
            Provider::Google => "google",
            Provider::Apple => "apple",
            Provider::Email => "email",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Provider::Github => "GitHub",
            Provider::Google => "Google",
            Provider::Apple => "Apple",
            Provider::Email => "email",
        }
    }

    /// Keycloak's `kc_idp_hint`, which skips the WeCoAp page. Email has none: it is that page.
    pub fn hint(self) -> Option<&'static str> {
        match self {
            Provider::Email => None,
            other => Some(other.id()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AccountError {
    #[error("No sign-in arrived within {}.", wait_text(*.0))]
    Timeout(Duration),
    #[error("Sign-in was not completed: {0}")]
    Denied(String),
    #[error("The sign-in server refused: {0}")]
    Refused(String),
    #[error("Could not reach the sign-in server: {0}")]
    Network(String),
    #[error("Could not store the HaryX sign-in: {0}")]
    Storage(String),
    #[error("Signed out of HaryX ({0}).")]
    SignedOut(String),
}

impl AccountError {
    /// The `code` of a `--json` error event (`packages/contracts/haryx/login-events.json`).
    pub fn code(&self) -> &'static str {
        match self {
            AccountError::Timeout(_) => "timeout",
            AccountError::Denied(_) => "denied",
            AccountError::Refused(_) => "refused",
            AccountError::Network(_) => "network",
            AccountError::Storage(_) => "storage",
            AccountError::SignedOut(_) => "signed_out",
        }
    }
}

fn wait_text(wait: Duration) -> String {
    match wait.as_secs() {
        s if s >= 60 && s % 60 == 0 => format!("{} minutes", s / 60),
        s => format!("{s} seconds"),
    }
}

/// Sign this machine in through the browser and keep the session. `provider` goes straight to
/// GitHub, Google or Apple; `None` (or email) shows the WeCoAp page with every method.
///
/// `open` gets the sign-in URL once the local listener is up; it shows or opens it.
pub fn login(
    backend: &Backend,
    store: &SessionStore,
    provider: Option<Provider>,
    timeout: Duration,
    open: impl FnOnce(&str),
) -> Result<User, AccountError> {
    let loopback = Loopback::bind().map_err(|e| AccountError::Network(e.to_string()))?;
    let pkce = Pkce::generate();
    let redirect = loopback.redirect_uri();
    open(&backend.authorize_url(provider, &redirect, &pkce.challenge)?);
    let arrival = loopback.wait(timeout)?;
    let outcome = match &arrival.callback {
        Callback::Code(code) => backend
            .exchange_code(code, &pkce.verifier, &redirect)
            .and_then(|session| keep(store, &session)),
        Callback::Denied(reason) => Err(AccountError::Denied(reason.clone())),
    };
    // The browser waits on its page until the outcome is known.
    arrival.respond(&match &outcome {
        Ok(user) => page::signed_in(&user.email),
        Err(err) => page::not_finished(&err.to_string()),
    });
    outcome
}

/// Sign this machine in from another device (RFC 8628, with PKCE) and keep the session.
///
/// `show` gets the link to open and the code it must show, once the server has issued them.
pub fn login_device(
    backend: &Backend,
    store: &SessionStore,
    timeout: Duration,
    show: impl FnOnce(&DeviceCode),
) -> Result<User, AccountError> {
    let pkce = Pkce::generate();
    let device = backend.start_device(&pkce.challenge)?;
    show(&device);
    let wait = timeout.min(device.expires_in);
    let deadline = Instant::now() + wait;
    let mut interval = device.interval.max(MIN_POLL);
    loop {
        if Instant::now() + interval > deadline {
            return Err(AccountError::Timeout(wait));
        }
        std::thread::sleep(interval);
        match backend.poll_device(&device, &pkce.verifier)? {
            DevicePoll::Pending => {}
            DevicePoll::SlowDown => interval += SLOW_DOWN,
            DevicePoll::Done(session) => return keep(store, &session),
        }
    }
}

fn keep(store: &SessionStore, session: &Session) -> Result<User, AccountError> {
    let _lock = store.lock()?;
    store.save(session)?;
    Ok(session.user.clone())
}

/// Forget this machine's session. Its refresh token is revoked when the issuer can be reached,
/// which ends this machine's SSO session only; this machine is signed out either way. `Ok(None)`
/// when nobody was signed in (a Supabase-era session is just forgotten).
pub fn logout(backend: &Backend, store: &SessionStore) -> Result<Option<User>, AccountError> {
    // Under the lock, so a renewal in another process can neither write the session back
    // afterwards nor rotate the refresh token out from under the revocation.
    let _lock = store.lock()?;
    let session = match store.load() {
        Ok(Some(session)) => session,
        Ok(None) => return Ok(None),
        Err(AccountError::SignedOut(_)) => {
            store.clear()?;
            return Ok(None);
        }
        Err(err) => return Err(err),
    };
    if session.issuer == backend.issuer()
        && let Err(err) = backend.revoke(&session.refresh_token)
    {
        log::debug!("server sign-out skipped: {err}");
    }
    store.clear()?;
    Ok(Some(session.user))
}

#[cfg(test)]
mod tests {
    use super::client::fake::{self, DEVICE, REVOKE, TOKEN, at, field};
    use super::*;
    use crate::oauth::tests::stub::{Reply, closed_base};
    use base64::Engine as _;
    use sha2::{Digest, Sha256};
    use std::io::{Read, Write};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    fn store() -> (tempfile::TempDir, SessionStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::file(dir.path().join("account"));
        (dir, store)
    }

    fn query_param(url: &str, name: &str) -> String {
        field(url.split_once('?').unwrap().1, name).unwrap()
    }

    /// What the fake browser saw: the sign-in URL, then the page it was answered with.
    #[derive(Default)]
    struct Seen {
        url: Mutex<String>,
        page: Mutex<Option<std::thread::JoinHandle<String>>>,
    }

    impl Seen {
        fn page(&self) -> String {
            self.page.lock().unwrap().take().unwrap().join().unwrap()
        }
    }

    /// A browser that follows the sign-in straight back to `redirect_uri` with `query`.
    fn browser(query: &'static str, seen: Arc<Seen>) -> impl FnOnce(&str) {
        move |url: &str| {
            *seen.url.lock().unwrap() = url.to_owned();
            let target = format!("{}?{query}", query_param(url, "redirect_uri"));
            let reader = std::thread::spawn(move || {
                let rest = target.strip_prefix("http://").unwrap();
                let (host, path) = rest.split_once('/').unwrap();
                let mut stream = std::net::TcpStream::connect(host).unwrap();
                write!(stream, "GET /{path} HTTP/1.1\r\nHost: {host}\r\n\r\n").unwrap();
                let mut out = String::new();
                let _ = stream.read_to_string(&mut out);
                out
            });
            *seen.page.lock().unwrap() = Some(reader);
        }
    }

    #[test]
    fn login_exchanges_the_code_with_the_matching_verifier_and_keeps_the_session() {
        let (_dir, store) = store();
        let stub = fake::keycloak(|_| Reply::json(200, &fake::tokens(Some("github"), "rt-1")));
        let backend = fake::backend(&stub);
        let seen = Arc::new(Seen::default());
        let user = login(
            &backend,
            &store,
            Some(Provider::Github),
            Duration::from_secs(10),
            browser("code=the-code", Arc::clone(&seen)),
        )
        .unwrap();
        assert_eq!(
            (user.email.as_str(), user.provider.as_str()),
            ("you@example.com", "github")
        );
        let kept = store.load().unwrap().unwrap();
        assert_eq!(
            (kept.refresh_token.as_str(), kept.sid.as_str()),
            ("rt-1", "sid-1")
        );
        let page = seen.page();
        assert!(page.contains("data-state=\"done\"") && page.contains("you@example.com"));

        let url = seen.url.lock().unwrap().clone();
        assert_eq!(query_param(&url, "kc_idp_hint"), "github");
        let redirect = query_param(&url, "redirect_uri");
        assert!(redirect.starts_with("http://127.0.0.1:"));
        let exchange = &at(&stub, TOKEN)[0].body;
        assert_eq!(field(exchange, "code").unwrap(), "the-code");
        assert_eq!(field(exchange, "redirect_uri").unwrap(), redirect);
        let verifier = field(exchange, "code_verifier").unwrap();
        let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(Sha256::digest(verifier.as_bytes()));
        assert_eq!(query_param(&url, "code_challenge"), challenge);
    }

    #[test]
    fn login_without_a_provider_shows_the_wecoap_page_and_records_email() {
        let (_dir, store) = store();
        let stub = fake::keycloak(|_| Reply::json(200, &fake::tokens(None, "rt")));
        let backend = fake::backend(&stub);
        let seen = Arc::new(Seen::default());
        let user = login(
            &backend,
            &store,
            None,
            Duration::from_secs(10),
            browser("code=c", Arc::clone(&seen)),
        )
        .unwrap();
        seen.page();
        assert!(!seen.url.lock().unwrap().contains("kc_idp_hint"));
        assert_eq!(user.provider, "email");
    }

    #[test]
    fn a_denied_sign_in_stores_nothing() {
        let (_dir, store) = store();
        let stub = fake::keycloak(|_| Reply::json(500, "{}"));
        let backend = fake::backend(&stub);
        let seen = Arc::new(Seen::default());
        let err = login(
            &backend,
            &store,
            Some(Provider::Google),
            Duration::from_secs(10),
            browser(
                "error=access_denied&error_description=denied",
                Arc::clone(&seen),
            ),
        )
        .unwrap_err();
        assert_eq!(err, AccountError::Denied("denied".into()));
        assert_eq!(err.code(), "denied");
        assert_eq!(store.load().unwrap(), None);
        assert!(seen.page().contains("data-state=\"failed\""));
        assert!(at(&stub, TOKEN).is_empty());
    }

    #[test]
    fn a_refused_exchange_shows_the_failed_page_not_the_done_one() {
        let (_dir, store) = store();
        let stub = fake::keycloak(|_| {
            Reply::json(
                400,
                r#"{"error":"invalid_grant","error_description":"Code not valid"}"#,
            )
        });
        let backend = fake::backend(&stub);
        let seen = Arc::new(Seen::default());
        let err = login(
            &backend,
            &store,
            Some(Provider::Apple),
            Duration::from_secs(10),
            browser("code=stale", Arc::clone(&seen)),
        )
        .unwrap_err();
        assert_eq!(err.code(), "refused");
        let page = seen.page();
        assert!(page.contains("data-state=\"failed\"") && page.contains("Code not valid"));
        assert_eq!(store.load().unwrap(), None);
    }

    #[test]
    fn an_unreachable_issuer_fails_before_the_browser_opens() {
        let (_dir, store) = store();
        let backend = Backend::new(&closed_base(), "api");
        let err = login(&backend, &store, None, Duration::from_secs(10), |_| {
            panic!("no browser for a sign-in that cannot finish")
        })
        .unwrap_err();
        assert!(matches!(err, AccountError::Network(_)));
    }

    #[test]
    fn device_login_waits_for_approval_and_keeps_the_session() {
        let (_dir, store) = store();
        let polls = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&polls);
        let stub = fake::keycloak(move |_| match counter.fetch_add(1, Ordering::SeqCst) {
            0 => Reply::json(400, r#"{"error":"authorization_pending"}"#),
            _ => Reply::json(200, &fake::tokens(Some("apple"), "rt-d")),
        });
        let backend = fake::backend(&stub);
        let shown = Mutex::new(String::new());
        let user = login_device(&backend, &store, Duration::from_secs(10), |d| {
            *shown.lock().unwrap() = format!("{} {}", d.url, d.user_code);
        })
        .unwrap();
        assert_eq!(user.provider, "apple");
        assert!(
            shown
                .lock()
                .unwrap()
                .ends_with("/device?user_code=WXYZ-ABCD WXYZ-ABCD")
        );
        assert_eq!(polls.load(Ordering::SeqCst), 2);
        assert_eq!(store.load().unwrap().unwrap().refresh_token, "rt-d");
        let challenge = field(&at(&stub, DEVICE)[0].body, "code_challenge").unwrap();
        let verifier = field(&at(&stub, TOKEN)[1].body, "code_verifier").unwrap();
        let expected = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(Sha256::digest(verifier.as_bytes()));
        assert_eq!(challenge, expected);
    }

    #[test]
    fn a_device_login_nobody_approves_times_out() {
        let (_dir, store) = store();
        let stub = fake::keycloak(|_| Reply::json(400, r#"{"error":"authorization_pending"}"#));
        let backend = fake::backend(&stub);
        let started = Instant::now();
        let err = login_device(&backend, &store, Duration::from_millis(1500), |_| {}).unwrap_err();
        assert!(matches!(err, AccountError::Timeout(_)));
        assert!(started.elapsed() < Duration::from_secs(3));
        assert_eq!(store.load().unwrap(), None);
    }

    fn signed_in(store: &SessionStore, backend: &Backend) {
        let session = Session {
            access_token: "at-1".into(),
            refresh_token: "rt-1".into(),
            expires_at: 4_000_000_000,
            user: User {
                id: "u-1".into(),
                email: "you@example.com".into(),
                provider: "github".into(),
            },
            issuer: backend.issuer().into(),
            sid: "sid-1".into(),
        };
        store.save(&session).unwrap();
    }

    #[test]
    fn logout_forgets_the_session_even_when_the_server_is_unreachable() {
        let (_dir, store) = store();
        let backend = Backend::new(&closed_base(), "api");
        signed_in(&store, &backend);
        let user = logout(&backend, &store).unwrap().unwrap();
        assert_eq!(user.id, "u-1");
        assert_eq!(store.load().unwrap(), None);
        assert_eq!(logout(&backend, &store).unwrap(), None);
    }

    #[test]
    fn logout_revokes_the_refresh_token() {
        let (_dir, store) = store();
        let stub = fake::keycloak(|_| Reply::json(500, "{}"));
        let backend = fake::backend(&stub);
        signed_in(&store, &backend);
        logout(&backend, &store).unwrap();
        assert_eq!(field(&at(&stub, REVOKE)[0].body, "token").unwrap(), "rt-1");
        assert!(at(&stub, TOKEN).is_empty(), "no renewal just to sign out");
    }

    #[test]
    fn logout_of_a_supabase_session_forgets_it_without_sending_it() {
        let (_dir, store) = store();
        let stub = fake::keycloak(|_| Reply::json(500, "{}"));
        let backend = fake::backend(&stub);
        store
            .vault()
            .write(
                "session",
                r#"{"access_token":"a","refresh_token":"r","expires_at":1,
                   "user":{"id":"u","email":"e","provider":"github"},"project":"https://p"}"#,
            )
            .unwrap();
        assert_eq!(logout(&backend, &store).unwrap(), None);
        assert_eq!(store.load().unwrap(), None);
        assert!(stub.requests().is_empty());
    }

    #[test]
    fn providers_hint_keycloak_except_email() {
        let hints: Vec<_> = [
            Provider::Github,
            Provider::Google,
            Provider::Apple,
            Provider::Email,
        ]
        .map(Provider::hint)
        .to_vec();
        assert_eq!(hints, [Some("github"), Some("google"), Some("apple"), None]);
    }

    #[test]
    fn timeouts_read_naturally() {
        assert_eq!(
            AccountError::Timeout(LOGIN_TIMEOUT).to_string(),
            "No sign-in arrived within 5 minutes."
        );
        assert_eq!(wait_text(Duration::from_secs(90)), "90 seconds");
    }
}

//! The HaryX account: signing this machine in to HaryX itself (not a Claude account).
//!
//! Sign-in is GitHub or Google through Supabase Auth, as a native app does it (RFC 8252): PKCE, the
//! browser, and a one-shot listener on `127.0.0.1`. haryx keeps only the tokens and who signed in.
//! Plan: `docs/haryx-account-plan.md`.

pub mod browser;
pub mod client;
mod loopback;
mod page;
mod pkce;
pub mod session;

use std::time::Duration;

pub use client::Backend;
pub use session::{Session, SessionStore, User};

use loopback::{Callback, Loopback};
use pkce::Pkce;

/// How long `login` waits for the browser to come back.
pub const LOGIN_TIMEOUT: Duration = Duration::from_secs(5 * 60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    Github,
    Google,
}

impl Provider {
    /// The Supabase provider name.
    pub fn id(self) -> &'static str {
        match self {
            Provider::Github => "github",
            Provider::Google => "google",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Provider::Github => "GitHub",
            Provider::Google => "Google",
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

/// Sign this machine in with `provider` and keep the session.
///
/// `open` gets the sign-in URL once the local listener is up; it shows or opens it.
pub fn login(
    backend: &Backend,
    store: &SessionStore,
    provider: Provider,
    timeout: Duration,
    open: impl FnOnce(&str),
) -> Result<User, AccountError> {
    let loopback = Loopback::bind().map_err(|e| AccountError::Network(e.to_string()))?;
    let pkce = Pkce::generate();
    open(&backend.authorize_url(provider, &loopback.redirect_uri(), &pkce.challenge));
    let arrival = loopback.wait(timeout)?;
    let outcome = match &arrival.callback {
        Callback::Code(code) => finish(backend, store, code, &pkce.verifier),
        Callback::Denied(reason) => Err(AccountError::Denied(reason.clone())),
    };
    // The browser waits on its page until the outcome is known.
    arrival.respond(&match &outcome {
        Ok(user) => page::signed_in(&user.email),
        Err(err) => page::not_finished(&err.to_string()),
    });
    outcome
}

fn finish(
    backend: &Backend,
    store: &SessionStore,
    code: &str,
    verifier: &str,
) -> Result<User, AccountError> {
    let session = backend.exchange_code(code, verifier)?;
    let _lock = store.lock()?;
    store.save(&session)?;
    Ok(session.user)
}

/// Forget this machine's session. The server is told too when it can be reached; this machine is
/// signed out either way. `Ok(None)` when nobody was signed in.
pub fn logout(
    backend: &Backend,
    store: &SessionStore,
    now: i64,
) -> Result<Option<User>, AccountError> {
    let Some(session) = store.load()? else {
        return Ok(None);
    };
    let ended = store
        .current(backend, now)
        .and_then(|current| match current {
            Some(current) => backend.sign_out(&current.access_token),
            None => Ok(()),
        });
    if let Err(err) = ended {
        log::debug!("server sign-out skipped: {err}");
    }
    // Under the lock, so a renewal in another process cannot write the session back afterwards.
    let _lock = store.lock()?;
    store.clear()?;
    Ok(Some(session.user))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oauth::tests::stub::{Reply, Stub, closed_base};
    use base64::Engine as _;
    use sha2::{Digest, Sha256};
    use std::io::{Read, Write};
    use std::sync::{Arc, Mutex};

    const SESSION: &str = r#"{"access_token":"at-1","refresh_token":"rt-1","expires_at":4000000000,
        "user":{"id":"u-1","email":"you@example.com","app_metadata":{"provider":"github"}}}"#;

    fn store() -> (tempfile::TempDir, SessionStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::file(dir.path().join("account"));
        (dir, store)
    }

    fn query_param(url: &str, name: &str) -> String {
        let query = url.split_once('?').unwrap().1;
        let raw = query
            .split('&')
            .find_map(|p| p.strip_prefix(&format!("{name}=")))
            .unwrap();
        raw.replace("%3A", ":").replace("%2F", "/")
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

    /// A browser that follows the sign-in straight back to `redirect_to` with `query`.
    fn browser(query: &'static str, seen: Arc<Seen>) -> impl FnOnce(&str) {
        move |url: &str| {
            *seen.url.lock().unwrap() = url.to_owned();
            let target = format!("{}?{query}", query_param(url, "redirect_to"));
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

    fn saved(project: &str) -> Session {
        client::session_from(&serde_json::from_str(SESSION).unwrap(), 0, project).unwrap()
    }

    #[test]
    fn login_exchanges_the_code_with_the_matching_verifier_and_keeps_the_session() {
        let (_dir, store) = store();
        let stub = Stub::start(|_| Reply::json(200, SESSION));
        let backend = Backend::new(stub.base(), "k");
        let seen = Arc::new(Seen::default());
        let user = login(
            &backend,
            &store,
            Provider::Github,
            Duration::from_secs(10),
            browser("code=the-code", Arc::clone(&seen)),
        )
        .unwrap();
        assert_eq!(user.email, "you@example.com");
        assert_eq!(store.load().unwrap().unwrap().refresh_token, "rt-1");
        let page = seen.page();
        assert!(page.contains("data-state=\"done\"") && page.contains("you@example.com"));

        let url = seen.url.lock().unwrap().clone();
        assert!(url.contains("provider=github"));
        assert!(query_param(&url, "redirect_to").starts_with("http://127.0.0.1:"));
        let exchange: serde_json::Value = serde_json::from_str(&stub.requests()[0].body).unwrap();
        assert_eq!(exchange["auth_code"], "the-code");
        let verifier = exchange["code_verifier"].as_str().unwrap();
        let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(Sha256::digest(verifier.as_bytes()));
        assert_eq!(query_param(&url, "code_challenge"), challenge);
    }

    #[test]
    fn a_denied_sign_in_stores_nothing() {
        let (_dir, store) = store();
        let backend = Backend::new(&closed_base(), "k");
        let seen = Arc::new(Seen::default());
        let err = login(
            &backend,
            &store,
            Provider::Google,
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
    }

    #[test]
    fn a_refused_exchange_shows_the_failed_page_not_the_done_one() {
        let (_dir, store) = store();
        let stub = Stub::start(|_| {
            Reply::json(
                400,
                r#"{"error_code":"flow_state_not_found","msg":"invalid flow state"}"#,
            )
        });
        let backend = Backend::new(stub.base(), "k");
        let seen = Arc::new(Seen::default());
        let err = login(
            &backend,
            &store,
            Provider::Github,
            Duration::from_secs(10),
            browser("code=stale", Arc::clone(&seen)),
        )
        .unwrap_err();
        assert_eq!(err.code(), "refused");
        let page = seen.page();
        assert!(page.contains("data-state=\"failed\"") && page.contains("invalid flow state"));
        assert_eq!(store.load().unwrap(), None);
    }

    #[test]
    fn logout_forgets_the_session_even_when_the_server_is_unreachable() {
        let (_dir, store) = store();
        let backend = Backend::new(&closed_base(), "k");
        store.save(&saved(backend.base())).unwrap();
        let user = logout(&backend, &store, 1_800_000_000).unwrap().unwrap();
        assert_eq!(user.id, "u-1");
        assert_eq!(store.load().unwrap(), None);
        assert_eq!(logout(&backend, &store, 1_800_000_000).unwrap(), None);
    }

    #[test]
    fn logout_tells_the_server_when_it_can() {
        let (_dir, store) = store();
        let stub = Stub::start(|_| Reply::json(204, ""));
        let backend = Backend::new(stub.base(), "k");
        store.save(&saved(backend.base())).unwrap();
        logout(&backend, &store, 1_800_000_000).unwrap();
        assert_eq!(
            stub.requests()[0].header("authorization"),
            Some("Bearer at-1")
        );
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

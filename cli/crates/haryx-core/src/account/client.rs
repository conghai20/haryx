//! Supabase Auth (`/auth/v1`) calls: the sign-in URL, the PKCE code exchange, renewal and sign-out.
//!
//! Only the project URL and its publishable key are needed. Both are public by design (row-level
//! security and the end-to-end encryption protect the data), so they are compiled in;
//! `HARYX_SUPABASE_URL` / `HARYX_SUPABASE_KEY` point a build at a dev project or a test stub.

use std::time::Duration;

use serde_json::{Value, json};

use super::session::{Session, User};
use super::{AccountError, Provider};
use crate::oauth::http::{self, FetchError};

pub const SUPABASE_URL: &str = "https://ygvcznwlugaxqqmlsssh.supabase.co";
pub const SUPABASE_KEY: &str = "sb_publishable_xQxLmUkWlSFVVC4ifAy6hA_KiPOefNN";
pub const URL_ENV: &str = "HARYX_SUPABASE_URL";
pub const KEY_ENV: &str = "HARYX_SUPABASE_KEY";

const TIMEOUT: Duration = Duration::from_secs(15);

/// One Supabase project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Backend {
    base: String,
    key: String,
}

impl Backend {
    pub fn new(base: &str, key: &str) -> Self {
        Backend {
            base: base.trim_end_matches('/').to_owned(),
            key: key.to_owned(),
        }
    }

    /// The project URL, recorded in each session so it is never sent to another project.
    pub fn base(&self) -> &str {
        &self.base
    }

    /// The compiled-in project, or the one named by the environment.
    pub fn from_env() -> Self {
        let var = |name: &str| std::env::var(name).ok().filter(|v| !v.trim().is_empty());
        Backend::new(
            &var(URL_ENV).unwrap_or_else(|| SUPABASE_URL.to_owned()),
            &var(KEY_ENV).unwrap_or_else(|| SUPABASE_KEY.to_owned()),
        )
    }

    /// Where the browser goes to sign in; it comes back to `redirect` with `?code=`.
    pub fn authorize_url(&self, provider: Provider, redirect: &str, challenge: &str) -> String {
        format!(
            "{}/auth/v1/authorize?provider={}&redirect_to={}&code_challenge={}&code_challenge_method=s256",
            self.base,
            provider.id(),
            percent_encode(redirect),
            percent_encode(challenge),
        )
    }

    /// Trade the code from the redirect for a session.
    pub fn exchange_code(&self, code: &str, verifier: &str) -> Result<Session, AccountError> {
        let body = json!({"auth_code": code, "code_verifier": verifier});
        self.token("pkce", &body)
    }

    /// A new session from the refresh token. Supabase rotates the refresh token on each call.
    pub fn refresh(&self, refresh_token: &str) -> Result<Session, AccountError> {
        self.token("refresh_token", &json!({"refresh_token": refresh_token}))
    }

    /// End this session on the server. Other devices stay signed in (`scope=local`).
    pub fn sign_out(&self, access_token: &str) -> Result<(), AccountError> {
        let bearer = format!("Bearer {access_token}");
        http::post(
            &format!("{}/auth/v1/logout?scope=local", self.base),
            &[("apikey", &self.key), ("authorization", &bearer)],
            b"",
            TIMEOUT,
        )
        .map(drop)
        .map_err(to_account_error)
    }

    fn token(&self, grant: &str, body: &Value) -> Result<Session, AccountError> {
        let resp = http::post(
            &format!("{}/auth/v1/token?grant_type={grant}", self.base),
            &[("apikey", &self.key), ("content-type", "application/json")],
            body.to_string().as_bytes(),
            TIMEOUT,
        )
        .map_err(to_account_error)?;
        let value = resp.json().map_err(to_account_error)?;
        session_from(&value, jiff::Timestamp::now().as_second(), &self.base)
    }
}

/// The parts of a Supabase session response HaryX keeps. `provider_token` and the profile are dropped.
pub(crate) fn session_from(
    value: &Value,
    now: i64,
    project: &str,
) -> Result<Session, AccountError> {
    let text = |v: &Value, key: &str| v.get(key).and_then(Value::as_str).map(str::to_owned);
    let missing = |what: &str| AccountError::Refused(format!("sign-in response has no {what}"));
    let user = value.get("user").ok_or_else(|| missing("user"))?;
    let expires_at = value
        .get("expires_at")
        .and_then(Value::as_i64)
        .or_else(|| {
            let expires_in = value.get("expires_in").and_then(Value::as_i64)?;
            Some(now + expires_in)
        })
        .ok_or_else(|| missing("expiry"))?;
    Ok(Session {
        access_token: text(value, "access_token").ok_or_else(|| missing("access token"))?,
        refresh_token: text(value, "refresh_token").ok_or_else(|| missing("refresh token"))?,
        expires_at,
        user: User {
            id: text(user, "id").ok_or_else(|| missing("user id"))?,
            email: text(user, "email").unwrap_or_default(),
            provider: user
                .get("app_metadata")
                .and_then(|m| text(m, "provider"))
                .unwrap_or_default(),
        },
        project: project.to_owned(),
    })
}

/// Only Supabase itself saying no (a JSON 400/401/403, as for a spent or revoked refresh token) is
/// [`AccountError::Refused`], which signs the user out. A 429, a proxy's 403 page and every 5xx are
/// [`AccountError::Network`]: try again later, keep the session.
fn to_account_error(err: FetchError) -> AccountError {
    if let FetchError::Http { status, body, .. } = &err
        && matches!(status, 400 | 401 | 403)
        && let Some(message) = server_message(body)
    {
        return AccountError::Refused(message);
    }
    AccountError::Network(err.to_string())
}

/// The message of a GoTrue error body, without control characters (it reaches a terminal).
fn server_message(body: &[u8]) -> Option<String> {
    let value: Value = serde_json::from_slice(body).ok()?;
    ["msg", "error_description", "message", "error_code", "error"]
        .iter()
        .find_map(|key| value.get(*key).and_then(Value::as_str))
        .map(|text| text.chars().filter(|c| !c.is_control()).collect())
}

/// RFC 3986 encoding of everything but the unreserved characters.
fn percent_encode(text: &str) -> String {
    text.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oauth::tests::stub::{Reply, Stub};

    const SESSION: &str = r#"{"access_token":"at-1","token_type":"bearer","expires_in":3600,
        "expires_at":1800000000,"refresh_token":"rt-1","provider_token":"gho_secret",
        "user":{"id":"u-1","email":"you@example.com","app_metadata":{"provider":"github"},
        "user_metadata":{"avatar_url":"x"}}}"#;

    fn stub(reply: Reply) -> (Stub, Backend) {
        let stub = Stub::start(move |_| reply.clone());
        let backend = Backend::new(&format!("{}/", stub.base()), "pk-test");
        (stub, backend)
    }

    #[test]
    fn authorize_url_encodes_the_redirect_and_names_the_provider() {
        let backend = Backend::new("https://p.supabase.co/", "k");
        assert_eq!(
            backend.authorize_url(Provider::Google, "http://127.0.0.1:5/callback", "ch-_"),
            "https://p.supabase.co/auth/v1/authorize?provider=google\
             &redirect_to=http%3A%2F%2F127.0.0.1%3A5%2Fcallback&code_challenge=ch-_\
             &code_challenge_method=s256"
        );
    }

    #[test]
    fn exchange_posts_the_code_and_keeps_only_what_haryx_needs() {
        let (stub, backend) = stub(Reply::json(200, SESSION));
        let session = backend.exchange_code("code-1", "verifier-1").unwrap();
        let req = &stub.requests()[0];
        assert_eq!(req.method, "POST");
        assert_eq!(req.path, "/auth/v1/token?grant_type=pkce");
        assert_eq!(req.header("apikey"), Some("pk-test"));
        let body: Value = serde_json::from_str(&req.body).unwrap();
        assert_eq!(
            body,
            json!({"auth_code": "code-1", "code_verifier": "verifier-1"})
        );
        assert_eq!(session.access_token, "at-1");
        assert_eq!(session.refresh_token, "rt-1");
        assert_eq!(session.expires_at, 1_800_000_000);
        assert_eq!(session.project, stub.base());
        assert_eq!(
            session.user,
            User {
                id: "u-1".into(),
                email: "you@example.com".into(),
                provider: "github".into()
            }
        );
        let stored = serde_json::to_string(&session).unwrap();
        assert!(!stored.contains("gho_secret") && !stored.contains("avatar"));
    }

    #[test]
    fn refresh_uses_the_refresh_grant() {
        let (stub, backend) = stub(Reply::json(200, SESSION));
        backend.refresh("rt-0").unwrap();
        let req = &stub.requests()[0];
        assert_eq!(req.path, "/auth/v1/token?grant_type=refresh_token");
        assert_eq!(
            serde_json::from_str::<Value>(&req.body).unwrap(),
            json!({"refresh_token": "rt-0"})
        );
    }

    #[test]
    fn a_4xx_is_refused_with_the_server_message() {
        let (_stub, backend) = stub(Reply::json(
            400,
            r#"{"code":400,"error_code":"refresh_token_already_used","msg":"Invalid Refresh Token: Already Used"}"#,
        ));
        let err = backend.refresh("rt-old").unwrap_err();
        assert_eq!(
            err,
            AccountError::Refused("Invalid Refresh Token: Already Used".into())
        );
    }

    #[test]
    fn rate_limits_and_proxy_pages_are_not_a_refusal() {
        for (status, body) in [
            (
                429,
                r#"{"error_code":"over_request_rate_limit","msg":"slow down"}"#,
            ),
            (403, "<html>Blocked by proxy</html>"),
            (407, "Proxy Authentication Required"),
        ] {
            let (_stub, backend) = stub(Reply::json(status, body));
            assert!(
                matches!(backend.refresh("rt").unwrap_err(), AccountError::Network(_)),
                "{status}"
            );
        }
    }

    #[test]
    fn server_messages_lose_control_characters() {
        let (_stub, backend) = stub(Reply::json(400, "{\"msg\":\"bad\\u001b]52;x\"}"));
        assert_eq!(
            backend.refresh("rt").unwrap_err(),
            AccountError::Refused("bad]52;x".into())
        );
    }

    #[test]
    fn a_5xx_or_unreachable_server_is_a_network_error() {
        let (_stub, backend) = stub(Reply::json(503, "{}"));
        assert!(matches!(
            backend.refresh("rt").unwrap_err(),
            AccountError::Network(_)
        ));
        let gone = Backend::new(&crate::oauth::tests::stub::closed_base(), "k");
        assert!(matches!(
            gone.refresh("rt").unwrap_err(),
            AccountError::Network(_)
        ));
    }

    #[test]
    fn sign_out_sends_the_bearer_token_with_local_scope() {
        let (stub, backend) = stub(Reply::json(204, ""));
        backend.sign_out("at-9").unwrap();
        let req = &stub.requests()[0];
        assert_eq!(req.path, "/auth/v1/logout?scope=local");
        assert_eq!(req.header("authorization"), Some("Bearer at-9"));
    }

    #[test]
    fn expiry_falls_back_to_expires_in() {
        let mut value: Value = serde_json::from_str(SESSION).unwrap();
        value.as_object_mut().unwrap().remove("expires_at");
        assert_eq!(session_from(&value, 1_000, "p").unwrap().expires_at, 4_600);
    }

    #[test]
    fn a_response_without_tokens_is_refused() {
        let err = session_from(&json!({"user": {"id": "u"}}), 0, "p").unwrap_err();
        assert!(matches!(err, AccountError::Refused(_)));
    }
}

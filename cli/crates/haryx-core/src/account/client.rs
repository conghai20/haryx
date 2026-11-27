//! WeCoAp SSO (Keycloak, OpenID Connect) for the CLI: discovery, the sign-in URL, the PKCE code
//! exchange, the device authorization grant, renewal, and sign-out as an RFC 7009 revocation of
//! the refresh token.
//!
//! The issuer, the client id and the HaryX API URL are public, so they are compiled in;
//! `HARYX_SSO_ISSUER` / `HARYX_API_URL` point a build at a local stack or a test stub.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Deserialize;
use serde_json::Value;

use super::session::{Session, User};
use super::{AccountError, Provider};
use crate::oauth::http::{self, FetchError};

pub const SSO_ISSUER: &str = "https://sso.wecoap.com/realms/wecoap";
/// The HaryX API, without a trailing slash: endpoints are `{API_URL}/sync/…`, `{API_URL}/profiles`.
pub const API_URL: &str = "https://haryx.wecoap.com/api/v1";
pub const ISSUER_ENV: &str = "HARYX_SSO_ISSUER";
pub const API_URL_ENV: &str = "HARYX_API_URL";
/// The realm's public client for the CLI (and, through it, both apps).
pub const CLIENT_ID: &str = "haryx-cli";
/// `offline_access`: a machine stays signed in past the realm's SSO idle limits.
const SCOPE: &str = "openid offline_access";
/// A password sign-in carries no `identity_provider` claim (Phase 0, A5).
const PASSWORD_PROVIDER: &str = "email";
const DEVICE_GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";
/// RFC 8628's default when the server names no polling interval.
const DEFAULT_INTERVAL_S: u64 = 5;

const TIMEOUT: Duration = Duration::from_secs(15);

/// One SSO issuer and the HaryX API that trusts it.
#[derive(Debug, Clone)]
pub struct Backend {
    issuer: String,
    api: String,
    /// The issuer's discovery document, fetched once per run and shared by clones.
    discovery: Arc<OnceLock<Discovery>>,
}

/// The endpoints HaryX reads from `.well-known/openid-configuration`.
#[derive(Debug, Clone, Deserialize)]
struct Discovery {
    issuer: String,
    authorization_endpoint: String,
    token_endpoint: String,
    revocation_endpoint: Option<String>,
    device_authorization_endpoint: Option<String>,
}

/// A started device sign-in (RFC 8628): the user opens `url` on any device and confirms `user_code`.
#[derive(Debug, Clone)]
pub struct DeviceCode {
    device_code: String,
    pub user_code: String,
    /// `verification_uri_complete` when sent (the code already filled in), else `verification_uri`.
    pub url: String,
    pub expires_in: Duration,
    pub interval: Duration,
}

/// One poll of a device sign-in.
#[derive(Debug)]
pub enum DevicePoll {
    Pending,
    /// Poll less often (RFC 8628 §3.5: five seconds more).
    SlowDown,
    Done(Box<Session>),
}

impl Backend {
    pub fn new(issuer: &str, api: &str) -> Self {
        Backend {
            issuer: issuer.trim_end_matches('/').to_owned(),
            api: api.trim_end_matches('/').to_owned(),
            discovery: Arc::default(),
        }
    }

    /// The compiled-in issuer and API, or the ones named by the environment.
    pub fn from_env() -> Self {
        let var = |name: &str| std::env::var(name).ok().filter(|v| !v.trim().is_empty());
        Backend::new(
            &var(ISSUER_ENV).unwrap_or_else(|| SSO_ISSUER.to_owned()),
            &var(API_URL_ENV).unwrap_or_else(|| API_URL.to_owned()),
        )
    }

    /// The issuer, recorded in each session so it is never sent to another one.
    pub fn issuer(&self) -> &str {
        &self.issuer
    }

    /// The HaryX API base URL (no trailing slash).
    pub fn api_url(&self) -> &str {
        &self.api
    }

    /// The WeCoAp account page, where the ID itself is managed and deleted (plan A10).
    pub fn account_url(&self) -> String {
        format!("{}/account", self.issuer)
    }

    fn discovery(&self) -> Result<&Discovery, AccountError> {
        if let Some(found) = self.discovery.get() {
            return Ok(found);
        }
        let url = format!("{}/.well-known/openid-configuration", self.issuer);
        let doc: Discovery = http::get(&url, &[("accept", "application/json")], TIMEOUT)
            .and_then(|resp| resp.json())
            .map_err(to_account_error)
            .and_then(|value| {
                serde_json::from_value(value).map_err(|e| {
                    AccountError::Network(format!("unreadable sign-in server settings: {e}"))
                })
            })?;
        // OpenID Connect Discovery §4.3: the document must name the issuer it was fetched from.
        if doc.issuer.trim_end_matches('/') != self.issuer {
            return Err(AccountError::Network(format!(
                "the sign-in server names another issuer: {}",
                clean(&doc.issuer)
            )));
        }
        Ok(self.discovery.get_or_init(|| doc))
    }

    /// Where the browser goes to sign in; it comes back to `redirect` with `?code=`. `provider`
    /// skips the WeCoAp page straight to GitHub, Google or Apple; email (or none) shows the page.
    pub fn authorize_url(
        &self,
        provider: Option<Provider>,
        redirect: &str,
        challenge: &str,
    ) -> Result<String, AccountError> {
        let mut params = vec![
            ("client_id", CLIENT_ID),
            ("response_type", "code"),
            ("scope", SCOPE),
            ("redirect_uri", redirect),
            ("code_challenge", challenge),
            ("code_challenge_method", "S256"),
        ];
        if let Some(hint) = provider.and_then(Provider::hint) {
            params.push(("kc_idp_hint", hint));
        }
        let endpoint = &self.discovery()?.authorization_endpoint;
        let sep = if endpoint.contains('?') { '&' } else { '?' };
        Ok(format!("{endpoint}{sep}{}", form(&params)))
    }

    /// Trade the code from the redirect for a session.
    pub fn exchange_code(
        &self,
        code: &str,
        verifier: &str,
        redirect: &str,
    ) -> Result<Session, AccountError> {
        let value = self.token(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", redirect),
            ("code_verifier", verifier),
        ])?;
        self.session_from(&value)
    }

    /// A new session from the refresh token. Keycloak rotates it; one not sent back stays valid.
    pub fn refresh(&self, refresh_token: &str) -> Result<Session, AccountError> {
        let mut value = self.token(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
        ])?;
        if let Some(map) = value.as_object_mut() {
            map.entry("refresh_token")
                .or_insert_with(|| Value::from(refresh_token));
        }
        self.session_from(&value)
    }

    /// End this machine's session on the server (RFC 7009). Other machines stay signed in.
    pub fn revoke(&self, refresh_token: &str) -> Result<(), AccountError> {
        let endpoint = self
            .discovery()?
            .revocation_endpoint
            .as_deref()
            .ok_or_else(|| AccountError::Network("the sign-in server cannot revoke".into()))?;
        let body = form(&[
            ("token", refresh_token),
            ("token_type_hint", "refresh_token"),
            ("client_id", CLIENT_ID),
        ]);
        http::post(endpoint, &form_headers(), body.as_bytes(), TIMEOUT)
            .map(drop)
            .map_err(to_account_error)
    }

    /// Start a device sign-in (RFC 8628), with PKCE: Keycloak requires it here too (Phase 0, A4).
    pub fn start_device(&self, challenge: &str) -> Result<DeviceCode, AccountError> {
        let endpoint = self
            .discovery()?
            .device_authorization_endpoint
            .as_deref()
            .ok_or_else(|| {
                AccountError::Network("the sign-in server has no device sign-in".into())
            })?;
        let body = form(&[
            ("client_id", CLIENT_ID),
            ("scope", SCOPE),
            ("code_challenge", challenge),
            ("code_challenge_method", "S256"),
        ]);
        let value = http::post(endpoint, &form_headers(), body.as_bytes(), TIMEOUT)
            .and_then(|resp| resp.json())
            .map_err(to_account_error)?;
        let text = |key: &str| value.get(key).and_then(Value::as_str).map(str::to_owned);
        let missing = |what: &str| AccountError::Network(format!("device sign-in has no {what}"));
        let seconds = |key: &str| value.get(key).and_then(Value::as_u64);
        Ok(DeviceCode {
            device_code: text("device_code").ok_or_else(|| missing("device code"))?,
            user_code: text("user_code").ok_or_else(|| missing("user code"))?,
            url: text("verification_uri_complete")
                .or_else(|| text("verification_uri"))
                .ok_or_else(|| missing("link"))?,
            expires_in: Duration::from_secs(seconds("expires_in").unwrap_or(0)),
            interval: Duration::from_secs(seconds("interval").unwrap_or(DEFAULT_INTERVAL_S)),
        })
    }

    /// Ask once whether the device sign-in was approved.
    pub fn poll_device(
        &self,
        device: &DeviceCode,
        verifier: &str,
    ) -> Result<DevicePoll, AccountError> {
        let result = self.token(&[
            ("grant_type", DEVICE_GRANT),
            ("device_code", &device.device_code),
            ("code_verifier", verifier),
        ]);
        match result {
            Ok(value) => Ok(DevicePoll::Done(Box::new(self.session_from(&value)?))),
            Err(TokenError::OAuth { error, description }) => match error.as_str() {
                "authorization_pending" => Ok(DevicePoll::Pending),
                "slow_down" => Ok(DevicePoll::SlowDown),
                "access_denied" => Err(AccountError::Denied(description.unwrap_or(error))),
                "expired_token" => Err(AccountError::Timeout(device.expires_in)),
                _ => Err(TokenError::OAuth { error, description }.into()),
            },
            Err(other) => Err(other.into()),
        }
    }

    fn token(&self, params: &[(&str, &str)]) -> Result<Value, TokenError> {
        let endpoint = &self.discovery()?.token_endpoint;
        let mut params = params.to_vec();
        params.push(("client_id", CLIENT_ID));
        let body = form(&params);
        let resp = http::post(endpoint, &form_headers(), body.as_bytes(), TIMEOUT)
            .map_err(TokenError::from)?;
        resp.json()
            .map_err(|e| TokenError::Other(to_account_error(e)))
    }

    /// The parts of a token response HaryX keeps. The ID token is dropped; who signed in is read
    /// from the access token's claims, which came straight from the issuer over TLS.
    fn session_from(&self, value: &Value) -> Result<Session, AccountError> {
        let text = |v: &Value, key: &str| v.get(key).and_then(Value::as_str).map(str::to_owned);
        let missing = |what: &str| AccountError::Network(format!("sign-in answer has no {what}"));
        let access_token = text(value, "access_token").ok_or_else(|| missing("access token"))?;
        let claims = jwt_claims(&access_token).ok_or_else(|| missing("readable access token"))?;
        let expires_in = value.get("expires_in").and_then(Value::as_i64).unwrap_or(0);
        Ok(Session {
            refresh_token: text(value, "refresh_token").ok_or_else(|| missing("refresh token"))?,
            expires_at: jiff::Timestamp::now().as_second() + expires_in,
            user: User {
                id: text(&claims, "sub").ok_or_else(|| missing("user id"))?,
                email: text(&claims, "email").unwrap_or_default(),
                provider: text(&claims, "identity_provider")
                    .unwrap_or_else(|| PASSWORD_PROVIDER.to_owned()),
            },
            issuer: self.issuer.clone(),
            sid: text(&claims, "sid").unwrap_or_default(),
            access_token,
        })
    }
}

/// A token-endpoint failure: Keycloak's own OAuth error, or anything else.
enum TokenError {
    OAuth {
        error: String,
        description: Option<String>,
    },
    Other(AccountError),
}

impl From<AccountError> for TokenError {
    fn from(err: AccountError) -> Self {
        TokenError::Other(err)
    }
}

impl From<FetchError> for TokenError {
    fn from(err: FetchError) -> Self {
        match oauth_error(&err) {
            Some((error, description)) => TokenError::OAuth { error, description },
            None => TokenError::Other(to_account_error(err)),
        }
    }
}

/// The Lessons rule: only Keycloak's JSON `invalid_grant` (a spent, revoked or expired refresh
/// token, or a bad code) is [`AccountError::Refused`], which signs the user out. Every other
/// status and body is [`AccountError::Network`]: try again later, keep the session.
impl From<TokenError> for AccountError {
    fn from(err: TokenError) -> Self {
        match err {
            TokenError::OAuth { error, description } if error == "invalid_grant" => {
                AccountError::Refused(description.unwrap_or(error))
            }
            TokenError::OAuth { error, description } => AccountError::Network(match description {
                Some(text) => format!("{error}: {text}"),
                None => error,
            }),
            TokenError::Other(err) => err,
        }
    }
}

/// `error` and `error_description` of an RFC 6749 §5.2 error body: a JSON 400 only, so a proxy's
/// page or a 401/403/429/5xx never reads as Keycloak's answer.
fn oauth_error(err: &FetchError) -> Option<(String, Option<String>)> {
    let FetchError::Http {
        status: 400, body, ..
    } = err
    else {
        return None;
    };
    let value: Value = serde_json::from_slice(body).ok()?;
    let field = |key: &str| value.get(key).and_then(Value::as_str).map(clean);
    Some((field("error")?, field("error_description")))
}

/// Any failure outside the token endpoint's OAuth errors: always transient.
fn to_account_error(err: FetchError) -> AccountError {
    AccountError::Network(err.to_string())
}

/// The payload of a JWT, unverified. Only for tokens the issuer just sent over TLS.
fn jwt_claims(token: &str) -> Option<Value> {
    let payload = token.split('.').nth(1)?;
    let bytes = URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')).ok()?;
    serde_json::from_slice(&bytes).ok().filter(Value::is_object)
}

/// Server text without control characters (it reaches a terminal).
fn clean(text: &str) -> String {
    text.chars().filter(|c| !c.is_control()).collect()
}

fn form_headers() -> [(&'static str, &'static str); 2] {
    [
        ("content-type", "application/x-www-form-urlencoded"),
        ("accept", "application/json"),
    ]
}

/// `application/x-www-form-urlencoded`, also used as a URL query.
fn form(params: &[(&str, &str)]) -> String {
    params
        .iter()
        .map(|(k, v)| format!("{}={}", percent_encode(k), percent_encode(v)))
        .collect::<Vec<_>>()
        .join("&")
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

/// A fake Keycloak on `tiny_http` for the account tests.
#[cfg(test)]
pub(crate) mod fake {
    use super::*;
    use crate::oauth::tests::stub::{Recorded, Reply, Stub};

    pub const REALM: &str = "/realms/t";
    pub const TOKEN: &str = "/realms/t/protocol/openid-connect/token";
    pub const REVOKE: &str = "/realms/t/protocol/openid-connect/revoke";
    pub const DEVICE: &str = "/realms/t/protocol/openid-connect/auth/device";

    pub fn jwt(claims: &Value) -> String {
        let part = |v: &Value| URL_SAFE_NO_PAD.encode(v.to_string());
        format!(
            "{}.{}.c2lnbmF0dXJl",
            part(&serde_json::json!({"alg": "RS256", "typ": "JWT"})),
            part(claims)
        )
    }

    /// A Keycloak token response for user `u-1`, signed in with `provider` (`None`: a password).
    pub fn tokens(provider: Option<&str>, refresh: &str) -> String {
        let mut claims = serde_json::json!({
            "iss": "http://kc/realms/t", "sub": "u-1", "email": "you@example.com",
            "sid": "sid-1", "azp": CLIENT_ID,
        });
        if let Some(p) = provider {
            claims["identity_provider"] = Value::from(p);
        }
        serde_json::json!({
            "access_token": jwt(&claims), "expires_in": 300, "refresh_expires_in": 0,
            "refresh_token": refresh, "token_type": "Bearer", "id_token": "id.token.dropped",
            "session_state": "sid-1", "scope": "openid offline_access email profile",
        })
        .to_string()
    }

    /// Discovery, revocation and device endpoints answered; the token endpoint by `token`.
    pub fn keycloak(token: impl Fn(&Recorded) -> Reply + Send + Sync + 'static) -> Stub {
        Stub::start(move |req| {
            let host = req.header("host").unwrap_or_default();
            let path = req.path.split('?').next().unwrap_or_default();
            match path {
                "/realms/t/.well-known/openid-configuration" => Reply::json(
                    200,
                    &serde_json::json!({
                        "issuer": format!("http://{host}{REALM}"),
                        "authorization_endpoint":
                            format!("http://{host}{REALM}/protocol/openid-connect/auth"),
                        "token_endpoint": format!("http://{host}{TOKEN}"),
                        "revocation_endpoint": format!("http://{host}{REVOKE}"),
                        "device_authorization_endpoint": format!("http://{host}{DEVICE}"),
                    })
                    .to_string(),
                ),
                TOKEN => token(req),
                REVOKE => Reply::json(200, ""),
                DEVICE => Reply::json(
                    200,
                    &serde_json::json!({
                        "device_code": "dc-1", "user_code": "WXYZ-ABCD",
                        "verification_uri": format!("http://{host}{REALM}/device"),
                        "verification_uri_complete":
                            format!("http://{host}{REALM}/device?user_code=WXYZ-ABCD"),
                        "expires_in": 600, "interval": 1,
                    })
                    .to_string(),
                ),
                _ => Reply::json(404, "{}"),
            }
        })
    }

    pub fn backend(stub: &Stub) -> Backend {
        Backend::new(
            &format!("{}{REALM}/", stub.base()),
            "http://127.0.0.1:9/api/v1",
        )
    }

    /// The requests that reached `path`.
    pub fn at(stub: &Stub, path: &str) -> Vec<Recorded> {
        stub.requests()
            .into_iter()
            .filter(|r| r.path.split('?').next() == Some(path))
            .collect()
    }

    /// One field of a form body.
    pub fn field(body: &str, name: &str) -> Option<String> {
        body.split('&').find_map(|pair| {
            let (k, v) = pair.split_once('=')?;
            (k == name).then(|| {
                v.replace("%3A", ":")
                    .replace("%2F", "/")
                    .replace("%20", " ")
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::fake::{self, DEVICE, REVOKE, TOKEN, at, field};
    use super::*;
    use crate::oauth::tests::stub::{Reply, Stub, closed_base};
    use serde_json::json;

    fn with_token(reply: Reply) -> (Stub, Backend) {
        let stub = fake::keycloak(move |_| reply.clone());
        let backend = fake::backend(&stub);
        (stub, backend)
    }

    fn query(url: &str, name: &str) -> Option<String> {
        field(url.split_once('?')?.1, name)
    }

    #[test]
    fn discovery_is_fetched_once_and_shared_by_clones() {
        let (stub, backend) = with_token(Reply::json(200, &fake::tokens(None, "rt")));
        let clone = backend.clone();
        backend
            .authorize_url(None, "http://127.0.0.1:5/cb", "ch")
            .unwrap();
        clone.refresh("rt-0").unwrap();
        let discovery: Vec<_> = stub
            .requests()
            .into_iter()
            .filter(|r| r.path.ends_with("/.well-known/openid-configuration"))
            .collect();
        assert_eq!(discovery.len(), 1);
        assert_eq!(
            discovery[0].path,
            "/realms/t/.well-known/openid-configuration"
        );
    }

    #[test]
    fn a_discovery_document_for_another_issuer_is_not_used() {
        let stub = Stub::start(|_| {
            Reply::json(
                200,
                r#"{"issuer":"https://evil.example/realms/t","authorization_endpoint":"x",
                    "token_endpoint":"https://evil.example/token"}"#,
            )
        });
        let err = fake::backend(&stub).refresh("rt").unwrap_err();
        assert!(matches!(err, AccountError::Network(m) if m.contains("another issuer")));
        assert_eq!(stub.requests().len(), 1, "the token never left");
    }

    #[test]
    fn authorize_url_hints_the_provider_and_asks_for_an_offline_pkce_sign_in() {
        let (stub, backend) = with_token(Reply::json(500, "{}"));
        for (provider, hint) in [
            (Some(Provider::Github), Some("github")),
            (Some(Provider::Google), Some("google")),
            (Some(Provider::Apple), Some("apple")),
            (Some(Provider::Email), None),
            (None, None),
        ] {
            let url = backend
                .authorize_url(provider, "http://127.0.0.1:5/callback/n", "ch-_")
                .unwrap();
            assert!(url.starts_with(&format!(
                "{}/realms/t/protocol/openid-connect/auth?client_id=haryx-cli&response_type=code",
                stub.base()
            )));
            assert_eq!(query(&url, "kc_idp_hint").as_deref(), hint, "{provider:?}");
            assert_eq!(query(&url, "scope").unwrap(), "openid offline_access");
            assert_eq!(
                query(&url, "redirect_uri").unwrap(),
                "http://127.0.0.1:5/callback/n"
            );
            assert_eq!(query(&url, "code_challenge").unwrap(), "ch-_");
            assert_eq!(query(&url, "code_challenge_method").unwrap(), "S256");
        }
    }

    #[test]
    fn exchange_posts_the_code_and_keeps_only_what_haryx_needs() {
        let (stub, backend) = with_token(Reply::json(200, &fake::tokens(Some("github"), "rt-1")));
        let before = jiff::Timestamp::now().as_second();
        let session = backend
            .exchange_code("code-1", "verifier-1", "http://127.0.0.1:5/callback/n")
            .unwrap();
        let req = &at(&stub, TOKEN)[0];
        assert_eq!(req.method, "POST");
        assert_eq!(
            req.header("content-type"),
            Some("application/x-www-form-urlencoded")
        );
        assert_eq!(
            req.body,
            "grant_type=authorization_code&code=code-1\
             &redirect_uri=http%3A%2F%2F127.0.0.1%3A5%2Fcallback%2Fn&code_verifier=verifier-1\
             &client_id=haryx-cli"
        );
        assert_eq!(session.refresh_token, "rt-1");
        assert!((before + 300..=before + 302).contains(&session.expires_at));
        assert_eq!(session.issuer, backend.issuer());
        assert_eq!(session.sid, "sid-1");
        assert_eq!(
            session.user,
            User {
                id: "u-1".into(),
                email: "you@example.com".into(),
                provider: "github".into()
            }
        );
        let stored = serde_json::to_string(&session).unwrap();
        assert!(!stored.contains("id.token.dropped") && !stored.contains("session_state"));
    }

    #[test]
    fn a_password_sign_in_has_no_identity_provider_claim_and_is_email() {
        let (_stub, backend) = with_token(Reply::json(200, &fake::tokens(None, "rt")));
        let session = backend.exchange_code("c", "v", "r").unwrap();
        assert_eq!(session.user.provider, "email");
    }

    #[test]
    fn refresh_uses_the_refresh_grant_and_keeps_a_token_not_sent_back() {
        let mut answer: Value = serde_json::from_str(&fake::tokens(None, "x")).unwrap();
        answer.as_object_mut().unwrap().remove("refresh_token");
        let (stub, backend) = with_token(Reply::json(200, &answer.to_string()));
        let session = backend.refresh("rt-0").unwrap();
        assert_eq!(
            at(&stub, TOKEN)[0].body,
            "grant_type=refresh_token&refresh_token=rt-0&client_id=haryx-cli"
        );
        assert_eq!(session.refresh_token, "rt-0");
    }

    #[test]
    fn revoke_sends_the_refresh_token_to_the_revocation_endpoint() {
        let (stub, backend) = with_token(Reply::json(500, "{}"));
        backend.revoke("rt-9").unwrap();
        assert_eq!(
            at(&stub, REVOKE)[0].body,
            "token=rt-9&token_type_hint=refresh_token&client_id=haryx-cli"
        );
    }

    #[test]
    fn device_sign_in_sends_pkce_and_polls_with_the_verifier() {
        let (stub, backend) = with_token(Reply::json(200, &fake::tokens(Some("google"), "rt-d")));
        let device = backend.start_device("ch-d").unwrap();
        assert_eq!(device.user_code, "WXYZ-ABCD");
        assert!(device.url.ends_with("/realms/t/device?user_code=WXYZ-ABCD"));
        assert_eq!(device.interval, Duration::from_secs(1));
        assert_eq!(device.expires_in, Duration::from_secs(600));
        let start = &at(&stub, DEVICE)[0].body;
        assert_eq!(field(start, "code_challenge").unwrap(), "ch-d");
        assert_eq!(field(start, "code_challenge_method").unwrap(), "S256");
        assert_eq!(field(start, "scope").unwrap(), "openid offline_access");

        let DevicePoll::Done(session) = backend.poll_device(&device, "ver-d").unwrap() else {
            panic!("not done");
        };
        assert_eq!(session.user.provider, "google");
        let poll = &at(&stub, TOKEN)[0].body;
        assert_eq!(field(poll, "grant_type").unwrap(), DEVICE_GRANT);
        assert_eq!(field(poll, "device_code").unwrap(), "dc-1");
        assert_eq!(field(poll, "code_verifier").unwrap(), "ver-d");
    }

    #[test]
    fn device_polls_read_pending_slow_down_denied_and_expired() {
        for (error, check) in [
            ("authorization_pending", "pending"),
            ("slow_down", "slow"),
            ("access_denied", "denied"),
            ("expired_token", "timeout"),
        ] {
            let body = json!({"error": error, "error_description": "why"}).to_string();
            let (_stub, backend) = with_token(Reply::json(400, &body));
            let device = backend.start_device("ch").unwrap();
            let got = match backend.poll_device(&device, "v") {
                Ok(DevicePoll::Pending) => "pending",
                Ok(DevicePoll::SlowDown) => "slow",
                Err(AccountError::Denied(why)) if why == "why" => "denied",
                Err(AccountError::Timeout(_)) => "timeout",
                other => panic!("{error}: {other:?}"),
            };
            assert_eq!(got, check);
        }
    }

    // One test per status class (Lessons): only a JSON 400 `invalid_grant` signs out.

    #[test]
    fn a_400_invalid_grant_is_refused_with_keycloaks_description() {
        let (_stub, backend) = with_token(Reply::json(
            400,
            r#"{"error":"invalid_grant","error_description":"Offline user session not found"}"#,
        ));
        assert_eq!(
            backend.refresh("rt").unwrap_err(),
            AccountError::Refused("Offline user session not found".into())
        );
    }

    #[test]
    fn another_400_oauth_error_is_not_a_refusal() {
        let (_stub, backend) = with_token(Reply::json(
            400,
            r#"{"error":"invalid_request","error_description":"Missing parameter"}"#,
        ));
        assert_eq!(
            backend.refresh("rt").unwrap_err(),
            AccountError::Network("invalid_request: Missing parameter".into())
        );
    }

    #[test]
    fn other_4xx_even_saying_invalid_grant_are_not_a_refusal() {
        for (status, body) in [
            (401, r#"{"error":"invalid_grant"}"#),
            (403, "<html>Blocked by proxy</html>"),
            (404, r#"{"error":"invalid_grant"}"#),
            (407, "Proxy Authentication Required"),
            (429, r#"{"error":"invalid_grant"}"#),
            (400, "<html>Bad Request</html>"),
        ] {
            let (_stub, backend) = with_token(Reply::json(status, body));
            assert!(
                matches!(backend.refresh("rt").unwrap_err(), AccountError::Network(_)),
                "{status} {body}"
            );
        }
    }

    #[test]
    fn a_5xx_is_a_network_error() {
        for status in [500, 502, 503] {
            let (_stub, backend) = with_token(Reply::json(status, r#"{"error":"invalid_grant"}"#));
            assert!(matches!(
                backend.refresh("rt").unwrap_err(),
                AccountError::Network(_)
            ));
        }
    }

    #[test]
    fn a_2xx_without_a_usable_session_is_a_network_error() {
        for body in [
            "not json",
            "{}",
            r#"{"access_token":"not-a-jwt","refresh_token":"rt"}"#,
        ] {
            let (_stub, backend) = with_token(Reply::json(200, body));
            assert!(
                matches!(backend.refresh("rt").unwrap_err(), AccountError::Network(_)),
                "{body}"
            );
        }
    }

    #[test]
    fn an_unreachable_server_is_a_network_error() {
        let gone = Backend::new(&closed_base(), "x");
        assert!(matches!(
            gone.refresh("rt").unwrap_err(),
            AccountError::Network(_)
        ));
    }

    #[test]
    fn server_messages_lose_control_characters() {
        let (_stub, backend) = with_token(Reply::json(
            400,
            "{\"error\":\"invalid_grant\",\"error_description\":\"bad\\u001b]52;x\"}",
        ));
        assert_eq!(
            backend.refresh("rt").unwrap_err(),
            AccountError::Refused("bad]52;x".into())
        );
    }

    #[test]
    fn config_defaults_to_production() {
        let b = Backend::new(SSO_ISSUER, API_URL);
        assert_eq!(b.issuer(), "https://sso.wecoap.com/realms/wecoap");
        assert_eq!(b.api_url(), "https://haryx.wecoap.com/api/v1");
        assert_eq!(
            b.account_url(),
            "https://sso.wecoap.com/realms/wecoap/account"
        );
        assert_eq!(
            Backend::new("https://i/", "https://a/v1/").api_url(),
            "https://a/v1"
        );
    }
}

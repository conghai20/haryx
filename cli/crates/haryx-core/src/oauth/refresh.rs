//! The refresh-token grant and the profile endpoint.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::http::{self, FetchError, HttpResponse};
use super::{Api, OAUTH_CLIENT_ID, USER_AGENT, now_ms, truthy};

/// Default budget for the refresh exchange. Callers holding locks other processes contend for
/// pass a budget comfortably inside the contenders' acquire timeout.
pub const REFRESH_TIMEOUT: Duration = Duration::from_secs(10);
const PROFILE_TIMEOUT: Duration = Duration::from_secs(5);

/// An account identity: `{"uuid", "email", "organizationUuid"}`, optional fields str-or-null.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountIdentity {
    pub uuid: String,
    pub email: Option<String>,
    pub organization_uuid: Option<String>,
}

/// Result of a refresh-token grant attempt.
///
/// `credentials` is the full rotated credentials JSON on success. `error` classifies failures so
/// callers can tell a dead lineage (permanent: quarantine) from a blip (transient: retry):
///
/// - `None` — success;
/// - `"invalid_grant"` — the token endpoint rejected the grant; re-login is required;
/// - `"invalid_client"` — our client id was rejected: systemic, no evidence about any slot;
/// - `"no_refresh_token"` — the stored credential carries no usable refresh token (permanent);
/// - `"transient"` — network/server error; the token may still be valid.
///
/// The consume gate (switcher) adds its own kinds (`consume-busy`, `live-login`, …).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RefreshOutcome {
    pub credentials: Option<String>,
    pub error: Option<String>,
    /// The identity the token endpoint optionally returns alongside a successful grant — a
    /// zero-request identity source. Opportunistic: `None` when omitted or malformed.
    pub token_account: Option<AccountIdentity>,
    /// Fingerprint of the generation actually consumed (POSTed). Set by the consume gate, which
    /// may substitute a fresher re-read for the caller's snapshot: strikes bind to these bytes.
    pub consumed_fp: Option<String>,
    /// Whether the consumed successor reached the stash. Only meaningful on a demoted
    /// (`transient` WITH credentials) outcome from the consume gate.
    pub stashed: bool,
}

impl RefreshOutcome {
    pub fn failed(error: &str) -> Self {
        RefreshOutcome {
            error: Some(error.to_string()),
            ..Default::default()
        }
    }
}

/// Identity from `account` / `organization` objects; requires a non-empty string `account.uuid`.
fn parse_identity(data: &Value, email_key: &str) -> Option<AccountIdentity> {
    let account = data.get("account")?.as_object()?;
    let uuid = account.get("uuid")?.as_str()?.trim();
    if uuid.is_empty() {
        return None;
    }
    let as_string = |v: Option<&Value>| v.and_then(Value::as_str).map(str::to_string);
    let org = data.get("organization").and_then(Value::as_object);
    Some(AccountIdentity {
        uuid: uuid.to_string(),
        email: as_string(account.get(email_key)),
        organization_uuid: as_string(org.and_then(|o| o.get("uuid"))),
    })
}

/// The optional identity in a token-endpoint response (`account.email_address`).
pub(crate) fn parse_token_account(resp: &Value) -> Option<AccountIdentity> {
    parse_identity(resp, "email_address")
}

/// Apply a successful grant's response to the stored payload; `None` when the response lacks
/// what a rotation needs (counted as transient).
fn rotate(
    mut data: Map<String, Value>,
    mut oauth: Map<String, Value>,
    resp: &Value,
    now_ms: i64,
) -> Option<String> {
    let access = resp.get("access_token")?.clone();
    let expires_at = match resp.get("expires_in")? {
        Value::Number(n) if n.is_i64() => json!(now_ms + n.as_i64()? * 1000),
        Value::Number(n) => json!(now_ms as f64 + n.as_f64()? * 1000.0),
        _ => return None,
    };
    oauth.insert("accessToken".into(), access);
    oauth.insert("expiresAt".into(), expires_at);
    if let Some(rt) = resp.get("refresh_token").filter(|v| truthy(v)) {
        oauth.insert("refreshToken".into(), rt.clone());
    }
    if let Some(scope) = resp.get("scope").filter(|v| truthy(v)) {
        let scopes: Vec<Value> = scope
            .as_str()?
            .split_whitespace()
            .map(Value::from)
            .collect();
        oauth.insert("scopes".into(), Value::Array(scopes));
    }
    data.insert("claudeAiOauth".into(), Value::Object(oauth));
    serde_json::to_string(&Value::Object(data)).ok()
}

/// The verdict of a refused grant: permanent only on a 400/401/403 whose JSON body's top-level
/// `error` (RFC 6749 §5.2) says so. A substring scan would misclassify, and a misclassified
/// permanent quarantines a live token; a misclassified transient costs one retry.
fn refused_verdict(err: &FetchError) -> &'static str {
    let FetchError::Http {
        status: 400 | 401 | 403,
        body,
        ..
    } = err
    else {
        return "transient";
    };
    let verdict: Option<Value> = serde_json::from_slice(body).ok();
    match verdict
        .as_ref()
        .and_then(|v| v.get("error"))
        .and_then(Value::as_str)
    {
        Some("invalid_grant") => "invalid_grant",
        Some("invalid_client") => "invalid_client",
        _ => "transient",
    }
}

impl Api {
    /// Refresh an OAuth access token via a direct token-endpoint POST.
    ///
    /// `no_refresh_token` is a PERMANENT verdict, so it demands a structurally complete OAuth
    /// object genuinely missing the field; an unparseable or non-object blob is more likely a torn
    /// read than a real credential shape, so it is transient.
    pub fn try_refresh_oauth_credentials(
        &self,
        credentials: &str,
        timeout: Duration,
    ) -> RefreshOutcome {
        let data = match serde_json::from_str::<Value>(credentials) {
            Ok(Value::Object(data)) => data,
            _ => return RefreshOutcome::failed("transient"),
        };
        let oauth = match data.get("claudeAiOauth") {
            Some(Value::Object(o)) if o.get("refreshToken").is_some_and(truthy) => o.clone(),
            _ => return RefreshOutcome::failed("no_refresh_token"),
        };
        let body = json!({
            "grant_type": "refresh_token",
            "refresh_token": oauth["refreshToken"],
            "client_id": OAUTH_CLIENT_ID,
        });
        let headers = [
            ("Content-Type", "application/json"),
            ("User-Agent", USER_AGENT),
        ];
        let sent = http::post(
            &self.token_url,
            &headers,
            body.to_string().as_bytes(),
            timeout,
        );
        match sent.and_then(|r: HttpResponse| r.json()) {
            Ok(resp) => match rotate(data, oauth, &resp, now_ms()) {
                Some(creds) => RefreshOutcome {
                    credentials: Some(creds),
                    token_account: parse_token_account(&resp),
                    ..Default::default()
                },
                None => {
                    log::debug!("OAuth refresh failed: unusable token response");
                    RefreshOutcome::failed("transient")
                }
            },
            Err(e) => {
                if let FetchError::Http { body, .. } = &e {
                    let text = String::from_utf8_lossy(body);
                    let head: String = text.chars().take(500).collect();
                    log::debug!("OAuth refresh failed: {e}, body: {head}");
                } else {
                    log::debug!("OAuth refresh failed: {e}");
                }
                RefreshOutcome::failed(refused_verdict(&e))
            }
        }
    }

    /// Refresh an OAuth access token; `None` on any failure.
    pub fn refresh_oauth_credentials(&self, credentials: &str) -> Option<String> {
        self.try_refresh_oauth_credentials(credentials, REFRESH_TIMEOUT)
            .credentials
    }

    /// Resolve an OAuth access token to its account identity, or `None`.
    ///
    /// `GET /api/oauth/profile` answers whose token this is. `None` on any failure means
    /// "unresolvable", never an error. A response counts as resolved only with a non-empty string
    /// `account.uuid` (a schema change that renames it stays on the fail-open path); `email` and
    /// `organizationUuid` are optional. Must not be called while any credential/config lock is
    /// held.
    pub fn fetch_oauth_profile(&self, access_token: &str) -> Option<AccountIdentity> {
        let auth = format!("Bearer {access_token}");
        let headers = [
            ("Authorization", auth.as_str()),
            ("Content-Type", "application/json"),
            ("User-Agent", USER_AGENT),
        ];
        let data =
            match http::get(&self.profile_url, &headers, PROFILE_TIMEOUT).and_then(|r| r.json()) {
                Ok(data) => data,
                Err(FetchError::Http { status: 401, .. }) => {
                    // Evidence, not proof: the live access token can't authenticate. Log only — the
                    // caller falls back to pre-identity behaviour and the user sees nothing.
                    log::warn!(
                        "OAuth profile returned 401 while resolving credential ownership; \
                     proceeding without identity (pre-fix behavior)."
                    );
                    return None;
                }
                Err(e) => {
                    log::debug!("OAuth profile fetch failed: {e}");
                    return None;
                }
            };
        let identity = parse_identity(&data, "email");
        if identity.is_none() {
            log::debug!("OAuth profile response missing account.uuid");
        }
        identity
    }
}

/// [`Api::try_refresh_oauth_credentials`] against [`Api::from_env`].
pub fn try_refresh_oauth_credentials(credentials: &str, timeout: Duration) -> RefreshOutcome {
    Api::from_env().try_refresh_oauth_credentials(credentials, timeout)
}

/// [`Api::fetch_oauth_profile`] against [`Api::from_env`].
pub fn fetch_oauth_profile(access_token: &str) -> Option<AccountIdentity> {
    Api::from_env().fetch_oauth_profile(access_token)
}

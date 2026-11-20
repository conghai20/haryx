//! Usage fetches: the endpoint request and the per-account flow with refresh and 401 retry.

use std::time::Duration;

use serde_json::Value;

use super::http::{self, FetchError};
use super::refresh::{REFRESH_TIMEOUT, RefreshOutcome};
use super::usage::{Usage, build_usage_result};
use super::{
    Api, OAUTH_BETA_HEADER, USER_AGENT, credential_fingerprint, extract_oauth_data,
    is_oauth_token_expired, is_setup_token, truthy,
};
use crate::PROG;

const USAGE_TIMEOUT: Duration = Duration::from_secs(5);

/// Refresh failures that will not resolve by retrying THIS pass, so the usage endpoint must not
/// be hit with the known-expired token. `consume-busy`: the retry re-enters the same held gate.
/// `live-login`: the gate declined a grant the live login also holds.
pub const DETERMINISTIC_REFRESH_ERRORS: &[&str] = &[
    "store-unmirrored",
    "invalid_client",
    "consume-busy",
    "stash-unreadable",
    "live-login",
];

/// Refresh verdicts that mean the lineage is permanently dead.
const DEAD_REFRESH_ERRORS: &[&str] = &["invalid_grant", "no_refresh_token"];

/// Result of a usage fetch attempt.
///
/// `usage` is the normalised reading on success (it can be `None` on a successful round trip
/// whose response carried no window data). `error` is `None` on success, else a
/// [`FetchError::classify`] kind or a pre-request kind (`no-access-token`, `refresh-failed`,
/// `setup-token`, a refresh verdict). `retry_after_s` is the server's Retry-After when sent.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UsageOutcome {
    pub usage: Option<Usage>,
    pub error: Option<String>,
    pub retry_after_s: Option<f64>,
    /// Fingerprint of the credential whose refresh token was POSTed when `error` is a permanent
    /// auth kind, so the store binds the strike to that generation.
    pub struck_fp: Option<String>,
}

impl UsageOutcome {
    pub fn ok(usage: Option<Usage>) -> Self {
        UsageOutcome {
            usage,
            ..Default::default()
        }
    }

    pub fn failed(kind: &str, retry_after_s: Option<f64>) -> Self {
        UsageOutcome {
            error: Some(kind.to_string()),
            retry_after_s,
            ..Default::default()
        }
    }
}

/// Persists a refreshed credential: `(account_num, email, credentials)`.
pub type PersistFn<'a> = &'a dyn Fn(&str, &str, &str) -> crate::Result<()>;
/// Supersedes the direct refresh POST (the switcher's consume gate): `(account_num, email,
/// snapshot)`.
pub type RefreshViaFn<'a> = &'a dyn Fn(&str, &str, &str) -> RefreshOutcome;

/// One account's usage fetch request.
#[derive(Clone, Copy)]
pub struct UsageFetch<'a> {
    pub account_num: &'a str,
    pub email: &'a str,
    pub credentials: &'a str,
    pub is_active: bool,
    pub persist_credentials: Option<PersistFn<'a>>,
    pub refresh_via: Option<RefreshViaFn<'a>>,
    /// Read a setup-token's usage from the header probe (`usage.headerProbe`).
    pub header_probe: bool,
    /// Also probe per-model buckets (`usage.fableProbe`).
    pub scoped_probe: bool,
    /// The account's stored reading, which shapes the scoped probe.
    pub last_usage: Option<&'a Value>,
}

impl<'a> UsageFetch<'a> {
    pub fn new(
        account_num: &'a str,
        email: &'a str,
        credentials: &'a str,
        is_active: bool,
    ) -> Self {
        UsageFetch {
            account_num,
            email,
            credentials,
            is_active,
            persist_credentials: None,
            refresh_via: None,
            header_probe: false,
            scoped_probe: false,
            last_usage: None,
        }
    }
}

/// One WARNING line with the cause (what users paste into public issues, so `context` never
/// carries the email); the full error stays at DEBUG.
fn log_usage_failure(context: &str, e: &FetchError, kind: &str, retry_after: Option<f64>) {
    let place = if context.is_empty() {
        String::new()
    } else {
        format!(" {context}")
    };
    let mut cause = match retry_after {
        Some(r) => format!("{kind}, retry-after {r:.0}s"),
        None => kind.to_string(),
    };
    if kind == "http-429" {
        // Scope-neutral: whether the budget counts per token or per account depends on the
        // org's 429 regime (see poll_policy).
        cause.push_str(" (usage-endpoint budget reached; backing off)");
    }
    log::warn!("Usage fetch failed{place}: {cause}");
    log::debug!("Usage fetch failure detail{place}: {e:?}");
}

fn classified(context: &str, e: &FetchError) -> UsageOutcome {
    let (kind, retry_after) = e.classify();
    log_usage_failure(context, e, &kind, retry_after);
    UsageOutcome::failed(&kind, retry_after)
}

/// Call the persist callback, warning loudly on failure.
fn persist(callback: Option<PersistFn<'_>>, account_num: &str, email: &str, credentials: &str) {
    let Some(callback) = callback else { return };
    if let Err(e) = callback(account_num, email, credentials) {
        log::warn!(
            "Refreshed OAuth token for account {account_num} ({email}) but failed to persist it: \
             {e}. The refresh token on disk may now be stale; if the next refresh fails with \
             invalid_grant, re-run `{PROG} --add-account` after logging in."
        );
        // stderr, not stdout: this runs inside `list --json`, whose stdout is one JSON object.
        crate::printer::warning_stderr(&format!(
            "Warning: failed to save refreshed token for account {account_num} ({email}). \
             If the next refresh fails, re-run `{PROG} --add-account` after logging in."
        ));
    }
}

fn oauth_str(oauth: Option<&serde_json::Map<String, Value>>, key: &str) -> Option<String> {
    oauth?
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

impl Api {
    /// Request raw utilization data from the usage endpoint.
    pub fn request_usage_data(&self, access_token: &str) -> Result<Value, FetchError> {
        let auth = format!("Bearer {access_token}");
        let headers = [
            ("Authorization", auth.as_str()),
            ("anthropic-beta", OAUTH_BETA_HEADER),
            ("User-Agent", USER_AGENT),
        ];
        http::get(&self.usage_url, &headers, USAGE_TIMEOUT)?.json()
    }

    fn usage_reading(&self, access_token: &str) -> Result<Option<Usage>, FetchError> {
        build_usage_result(&self.request_usage_data(access_token)?)
    }

    /// 5-hour and 7-day utilization from the usage endpoint; `None` on any failure.
    pub fn fetch_usage(&self, access_token: &str) -> Option<Usage> {
        match self.usage_reading(access_token) {
            Ok(usage) => usage,
            Err(e) => {
                classified("", &e);
                None
            }
        }
    }

    fn refresh(&self, req: &UsageFetch<'_>, snapshot: &str) -> RefreshOutcome {
        match req.refresh_via {
            Some(via) => via(req.account_num, req.email, snapshot),
            None => self.try_refresh_oauth_credentials(snapshot, REFRESH_TIMEOUT),
        }
    }

    /// Fetch usage for an account, refreshing expired tokens for inactive accounts only.
    ///
    /// Active accounts are never refreshed — Claude Code owns those credentials. `refresh_via`
    /// supersedes the direct POST (the consume gate persists internally, so
    /// `persist_credentials` is then unused for the refresh). A setup-token never reaches the
    /// usage endpoint: with `header_probe` its usage comes from the header probe, paced exactly
    /// like an endpoint fetch; without it the outcome is the `setup-token` error and no request
    /// is sent.
    pub fn try_fetch_usage_for_account(&self, req: &UsageFetch<'_>) -> UsageOutcome {
        let context = format!("for account {}", req.account_num); // no email: paste-safe
        let mut oauth = extract_oauth_data(req.credentials);
        let Some(mut access_token) = oauth_str(oauth.as_ref(), "accessToken") else {
            return UsageOutcome::failed("no-access-token", None);
        };
        if is_setup_token(oauth.as_ref()) {
            if req.header_probe {
                return self.usage_from_probe(
                    &access_token,
                    &context,
                    req.scoped_probe,
                    req.last_usage,
                );
            }
            return UsageOutcome::failed("setup-token", None);
        }
        let mut working = req.credentials.to_string();
        let has_rt = |o: &Option<serde_json::Map<String, Value>>| {
            o.as_ref()
                .and_then(|o| o.get("refreshToken"))
                .is_some_and(truthy)
        };
        let expired = oauth
            .as_ref()
            .is_some_and(|o| is_oauth_token_expired(o.get("expiresAt")));

        if !req.is_active && has_rt(&oauth) && expired {
            let refresh = self.refresh(req, &working);
            let err = refresh.error.as_deref().unwrap_or_default();
            if let Some(creds) = refresh.credentials.filter(|c| !c.is_empty()) {
                working = creds;
                if req.refresh_via.is_none() {
                    persist(
                        req.persist_credentials,
                        req.account_num,
                        req.email,
                        &working,
                    );
                }
                oauth = extract_oauth_data(&working).or(oauth);
                access_token = oauth_str(oauth.as_ref(), "accessToken").unwrap_or(access_token);
            } else if DEAD_REFRESH_ERRORS.contains(&err) {
                // The lineage is server-rejected (or structurally absent): permanently dead.
                // Don't add a 401/429 to a lost cause; the strike binds to the POSTed bytes.
                return UsageOutcome {
                    struck_fp: refresh
                        .consumed_fp
                        .or_else(|| credential_fingerprint(&working)),
                    ..UsageOutcome::failed(err, None)
                };
            } else if DETERMINISTIC_REFRESH_ERRORS.contains(&err) {
                return UsageOutcome::failed(err, None);
            }
            // A transient refresh failure falls through to try the (expired) token; the 401 path
            // below retries the refresh.
        }

        match self.usage_reading(&access_token) {
            Ok(usage) => UsageOutcome::ok(usage),
            Err(e @ FetchError::Http { status: 401, .. }) if !req.is_active && has_rt(&oauth) => {
                self.retry_after_401(req, &context, &e, &working)
            }
            Err(e) => classified(&context, &e),
        }
    }

    /// Retry once after refreshing on 401 (inactive accounts only). A server-rejected grant is
    /// surfaced distinctly (not the generic `refresh-failed`) so the store can quarantine.
    fn retry_after_401(
        &self,
        req: &UsageFetch<'_>,
        context: &str,
        e: &FetchError,
        working: &str,
    ) -> UsageOutcome {
        let refresh = self.refresh(req, working);
        let Some(creds) = refresh.credentials.filter(|c| !c.is_empty()) else {
            let (kind, _) = e.classify();
            log_usage_failure(context, e, &kind, None);
            let err = refresh.error.as_deref().unwrap_or_default();
            let dead = DEAD_REFRESH_ERRORS.contains(&err);
            // Deterministic kinds keep their identity here too, so the remedy note still shows.
            let distinct = dead || DETERMINISTIC_REFRESH_ERRORS.contains(&err);
            return UsageOutcome {
                struck_fp: if dead {
                    refresh
                        .consumed_fp
                        .or_else(|| credential_fingerprint(working))
                } else {
                    None
                },
                ..UsageOutcome::failed(if distinct { err } else { "refresh-failed" }, None)
            };
        };
        if req.refresh_via.is_none() {
            persist(req.persist_credentials, req.account_num, req.email, &creds);
        }
        let Some(new_token) = oauth_str(extract_oauth_data(&creds).as_ref(), "accessToken") else {
            return UsageOutcome::failed("refresh-failed", None);
        };
        match self.usage_reading(&new_token) {
            Ok(usage) => UsageOutcome::ok(usage),
            Err(retry_error) => classified(&format!("{context} after refresh"), &retry_error),
        }
    }

    /// Usage or `None` (see [`Api::try_fetch_usage_for_account`] for the cause).
    pub fn fetch_usage_for_account(
        &self,
        account_num: &str,
        email: &str,
        credentials: &str,
        is_active: bool,
        persist_credentials: Option<PersistFn<'_>>,
    ) -> Option<Usage> {
        let req = UsageFetch {
            persist_credentials,
            ..UsageFetch::new(account_num, email, credentials, is_active)
        };
        self.try_fetch_usage_for_account(&req).usage
    }
}

/// [`Api::try_fetch_usage_for_account`] against [`Api::from_env`].
pub fn try_fetch_usage_for_account(req: &UsageFetch<'_>) -> UsageOutcome {
    Api::from_env().try_fetch_usage_for_account(req)
}

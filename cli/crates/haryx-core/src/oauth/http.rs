//! The HTTP client behind every endpoint, its failures, and their classification.
//!
//! Requests go through `ureq` with rustls and the platform certificate verifier (the counterpart
//! of upstream's `truststore`), so a corporate CA in the OS trust store is honoured.

use std::time::Duration;

use serde_json::Value;

/// Header names are stored lowercased; lookups are case-insensitive. The first value wins, as with
/// Python's `email.message.Message.get`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Headers(Vec<(String, String)>);

impl Headers {
    pub fn new<K: AsRef<str>, V: Into<String>>(pairs: impl IntoIterator<Item = (K, V)>) -> Self {
        Headers(
            pairs
                .into_iter()
                .map(|(k, v)| (k.as_ref().to_ascii_lowercase(), v.into()))
                .collect(),
        )
    }

    pub fn get(&self, name: &str) -> Option<&str> {
        let name = name.to_ascii_lowercase();
        self.0
            .iter()
            .find(|(k, _)| *k == name)
            .map(|(_, v)| v.as_str())
    }

    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.0.iter().map(|(k, _)| k.as_str())
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// Why a request (or the reading of its response) failed.
#[derive(Debug, Clone, PartialEq)]
pub enum FetchError {
    /// The server answered with a non-2xx status.
    Http {
        status: u16,
        headers: Headers,
        body: Vec<u8>,
    },
    Timeout,
    /// A certificate the client refused (untrusted chain, wrong name, expired).
    TlsCert(String),
    /// Unreachable host, refused connection, other transport failure.
    Network(String),
    /// The body is not JSON.
    Json(String),
    /// A reported usage window that cannot be read as a measurement.
    InvalidUsage(String),
    /// Anything else; `kind` is upstream's fallback (the Python exception type name).
    Other {
        kind: String,
        detail: String,
    },
}

impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FetchError::Http { status, .. } => write!(f, "HTTP Error {status}"),
            FetchError::Timeout => write!(f, "timed out"),
            FetchError::TlsCert(d) => write!(f, "certificate verify failed: {d}"),
            FetchError::Network(d) | FetchError::Json(d) | FetchError::InvalidUsage(d) => {
                write!(f, "{d}")
            }
            FetchError::Other { kind, detail } => write!(f, "{kind}: {detail}"),
        }
    }
}

impl std::error::Error for FetchError {}

/// The `Retry-After` header in its seconds form, clamped at 0 (the HTTP-date form is ignored).
fn retry_after(headers: &Headers) -> Option<f64> {
    let raw = headers.get("retry-after")?;
    if raw.is_empty() {
        return None;
    }
    raw.trim()
        .parse::<f64>()
        .ok()
        .filter(|v| !v.is_nan())
        .map(|v| v.max(0.0))
}

/// The org-policy 403 (a lapsed plan) names its cause only in the body.
fn org_policy_code(body: &[u8]) -> Option<String> {
    let head = &body[..body.len().min(4096)];
    let v: Value = serde_json::from_slice(head).ok()?;
    v.get("error")?
        .get("details")?
        .get("error_code")?
        .as_str()
        .map(str::to_string)
}

impl FetchError {
    /// `(kind, retry_after_s)`: `kind` is a short stable token for logs and backoff decisions
    /// (`http-429`, `timeout`, `tls-cert`, `network`, `bad-response`,
    /// `oauth_not_allowed_for_organization` for the org-policy 403, or a fallback name);
    /// `retry_after_s` is the server's `Retry-After` when sent.
    pub fn classify(&self) -> (String, Option<f64>) {
        match self {
            FetchError::Http {
                status,
                headers,
                body,
            } => {
                let wait = retry_after(headers);
                if *status == 403 {
                    let code = org_policy_code(body);
                    if code.as_deref() == Some("oauth_not_allowed_for_organization") {
                        return (code.unwrap_or_default(), wait);
                    }
                }
                (format!("http-{status}"), wait)
            }
            FetchError::Timeout => ("timeout".into(), None),
            FetchError::TlsCert(_) => ("tls-cert".into(), None),
            FetchError::Network(_) => ("network".into(), None),
            FetchError::Json(_) | FetchError::InvalidUsage(_) => ("bad-response".into(), None),
            FetchError::Other { kind, .. } => (kind.clone(), None),
        }
    }

    /// Just the kind of [`FetchError::classify`].
    pub fn kind(&self) -> String {
        self.classify().0
    }

    pub fn status(&self) -> Option<u16> {
        match self {
            FetchError::Http { status, .. } => Some(*status),
            _ => None,
        }
    }
}

/// A 2xx response.
#[derive(Debug, Clone)]
pub struct HttpResponse {
    pub headers: Headers,
    pub body: Vec<u8>,
}

impl HttpResponse {
    pub fn json(&self) -> Result<Value, FetchError> {
        serde_json::from_slice(&self.body).map_err(|e| FetchError::Json(e.to_string()))
    }
}

/// Whether an error (or anything in its source chain) is a refused peer certificate. ureq does
/// not expose rustls' error type without a direct dependency, so the chain's text is read.
fn is_cert_error(err: &(dyn std::error::Error + 'static)) -> bool {
    let mut cur: Option<&(dyn std::error::Error + 'static)> = Some(err);
    while let Some(e) = cur {
        let text = format!("{e} {e:?}");
        if text.contains("invalid peer certificate") || text.contains("InvalidCertificate") {
            return true;
        }
        cur = e.source();
    }
    false
}

fn map_transport(err: ureq::Error) -> FetchError {
    if is_cert_error(&err) {
        return FetchError::TlsCert(err.to_string());
    }
    match &err {
        ureq::Error::Timeout(_) => FetchError::Timeout,
        ureq::Error::Io(io)
            if matches!(
                io.kind(),
                std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
            ) =>
        {
            FetchError::Timeout
        }
        _ => FetchError::Network(err.to_string()),
    }
}

fn agent(timeout: Duration) -> ureq::Agent {
    use ureq::tls::{RootCerts, TlsConfig};
    ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .http_status_as_error(false)
        .tls_config(
            TlsConfig::builder()
                .root_certs(RootCerts::PlatformVerifier)
                .build(),
        )
        .build()
        .into()
}

fn collect(mut resp: ureq::http::Response<ureq::Body>) -> Result<HttpResponse, FetchError> {
    let status = resp.status().as_u16();
    let headers = Headers::new(resp.headers().iter().map(|(k, v)| {
        (
            k.as_str().to_string(),
            String::from_utf8_lossy(v.as_bytes()).into_owned(),
        )
    }));
    let body = resp.body_mut().read_to_vec().map_err(map_transport)?;
    if (200..300).contains(&status) {
        Ok(HttpResponse { headers, body })
    } else {
        Err(FetchError::Http {
            status,
            headers,
            body,
        })
    }
}

/// `GET url` with `headers`, bounded by `timeout` end to end.
pub(crate) fn get(
    url: &str,
    headers: &[(&str, &str)],
    timeout: Duration,
) -> Result<HttpResponse, FetchError> {
    let req = headers
        .iter()
        .fold(agent(timeout).get(url), |r, (k, v)| r.header(*k, *v));
    collect(req.call().map_err(map_transport)?)
}

/// `POST url` with `headers` and `body`, bounded by `timeout` end to end.
pub(crate) fn post(
    url: &str,
    headers: &[(&str, &str)],
    body: &[u8],
    timeout: Duration,
) -> Result<HttpResponse, FetchError> {
    let req = headers
        .iter()
        .fold(agent(timeout).post(url), |r, (k, v)| r.header(*k, *v));
    collect(req.send(body).map_err(map_transport)?)
}

pub(crate) fn patch(
    url: &str,
    headers: &[(&str, &str)],
    body: &[u8],
    timeout: Duration,
) -> Result<HttpResponse, FetchError> {
    let req = headers
        .iter()
        .fold(agent(timeout).patch(url), |r, (k, v)| r.header(*k, *v));
    collect(req.send(body).map_err(map_transport)?)
}

pub(crate) fn delete(
    url: &str,
    headers: &[(&str, &str)],
    timeout: Duration,
) -> Result<HttpResponse, FetchError> {
    let req = headers
        .iter()
        .fold(agent(timeout).delete(url), |r, (k, v)| r.header(*k, *v));
    collect(req.call().map_err(map_transport)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn http(status: u16, headers: &[(&str, &str)], body: &str) -> FetchError {
        FetchError::Http {
            status,
            headers: Headers::new(headers.iter().copied()),
            body: body.as_bytes().to_vec(),
        }
    }

    #[test]
    fn http_codes() {
        for code in [401, 403, 404, 429, 500, 503] {
            assert_eq!(
                http(code, &[], "").classify(),
                (format!("http-{code}"), None)
            );
        }
    }

    #[test]
    fn org_policy_403_names_itself() {
        let body = r#"{"error":{"details":{"error_code":"oauth_not_allowed_for_organization"}}}"#;
        assert_eq!(
            http(403, &[], body).kind(),
            "oauth_not_allowed_for_organization"
        );
        let other = r#"{"error":{"details":{"error_code":"something_else"}}}"#;
        assert_eq!(http(403, &[], other).kind(), "http-403");
        assert_eq!(http(403, &[], "not json").kind(), "http-403");
        // Only a 403 reads the body.
        assert_eq!(http(401, &[], body).kind(), "http-401");
    }

    #[test]
    fn retry_after_forms() {
        assert_eq!(
            http(429, &[("Retry-After", "120")], "").classify().1,
            Some(120.0)
        );
        assert_eq!(
            http(429, &[("retry-after", " 7 ")], "").classify().1,
            Some(7.0)
        );
        assert_eq!(
            http(429, &[("Retry-After", "Wed, 21 Oct 2015 07:28:00 GMT")], "")
                .classify()
                .1,
            None
        );
        assert_eq!(
            http(429, &[("Retry-After", "-5")], "").classify().1,
            Some(0.0)
        );
        assert_eq!(http(429, &[], "").classify().1, None);
    }

    #[test]
    fn transport_kinds() {
        assert_eq!(FetchError::Timeout.kind(), "timeout");
        assert_eq!(FetchError::Network("refused".into()).kind(), "network");
        assert_eq!(FetchError::TlsCert("x".into()).kind(), "tls-cert");
        assert_eq!(FetchError::Json("x".into()).kind(), "bad-response");
        assert_eq!(FetchError::InvalidUsage("x".into()).kind(), "bad-response");
        let other = FetchError::Other {
            kind: "RuntimeError".into(),
            detail: "boom".into(),
        };
        assert_eq!(other.kind(), "RuntimeError");
    }

    #[test]
    fn headers_are_case_insensitive_first_wins() {
        let h = Headers::new([("X-A", "1"), ("x-a", "2")]);
        assert_eq!(h.get("x-A"), Some("1"));
        assert_eq!(h.keys().collect::<Vec<_>>(), ["x-a", "x-a"]);
    }
}

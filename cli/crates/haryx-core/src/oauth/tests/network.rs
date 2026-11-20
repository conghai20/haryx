//! The usage endpoint, token refresh, profile and per-account fetch against the stub.

use super::*;

// --- network: usage endpoint ---

fn usage_body() -> String {
    json!({"five_hour": {"utilization": 22.0, "resets_at": null}, "seven_day": {"utilization": 61.0}}).to_string()
}

#[test]
fn fetch_usage_success_sends_the_documented_headers() {
    let stub = Stub::start(|_| Reply::json(200, &usage_body()));
    let usage = stub.api().fetch_usage("sk-test").unwrap();
    assert_eq!(usage["five_hour"]["pct"], json!(22.0));
    let req = &stub.requests()[0];
    assert_eq!(
        (req.method.as_str(), req.path.as_str()),
        ("GET", "/api/oauth/usage")
    );
    assert_eq!(req.header("authorization"), Some("Bearer sk-test"));
    assert_eq!(req.header("anthropic-beta"), Some(OAUTH_BETA_HEADER));
    assert_eq!(req.header("user-agent"), Some(USER_AGENT));
}

#[test]
fn fetch_usage_failures_are_none() {
    let stub = Stub::start(|_| Reply::json(200, "not json"));
    assert_eq!(stub.api().fetch_usage("t"), None);
    let api = Api::with_base(&stub::closed_base());
    assert_eq!(api.fetch_usage("t"), None);
    assert_eq!(api.request_usage_data("t").unwrap_err().kind(), "network");
}

#[test]
fn api_from_env_and_base() {
    assert_eq!(Api::default().usage_url, USAGE_URL);
    let api = Api::with_base("http://127.0.0.1:9/");
    assert_eq!(api.token_url, "http://127.0.0.1:9/v1/oauth/token");
    assert_eq!(api.messages_url, "http://127.0.0.1:9/v1/messages");
    assert_eq!(api.profile_url, "http://127.0.0.1:9/api/oauth/profile");
}

// --- network: refresh ---

fn token_reply(extra: Value) -> Reply {
    let mut body = json!({"access_token": "new-at", "expires_in": 3600, "refresh_token": "new-rt", "scope": "user:inference user:profile"});
    if let (Some(b), Some(e)) = (body.as_object_mut(), extra.as_object()) {
        b.extend(e.clone());
    }
    Reply::json(200, &body.to_string())
}

#[test]
fn refresh_sends_correct_body_and_rotates() {
    let stub = Stub::start(|_| token_reply(json!({})));
    let before = now_ms();
    let out = stub
        .api()
        .try_refresh_oauth_credentials(&creds("old", Some("rt-1"), 1), REFRESH_TIMEOUT);
    assert_eq!(out.error, None);
    let oauth = extract_oauth_data(out.credentials.as_deref().unwrap()).unwrap();
    assert_eq!(oauth["accessToken"], json!("new-at"));
    assert_eq!(oauth["refreshToken"], json!("new-rt"));
    assert_eq!(oauth["scopes"], json!(["user:inference", "user:profile"]));
    let exp = oauth["expiresAt"].as_i64().unwrap();
    assert!(exp >= before + 3_600_000 && exp <= now_ms() + 3_600_000);
    let req = &stub.requests()[0];
    assert_eq!(
        (req.method.as_str(), req.path.as_str()),
        ("POST", "/v1/oauth/token")
    );
    assert_eq!(req.header("content-type"), Some("application/json"));
    let body: Value = serde_json::from_str(&req.body).unwrap();
    assert_eq!(
        body,
        json!({"grant_type": "refresh_token", "refresh_token": "rt-1", "client_id": OAUTH_CLIENT_ID})
    );
}

#[test]
fn refresh_verdicts() {
    let cases: [(u16, &str, &str); 6] = [
        (400, r#"{"error":"invalid_grant"}"#, "invalid_grant"),
        (
            400,
            r#"{"error":"invalid_grant","error_description":"Refresh token revoked"}"#,
            "invalid_grant",
        ),
        (401, r#"{"error":"invalid_client"}"#, "invalid_client"),
        (400, r#"{"error":"server_busy"}"#, "transient"),
        (
            400,
            r#"{"error":{"message":"see invalid_grant docs"}}"#,
            "transient",
        ),
        (500, r#"{"error":"invalid_grant"}"#, "transient"),
    ];
    for (status, body, verdict) in cases {
        let body = body.to_string();
        let stub = Stub::start(move |_| Reply::json(status, &body));
        let out = stub
            .api()
            .try_refresh_oauth_credentials(&creds("a", Some("r"), 1), REFRESH_TIMEOUT);
        assert_eq!(out.error.as_deref(), Some(verdict), "{status}");
        assert_eq!(out.credentials, None);
    }
    let stub = Stub::start(|_| Reply::json(400, "<html>"));
    let out = stub
        .api()
        .try_refresh_oauth_credentials(&creds("a", Some("r"), 1), REFRESH_TIMEOUT);
    assert_eq!(out.error.as_deref(), Some("transient"));
}

#[test]
fn refresh_structural_guards() {
    let api = Api::with_base(&stub::closed_base());
    let t = REFRESH_TIMEOUT;
    assert_eq!(
        api.try_refresh_oauth_credentials(&creds("a", Some("r"), 1), t)
            .error
            .as_deref(),
        Some("transient")
    );
    assert_eq!(
        api.try_refresh_oauth_credentials(&creds("a", None, 1), t)
            .error
            .as_deref(),
        Some("no_refresh_token")
    );
    assert_eq!(
        api.try_refresh_oauth_credentials("{torn", t)
            .error
            .as_deref(),
        Some("transient")
    );
    assert_eq!(
        api.try_refresh_oauth_credentials("[1,2]", t)
            .error
            .as_deref(),
        Some("transient")
    );
    assert_eq!(
        api.refresh_oauth_credentials(&creds("a", Some("r"), 1)),
        None
    );
}

#[test]
fn token_account_parsing() {
    let cases = [
        (
            json!({"account": {"uuid": " u-1 ", "email_address": "a@b.c"}, "organization": {"uuid": "o-1"}}),
            Some(AccountIdentity {
                uuid: "u-1".into(),
                email: Some("a@b.c".into()),
                organization_uuid: Some("o-1".into()),
            }),
        ),
        (json!({}), None),
        (json!({"account": {"email_address": "a@b.c"}}), None),
        (json!({"account": {"uuid": 42}}), None),
        (
            json!({"account": {"uuid": "u", "email_address": 7}, "organization": {"uuid": 9}}),
            Some(AccountIdentity {
                uuid: "u".into(),
                email: None,
                organization_uuid: None,
            }),
        ),
    ];
    for (extra, want) in cases {
        let stub = Stub::start(move |_| token_reply(extra.clone()));
        let out = stub
            .api()
            .try_refresh_oauth_credentials(&creds("a", Some("r"), 1), REFRESH_TIMEOUT);
        assert_eq!(out.error, None);
        assert_eq!(out.token_account, want);
    }
}

// --- network: profile ---

#[test]
fn fetch_oauth_profile_cases() {
    let cases = [
        (
            200,
            json!({"account": {"uuid": "u-1", "email": "a@b.c"}, "organization": {"uuid": "o-1"}})
                .to_string(),
            Some(AccountIdentity {
                uuid: "u-1".into(),
                email: Some("a@b.c".into()),
                organization_uuid: Some("o-1".into()),
            }),
        ),
        (
            200,
            json!({"account": {"uuid": "  u-2  "}}).to_string(),
            Some(AccountIdentity {
                uuid: "u-2".into(),
                email: None,
                organization_uuid: None,
            }),
        ),
        (200, json!({"organization": {}}).to_string(), None),
        (
            200,
            json!({"account": {"email": "a@b.c"}}).to_string(),
            None,
        ),
        (200, json!({"account": {"uuid": 5}}).to_string(), None),
        (200, json!({"account": {"uuid": "   "}}).to_string(), None),
        (200, "not json".to_string(), None),
        (401, "{}".to_string(), None),
    ];
    for (status, body, want) in cases {
        let stub = Stub::start(move |_| Reply::json(status, &body));
        assert_eq!(stub.api().fetch_oauth_profile("tok"), want);
        let req = &stub.requests()[0];
        assert_eq!(req.path, "/api/oauth/profile");
        assert_eq!(req.header("authorization"), Some("Bearer tok"));
    }
    assert_eq!(
        Api::with_base(&stub::closed_base()).fetch_oauth_profile("t"),
        None
    );
}

// --- network: per-account fetch ---

fn account_stub(usage_status: impl Fn(&str) -> Reply + Send + Sync + 'static) -> Stub {
    Stub::start(move |req| match req.path.as_str() {
        "/v1/oauth/token" => token_reply(json!({})),
        _ => usage_status(req.header("authorization").unwrap_or_default()),
    })
}

pub(super) fn paths(stub: &Stub) -> Vec<String> {
    stub.requests().into_iter().map(|r| r.path).collect()
}

#[test]
fn refreshes_expired_token_before_usage_fetch_and_persists() {
    let stub = account_stub(|auth| {
        assert_eq!(auth, "Bearer new-at");
        Reply::json(200, &usage_body())
    });
    let saved = RefCell::new(Vec::new());
    let persist = |num: &str, email: &str, c: &str| -> crate::Result<()> {
        saved
            .borrow_mut()
            .push((num.to_string(), email.to_string(), c.to_string()));
        Ok(())
    };
    let c = creds("old-at", Some("rt"), expired_ms());
    let req = UsageFetch {
        persist_credentials: Some(&persist),
        ..UsageFetch::new("2", "a@b.c", &c, false)
    };
    let out = stub.api().try_fetch_usage_for_account(&req);
    assert_eq!(out.error, None);
    assert_eq!(out.usage.unwrap()["seven_day"]["pct"], json!(61.0));
    assert_eq!(paths(&stub), ["/v1/oauth/token", "/api/oauth/usage"]);
    let saved = saved.borrow();
    assert_eq!(saved.len(), 1);
    assert_eq!(extract_access_token(&saved[0].2).as_deref(), Some("new-at"));
}

#[test]
fn retries_401_with_token_refresh() {
    let stub = account_stub(|auth| {
        if auth == "Bearer old-at" {
            Reply::json(401, "{}")
        } else {
            Reply::json(200, &usage_body())
        }
    });
    let c = creds("old-at", Some("rt"), far_future_ms());
    let out = stub
        .api()
        .try_fetch_usage_for_account(&UsageFetch::new("2", "a@b.c", &c, false));
    assert_eq!(out.error, None);
    assert!(out.usage.is_some());
    assert_eq!(
        paths(&stub),
        ["/api/oauth/usage", "/v1/oauth/token", "/api/oauth/usage"]
    );
}

#[test]
fn valid_token_fetches_without_refresh() {
    let stub = account_stub(|_| Reply::json(200, &usage_body()));
    let c = creds("at", Some("rt"), far_future_ms());
    let out = stub
        .api()
        .try_fetch_usage_for_account(&UsageFetch::new("2", "a@b.c", &c, false));
    assert!(out.usage.is_some());
    assert_eq!(paths(&stub), ["/api/oauth/usage"]);
}

#[test]
fn active_account_never_refreshes() {
    let stub = account_stub(|_| Reply::json(401, "{}"));
    let c = creds("at", Some("rt"), expired_ms());
    let out = stub
        .api()
        .try_fetch_usage_for_account(&UsageFetch::new("1", "a@b.c", &c, true));
    assert_eq!(out.error.as_deref(), Some("http-401"));
    assert_eq!(paths(&stub), ["/api/oauth/usage"]);
}

#[test]
fn refresh_failure_on_401_is_refresh_failed() {
    let stub = Stub::start(|req| match req.path.as_str() {
        "/v1/oauth/token" => Reply::json(503, "{}"),
        _ => Reply::json(401, "{}"),
    });
    let c = creds("at", Some("rt"), far_future_ms());
    let out = stub
        .api()
        .try_fetch_usage_for_account(&UsageFetch::new("2", "a@b.c", &c, false));
    assert_eq!(out.error.as_deref(), Some("refresh-failed"));
    assert_eq!(out.usage, None);
    assert_eq!(out.struck_fp, None);
}

#[test]
fn outcome_kinds() {
    let stub = account_stub(|_| Reply::json(429, "{}").header("Retry-After", "318"));
    let c = creds("at", Some("rt"), far_future_ms());
    let out = stub
        .api()
        .try_fetch_usage_for_account(&UsageFetch::new("2", "a@b.c", &c, false));
    assert_eq!(out.error.as_deref(), Some("http-429"));
    assert_eq!(out.retry_after_s, Some(318.0));

    let none = json!({"claudeAiOauth": {"refreshToken": "r"}}).to_string();
    let out = stub
        .api()
        .try_fetch_usage_for_account(&UsageFetch::new("2", "e", &none, false));
    assert_eq!(out.error.as_deref(), Some("no-access-token"));

    let closed = Api::with_base(&stub::closed_base());
    let out = closed.try_fetch_usage_for_account(&UsageFetch::new("2", "e", &c, false));
    assert_eq!(out.error.as_deref(), Some("network"));

    let bad = Stub::start(|_| Reply::json(200, r#"{"five_hour": {"utilization": "x"}}"#));
    let out = bad
        .api()
        .try_fetch_usage_for_account(&UsageFetch::new("2", "e", &c, false));
    assert_eq!(out.error.as_deref(), Some("bad-response"));
}

#[test]
fn invalid_grant_is_permanent_with_a_strike() {
    // Proactive refresh: no usage request is spent on a dead lineage.
    let stub = Stub::start(|req| match req.path.as_str() {
        "/v1/oauth/token" => Reply::json(400, r#"{"error":"invalid_grant"}"#),
        _ => Reply::json(401, "{}"),
    });
    let c = creds("at", Some("rt"), expired_ms());
    let out = stub
        .api()
        .try_fetch_usage_for_account(&UsageFetch::new("2", "e", &c, false));
    assert_eq!(out.error.as_deref(), Some("invalid_grant"));
    assert_eq!(out.struck_fp, credential_fingerprint(&c));
    assert_eq!(paths(&stub), ["/v1/oauth/token"]);
    // 401 path.
    let c = creds("at", Some("rt"), far_future_ms());
    let out = stub
        .api()
        .try_fetch_usage_for_account(&UsageFetch::new("2", "e", &c, false));
    assert_eq!(out.error.as_deref(), Some("invalid_grant"));
    assert_eq!(out.struck_fp, credential_fingerprint(&c));
}

#[test]
fn deterministic_refresh_refusals_spend_no_request() {
    let stub = account_stub(|_| Reply::json(401, "{}"));
    let c = creds("at", Some("rt"), expired_ms());
    for kind in DETERMINISTIC_REFRESH_ERRORS {
        let via = |_: &str, _: &str, _: &str| RefreshOutcome::failed(kind);
        let req = UsageFetch {
            refresh_via: Some(&via),
            ..UsageFetch::new("2", "e", &c, false)
        };
        let out = stub.api().try_fetch_usage_for_account(&req);
        assert_eq!(out.error.as_deref(), Some(*kind));
    }
    assert!(stub.requests().is_empty());
    // The consume gate's fingerprint wins for the strike.
    let via = |_: &str, _: &str, _: &str| RefreshOutcome {
        consumed_fp: Some("sha256:gate".into()),
        ..RefreshOutcome::failed("invalid_grant")
    };
    let req = UsageFetch {
        refresh_via: Some(&via),
        ..UsageFetch::new("2", "e", &c, false)
    };
    assert_eq!(
        stub.api()
            .try_fetch_usage_for_account(&req)
            .struck_fp
            .as_deref(),
        Some("sha256:gate")
    );
}

#[test]
fn transient_refresh_falls_through_and_is_not_permanent() {
    let stub = account_stub(|_| Reply::json(401, "{}"));
    let c = creds("at", Some("rt"), expired_ms());
    let via = |_: &str, _: &str, _: &str| RefreshOutcome::failed("transient");
    let req = UsageFetch {
        refresh_via: Some(&via),
        ..UsageFetch::new("2", "e", &c, false)
    };
    let out = stub.api().try_fetch_usage_for_account(&req);
    assert_eq!(out.error.as_deref(), Some("refresh-failed"));
    assert_eq!(out.struck_fp, None);
}

#[test]
fn persist_failure_does_not_fail_the_fetch() {
    let stub = account_stub(|_| Reply::json(200, &usage_body()));
    let persist = |_: &str, _: &str, _: &str| -> crate::Result<()> {
        Err(crate::Error::General("disk full".into()))
    };
    let c = creds("at", Some("rt"), expired_ms());
    let req = UsageFetch {
        persist_credentials: Some(&persist),
        ..UsageFetch::new("2", "e", &c, false)
    };
    assert!(stub.api().try_fetch_usage_for_account(&req).usage.is_some());
}

/// The only test that reads the process environment: every other test passes its `Api` directly,
/// so setting the variable here cannot race another test.
#[test]
fn haryx_api_base_env_points_every_endpoint_at_the_stub() {
    let stub =
        Stub::start(|_| Reply::json(200, &json!({"account": {"uuid": "u-env"}}).to_string()));
    let base = stub
        .api()
        .profile_url
        .trim_end_matches("/api/oauth/profile")
        .to_string();
    // SAFETY: no other test reads or writes HARYX_API_BASE (see above).
    unsafe { std::env::set_var(API_BASE_ENV, &base) };
    let from_env = Api::from_env();
    let identity = fetch_oauth_profile("tok");
    // SAFETY: as above.
    unsafe { std::env::remove_var(API_BASE_ENV) };
    assert_eq!(from_env, Api::with_base(&base));
    assert_eq!(identity.map(|i| i.uuid).as_deref(), Some("u-env"));
    assert_eq!(Api::from_env(), Api::default());
}

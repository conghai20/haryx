//! `haryx login`, `logout`, `whoami` end to end: the real binary against a fake Keycloak, a fake
//! browser and on macOS a fake `security` (`common`).

// The fakes are `#!/bin/sh` scripts.
#![cfg(unix)]

mod common;

use common::*;
use serde_json::Value;

#[test]
fn whoami_signed_out_matches_the_contract() {
    let s = Sandbox::new(200);
    let out = s.haryx(&["whoami", "--json"], "");
    assert!(out.status.success());
    let status = &json_lines(&out)[0];
    assert_eq!(status["signedIn"], false);
    assert_eq!(status["user"], Value::Null);
    assert!(same_shape(status, &whoami_without_consents()));

    let text = s.haryx(&["whoami"], "");
    assert!(String::from_utf8_lossy(&text.stdout).contains("Not signed in to HaryX"));
}

#[test]
fn login_through_the_browser_keeps_the_session_and_whoami_shows_it() {
    let s = Sandbox::new(200);
    let out = s.haryx(&["login", "--json"], "code=the-code");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let events = json_lines(&out);
    let fixture = contract("login-events.json");
    assert_eq!(events.len(), 2);
    assert_eq!(events[0]["event"], "browser");
    assert_eq!(events[0]["opened"], true);
    assert!(same_shape(&events[0], &fixture[0]));
    assert_eq!(events[1]["event"], "signed_in");
    assert_eq!(events[1]["user"]["email"], "you@example.com");
    assert!(same_shape(&events[1], &fixture[1]));

    let url = events[0]["url"].as_str().unwrap();
    assert!(url.starts_with(&format!(
        "{}/protocol/openid-connect/auth?client_id=haryx-cli&response_type=code",
        s.keycloak.issuer
    )));
    assert!(url.contains("&redirect_uri=http%3A%2F%2F127.0.0.1%3A"));
    assert!(!url.contains("kc_idp_hint"), "always the WeCoAp page");
    let token = format!("{OIDC}/token");
    assert_eq!(
        s.keycloak.paths(),
        [
            format!("{REALM}/.well-known/openid-configuration"),
            token.clone()
        ]
    );
    let exchange = s.keycloak.body(&token);
    assert!(exchange.starts_with("grant_type=authorization_code&code=the-code&"));
    assert!(exchange.contains("&code_verifier=") && exchange.ends_with("&client_id=haryx-cli"));

    let stored = std::fs::read_to_string(s.stored_session()).unwrap();
    assert!(!stored.contains("id_never_kept"));
    assert!(stored.contains(r#""sid":"sid-test""#));

    let whoami = json_lines(&s.haryx(&["whoami", "--json"], ""));
    assert_eq!(whoami[0]["signedIn"], true);
    assert_eq!(whoami[0]["user"]["provider"], "github");
    assert_eq!(whoami[0]["user"]["id"], "u-test");
    assert!(
        whoami[0].get("consents").is_none(),
        "no API here: no consents, still signed in"
    );
    assert!(same_shape(&whoami[0], &whoami_without_consents()));
    let text = s.haryx(&["whoami"], "");
    assert!(
        String::from_utf8_lossy(&text.stdout)
            .contains("Signed in to HaryX as you@example.com (GitHub).")
    );
}

#[test]
fn login_offers_every_method_on_the_wecoap_page() {
    let s = Sandbox::new(200);
    let out = s.haryx(&["login", "--json"], "code=c");
    assert!(out.status.success());
    let url = json_lines(&out)[0]["url"].as_str().unwrap().to_owned();
    assert!(!url.contains("kc_idp_hint"), "{url}");
    assert!(url.contains("scope=openid%20offline_access"));
}

#[test]
fn no_browser_signs_in_with_the_device_grant() {
    let s = Sandbox::new(200);
    let out = s.haryx(&["login", "--no-browser", "--json"], "");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let events = json_lines(&out);
    assert_eq!(events[0]["event"], "browser");
    assert_eq!(events[0]["opened"], false);
    assert_eq!(
        events[0]["url"],
        format!("{}/device?user_code=WXYZ-ABCD", s.keycloak.issuer)
    );
    assert!(same_shape(&events[0], &contract("login-events.json")[0]));
    assert_eq!(events[1]["event"], "signed_in");
    assert!(!s.home().join("browser.log").exists(), "no browser started");

    let device = s.keycloak.body(&format!("{OIDC}/auth/device"));
    assert!(device.contains("&code_challenge=") && device.contains("code_challenge_method=S256"));
    let poll = s.keycloak.body(&format!("{OIDC}/token"));
    assert!(poll.starts_with(
        "grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Adevice_code&device_code=dc-test&"
    ));
    assert!(poll.contains("&code_verifier="));

    let text = s.haryx(&["logout"], "");
    assert!(text.status.success());
    let out = s.haryx(&["login", "--no-browser"], "");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("/device?user_code=WXYZ-ABCD")
            && stdout.contains("shows the code WXYZ-ABCD"),
        "{stdout}"
    );
}

#[test]
fn a_denied_sign_in_is_an_error_event_and_stores_nothing() {
    let s = Sandbox::new(200);
    let out = s.haryx(
        &["login", "--json"],
        "error=access_denied&error_description=The+user+denied+access",
    );
    assert_eq!(out.status.code(), Some(1));
    let events = json_lines(&out);
    assert_eq!(events[1]["event"], "error");
    assert_eq!(events[1]["code"], "denied");
    assert!(same_shape(&events[1], &contract("login-events.json")[2]));
    assert!(
        !s.keycloak.paths().iter().any(|p| p.ends_with("/token")),
        "no code, no exchange"
    );
    assert!(!s.stored_session().exists());
}

#[test]
fn a_refused_exchange_is_reported_in_text() {
    let s = Sandbox::new(400);
    let out = s.haryx(&["login"], "code=stale");
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("The sign-in server refused: Code not valid"),
        "{stderr}"
    );
    assert!(!s.stored_session().exists());
}

#[test]
fn logout_revokes_the_refresh_token_and_forgets_the_session() {
    let s = Sandbox::new(200);
    assert!(s.haryx(&["login"], "code=c").status.success());
    let out = s.haryx(&["logout", "--json"], "");
    assert!(out.status.success());
    assert_eq!(json_lines(&out)[0]["signedIn"], false);
    assert_eq!(
        s.keycloak.body(&format!("{OIDC}/revoke")),
        "token=rt-test&token_type_hint=refresh_token&client_id=haryx-cli"
    );
    assert!(!s.stored_session().exists());

    let again = s.haryx(&["logout"], "");
    assert!(String::from_utf8_lossy(&again.stdout).contains("was not signed in"));
}

#[test]
fn a_supabase_session_is_signed_out_and_says_to_sign_in_again() {
    let s = Sandbox::new(200);
    let path = s.stored_session();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        &path,
        r#"{"access_token":"at","refresh_token":"rt-old","expires_at":4000000000,
            "user":{"id":"u","email":"you@example.com","provider":"github"},
            "project":"https://ygvcznwlugaxqqmlsssh.supabase.co"}"#,
    )
    .unwrap();

    let text = s.haryx(&["whoami"], "");
    assert!(text.status.success());
    let stdout = String::from_utf8_lossy(&text.stdout);
    assert!(
        stdout.contains(
            "Not signed in to HaryX: HaryX now signs in with a WeCoAp ID; run 'haryx login'"
        ),
        "{stdout}"
    );
    let status = &json_lines(&s.haryx(&["whoami", "--json"], ""))[0];
    assert_eq!(status["signedIn"], false);
    assert!(same_shape(status, &whoami_without_consents()));

    assert!(s.haryx(&["logout"], "").status.success());
    assert!(!path.exists());
    assert!(s.keycloak.paths().is_empty(), "its tokens go nowhere");
}

#[test]
fn account_delete_points_at_the_wecoap_account_page() {
    let s = Sandbox::new(200);
    let out = s.haryx(&["account", "delete", "--yes", "--json"], "");
    assert_eq!(out.status.code(), Some(1));
    let answer = &json_lines(&out)[0];
    assert_eq!(answer["ok"], false);
    assert_eq!(answer["code"], "unsupported");
    let message = answer["message"].as_str().unwrap();
    assert!(
        message.contains(&format!("{}/account", s.keycloak.issuer)),
        "{message}"
    );
}

#[test]
fn claude_login_is_untouched_by_the_new_top_level_login() {
    let s = Sandbox::new(200);
    let out = s.haryx(&["login", "2"], "");
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("use 'haryx ai login 2'"));
}

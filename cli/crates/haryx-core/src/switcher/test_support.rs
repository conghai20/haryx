//! Test scaffolding for the switcher: injectable faults and a sandboxed switcher over a temp home and a
//! fake Keychain.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use serde_json::{Value, json};

use super::{JsonObject, Switcher};
use crate::credentials::test_support::{FakeKeychain, Sandbox};
use crate::error::{Error, Result};

/// Faults a test can arm: fail the Nth and later slot-credential writes, or every JSON write.
#[derive(Default)]
pub(crate) struct Faults {
    credential_writes: AtomicUsize,
    /// 0: never. Otherwise the first failing write (1-based).
    fail_credential_write_from: AtomicUsize,
    /// Fail only that one write rather than every later one.
    fail_once: AtomicBool,
    fail_write_json: AtomicBool,
}

impl Faults {
    pub fn fail_credential_writes_from(&self, nth: usize, once: bool) {
        self.credential_writes.store(0, Ordering::SeqCst);
        self.fail_credential_write_from.store(nth, Ordering::SeqCst);
        self.fail_once.store(once, Ordering::SeqCst);
    }

    pub fn fail_write_json(&self, fail: bool) {
        self.fail_write_json.store(fail, Ordering::SeqCst);
    }

    pub fn heal(&self) {
        self.fail_credential_write_from.store(0, Ordering::SeqCst);
        self.fail_write_json(false);
    }

    pub(super) fn check_credential_write(&self) -> Result<()> {
        let n = self.credential_writes.fetch_add(1, Ordering::SeqCst) + 1;
        let from = self.fail_credential_write_from.load(Ordering::SeqCst);
        let once = self.fail_once.load(Ordering::SeqCst);
        if from != 0 && (n == from || (!once && n > from)) {
            return Err(Error::Io(std::io::Error::other("disk full (injected)")));
        }
        Ok(())
    }

    pub(super) fn check_write_json(&self) -> Result<()> {
        if self.fail_write_json.load(Ordering::SeqCst) {
            return Err(Error::Io(std::io::Error::other("disk full (injected)")));
        }
        Ok(())
    }
}

/// A switcher over a sandboxed home and store.
pub(crate) struct TestSwitcher {
    pub sandbox: Sandbox,
    pub switcher: Switcher,
}

impl std::ops::Deref for TestSwitcher {
    type Target = Switcher;
    fn deref(&self) -> &Switcher {
        &self.switcher
    }
}

impl TestSwitcher {
    pub fn linux() -> Self {
        Self::build(Sandbox::linux())
    }

    pub fn macos() -> Self {
        Self::build(Sandbox::macos())
    }

    pub fn build(sandbox: Sandbox) -> Self {
        let mut switcher = Switcher::new(
            sandbox.host(),
            sandbox.keychain.clone(),
            sandbox.tunables,
            false,
        );
        switcher.session_retry_delay = Duration::ZERO;
        TestSwitcher { sandbox, switcher }
    }

    /// Rebuild the switcher after the sandbox's environment changed.
    pub fn rebuilt(self) -> Self {
        Self::build(self.sandbox)
    }

    pub fn keychain(&self) -> Arc<FakeKeychain> {
        self.sandbox.keychain.clone()
    }

    /// Set up the store directories and write `data` as the roster.
    pub fn with_roster(self, data: Value) -> Self {
        self.setup_directories().unwrap();
        self.write_roster(data);
        self
    }

    pub fn write_roster(&self, data: Value) {
        let Value::Object(map) = data else {
            panic!("roster must be an object")
        };
        self.write_json(&self.sequence_file, &map).unwrap();
    }

    pub fn roster(&self) -> JsonObject {
        self.get_sequence_data().unwrap().unwrap()
    }

    pub fn email_at(&self, num: &str) -> String {
        self.roster()["accounts"][num]["email"]
            .as_str()
            .unwrap_or_default()
            .to_owned()
    }

    /// Write the live `.claude.json` with an `oauthAccount`.
    pub fn set_live(&self, email: &str, org: &str) {
        let path = self.claude_config_path();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let config = json!({"oauthAccount": {"emailAddress": email, "accountUuid": "uuid-live",
            "organizationUuid": org}});
        std::fs::write(path, config.to_string()).unwrap();
    }
}

/// Upstream's `sample_sequence_data` fixture.
pub(crate) fn sample() -> Value {
    json!({
        "activeAccountNumber": 1,
        "lastUpdated": "2024-01-01T00:00:00Z",
        "sequence": [1, 2],
        "accounts": {
            "1": {"email": "account1@example.com", "uuid": "uuid-1", "added": "2024-01-01T00:00:00Z"},
            "2": {"email": "account2@example.com", "uuid": "uuid-2", "added": "2024-01-02T00:00:00Z"},
        },
    })
}

/// Upstream's `sample_sequence_data_with_org` fixture: one email, two orgs.
pub(crate) fn sample_with_org() -> Value {
    json!({
        "activeAccountNumber": 1,
        "lastUpdated": "2024-01-01T00:00:00Z",
        "sequence": [1, 2],
        "accounts": {
            "1": {"email": "user@example.com", "uuid": "user-uuid",
                  "organizationUuid": "org-uuid-5678", "organizationName": "Acme Corp",
                  "added": "2024-01-01T00:00:00Z"},
            "2": {"email": "user@example.com", "uuid": "user-uuid-personal",
                  "organizationUuid": "", "organizationName": "",
                  "added": "2024-01-02T00:00:00Z"},
        },
    })
}

/// An OAuth credential with a given access token and expiry.
pub(crate) fn creds(token: &str, expires_at: i64) -> String {
    json!({"claudeAiOauth": {"accessToken": token, "refreshToken": format!("rt-{token}"),
        "expiresAt": expires_at}})
    .to_string()
}

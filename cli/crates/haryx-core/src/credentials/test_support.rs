//! An in-memory Keychain and a sandboxed store, so no test touches the real Keychain or home.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tempfile::TempDir;

use super::{CredentialStore, KeychainBackend, StoreHost, Tunables};
use crate::keychain::KeychainError;
use crate::paths::{HostOs, PathEnv};

pub(crate) const ACCOUNT: &str = "tester";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Op {
    Get,
    Set,
    Delete,
}

type FailRule = Box<dyn Fn(Op, &str, &str) -> bool + Send + Sync>;
type Hook = Arc<dyn Fn(Op, &str, &str) + Send + Sync>;

/// A fake Keychain: a map of `(service, account) -> value`, a call log, an injectable failure rule and a hook
/// run before each op.
#[derive(Default)]
pub(crate) struct FakeKeychain {
    pub data: Mutex<HashMap<(String, String), String>>,
    pub calls: Mutex<Vec<(Op, String, String)>>,
    fail: Mutex<Option<FailRule>>,
    hook: Mutex<Option<Hook>>,
    pub delay: Mutex<Duration>,
}

impl FakeKeychain {
    pub fn put(&self, service: &str, value: &str) {
        self.put_for(service, ACCOUNT, value);
    }

    pub fn put_for(&self, service: &str, account: &str, value: &str) {
        self.data
            .lock()
            .unwrap()
            .insert((service.into(), account.into()), value.into());
    }

    pub fn get(&self, service: &str, account: &str) -> Option<String> {
        self.data
            .lock()
            .unwrap()
            .get(&(service.to_string(), account.to_string()))
            .cloned()
    }

    pub fn remove(&self, service: &str) {
        self.data
            .lock()
            .unwrap()
            .remove(&(service.to_string(), ACCOUNT.to_string()));
    }

    pub fn fail_when(&self, rule: impl Fn(Op, &str, &str) -> bool + Send + Sync + 'static) {
        *self.fail.lock().unwrap() = Some(Box::new(rule));
    }

    pub fn fail_all(&self) {
        self.fail_when(|_, _, _| true);
    }

    pub fn heal(&self) {
        *self.fail.lock().unwrap() = None;
    }

    pub fn on_op(&self, hook: impl Fn(Op, &str, &str) + Send + Sync + 'static) {
        *self.hook.lock().unwrap() = Some(Arc::new(hook));
    }

    pub fn clear_hook(&self) {
        *self.hook.lock().unwrap() = None;
    }

    pub fn services_seen(&self, op: Op) -> Vec<String> {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(o, _, _)| *o == op)
            .map(|(_, s, _)| s.clone())
            .collect()
    }

    pub fn calls_for(&self, op: Op, account: &str) -> usize {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(o, _, a)| *o == op && a == account)
            .count()
    }

    pub fn all_calls(&self) -> Vec<(Op, String, String)> {
        self.calls.lock().unwrap().clone()
    }

    fn enter(&self, op: Op, service: &str, account: &str) -> Result<(), KeychainError> {
        self.calls
            .lock()
            .unwrap()
            .push((op, service.into(), account.into()));
        let delay = *self.delay.lock().unwrap();
        if !delay.is_zero() {
            std::thread::sleep(delay);
        }
        // Cloned out so a hook may re-enter the fake without deadlocking on this mutex.
        let hook = self.hook.lock().unwrap().clone();
        if let Some(hook) = hook {
            hook(op, service, account);
        }
        let failing = self
            .fail
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|rule| rule(op, service, account));
        if failing {
            return Err(KeychainError::Failed(
                "security failed (rc=36): locked".into(),
            ));
        }
        Ok(())
    }
}

impl KeychainBackend for FakeKeychain {
    fn get_password(&self, service: &str, account: &str) -> Result<Option<String>, KeychainError> {
        self.enter(Op::Get, service, account)?;
        Ok(self.get(service, account))
    }

    fn set_password(
        &self,
        service: &str,
        account: &str,
        password: &str,
    ) -> Result<(), KeychainError> {
        self.enter(Op::Set, service, account)?;
        self.put_for(service, account, password);
        Ok(())
    }

    fn delete_password(&self, service: &str, account: &str) -> Result<(), KeychainError> {
        self.enter(Op::Delete, service, account)?;
        self.data
            .lock()
            .unwrap()
            .remove(&(service.to_string(), account.to_string()));
        Ok(())
    }
}

/// A sandbox: a temp home, a store root inside it, a fake Keychain.
pub(crate) struct Sandbox {
    /// Held so the directory lives as long as the sandbox.
    _tmp: TempDir,
    pub env: PathEnv,
    pub keychain: Arc<FakeKeychain>,
    pub platform: HostOs,
    pub tunables: Tunables,
}

impl Sandbox {
    pub fn macos() -> Self {
        Self::on(HostOs::MacOs)
    }

    pub fn linux() -> Self {
        Self::on(HostOs::Linux)
    }

    fn on(platform: HostOs) -> Self {
        let tmp = TempDir::new().unwrap();
        // Canonical, so a path given as CLAUDE_CONFIG_DIR hashes the same string the test computes.
        let home = crate::fsutil::canonicalize(tmp.path()).unwrap();
        let mut env = PathEnv::with_home(&home);
        env.os = platform;
        Sandbox {
            _tmp: tmp,
            env,
            keychain: Arc::new(FakeKeychain::default()),
            platform,
            tunables: Tunables {
                active_read_retry_delay: Duration::ZERO,
                ..Tunables::default()
            },
        }
    }

    pub fn home(&self) -> PathBuf {
        self.env.home.clone()
    }

    pub fn dir(&self, name: &str) -> PathBuf {
        let path = self.home().join(name);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    pub fn set_config_dir(&mut self, dir: &Path) {
        self.env.claude_config_dir = Some(dir.as_os_str().to_owned());
    }

    pub fn set_secure_dir(&mut self, dir: Option<&Path>) {
        self.env.secure_storage_dir =
            Some(dir.map_or_else(Default::default, |d| d.as_os_str().to_owned()));
    }

    pub fn store_root(&self) -> PathBuf {
        self.home().join("store")
    }

    pub fn host(&self) -> StoreHost {
        let root = self.store_root();
        StoreHost {
            platform: self.platform,
            credentials_dir: root.join("credentials"),
            sequence_file: root.join("sequence.json"),
            backup_service: super::HARYX_BACKUP_SERVICE.into(),
            keychain_account: ACCOUNT.into(),
            env: self.env.clone(),
        }
    }

    pub fn store(&self) -> CredentialStore {
        CredentialStore::with_keychain(self.host(), self.keychain.clone(), self.tunables)
    }

    /// The default profile's live credentials file.
    pub fn default_cred_file(&self) -> PathBuf {
        crate::paths::credentials_path_in(&self.env)
    }
}

pub(crate) fn creds(tag: &str) -> String {
    format!(
        r#"{{"claudeAiOauth": {{"accessToken": "sk-{tag}", "refreshToken": "rt-{tag}", "expiresAt": 9999999999000}}}}"#
    )
}

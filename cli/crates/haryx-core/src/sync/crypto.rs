//! The sync key: one age X25519 identity per HaryX account, wrapped by the user's passphrase.
//!
//! Every synced row is age-encrypted to the key, so the server holds ciphertext only. Row keys are
//! HMAC-SHA256 digests under a subkey of the sync key, so the server can neither read nor guess them
//! from a list of emails. Changing the passphrase re-wraps the same key; nothing else changes.
//!
//! The `keyring` row is `{"wrapped": …, "check": …}`: the wrapped key, and a digest of the key that
//! lets a machine tell, without the passphrase, whether the key it holds is still the account's.

use std::str::FromStr as _;

use age::secrecy::{ExposeSecret as _, SecretString};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use hmac::{Hmac, Mac as _};
use sha2::{Digest as _, Sha256};

use super::SyncError;

/// Shortest sync passphrase. The wrapped key sits on the server, so a short one could be guessed
/// offline by anyone who obtained the database.
pub const MIN_PASSPHRASE_LEN: usize = 12;
const HMAC_CONTEXT: &[u8] = b"haryx-sync-row-key-v1";
/// The row-key label the keyring check is derived from; no row is ever named by it.
const CHECK_LABEL: &str = "keyring-check";
/// The scrypt work factor (`log_n`) every keyring is wrapped at. The web app unwraps the same
/// keyring in the browser: Phase 0 measured 0.6–1.5 s there at 18, 12.7 s in Safari at 20, and
/// both Chrome and Safari refuse 21. age would otherwise tune it per machine with no ceiling.
const WRAP_LOG_N: u8 = 18;
/// The heaviest keyring `unwrap` opens: older ones age tuned to 19 or 20 still open.
const MAX_UNWRAP_LOG_N: u8 = 20;

#[derive(serde::Serialize, serde::Deserialize)]
struct Keyring {
    wrapped: String,
    check: String,
}

/// The key check a `keyring` row carries; `None` when the row is not one this version reads.
pub fn keyring_check(body: &str) -> Option<String> {
    serde_json::from_str::<Keyring>(body).ok().map(|k| k.check)
}

pub struct SyncKey {
    identity: age::x25519::Identity,
}

impl SyncKey {
    pub fn generate() -> Self {
        SyncKey {
            identity: age::x25519::Identity::generate(),
        }
    }

    /// The `AGE-SECRET-KEY-1…` form kept in this machine's vault.
    pub fn to_secret(&self) -> String {
        self.identity.to_string().expose_secret().to_owned()
    }

    pub fn from_secret(text: &str) -> Result<Self, SyncError> {
        let identity = age::x25519::Identity::from_str(text.trim())
            .map_err(|e| SyncError::Local(format!("the stored sync key is unreadable: {e}")))?;
        Ok(SyncKey { identity })
    }

    /// The `keyring` row: the key encrypted with `passphrase` (age scrypt), and its check.
    pub fn wrap(&self, passphrase: &str) -> Result<String, SyncError> {
        check_passphrase(passphrase)?;
        let mut recipient = age::scrypt::Recipient::new(SecretString::from(passphrase.to_owned()));
        recipient.set_work_factor(WRAP_LOG_N);
        let cipher = age::encrypt(&recipient, self.to_secret().as_bytes())
            .map_err(|e| SyncError::Local(format!("could not wrap the sync key: {e}")))?;
        let ring = Keyring {
            wrapped: STANDARD.encode(cipher),
            check: self.check(),
        };
        serde_json::to_string(&ring).map_err(|e| SyncError::Local(e.to_string()))
    }

    /// The key from a `keyring` row. A wrong passphrase is [`SyncError::WrongPassphrase`]; a row
    /// wrapped above [`MAX_UNWRAP_LOG_N`] is refused before any scrypt work is done.
    pub fn unwrap(body: &str, passphrase: &str) -> Result<Self, SyncError> {
        let unreadable = || SyncError::Server("the keyring row is not one this HaryX reads".into());
        let ring: Keyring = serde_json::from_str(body).map_err(|_| unreadable())?;
        let cipher = STANDARD
            .decode(ring.wrapped.trim())
            .map_err(|_| unreadable())?;
        let mut identity = age::scrypt::Identity::new(SecretString::from(passphrase.to_owned()));
        identity.set_max_work_factor(MAX_UNWRAP_LOG_N);
        let plain = age::decrypt(&identity, &cipher).map_err(|e| match e {
            age::DecryptError::ExcessiveWork { required, .. } => SyncError::Local(format!(
                "Your sync keyring was locked with a scrypt work factor of {required}, more than \
                 HaryX opens (up to {MAX_UNWRAP_LOG_N}). On a machine where sync is still \
                 unlocked, update HaryX and run 'haryx sync passphrase' to lock it again."
            )),
            _ => SyncError::WrongPassphrase,
        })?;
        let text = String::from_utf8(plain).map_err(|_| SyncError::WrongPassphrase)?;
        let key = Self::from_secret(&text)?;
        if key.check() != ring.check {
            return Err(unreadable());
        }
        Ok(key)
    }

    /// Identifies this key without revealing it: equal checks, same key.
    pub fn check(&self) -> String {
        self.row_key(CHECK_LABEL)
    }

    /// `plain` encrypted to this key, base64: a row body.
    pub fn seal(&self, plain: &[u8]) -> Result<String, SyncError> {
        let cipher = age::encrypt(&self.identity.to_public(), plain)
            .map_err(|e| SyncError::Local(format!("could not encrypt: {e}")))?;
        Ok(STANDARD.encode(cipher))
    }

    /// A row body opened. `None`: it was not written with this key (or not by HaryX at all).
    pub fn open(&self, body: &str) -> Option<Vec<u8>> {
        let cipher = STANDARD.decode(body.trim()).ok()?;
        age::decrypt(&self.identity, &cipher).ok()
    }

    /// `plain` encrypted to this key, as raw age bytes: a profile file in storage.
    pub fn seal_bytes(&self, plain: &[u8]) -> Result<Vec<u8>, SyncError> {
        age::encrypt(&self.identity.to_public(), plain)
            .map_err(|e| SyncError::Local(format!("could not encrypt: {e}")))
    }

    pub fn open_bytes(&self, cipher: &[u8]) -> Option<Vec<u8>> {
        age::decrypt(&self.identity, cipher).ok()
    }

    /// The opaque row key for `label` (for example `account:you@example.com|org`): lowercase hex.
    pub fn row_key(&self, label: &str) -> String {
        let subkey = Sha256::new()
            .chain_update(HMAC_CONTEXT)
            .chain_update(self.to_secret().as_bytes())
            .finalize();
        let mut mac =
            Hmac::<Sha256>::new_from_slice(&subkey).expect("HMAC accepts a key of any length");
        mac.update(label.as_bytes());
        mac.finalize()
            .into_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }
}

pub fn check_passphrase(passphrase: &str) -> Result<(), SyncError> {
    if passphrase.chars().count() < MIN_PASSPHRASE_LEN {
        return Err(SyncError::Invalid(format!(
            "The sync passphrase must have at least {MIN_PASSPHRASE_LEN} characters"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const PASS: &str = "correct horse battery";

    #[test]
    fn rows_round_trip_and_a_stranger_key_cannot_open_them() {
        let key = SyncKey::generate();
        let body = key.seal(b"{\"a\":1}").unwrap();
        assert!(!body.contains("\"a\""));
        assert_eq!(key.open(&body).unwrap(), b"{\"a\":1}");
        let other = SyncKey::generate();
        assert!(other.open(&body).is_none());
        assert!(key.open("not base64 at all!").is_none());
    }

    #[test]
    fn the_wrapped_key_opens_only_with_its_passphrase() {
        let key = SyncKey::generate();
        let wrapped = key.wrap(PASS).unwrap();
        assert!(!wrapped.contains(&key.to_secret()));
        let back = SyncKey::unwrap(&wrapped, PASS).unwrap();
        assert_eq!(back.to_secret(), key.to_secret());
        assert_eq!(keyring_check(&wrapped), Some(key.check()));
        assert_ne!(key.check(), SyncKey::generate().check());
        assert!(matches!(
            SyncKey::unwrap(&wrapped, "wrong horse battery"),
            Err(SyncError::WrongPassphrase)
        ));
    }

    /// The scrypt `log_n` in a keyring row's age header.
    fn header_log_n(body: &str) -> u8 {
        let ring: Keyring = serde_json::from_str(body).unwrap();
        let cipher = STANDARD.decode(ring.wrapped).unwrap();
        let text = String::from_utf8_lossy(&cipher);
        let line = text.lines().find(|l| l.starts_with("-> scrypt ")).unwrap();
        line.rsplit(' ').next().unwrap().parse().unwrap()
    }

    /// A keyring row whose header claims `log_n`, wrapped at 2 so no test pays for real scrypt
    /// work. age checks the work factor before deriving anything, so the claim is all it reads.
    fn keyring_claiming(key: &SyncKey, log_n: u8) -> String {
        let mut recipient = age::scrypt::Recipient::new(SecretString::from(PASS.to_owned()));
        recipient.set_work_factor(2);
        let cipher = age::encrypt(&recipient, key.to_secret().as_bytes()).unwrap();
        let mut text = cipher;
        let at = text.windows(10).position(|w| w == b"-> scrypt ").unwrap();
        let end = at + text[at..].iter().position(|&b| b == b'\n').unwrap();
        assert_eq!(&text[end - 2..end], b" 2");
        text.splice(end - 1..end, log_n.to_string().into_bytes());
        serde_json::to_string(&Keyring {
            wrapped: STANDARD.encode(text),
            check: key.check(),
        })
        .unwrap()
    }

    #[test]
    fn wrap_pins_the_work_factor_at_18() {
        let wrapped = SyncKey::generate().wrap(PASS).unwrap();
        assert_eq!(header_log_n(&wrapped), 18);
    }

    #[test]
    fn a_keyring_above_20_is_refused_and_one_at_or_below_still_opens() {
        let key = SyncKey::generate();
        let cheap = keyring_claiming(&key, 2);
        assert_eq!(header_log_n(&cheap), 2);
        let back = SyncKey::unwrap(&cheap, PASS).unwrap();
        assert_eq!(back.to_secret(), key.to_secret());

        let heavy = keyring_claiming(&key, 21);
        assert_eq!(header_log_n(&heavy), 21);
        match SyncKey::unwrap(&heavy, PASS) {
            Err(SyncError::Local(msg)) => {
                assert!(msg.contains("21"), "{msg}");
                assert!(msg.contains("haryx sync passphrase"), "{msg}");
            }
            Err(other) => panic!("expected the work-factor refusal, got {other}"),
            Ok(_) => panic!("a log_n 21 keyring was opened"),
        }
    }

    #[test]
    fn short_passphrases_are_refused() {
        assert!(matches!(
            SyncKey::generate().wrap("short"),
            Err(SyncError::Invalid(_))
        ));
    }

    #[test]
    fn the_secret_form_round_trips() {
        let key = SyncKey::generate();
        let back = SyncKey::from_secret(&key.to_secret()).unwrap();
        assert_eq!(back.row_key("x"), key.row_key("x"));
        assert!(SyncKey::from_secret("nonsense").is_err());
    }

    #[test]
    fn row_keys_are_stable_hex_and_key_dependent() {
        let key = SyncKey::generate();
        let a = key.row_key("account:you@example.com|");
        assert_eq!(a, key.row_key("account:you@example.com|"));
        assert_eq!(a.len(), 64);
        assert!(
            a.bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        );
        assert_ne!(a, key.row_key("account:me@example.com|"));
        assert_ne!(a, SyncKey::generate().row_key("account:you@example.com|"));
    }
}

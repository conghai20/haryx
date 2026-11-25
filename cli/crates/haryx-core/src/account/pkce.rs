//! PKCE (RFC 7636): a one-time verifier and its S256 challenge.
//!
//! The challenge goes into the sign-in URL; the verifier never leaves this process until the code
//! exchange. A code caught by any other local listener is useless without it.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::{Digest, Sha256};

/// Bytes of randomness in a verifier: 32 bytes give the 43-character minimum RFC 7636 allows.
const VERIFIER_BYTES: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pkce {
    pub verifier: String,
    pub challenge: String,
}

impl Pkce {
    /// A fresh verifier from the thread CSPRNG, and its challenge.
    pub fn generate() -> Self {
        Self::from_verifier(URL_SAFE_NO_PAD.encode(rand::random::<[u8; VERIFIER_BYTES]>()))
    }

    fn from_verifier(verifier: String) -> Self {
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        Pkce {
            verifier,
            challenge,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn challenge_is_the_rfc_7636_example() {
        // RFC 7636 appendix B.
        let pkce = Pkce::from_verifier("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk".into());
        assert_eq!(
            pkce.challenge,
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn generated_verifiers_are_url_safe_43_chars_and_unique() {
        let a = Pkce::generate();
        let b = Pkce::generate();
        assert_eq!(a.verifier.len(), 43);
        assert!(
            a.verifier
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
        );
        assert_ne!(a.verifier, b.verifier);
        assert_eq!(a, Pkce::from_verifier(a.verifier.clone()));
    }
}

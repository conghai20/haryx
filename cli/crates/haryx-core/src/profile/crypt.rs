//! Passphrase encryption of a profile's secrets (age, scrypt recipient). Files decrypt with the `age` CLI
//! too: `age -d secrets.age`.

use std::io::{Read, Write};

use age::secrecy::SecretString;

use crate::error::{Error, Result};

/// Shortest passphrase accepted for an export.
pub const MIN_PASSPHRASE_LEN: usize = 8;

pub fn encrypt(plain: &[u8], passphrase: &str) -> Result<Vec<u8>> {
    if passphrase.chars().count() < MIN_PASSPHRASE_LEN {
        return Err(Error::Validation(format!(
            "The passphrase must have at least {MIN_PASSPHRASE_LEN} characters"
        )));
    }
    let encryptor = age::Encryptor::with_user_passphrase(SecretString::from(passphrase.to_owned()));
    let mut out = Vec::new();
    let mut writer = encryptor
        .wrap_output(&mut out)
        .map_err(|e| Error::General(format!("Could not encrypt: {e}")))?;
    writer.write_all(plain)?;
    writer.finish()?;
    Ok(out)
}

pub fn decrypt(cipher: &[u8], passphrase: &str) -> Result<Vec<u8>> {
    let decryptor = age::Decryptor::new(cipher)
        .map_err(|e| Error::Validation(format!("Not an encrypted profile part: {e}")))?;
    let identity = age::scrypt::Identity::new(SecretString::from(passphrase.to_owned()));
    let mut reader = decryptor
        .decrypt(std::iter::once(&identity as &dyn age::Identity))
        .map_err(|_| Error::Validation("Wrong passphrase".into()))?;
    let mut out = Vec::new();
    reader.read_to_end(&mut out)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_rejects_wrong_passphrase() {
        let cipher = encrypt(b"hello", "correct horse").unwrap();
        assert_eq!(decrypt(&cipher, "correct horse").unwrap(), b"hello");
        assert!(decrypt(&cipher, "wrong one!").is_err());
    }

    #[test]
    fn refuses_short_passphrase() {
        assert!(encrypt(b"x", "short").is_err());
    }
}

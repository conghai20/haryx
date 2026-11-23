//! The on-disk form of a credential file (slot backups, their `.prev`, unclaimed stash entries).
//!
//! macOS and Linux keep upstream's form: the credential base64-encoded, protected only by the file's
//! 0600 mode. Windows has no file modes worth relying on, so it seals the credential with DPAPI for the
//! current user (`CryptProtectData`) and stores `dpapi1:` followed by the base64 of the sealed bytes.
//! Only the same Windows user on the same machine can open it, which matches the store living in the
//! local, not roaming, profile.
//!
//! Reading accepts both forms everywhere, so a plain file written before sealing existed (or copied in
//! from another OS) still opens on Windows. A sealed file read anywhere but Windows is an error, never
//! decoded as base64 garbage.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;

/// Marks a DPAPI-sealed file. The digit versions the format.
pub const DPAPI_PREFIX: &str = "dpapi1:";

/// The text to write for `credentials` on this OS.
pub fn seal(credentials: &str) -> std::io::Result<String> {
    #[cfg(windows)]
    {
        let sealed = dpapi::protect(credentials.as_bytes())?;
        Ok(format!("{DPAPI_PREFIX}{}", STANDARD.encode(sealed)))
    }
    #[cfg(not(windows))]
    {
        Ok(STANDARD.encode(credentials.as_bytes()))
    }
}

/// The credential inside a file's contents (surrounding whitespace ignored), in either form.
pub fn unseal(contents: &[u8]) -> Result<String, String> {
    let contents = contents.trim_ascii();
    let bytes = match contents.strip_prefix(DPAPI_PREFIX.as_bytes()) {
        Some(sealed) => open_sealed(&decode(sealed)?)?,
        None => decode(contents)?,
    };
    String::from_utf8(bytes).map_err(|e| e.to_string())
}

/// Strict base64: junk is rejected, not skipped, so a corrupt file cannot pass as a credential.
fn decode(encoded: &[u8]) -> Result<Vec<u8>, String> {
    STANDARD.decode(encoded).map_err(|e| e.to_string())
}

#[cfg(windows)]
fn open_sealed(sealed: &[u8]) -> Result<Vec<u8>, String> {
    dpapi::unprotect(sealed).map_err(|e| format!("cannot unseal (DPAPI): {e}"))
}

#[cfg(not(windows))]
fn open_sealed(_sealed: &[u8]) -> Result<Vec<u8>, String> {
    Err("this file was sealed by Windows DPAPI and opens only for its Windows user".into())
}

#[cfg(windows)]
mod dpapi {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{
        CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
    };

    pub fn protect(plain: &[u8]) -> std::io::Result<Vec<u8>> {
        transform(plain, true)
    }

    pub fn unprotect(sealed: &[u8]) -> std::io::Result<Vec<u8>> {
        transform(sealed, false)
    }

    fn transform(input: &[u8], protect: bool) -> std::io::Result<Vec<u8>> {
        let len = u32::try_from(input.len())
            .map_err(|_| std::io::Error::other("credential too large for DPAPI"))?;
        let blob_in = CRYPT_INTEGER_BLOB {
            cbData: len,
            pbData: input.as_ptr().cast_mut(),
        };
        let mut blob_out = CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: std::ptr::null_mut(),
        };
        // SAFETY: `blob_in` points at `input`, which outlives the call and is only read. Optional
        // arguments are null. On success DPAPI allocates `blob_out` with LocalAlloc; it is copied out and
        // freed below.
        let ok = unsafe {
            if protect {
                CryptProtectData(
                    &blob_in,
                    std::ptr::null(),
                    std::ptr::null(),
                    std::ptr::null(),
                    std::ptr::null(),
                    CRYPTPROTECT_UI_FORBIDDEN,
                    &mut blob_out,
                )
            } else {
                CryptUnprotectData(
                    &blob_in,
                    std::ptr::null_mut(),
                    std::ptr::null(),
                    std::ptr::null(),
                    std::ptr::null(),
                    CRYPTPROTECT_UI_FORBIDDEN,
                    &mut blob_out,
                )
            }
        };
        if ok == 0 {
            return Err(std::io::Error::last_os_error());
        }
        // SAFETY: on success `blob_out` holds `cbData` initialised bytes at `pbData`.
        let out = unsafe {
            std::slice::from_raw_parts(blob_out.pbData, blob_out.cbData as usize).to_vec()
        };
        // SAFETY: `pbData` was allocated by DPAPI with LocalAlloc and is freed exactly once.
        unsafe { LocalFree(blob_out.pbData.cast()) };
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CREDS: &str = r#"{"claudeAiOauth":{"accessToken":"sk-ant-oat01-x","refreshToken":"r"}}"#;

    #[test]
    fn round_trips_on_this_os() {
        let sealed = seal(CREDS).unwrap();
        assert_ne!(sealed, CREDS, "never stored as plain text");
        assert_eq!(unseal(sealed.as_bytes()).unwrap(), CREDS);
    }

    #[test]
    fn reads_plain_base64_from_before_sealing() {
        let plain = STANDARD.encode(CREDS);
        assert_eq!(unseal(format!("  {plain}\n").as_bytes()).unwrap(), CREDS);
    }

    #[test]
    fn rejects_corrupt_contents() {
        assert!(unseal(b"not base64 !!").is_err());
        assert!(unseal(b"dpapi1:not base64 !!").is_err());
        assert!(unseal(STANDARD.encode([0xff, 0xfe]).as_bytes()).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn windows_seals_with_dpapi() {
        let sealed = seal(CREDS).unwrap();
        assert!(sealed.starts_with(DPAPI_PREFIX));
        assert!(!sealed.contains(&STANDARD.encode(CREDS)));
    }

    #[cfg(windows)]
    #[test]
    fn windows_rejects_tampered_seal() {
        let sealed = seal(CREDS).unwrap();
        let mut bytes = decode(&sealed.as_bytes()[DPAPI_PREFIX.len()..]).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 0xff;
        let tampered = format!("{DPAPI_PREFIX}{}", STANDARD.encode(bytes));
        assert!(unseal(tampered.as_bytes()).is_err());
    }

    #[cfg(not(windows))]
    #[test]
    fn non_windows_refuses_a_dpapi_file() {
        let fake = format!("{DPAPI_PREFIX}{}", STANDARD.encode(b"opaque"));
        let err = unseal(fake.as_bytes()).unwrap_err();
        assert!(err.contains("Windows"), "{err}");
    }
}

//! This machine as HaryX names it to its server and to the user's other machines: a random id, a
//! name and its system.

use std::path::Path;

/// File under the store's cache directory holding [`machine_id`].
const MACHINE_ID_FILE: &str = "machine-id";
/// The API's limit on a machine name, in characters (`machines_name_check`).
const MAX_NAME: usize = 64;

/// This machine's id: a random lowercase hyphenated UUID (v4), made on first use and kept in the
/// store, so it says nothing about the machine. The machine registry (`PUT /machines/self`), the
/// usage share's leases and the per-machine usage rows (`usage:<machine_id>:<yyyy-mm>`) all name
/// this machine by it.
///
/// Before the machine registry it was 16 hex characters; such an id is replaced by a UUID. No
/// server holds the old one: sign-in, and with it the usage share, was never released.
pub fn machine_id(store_root: &Path) -> String {
    let path = store_root.join("cache").join(MACHINE_ID_FILE);
    if let Ok(text) = std::fs::read_to_string(&path)
        && is_uuid(text.trim())
    {
        return text.trim().to_owned();
    }
    // ponytail: two processes making the id at the same moment each write their own and the last
    // one stays; the loser's single heartbeat leaves one stale machine row, renamed or withdrawn
    // from the web.
    let id = new_uuid();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Err(err) = std::fs::write(&path, &id) {
        log::warn!("could not save this machine's id: {err}");
    }
    id
}

/// Lowercase and hyphenated, as the API requires.
fn is_uuid(text: &str) -> bool {
    text.len() == 36
        && text.bytes().enumerate().all(|(i, b)| match i {
            8 | 13 | 18 | 23 => b == b'-',
            _ => b.is_ascii_digit() || (b'a'..=b'f').contains(&b),
        })
}

fn new_uuid() -> String {
    let mut b: [u8; 16] = rand::random();
    b[6] = (b[6] & 0x0f) | 0x40; // version 4
    b[8] = (b[8] & 0x3f) | 0x80; // RFC 4122 variant
    let hex: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    )
}

/// This machine's system, as the API names it.
pub fn os() -> &'static str {
    if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(windows) {
        "windows"
    } else {
        "linux"
    }
}

/// The name a machine is registered with on its first heartbeat: the host name, without macOS's
/// `.local`, cut to what the API takes. A rename (CLI or web) replaces it for good.
pub fn default_name() -> String {
    hostname()
        .map(|name| clean_name(&name))
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| os().to_owned())
}

fn clean_name(name: &str) -> String {
    let name = name.trim();
    name.strip_suffix(".local")
        .unwrap_or(name)
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_NAME)
        .collect::<String>()
        .trim()
        .to_owned()
}

#[cfg(unix)]
fn hostname() -> Option<String> {
    let mut buf = [0u8; 256];
    // SAFETY: `buf` is writable for the length passed, and outlives the call.
    let rc = unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) };
    if rc != 0 {
        return None;
    }
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    Some(String::from_utf8_lossy(&buf[..end]).into_owned())
}

#[cfg(windows)]
fn hostname() -> Option<String> {
    std::env::var("COMPUTERNAME").ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_id_is_a_uuid_kept_across_calls() {
        let dir = tempfile::tempdir().unwrap();
        let id = machine_id(dir.path());
        assert!(is_uuid(&id), "{id}");
        assert_eq!(&id[14..15], "4");
        assert_eq!(machine_id(dir.path()), id);
    }

    #[test]
    fn an_old_hex_id_is_replaced_by_a_uuid() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("cache")).unwrap();
        std::fs::write(dir.path().join("cache/machine-id"), "0123456789abcdef").unwrap();
        let id = machine_id(dir.path());
        assert!(is_uuid(&id), "{id}");
        let saved = std::fs::read_to_string(dir.path().join("cache/machine-id")).unwrap();
        assert_eq!(saved, id);
    }

    #[test]
    fn uuids_are_lowercase_and_hyphenated() {
        assert!(is_uuid("9b2e5c1a-4f0d-4a67-8d3e-1c7a5b0e2f91"));
        assert!(!is_uuid("9B2E5C1A-4F0D-4A67-8D3E-1C7A5B0E2F91"));
        assert!(!is_uuid("9b2e5c1a4f0d4a678d3e1c7a5b0e2f91"));
        assert!(!is_uuid("0123456789abcdef"));
    }

    #[test]
    fn names_lose_local_control_characters_and_length() {
        assert_eq!(clean_name("Hais-MacBook-Pro.local\n"), "Hais-MacBook-Pro");
        assert_eq!(clean_name(&"x".repeat(80)).chars().count(), MAX_NAME);
        assert_eq!(clean_name("a\u{1b}b"), "ab");
        assert!(!default_name().is_empty());
    }
}

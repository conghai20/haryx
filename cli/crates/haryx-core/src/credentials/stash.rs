//! Internal safety copies of unclaimed credentials.
//!
//! Write-only preservation for live credential bytes a switch positively attributed to someone other than
//! the outgoing slot (never overwrite the live store without preserving what was in it: the bytes may be the
//! only live copy of some account's refresh token). Entries are append-only base64 files plus a JSON manifest
//! carrying the classification evidence; nothing consumes them automatically except the consume gate's
//! adopt path.
//!
//! Deliberately 0600 files on every platform, never the Keychain: a failed safety-copy write aborts the
//! switch by design, and that abort path must not inherit the Keychain's failure modes.

use std::path::PathBuf;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use rand::RngCore as _;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use super::CredentialStore;
use crate::fsutil::{self, WriteOptions};
use crate::locking::{DEFAULT_LOCK_TIMEOUT, FileLock};
use crate::{Error, Result};

const MANIFEST_NAME: &str = ".unclaimed-manifest.json";
const MANIFEST_LOCK_NAME: &str = ".unclaimed-manifest.lock";
const ENTRY_PREFIX: &str = ".unclaimed-";
const ENTRY_SUFFIX: &str = ".enc";

/// The three-way verdict of a manifest read.
///
/// "No rows" and "could not establish the rows" have opposite consequences: a stash row is the SOLE record of
/// a generation a prior gate pass already consumed, so reading a failure as "nothing stashed" POSTs the spent
/// generation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ManifestVerdict {
    /// Read fine, including a genuinely absent manifest.
    Ok,
    /// The bytes exist and could not be read. Transient: every caller defers.
    Unreadable,
    /// The bytes were read and are not a manifest. Permanent: the repair is the set-aside on the next write,
    /// so callers decide by whether entry bytes are at risk ([`CredentialStore::stash_entry_files_exist`]).
    Corrupt,
}

impl CredentialStore {
    fn stash_manifest_path(&self) -> PathBuf {
        self.host.credentials_dir.join(MANIFEST_NAME)
    }

    fn stash_entry_path(&self, entry_id: &str) -> PathBuf {
        self.host
            .credentials_dir
            .join(format!("{ENTRY_PREFIX}{entry_id}{ENTRY_SUFFIX}"))
    }

    /// The manifest's entries with a verdict. Reads BYTES so undecodable content is `Corrupt`, never an I/O
    /// failure; a parseable document without an object `entries` member is `Corrupt` too.
    pub fn read_stash_manifest_ex(&self) -> (Map<String, Value>, ManifestVerdict) {
        let raw = match std::fs::read(self.stash_manifest_path()) {
            Ok(raw) => raw,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return (Map::new(), ManifestVerdict::Ok);
            }
            Err(err) => {
                log::warn!("Unclaimed manifest unreadable: {err}");
                return (Map::new(), ManifestVerdict::Unreadable);
            }
        };
        let parsed = std::str::from_utf8(&raw)
            .map_err(|e| e.to_string())
            .and_then(|text| serde_json::from_str::<Value>(text).map_err(|e| e.to_string()));
        match parsed {
            Err(err) => {
                log::warn!("Failed to read unclaimed manifest: {err}");
                (Map::new(), ManifestVerdict::Corrupt)
            }
            Ok(Value::Object(mut doc)) => match doc.remove("entries") {
                Some(Value::Object(entries)) => (entries, ManifestVerdict::Ok),
                _ => {
                    log::warn!("Unclaimed manifest parses but has no valid 'entries' member");
                    (Map::new(), ManifestVerdict::Corrupt)
                }
            },
            Ok(_) => {
                log::warn!("Failed to read unclaimed manifest: not a JSON object");
                (Map::new(), ManifestVerdict::Corrupt)
            }
        }
    }

    /// Whether any stashed credential's BYTES are on disk. Asked when the manifest is corrupt. An unlistable
    /// directory answers true (not evidence that nothing is at risk); a missing one answers false.
    pub fn stash_entry_files_exist(&self) -> bool {
        let entries = match std::fs::read_dir(&self.host.credentials_dir) {
            Ok(entries) => entries,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return false,
            Err(_) => return true,
        };
        for entry in entries {
            let Ok(entry) = entry else {
                return true;
            };
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with(ENTRY_PREFIX) && name.ends_with(ENTRY_SUFFIX) {
                return true;
            }
        }
        false
    }

    pub fn read_stash_manifest(&self) -> Map<String, Value> {
        self.read_stash_manifest_ex().0
    }

    /// Write the manifest. A corrupt existing manifest is set aside as `<name>.corrupt-<unix secs>` rather
    /// than clobbered: its rows are classification evidence.
    fn write_stash_manifest(&self, entries: Map<String, Value>) -> Result<()> {
        std::fs::create_dir_all(&self.host.credentials_dir)?;
        let path = self.stash_manifest_path();
        if path.exists() {
            let parses = std::fs::read_to_string(&path)
                .ok()
                .is_some_and(|text| serde_json::from_str::<Value>(&text).is_ok());
            if !parses {
                let secs = jiff::Timestamp::now().as_second();
                let aside = path.with_file_name(format!("{MANIFEST_NAME}.corrupt-{secs}"));
                match std::fs::rename(&path, &aside) {
                    Ok(()) => log::warn!(
                        "Unreadable unclaimed manifest preserved as {}",
                        aside.file_name().unwrap_or_default().to_string_lossy()
                    ),
                    Err(err) => log::warn!("Could not preserve corrupt unclaimed manifest: {err}"),
                }
            }
        }
        let doc = json!({"schemaVersion": 1, "entries": Value::Object(entries)});
        atomic_write_json(&path, &doc)
    }

    /// Apply `mutate` to the manifest as one atomic step, under the manifest's own lock (deliberately not
    /// the slot lock: the stash writer runs precisely when the slot lock was unavailable).
    ///
    /// Fails on a lock timeout, a write failure, or an UNREADABLE manifest: rewriting a valid-but-unreadable
    /// manifest from an empty read would orphan every stashed successor it maps. A CORRUPT one proceeds, so
    /// the set-aside can happen at all.
    pub fn mutate_stash_manifest(
        &self,
        mutate: impl FnOnce(&mut Map<String, Value>),
    ) -> Result<()> {
        std::fs::create_dir_all(&self.host.credentials_dir)?;
        let _guard = FileLock::lock(
            self.host.credentials_dir.join(MANIFEST_LOCK_NAME),
            DEFAULT_LOCK_TIMEOUT,
        )?;
        let (mut entries, verdict) = self.read_stash_manifest_ex();
        if verdict == ManifestVerdict::Unreadable {
            return Err(Error::CredentialRead(
                "the unclaimed manifest is unreadable; refusing to rewrite it from an empty read, which \
                 would orphan every stashed successor it maps"
                    .into(),
            ));
        }
        mutate(&mut entries);
        self.write_stash_manifest(entries)
    }

    /// Stash a credential of unknown provenance; returns the entry id
    /// (`<UTC %Y%m%dT%H%M%S>-<sha256[:12]>-<6 hex nonce>`).
    ///
    /// Fails on any failure: callers use a successful stash as the license to overwrite the live store. The
    /// entry file is written before the manifest: an entry without metadata is recoverable, a row without
    /// bytes is not.
    pub fn write_unclaimed_credential(
        &self,
        credentials: &str,
        context: &Map<String, Value>,
    ) -> Result<String> {
        let now = jiff::Timestamp::now();
        let digest = Sha256::digest(credentials.as_bytes());
        let digest_hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
        let mut nonce = [0u8; 3];
        rand::rng().fill_bytes(&mut nonce);
        let nonce_hex: String = nonce.iter().map(|b| format!("{b:02x}")).collect();
        let entry_id = format!(
            "{}-{}-{nonce_hex}",
            now.strftime("%Y%m%dT%H%M%S"),
            &digest_hex[..12]
        );
        std::fs::create_dir_all(&self.host.credentials_dir)?;
        let encoded = STANDARD.encode(credentials.as_bytes());
        fsutil::atomic_write_text(
            &self.stash_entry_path(&entry_id),
            &encoded,
            WriteOptions::default(),
        )?;
        let mut row = Map::new();
        row.insert(
            "createdAt".into(),
            Value::String(now.strftime("%Y-%m-%dT%H:%M:%SZ").to_string()),
        );
        row.extend(context.iter().map(|(k, v)| (k.clone(), v.clone())));
        let id = entry_id.clone();
        self.mutate_stash_manifest(move |entries| {
            entries.insert(id, Value::Object(row));
        })?;
        Ok(entry_id)
    }

    /// Manifest entries by id, plus orphaned entry files (no metadata) as `{"createdAt": null}`.
    pub fn list_unclaimed_credentials(&self) -> Map<String, Value> {
        let mut entries = self.read_stash_manifest();
        let Ok(dir) = std::fs::read_dir(&self.host.credentials_dir) else {
            return entries;
        };
        let mut orphans: Vec<String> = dir
            .filter_map(|e| e.ok())
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                let id = name
                    .strip_prefix(ENTRY_PREFIX)?
                    .strip_suffix(ENTRY_SUFFIX)?;
                Some(id.to_string())
            })
            .collect();
        orphans.sort();
        for id in orphans {
            entries
                .entry(id)
                .or_insert_with(|| json!({"createdAt": null}));
        }
        entries
    }

    /// Decode one stashed credential: `(value, unreadable)`. `unreadable` when the entry file EXISTS but could
    /// not be read (the entry is the sole copy of a consumed generation, so a transient failure is not
    /// "nothing to adopt"). Absent or corrupt entries are `("", false)`.
    pub fn read_unclaimed_credential(&self, entry_id: &str) -> (String, bool) {
        let encoded = match std::fs::read(self.stash_entry_path(entry_id)) {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return (String::new(), false);
            }
            Err(err) => {
                log::warn!("Unclaimed credential {entry_id} unreadable: {err}");
                return (String::new(), true);
            }
        };
        let decoded = STANDARD
            .decode(encoded.trim_ascii())
            .map_err(|e| e.to_string())
            .and_then(|bytes| String::from_utf8(bytes).map_err(|e| e.to_string()));
        match decoded {
            Ok(value) => (value, false),
            Err(err) => {
                log::warn!("Failed to decode unclaimed credential {entry_id}: {err}");
                (String::new(), false)
            }
        }
    }

    /// Delete a stash entry (bytes + manifest row) after it was adopted. Fails if the manifest cannot be
    /// rewritten; callers retiring an entry swallow and log that.
    pub fn remove_unclaimed_credential(&self, entry_id: &str) -> Result<()> {
        if let Err(err) = std::fs::remove_file(self.stash_entry_path(entry_id)) {
            if err.kind() != std::io::ErrorKind::NotFound {
                log::warn!("Failed to remove unclaimed credential {entry_id}: {err}");
            }
        }
        self.mutate_stash_manifest(|entries| {
            entries.remove(entry_id);
        })
    }
}

/// Upstream `settings.atomic_write_json` (settings is ported separately): write THROUGH a symlink, 0700 on
/// the directory the store owns, 0600 file, `json.dumps(indent=2)` layout.
fn atomic_write_json(path: &std::path::Path, data: &Value) -> Result<()> {
    let target = fsutil::publish_target(path)?;
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    #[cfg(unix)]
    if let Some(parent) = path.parent() {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))?;
    }
    let text = serde_json::to_string_pretty(data)?;
    fsutil::atomic_write_text(&target, &text, WriteOptions::default())?;
    Ok(())
}

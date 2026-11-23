//! The `.haryx-profile` file: a gzip tar whose first entry is `manifest.json`.
//!
//! The reader treats the file as untrusted: every entry must be a plain relative path to a regular file
//! or directory, listed in the manifest with a matching hash, and the extracted total may not exceed what
//! the manifest declares.

use std::io::{Read, Write};
use std::path::{Component, Path};

use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;

use super::manifest::{MANIFEST_FILE, Manifest};
use super::walk;
use crate::error::{Error, Result};

/// Largest manifest read before anything else.
const MAX_MANIFEST_BYTES: u64 = 64 * 1024 * 1024;
/// Hard limits that hold whatever the manifest claims (decompression bombs).
const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_TOTAL_BYTES: u64 = 16 * 1024 * 1024 * 1024;
const MAX_ENTRIES: usize = 1_000_000;

/// Pack a profile directory into `out` (written to a temp file beside it, then renamed).
pub fn write(profile_dir: &Path, manifest: &Manifest, out: &Path) -> Result<()> {
    let tmp = out.with_extension("partial");
    let result = write_to(profile_dir, manifest, &tmp);
    match result {
        Ok(()) => {
            std::fs::rename(&tmp, out)?;
            Ok(())
        }
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

fn write_to(profile_dir: &Path, manifest: &Manifest, tmp: &Path) -> Result<()> {
    walk::remove_any(tmp)?;
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(tmp)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(tmp, std::fs::Permissions::from_mode(0o600))?;
    }
    let mut builder = tar::Builder::new(GzEncoder::new(file, Compression::default()));
    builder.mode(tar::HeaderMode::Deterministic);
    let manifest_bytes = serde_json::to_vec_pretty(manifest)?;
    append_bytes(&mut builder, MANIFEST_FILE, &manifest_bytes, 0o600)?;
    for rel in manifest.files.keys() {
        let path = profile_dir.join(rel);
        let data = std::fs::read(&path)
            .map_err(|e| Error::General(format!("Cannot read {}: {e}", path.display())))?;
        append_bytes(&mut builder, rel, &data, file_mode(&path))?;
    }
    let encoder = builder.into_inner()?;
    let mut file = encoder.finish()?;
    file.flush()?;
    file.sync_all()?;
    Ok(())
}

fn file_mode(path: &Path) -> u32 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let exec = std::fs::metadata(path)
            .map(|m| m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false);
        if exec { 0o700 } else { 0o600 }
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        0o600
    }
}

fn append_bytes<W: Write>(
    builder: &mut tar::Builder<W>,
    rel: &str,
    data: &[u8],
    mode: u32,
) -> Result<()> {
    let mut header = tar::Header::new_gnu();
    header.set_size(data.len() as u64);
    header.set_mode(mode);
    header.set_entry_type(tar::EntryType::Regular);
    builder.append_data(&mut header, rel, data)?;
    Ok(())
}

/// Read only the manifest of an exported file.
pub fn read_manifest(file: &Path) -> Result<Manifest> {
    let mut archive = open(file)?;
    let mut entries = archive.entries()?;
    first_manifest(&mut entries)
}

fn open(file: &Path) -> Result<tar::Archive<GzDecoder<std::fs::File>>> {
    let handle = std::fs::File::open(file)
        .map_err(|e| Error::General(format!("Cannot open {}: {e}", file.display())))?;
    Ok(tar::Archive::new(GzDecoder::new(handle)))
}

fn first_manifest<R: Read>(entries: &mut tar::Entries<'_, R>) -> Result<Manifest> {
    let entry = entries
        .next()
        .ok_or_else(|| Error::Validation("Empty profile file".into()))?
        .map_err(|e| Error::Validation(format!("Not a haryx profile file: {e}")))?;
    let path = entry.path()?.to_string_lossy().into_owned();
    if path != MANIFEST_FILE {
        return Err(Error::Validation(
            "Not a haryx profile file (no manifest first)".into(),
        ));
    }
    if entry.size() > MAX_MANIFEST_BYTES {
        return Err(Error::Validation("Profile manifest is too large".into()));
    }
    let mut text = String::new();
    entry
        .take(MAX_MANIFEST_BYTES)
        .read_to_string(&mut text)
        .map_err(|e| Error::Validation(format!("Unreadable profile manifest: {e}")))?;
    Manifest::parse(&text)
}

/// Extract an exported file into `dest` (which must not exist), checking every entry.
pub fn extract(file: &Path, dest: &Path) -> Result<Manifest> {
    if dest.exists() {
        return Err(Error::General(format!("{} already exists", dest.display())));
    }
    let staging = dest.with_extension("extracting");
    walk::remove_any(&staging)?;
    walk::mkdirs(&staging)?;
    match extract_into(file, &staging) {
        Ok(manifest) => {
            std::fs::rename(&staging, dest)?;
            Ok(manifest)
        }
        Err(e) => {
            let _ = walk::remove_any(&staging);
            Err(e)
        }
    }
}

fn extract_into(file: &Path, staging: &Path) -> Result<Manifest> {
    let mut archive = open(file)?;
    let mut entries = archive.entries()?;
    let manifest = first_manifest(&mut entries)?;
    if manifest.total_bytes > MAX_TOTAL_BYTES || manifest.files.len() > MAX_ENTRIES {
        return Err(Error::Validation(
            "Profile is larger than haryx accepts".into(),
        ));
    }
    let mut seen = std::collections::BTreeSet::new();
    let mut total: u64 = 0;
    let mut count: usize = 0;
    for entry in entries {
        count += 1;
        if count > MAX_ENTRIES {
            return Err(Error::Validation("Profile has too many entries".into()));
        }
        let mut entry = entry.map_err(|e| Error::Validation(format!("Damaged profile: {e}")))?;
        let rel = entry.path()?.to_string_lossy().into_owned();
        check_rel(&rel)?;
        match entry.header().entry_type() {
            tar::EntryType::Regular => {}
            tar::EntryType::Directory => continue,
            other => {
                return Err(Error::Validation(format!(
                    "Profile entry {rel} has a forbidden type ({other:?})"
                )));
            }
        }
        let record = manifest.files.get(&rel).ok_or_else(|| {
            Error::Validation(format!("Profile entry {rel} is not in its manifest"))
        })?;
        total = total.saturating_add(record.size);
        if total > manifest.total_bytes || record.size > MAX_FILE_BYTES {
            return Err(Error::Validation(
                "Profile is larger than its manifest declares".into(),
            ));
        }
        let path = staging.join(&rel);
        walk::ensure_parent(&path)?;
        let (written, hash) = stream_to_file(&mut entry, record.size + 1, &path)?;
        if written != record.size || hash != record.sha256 {
            return Err(Error::Validation(format!(
                "Profile entry {rel} does not match its manifest"
            )));
        }
        let mode = entry.header().mode().unwrap_or(0o600) & 0o700;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode | 0o600))?;
        }
        #[cfg(not(unix))]
        let _ = mode;
        seen.insert(rel);
    }
    if let Some(missing) = manifest.files.keys().find(|k| !seen.contains(*k)) {
        return Err(Error::Validation(format!(
            "Profile is missing {missing} listed in its manifest"
        )));
    }
    manifest.save(staging)?;
    Ok(manifest)
}

/// Copy at most `limit` bytes to a new file while hashing them: (bytes written, sha256 hex).
fn stream_to_file(reader: &mut dyn Read, limit: u64, path: &Path) -> Result<(u64, String)> {
    use sha2::{Digest, Sha256};
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    let mut hasher = Sha256::new();
    let mut limited = reader.take(limit);
    let mut buf = vec![0u8; 64 * 1024];
    let mut written: u64 = 0;
    loop {
        let n = limited.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        file.write_all(&buf[..n])?;
        written += n as u64;
    }
    let hash = hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    Ok((written, hash))
}

/// A relative path of normal components only (no `/`, `..`, drive or root).
fn check_rel(rel: &str) -> Result<()> {
    let path = Path::new(rel);
    let ok = !rel.is_empty() && path.components().all(|c| matches!(c, Component::Normal(_)));
    if ok {
        Ok(())
    } else {
        Err(Error::Validation(format!(
            "Profile entry has an unsafe path: {rel}"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::super::manifest::FileRecord;
    use super::*;

    fn sample_profile(dir: &Path) -> Manifest {
        let file = dir.join("files/claude/CLAUDE.md");
        walk::ensure_parent(&file).unwrap();
        std::fs::write(&file, "hello").unwrap();
        let mut manifest = super::super::manifest::tests::sample();
        manifest.files.insert(
            "files/claude/CLAUDE.md".into(),
            FileRecord {
                sha256: walk::sha256_hex(b"hello"),
                size: 5,
            },
        );
        manifest.total_bytes = 5;
        manifest
    }

    #[test]
    fn round_trips_a_profile() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        let manifest = sample_profile(&src);
        let out = dir.path().join("p.haryx-profile");
        write(&src, &manifest, &out).unwrap();
        assert_eq!(read_manifest(&out).unwrap().name, "work");
        let dest = dir.path().join("dest");
        extract(&out, &dest).unwrap();
        assert_eq!(
            std::fs::read_to_string(dest.join("files/claude/CLAUDE.md")).unwrap(),
            "hello"
        );
    }

    #[test]
    fn rejects_tampered_content() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        let mut manifest = sample_profile(&src);
        manifest
            .files
            .get_mut("files/claude/CLAUDE.md")
            .unwrap()
            .sha256 = "0".repeat(64);
        let out = dir.path().join("p.haryx-profile");
        write(&src, &manifest, &out).unwrap();
        let err = extract(&out, &dir.path().join("dest")).unwrap_err();
        assert!(err.to_string().contains("does not match"));
        assert!(!dir.path().join("dest").exists());
    }

    #[test]
    fn rejects_unsafe_paths() {
        assert!(check_rel("../x").is_err());
        assert!(check_rel("/etc/passwd").is_err());
        assert!(check_rel("files/claude/x").is_ok());
    }

    #[test]
    fn rejects_archive_without_manifest_first() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("bad.haryx-profile");
        let file = std::fs::File::create(&out).unwrap();
        let mut builder = tar::Builder::new(GzEncoder::new(file, Compression::default()));
        append_bytes(&mut builder, "files/x", b"x", 0o600).unwrap();
        builder.into_inner().unwrap().finish().unwrap();
        assert!(read_manifest(&out).is_err());
    }
}

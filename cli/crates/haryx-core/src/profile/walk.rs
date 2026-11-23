//! File-tree helpers: list a tree (following symlinks, with a loop guard), hash it, copy it.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use super::catalog::ignored_child;
use crate::error::{Error, Result};

/// Deepest directory nesting followed; guards against symlink loops.
const MAX_DEPTH: usize = 64;

/// One regular file of a tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileEntry {
    /// Path relative to the walked root, `/`-separated.
    pub rel: String,
    pub abs: PathBuf,
}

/// Every regular file under `root` (or `root` itself when it is a file), sorted by relative path.
/// Symlinks are followed; anything else (sockets, devices) is skipped.
pub fn files(root: &Path) -> Result<Vec<FileEntry>> {
    let meta = std::fs::metadata(root)?;
    if meta.is_file() {
        return Ok(vec![FileEntry {
            rel: String::new(),
            abs: root.to_path_buf(),
        }]);
    }
    let mut out = Vec::new();
    walk_dir(root, "", 0, &mut out)?;
    out.sort_by(|a, b| a.rel.cmp(&b.rel));
    Ok(out)
}

fn walk_dir(dir: &Path, prefix: &str, depth: usize, out: &mut Vec<FileEntry>) -> Result<()> {
    if depth > MAX_DEPTH {
        return Err(Error::General(format!(
            "{} is nested too deeply (symlink loop?)",
            dir.display()
        )));
    }
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if ignored_child(&name) {
            continue;
        }
        let path = entry.path();
        let rel = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        // Follow symlinks; a dangling link is skipped.
        let Ok(meta) = std::fs::metadata(&path) else {
            continue;
        };
        if meta.is_dir() {
            walk_dir(&path, &rel, depth + 1, out)?;
        } else if meta.is_file() {
            out.push(FileEntry { rel, abs: path });
        }
    }
    Ok(())
}

/// Total bytes of the files.
pub fn total_size(entries: &[FileEntry]) -> u64 {
    entries
        .iter()
        .filter_map(|e| std::fs::metadata(&e.abs).ok())
        .map(|m| m.len())
        .sum()
}

pub fn sha256_hex(data: &[u8]) -> String {
    let digest = Sha256::digest(data);
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// Hash of a tree given as (relative path, content hash) pairs, order-independent.
pub fn tree_hash(mut parts: Vec<(String, String)>) -> String {
    parts.sort();
    let joined: String = parts
        .iter()
        .map(|(rel, hash)| format!("{rel}\0{hash}\n"))
        .collect();
    sha256_hex(joined.as_bytes())
}

/// Create `path`'s parent directories (0700 for new ones).
pub fn ensure_parent(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        mkdirs(parent)?;
    }
    Ok(())
}

/// `mkdir -p` with 0700 for created directories.
pub fn mkdirs(dir: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)?;
    }
    #[cfg(not(unix))]
    std::fs::create_dir_all(dir)?;
    Ok(())
}

/// Write bytes to a new or existing file, creating parents (file mode 0600 for new files).
pub fn write_file(path: &Path, data: &[u8]) -> Result<()> {
    ensure_parent(path)?;
    crate::fsutil::atomic_write_bytes(
        path,
        data,
        crate::fsutil::WriteOptions {
            mode: existing_mode(path).unwrap_or(0o600),
            follow_symlinks: true,
        },
    )?;
    Ok(())
}

/// The permission bits of an existing file, so an update keeps a script executable.
fn existing_mode(path: &Path) -> Option<u32> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path)
            .ok()
            .map(|m| m.permissions().mode() & 0o777)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        None
    }
}

/// Copy permission bits from `from` onto `to` (keeps scripts executable).
pub fn copy_mode(from: &Path, to: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = std::fs::metadata(from) {
            let mode = meta.permissions().mode() & 0o777;
            let _ = std::fs::set_permissions(to, std::fs::Permissions::from_mode(mode));
        }
    }
    #[cfg(not(unix))]
    {
        let _ = (from, to);
    }
}

/// Remove a file, a directory tree, or a symlink (the link only). Missing is fine.
pub fn remove_any(path: &Path) -> Result<()> {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return Ok(());
    };
    if meta.is_dir() {
        std::fs::remove_dir_all(path)?;
    } else {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

/// Copy a file or tree from `from` to `to` byte for byte, following symlinks in the source.
pub fn copy_tree(from: &Path, to: &Path) -> Result<()> {
    let meta = std::fs::metadata(from)?;
    if meta.is_file() {
        ensure_parent(to)?;
        std::fs::copy(from, to)?;
        return Ok(());
    }
    mkdirs(to)?;
    for entry in files(from)? {
        let dest = to.join(&entry.rel);
        ensure_parent(&dest)?;
        std::fs::copy(&entry.abs, &dest)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_files_sorted_and_follows_symlinks() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        std::fs::create_dir_all(root.join("b")).unwrap();
        std::fs::write(root.join("b/two"), "2").unwrap();
        std::fs::write(root.join("a"), "1").unwrap();
        std::fs::write(root.join(".DS_Store"), "x").unwrap();
        let other = dir.path().join("other");
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(other.join("three"), "3").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&other, root.join("c")).unwrap();
        let rels: Vec<String> = files(&root).unwrap().into_iter().map(|e| e.rel).collect();
        #[cfg(unix)]
        assert_eq!(rels, vec!["a", "b/two", "c/three"]);
    }

    #[test]
    fn tree_hash_ignores_order() {
        let a = tree_hash(vec![("x".into(), "1".into()), ("y".into(), "2".into())]);
        let b = tree_hash(vec![("y".into(), "2".into()), ("x".into(), "1".into())]);
        assert_eq!(a, b);
    }
}

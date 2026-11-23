//! Directory → account mappings for `haryx run` auto-resolution. Port of `claude_swap/mappings.py`.
//!
//! Maps a normalized absolute directory path to a stored account identity (email + organizationUuid).
//! `haryx run` with no account argument resolves the current working directory to the nearest mapped
//! ancestor and launches that account in session mode.
//!
//! Persisted to `<store root>/mappings.json`. Identity is the stable (email, organizationUuid) composite
//! rather than the slot number, since slot numbers are reused when accounts are removed and re-added.
//! Deliberately decoupled from the switcher: callers resolve an entry's (email, org) to a live slot.

use std::io;
use std::path::{Component, Path, PathBuf};

use serde_json::{Map, Value};

use crate::error::Result;
use crate::fsutil::{WriteOptions, atomic_write_text};
use crate::models::get_timestamp;
use crate::models::pyfmt::json_dumps_pretty;
use crate::paths;

pub const SCHEMA_VERSION: i64 = 1;
pub const MAPPINGS_FILENAME: &str = "mappings.json";

/// The mapping table: normalized path → entry object (`email`, `organizationUuid`, `added`, and any
/// other field a hand edit or a later version put there, which survives a rewrite).
pub type Mappings = Map<String, Value>;

/// Typed view of one mapping entry. Missing or non-string fields read as empty.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MappingEntry {
    pub email: String,
    pub organization_uuid: String,
    pub added: String,
}

impl MappingEntry {
    pub fn from_value(entry: &Value) -> Self {
        let text = |key: &str| {
            entry
                .get(key)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        MappingEntry {
            email: text("email"),
            organization_uuid: text("organizationUuid"),
            added: text("added"),
        }
    }
}

/// Normalize a path to a stable, comparable mapping key: `~` expanded, made absolute, symlinks resolved
/// (as far as the path exists; the rest is joined lexically, like Python's non-strict `Path.resolve`).
/// Case is kept: `os.path.normcase` is a no-op on POSIX.
pub fn normalize_path(path: &Path) -> Result<String> {
    let expanded = expand_user(path)?;
    let absolute = if expanded.is_absolute() {
        expanded
    } else {
        std::env::current_dir()?.join(expanded)
    };
    Ok(resolve_lenient(&absolute).to_string_lossy().into_owned())
}

fn expand_user(path: &Path) -> Result<PathBuf> {
    match path.strip_prefix("~") {
        Ok(rest) => Ok(paths::home_dir()?.join(rest)),
        Err(_) => Ok(path.to_path_buf()),
    }
}

/// Resolve each existing prefix through the filesystem (following symlinks); past the first missing
/// component, apply `.` and `..` lexically.
fn resolve_lenient(absolute: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::Prefix(_) | Component::RootDir => out.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            Component::Normal(name) => {
                out.push(name);
                if let Ok(real) = crate::fsutil::canonicalize(&out) {
                    out = real;
                }
            }
        }
    }
    out
}

/// Reads and writes `<store root>/mappings.json`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MappingStore {
    pub path: PathBuf,
}

impl MappingStore {
    pub fn new(store_root: &Path) -> Self {
        MappingStore {
            path: store_root.join(MAPPINGS_FILENAME),
        }
    }

    /// The normalized-path → entry map; empty when the file is missing, corrupt or the wrong shape.
    pub fn load(&self) -> Mappings {
        let Ok(text) = std::fs::read_to_string(&self.path) else {
            return Mappings::new();
        };
        match serde_json::from_str::<Value>(&text) {
            Ok(Value::Object(mut data)) => match data.remove("mappings") {
                Some(Value::Object(mappings)) => mappings,
                _ => Mappings::new(),
            },
            _ => Mappings::new(),
        }
    }

    /// The full mapping table (same as [`MappingStore::load`]).
    pub fn all(&self) -> Mappings {
        self.load()
    }

    /// Exact-match lookup for a path (no ancestor walk).
    pub fn get(&self, path: &Path) -> Result<Option<Value>> {
        let key = normalize_path(path)?;
        Ok(self.load().remove(&key))
    }

    /// Upsert a mapping for `path` and persist atomically.
    pub fn set(&self, path: &Path, email: &str, org_uuid: &str) -> Result<()> {
        let mut mappings = self.load();
        let entry: Map<String, Value> = [
            ("email", email),
            ("organizationUuid", org_uuid),
            ("added", get_timestamp().as_str()),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), Value::String(v.to_owned())))
        .collect();
        mappings.insert(normalize_path(path)?, Value::Object(entry));
        self.write(&mappings)
    }

    /// Delete the mapping for `path`; whether one was removed.
    pub fn remove(&self, path: &Path) -> Result<bool> {
        let mut mappings = self.load();
        if mappings.shift_remove(&normalize_path(path)?).is_none() {
            return Ok(false);
        }
        self.write(&mappings)?;
        Ok(true)
    }

    /// Drop every mapping pointing at (email, org_uuid); the number removed.
    pub fn prune_account(&self, email: &str, org_uuid: &str) -> Result<usize> {
        let mappings = self.load();
        let before = mappings.len();
        let kept: Mappings = mappings
            .into_iter()
            .filter(|(_, entry)| {
                let entry = MappingEntry::from_value(entry);
                !(entry.email == email && entry.organization_uuid == org_uuid)
            })
            .collect();
        let removed = before - kept.len();
        if removed > 0 {
            self.write(&kept)?;
        }
        Ok(removed)
    }

    /// The (key, entry) of the longest mapped ancestor of `cwd` (or `cwd` itself). All candidates lie on
    /// the single root→cwd chain, so the longest key is the deepest match and nested folders inherit the
    /// closest mapping. Ancestry is by whole path components: `/foo/bar` does not match `/foo/barbaz`.
    pub fn resolve(&self, cwd: &Path) -> Result<Option<(String, Value)>> {
        let target = PathBuf::from(normalize_path(cwd)?);
        Ok(self
            .load()
            .into_iter()
            .filter(|(key, _)| target.starts_with(Path::new(key)))
            .fold(
                None,
                |best: Option<(String, Value)>, (key, entry)| match best {
                    Some(b) if b.0.len() >= key.len() => Some(b),
                    _ => Some((key, entry)),
                },
            ))
    }

    /// Atomically write the mappings file (temp beside it + rename), the store directory kept 0700.
    fn write(&self, mappings: &Mappings) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
            set_private_dir(parent)?;
        }
        let payload = serde_json::json!({"schemaVersion": SCHEMA_VERSION, "mappings": mappings});
        atomic_write_text(
            &self.path,
            &json_dumps_pretty(&payload),
            WriteOptions::default(),
        )?;
        Ok(())
    }
}

#[cfg(unix)]
fn set_private_dir(dir: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
}

#[cfg(not(unix))]
fn set_private_dir(_dir: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (tempfile::TempDir, MappingStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = MappingStore::new(&dir.path().join("backup"));
        (dir, store)
    }

    fn mkdir(path: &Path) -> &Path {
        std::fs::create_dir_all(path).unwrap();
        path
    }

    fn email_of(entry: &Value) -> String {
        MappingEntry::from_value(entry).email
    }

    #[test]
    fn set_then_get_exact() {
        let (dir, store) = setup();
        let repo = dir.path().join("work").join("app");
        mkdir(&repo);
        store.set(&repo, "work@co.com", "org-1").unwrap();
        let entry = MappingEntry::from_value(&store.get(&repo).unwrap().unwrap());
        assert_eq!(entry.email, "work@co.com");
        assert_eq!(entry.organization_uuid, "org-1");
        assert!(!entry.added.is_empty());
    }

    #[test]
    fn get_missing_returns_none() {
        let (dir, store) = setup();
        assert_eq!(store.get(&dir.path().join("nope")).unwrap(), None);
    }

    #[test]
    fn resolve_exact_nested_and_longest() {
        let (dir, store) = setup();
        let outer = dir.path().join("work");
        let inner = outer.join("client");
        let cwd = inner.join("src").join("deep");
        mkdir(&cwd);
        store.set(&outer, "outer@x.com", "").unwrap();

        let (key, entry) = store.resolve(&outer).unwrap().unwrap();
        assert_eq!(key, normalize_path(&outer).unwrap());
        assert_eq!(email_of(&entry), "outer@x.com");
        assert_eq!(
            email_of(&store.resolve(&cwd).unwrap().unwrap().1),
            "outer@x.com"
        );

        store.set(&inner, "inner@x.com", "").unwrap();
        assert_eq!(
            email_of(&store.resolve(&cwd).unwrap().unwrap().1),
            "inner@x.com"
        );
    }

    #[test]
    fn resolve_sibling_prefix_and_unmapped_do_not_match() {
        let (dir, store) = setup();
        let mapped = mkdir(&dir.path().join("foo").join("bar")).to_path_buf();
        let sibling = mkdir(&dir.path().join("foo").join("barbaz")).to_path_buf();
        store.set(&mapped, "a@x.com", "").unwrap();
        assert_eq!(store.resolve(&sibling).unwrap(), None);
        let other = mkdir(&dir.path().join("b")).to_path_buf();
        assert_eq!(store.resolve(&other).unwrap(), None);
    }

    #[test]
    fn remove_and_overwrite() {
        let (dir, store) = setup();
        let repo = mkdir(&dir.path().join("repo")).to_path_buf();
        store.set(&repo, "a@x.com", "").unwrap();
        store.set(&repo, "b@x.com", "org-9").unwrap();
        let entry = MappingEntry::from_value(&store.get(&repo).unwrap().unwrap());
        assert_eq!(
            (entry.email.as_str(), entry.organization_uuid.as_str()),
            ("b@x.com", "org-9")
        );
        assert_eq!(store.all().len(), 1);
        assert!(store.remove(&repo).unwrap());
        assert_eq!(store.get(&repo).unwrap(), None);
        assert!(!store.remove(&repo).unwrap());
    }

    #[test]
    fn prune_account() {
        let (dir, store) = setup();
        let paths: Vec<PathBuf> = ["a", "b", "c"]
            .iter()
            .map(|n| mkdir(&dir.path().join(n)).to_path_buf())
            .collect();
        store.set(&paths[0], "work@x.com", "org-1").unwrap();
        store.set(&paths[1], "work@x.com", "org-1").unwrap();
        store.set(&paths[2], "personal@x.com", "").unwrap();
        assert_eq!(store.prune_account("work@x.com", "org-1").unwrap(), 2);
        assert_eq!(store.get(&paths[0]).unwrap(), None);
        assert_eq!(store.get(&paths[1]).unwrap(), None);
        assert!(store.get(&paths[2]).unwrap().is_some());
        assert_eq!(store.prune_account("nobody@x.com", "").unwrap(), 0);
    }

    #[test]
    fn missing_or_corrupt_file_is_empty() {
        let (dir, store) = setup();
        assert!(store.load().is_empty());
        assert!(store.all().is_empty());
        assert_eq!(store.resolve(dir.path()).unwrap(), None);
        mkdir(store.path.parent().unwrap());
        for text in ["{ not json", "[1]", r#"{"mappings": []}"#] {
            std::fs::write(&store.path, text).unwrap();
            assert!(store.load().is_empty(), "{text}");
        }
    }

    #[test]
    fn normalize_path_expands_and_resolves() {
        let dir = tempfile::tempdir().unwrap();
        let repo = mkdir(&dir.path().join("repo")).to_path_buf();
        let a = normalize_path(&repo).unwrap();
        let b = normalize_path(Path::new(&format!("{}/", repo.display()))).unwrap();
        let c = normalize_path(&repo.join(".")).unwrap();
        let d = normalize_path(&repo.join("sub").join("..")).unwrap();
        assert_eq!(a, b);
        assert_eq!(a, c);
        assert_eq!(a, d);
        // Symlinks resolve (on macOS the temp dir itself sits behind /var -> /private/var).
        assert_eq!(
            a,
            crate::fsutil::canonicalize(&repo)
                .unwrap()
                .to_string_lossy()
        );
        // A missing tail is kept lexically.
        let missing = normalize_path(&repo.join("x").join("..").join("y")).unwrap();
        assert_eq!(missing, Path::new(&a).join("y").to_string_lossy());
    }

    #[cfg(unix)]
    #[test]
    fn persisted_schema_and_private_dir() {
        use std::os::unix::fs::PermissionsExt;
        let (dir, store) = setup();
        let repo = mkdir(&dir.path().join("repo")).to_path_buf();
        store.set(&repo, "a@x.com", "org-1").unwrap();
        let data: Value =
            serde_json::from_str(&std::fs::read_to_string(&store.path).unwrap()).unwrap();
        assert_eq!(data["schemaVersion"], 1);
        assert!(
            data["mappings"]
                .get(normalize_path(&repo).unwrap())
                .is_some()
        );
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(store.path.parent().unwrap()), 0o700);
        assert_eq!(mode(&store.path), 0o600);
        // No temp file left behind.
        assert_eq!(
            std::fs::read_dir(store.path.parent().unwrap())
                .unwrap()
                .count(),
            1
        );
    }

    #[test]
    fn unknown_entry_fields_survive_a_rewrite() {
        let (dir, store) = setup();
        mkdir(store.path.parent().unwrap());
        let repo = mkdir(&dir.path().join("repo")).to_path_buf();
        let key = normalize_path(&repo).unwrap();
        let other = normalize_path(&dir.path().join("other")).unwrap();
        let seed = serde_json::json!({"schemaVersion": 1, "mappings": {
            other.clone(): {"email": "o@x.com", "organizationUuid": "", "added": "", "note": "keep"}
        }});
        std::fs::write(&store.path, seed.to_string()).unwrap();
        store.set(&repo, "a@x.com", "").unwrap();
        let all = store.all();
        assert_eq!(all[&other]["note"], "keep");
        assert!(all.contains_key(&key));
    }
}

//! Saved profiles under `<store>/profiles/<name>/`: list, save, import, export, delete.

use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use super::catalog::{self, Category};
use super::collect::{self, CollectRequest};
use super::items::JsonDoc;
use super::manifest::{FILES_DIR, Manifest, Options, SECRETS_AGE_FILE, SECRETS_FILE, SecretsDoc};
use super::{
    DEFAULT_PROFILE, PluginsMode, ProfileEnv, SecretsMode, Selection, archive, crypt, secrets, walk,
};
use crate::error::{Error, Result};

const MAX_NAME_LEN: usize = 64;
const STATE_FILE: &str = ".state.json";

/// A saved profile, as `list` shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Summary {
    pub name: String,
    pub created_at: String,
    pub categories: Vec<String>,
    pub total_bytes: u64,
    pub secrets: SecretsMode,
    pub plugins: PluginsMode,
    pub has_credentials: bool,
    pub source_os: String,
    pub last_applied: bool,
}

pub struct ProfileStore {
    root: PathBuf,
}

/// A profile name: letters, digits, `.`, `_`, `-`; not hidden; not `default`.
pub fn validate_name(name: &str) -> Result<()> {
    let valid = !name.is_empty()
        && name.len() <= MAX_NAME_LEN
        && !name.starts_with('.')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    if !valid {
        return Err(Error::Validation(format!(
            "Invalid profile name '{name}': use up to {MAX_NAME_LEN} letters, digits, '.', '_' or '-'"
        )));
    }
    if name == DEFAULT_PROFILE {
        return Err(Error::Validation(format!(
            "'{DEFAULT_PROFILE}' is the live ~/.claude and cannot be saved, imported or deleted"
        )));
    }
    Ok(())
}

impl ProfileStore {
    pub fn new(env: &ProfileEnv) -> Self {
        ProfileStore {
            root: env.profiles_dir(),
        }
    }

    pub fn dir(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }

    pub fn exists(&self, name: &str) -> bool {
        self.dir(name)
            .join(super::manifest::MANIFEST_FILE)
            .is_file()
    }

    /// The directory of an existing profile, or a "not found" error.
    pub fn require(&self, name: &str) -> Result<PathBuf> {
        if self.exists(name) {
            Ok(self.dir(name))
        } else {
            Err(Error::General(format!(
                "No saved profile named '{name}' (see 'haryx ai profile list')"
            )))
        }
    }

    pub fn list(&self) -> Result<Vec<Summary>> {
        let last = self.last_applied();
        let Ok(read) = std::fs::read_dir(&self.root) else {
            return Ok(Vec::new());
        };
        let mut out: Vec<Summary> = read
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| validate_name(n).is_ok() && self.exists(n))
            .filter_map(|n| Manifest::load(&self.dir(&n)).ok().map(|m| (n, m)))
            .map(|(name, m)| Summary {
                last_applied: last.as_deref() == Some(name.as_str()),
                name,
                created_at: m.created_at,
                categories: m.categories,
                total_bytes: m.total_bytes,
                secrets: m.options.secrets,
                plugins: m.options.plugins,
                has_credentials: m.has_credentials,
                source_os: m.source.os,
            })
            .collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }

    /// Snapshot the live setup as `name`.
    pub fn save(&self, env: &ProfileEnv, req: &CollectRequest, force: bool) -> Result<Manifest> {
        validate_name(&req.name)?;
        self.check_free(&req.name, force)?;
        let tmp = self.temp_dir(&req.name)?;
        let result = collect::collect(env, &tmp, req);
        self.publish(&req.name, tmp, result)
    }

    /// Store an exported file as `name`.
    pub fn import(&self, name: &str, file: &Path, force: bool) -> Result<Manifest> {
        validate_name(name)?;
        self.check_free(name, force)?;
        let tmp = self.temp_dir(name)?;
        walk::remove_any(&tmp)?;
        let result = archive::extract(file, &tmp).and_then(|m| {
            let renamed = Manifest {
                name: name.to_owned(),
                ..m
            };
            renamed.save(&tmp)?;
            Ok(renamed)
        });
        self.publish(name, tmp, result)
    }

    pub fn delete(&self, name: &str) -> Result<()> {
        validate_name(name)?;
        let dir = self.require(name)?;
        walk::remove_any(&dir)?;
        if self.last_applied().as_deref() == Some(name) {
            self.write_state(None)?;
        }
        Ok(())
    }

    fn check_free(&self, name: &str, force: bool) -> Result<()> {
        if self.exists(name) && !force {
            return Err(Error::General(format!(
                "A profile named '{name}' already exists; use --force to replace it"
            )));
        }
        Ok(())
    }

    fn temp_dir(&self, name: &str) -> Result<PathBuf> {
        walk::mkdirs(&self.root)?;
        let tmp = self
            .root
            .join(format!(".tmp-{name}-{}", std::process::id()));
        walk::remove_any(&tmp)?;
        Ok(tmp)
    }

    /// Move a finished temp profile into place, replacing an older one; clean up on failure.
    fn publish(&self, name: &str, tmp: PathBuf, result: Result<Manifest>) -> Result<Manifest> {
        let manifest = match result {
            Ok(m) => m,
            Err(e) => {
                let _ = walk::remove_any(&tmp);
                return Err(e);
            }
        };
        let dir = self.dir(name);
        walk::remove_any(&dir)?;
        std::fs::rename(&tmp, &dir)?;
        Ok(manifest)
    }

    pub fn last_applied(&self) -> Option<String> {
        let text = std::fs::read_to_string(self.root.join(STATE_FILE)).ok()?;
        let value: Value = serde_json::from_str(&text).ok()?;
        value
            .get("last_applied")
            .and_then(Value::as_str)
            .map(str::to_owned)
    }

    pub fn write_state(&self, last_applied: Option<&str>) -> Result<()> {
        let value = serde_json::json!({
            "last_applied": last_applied,
            "at": jiff::Timestamp::now().to_string(),
        });
        walk::write_file(
            &self.root.join(STATE_FILE),
            serde_json::to_string_pretty(&value)?.as_bytes(),
        )
    }
}

/// The secret values of a profile: `secrets.json`, or `secrets.age` opened with the passphrase that
/// `passphrase` supplies (asked only when needed).
pub fn load_secrets(
    dir: &Path,
    passphrase: &mut dyn FnMut() -> Option<String>,
) -> Result<SecretsDoc> {
    let plain = dir.join(SECRETS_FILE);
    if plain.is_file() {
        return SecretsDoc::from_bytes(&std::fs::read(&plain)?);
    }
    let sealed = dir.join(SECRETS_AGE_FILE);
    if sealed.is_file() {
        let pass = passphrase().ok_or_else(|| {
            Error::Validation("This profile's secrets are encrypted; a passphrase is needed".into())
        })?;
        let data = crypt::decrypt(&std::fs::read(&sealed)?, &pass)?;
        return SecretsDoc::from_bytes(&data);
    }
    Ok(SecretsDoc::default())
}

/// What to export.
pub struct ExportRequest {
    pub selection: Selection,
    pub secrets: SecretsMode,
    pub plugins: PluginsMode,
    pub passphrase: Option<String>,
}

/// Outcome of an export.
pub struct Exported {
    pub manifest: Manifest,
    pub warnings: Vec<String>,
}

/// Write `out` from the profile in `src` (a saved profile, or a fresh snapshot of `default`).
pub fn export(
    src: &Path,
    secrets_doc: &SecretsDoc,
    req: &ExportRequest,
    work: &Path,
    out: &Path,
) -> Result<Exported> {
    if req.secrets == SecretsMode::Local {
        return Err(Error::Validation(
            "Exports use --secrets redact or encrypt".into(),
        ));
    }
    let manifest = Manifest::load(src)?;
    let selection = req.selection.intersect(&manifest.category_set());
    let wants_credentials = selection.has(Category::Credentials);
    if wants_credentials && req.secrets != SecretsMode::Encrypt {
        return Err(Error::Validation(
            "--credentials needs --secrets encrypt (credentials are never written in plain text)"
                .into(),
        ));
    }
    if wants_credentials && secrets_doc.credentials.is_none() {
        return Err(Error::Validation(
            "This profile holds no credentials".into(),
        ));
    }
    let mut warnings = Vec::new();
    let bundle = req.plugins == PluginsMode::Bundle && selection.has(Category::Plugins);
    if bundle && manifest.options.plugins != PluginsMode::Bundle {
        warnings
            .push("The profile was saved without plugin files; exporting plugin references".into());
    }
    let plugins_mode = if bundle && manifest.options.plugins == PluginsMode::Bundle {
        PluginsMode::Bundle
    } else {
        PluginsMode::Ref
    };
    walk::remove_any(work)?;
    walk::mkdirs(work)?;
    let result = build_export(
        src,
        &manifest,
        secrets_doc,
        req,
        &selection,
        plugins_mode,
        work,
    )
    .and_then(|m| {
        archive::write(work, &m, out)?;
        Ok(m)
    });
    let _ = walk::remove_any(work);
    result.map(|manifest| Exported { manifest, warnings })
}

#[allow(clippy::too_many_arguments)]
fn build_export(
    src: &Path,
    manifest: &Manifest,
    secrets_doc: &SecretsDoc,
    req: &ExportRequest,
    selection: &Selection,
    plugins_mode: PluginsMode,
    work: &Path,
) -> Result<Manifest> {
    let redact = req.secrets == SecretsMode::Redact;
    for rel in manifest.files.keys() {
        if rel.starts_with(&format!("{FILES_DIR}/json/")) {
            continue;
        }
        let Some(category) = stored_category(rel) else {
            copy_with(src, work, rel, false)?;
            continue;
        };
        let plugin_file = category == Category::Plugins;
        if !selection.has(category) || (plugin_file && plugins_mode != PluginsMode::Bundle) {
            continue;
        }
        copy_with(src, work, rel, redact && category == Category::History)?;
    }
    for doc in JsonDoc::ALL {
        let plugin_doc = matches!(doc, JsonDoc::InstalledPlugins | JsonDoc::KnownMarketplaces);
        if plugin_doc && plugins_mode != PluginsMode::Bundle {
            continue;
        }
        let path = super::items::stored_json_path(src, doc);
        if let Some(value) = super::items::read_json(&path)?
            && let Some(kept) = filter_doc(doc, &value, selection)
        {
            walk::write_file(
                &super::items::stored_json_path(work, doc),
                serde_json::to_string_pretty(&kept)?.as_bytes(),
            )?;
        }
    }
    let used = used_secret_names(work)?;
    let values: std::collections::BTreeMap<String, String> = secrets_doc
        .values
        .iter()
        .filter(|(k, _)| used.contains(*k))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let credentials = selection
        .has(Category::Credentials)
        .then(|| secrets_doc.credentials.clone())
        .flatten();
    if req.secrets == SecretsMode::Encrypt && (!values.is_empty() || credentials.is_some()) {
        let pass = req
            .passphrase
            .as_deref()
            .ok_or_else(|| Error::Validation("--secrets encrypt needs a passphrase".into()))?;
        let doc = SecretsDoc {
            values: values.clone(),
            credentials: credentials.clone(),
        };
        let sealed = crypt::encrypt(&doc.to_bytes()?, pass)?;
        walk::write_file(&work.join(SECRETS_AGE_FILE), &sealed)?;
    }
    let exported = Manifest {
        options: Options {
            secrets: req.secrets,
            plugins: plugins_mode,
        },
        categories: selection
            .categories
            .iter()
            .map(|c| c.name().to_owned())
            .collect(),
        plugins: if selection.has(Category::Plugins) {
            manifest.plugins.clone()
        } else {
            Vec::new()
        },
        secrets: used.into_iter().collect(),
        has_credentials: credentials.is_some(),
        ..manifest.clone()
    };
    collect::finish_manifest(work, exported)
}

/// The category of a stored file under `files/claude/`, `None` for anything else (extra folders).
fn stored_category(rel: &str) -> Option<Category> {
    let rest = rel.strip_prefix(&format!("{FILES_DIR}/claude/"))?;
    let top = rest.split('/').next()?;
    catalog::classify(top)
}

fn copy_with(src: &Path, dst: &Path, rel: &str, redact_history: bool) -> Result<()> {
    if !rel.starts_with(&format!("{FILES_DIR}/")) {
        return Ok(());
    }
    let from = src.join(rel);
    let to = dst.join(rel);
    let data = std::fs::read(&from)?;
    let out = match (redact_history, std::str::from_utf8(&data)) {
        (true, Ok(text)) if secrets::has_placeholder(text) => {
            secrets::redact_placeholders(text).into_owned().into_bytes()
        }
        _ => data,
    };
    walk::write_file(&to, &out)?;
    walk::copy_mode(&from, &to);
    Ok(())
}

/// A stored JSON document reduced to the selected categories.
fn filter_doc(doc: JsonDoc, value: &Value, selection: &Selection) -> Option<Value> {
    let entries = doc.entries(value)?;
    let kept: Map<String, Value> = entries
        .iter()
        .filter(|(k, _)| doc.category_of(k).is_some_and(|c| selection.has(c)))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    if kept.is_empty() {
        return None;
    }
    Some(match doc.container() {
        Some(c) => Value::Object([(c.to_owned(), Value::Object(kept))].into_iter().collect()),
        None => Value::Object(kept),
    })
}

/// Placeholder names used by the files of a profile directory.
pub fn used_secret_names(dir: &Path) -> Result<std::collections::BTreeSet<String>> {
    let files_dir = dir.join(FILES_DIR);
    if !files_dir.exists() {
        return Ok(Default::default());
    }
    let mut names = std::collections::BTreeSet::new();
    for entry in walk::files(&files_dir)? {
        let data = std::fs::read(&entry.abs)?;
        if let Ok(text) = std::str::from_utf8(&data) {
            names.extend(secrets::placeholder_names(text));
        }
    }
    Ok(names)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_names() {
        assert!(validate_name("work-2").is_ok());
        assert!(validate_name("default").is_err());
        assert!(validate_name("../x").is_err());
        assert!(validate_name(".hidden").is_err());
        assert!(validate_name("").is_err());
    }

    #[test]
    fn stored_categories_follow_top_entry() {
        assert_eq!(
            stored_category("files/claude/projects/x/y.jsonl"),
            Some(Category::History)
        );
        assert_eq!(
            stored_category("files/claude/skills/a/SKILL.md"),
            Some(Category::Skills)
        );
        assert_eq!(stored_category("files/extra/0/a"), None);
    }
}

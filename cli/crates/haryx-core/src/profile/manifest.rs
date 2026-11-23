//! `manifest.json` of a profile, and the secrets document stored beside it.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::catalog::Category;
use super::{PluginsMode, SecretsMode};
use crate::error::{Error, Result};

pub const SCHEMA: u32 = 1;
pub const KIND: &str = "haryx-profile";
pub const MANIFEST_FILE: &str = "manifest.json";
pub const SECRETS_FILE: &str = "secrets.json";
pub const SECRETS_AGE_FILE: &str = "secrets.age";
pub const FILES_DIR: &str = "files";

/// Where the profile was made.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Source {
    pub os: String,
    pub arch: String,
}

impl Source {
    pub fn current() -> Self {
        Source {
            os: std::env::consts::OS.to_owned(),
            arch: std::env::consts::ARCH.to_owned(),
        }
    }
}

/// A plugin carried by reference.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginRef {
    /// `name@marketplace`.
    pub id: String,
    pub marketplace: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// The marketplace's `source` object from `known_marketplaces.json`.
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub source: Value,
}

/// An `--include-dir` folder.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IncludeDir {
    /// Original location, home tokenized.
    pub origin: String,
    /// Folder inside the profile (`files/extra/<n>`).
    pub path: String,
}

/// Size and hash of one stored file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileRecord {
    pub sha256: String,
    pub size: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Options {
    pub secrets: SecretsMode,
    pub plugins: PluginsMode,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub schema: u32,
    pub kind: String,
    pub name: String,
    pub haryx_version: String,
    pub created_at: String,
    pub source: Source,
    pub options: Options,
    /// Category names the profile covers.
    pub categories: Vec<String>,
    #[serde(default)]
    pub plugins: Vec<PluginRef>,
    #[serde(default)]
    pub include_dirs: Vec<IncludeDir>,
    /// Executables hooks and MCP servers call.
    #[serde(default)]
    pub requires: Vec<String>,
    /// Names of the secrets the files refer to.
    #[serde(default)]
    pub secrets: Vec<String>,
    #[serde(default)]
    pub has_credentials: bool,
    /// Every stored file (path relative to the profile root) but the manifest itself.
    #[serde(default)]
    pub files: BTreeMap<String, FileRecord>,
    pub total_bytes: u64,
}

impl Manifest {
    pub fn category_set(&self) -> std::collections::BTreeSet<Category> {
        self.categories
            .iter()
            .filter_map(|c| Category::parse(c))
            .collect()
    }

    pub fn load(dir: &Path) -> Result<Manifest> {
        let path = dir.join(MANIFEST_FILE);
        let text = std::fs::read_to_string(&path)
            .map_err(|e| Error::General(format!("Cannot read {}: {e}", path.display())))?;
        Self::parse(&text)
    }

    pub fn parse(text: &str) -> Result<Manifest> {
        let manifest: Manifest = serde_json::from_str(text)
            .map_err(|e| Error::Validation(format!("Not a haryx profile manifest: {e}")))?;
        if manifest.kind != KIND {
            return Err(Error::Validation(format!(
                "Not a haryx profile (kind '{}')",
                manifest.kind
            )));
        }
        if manifest.schema > SCHEMA {
            return Err(Error::Validation(format!(
                "This profile uses format {} but this haryx reads up to {SCHEMA}; upgrade haryx",
                manifest.schema
            )));
        }
        Ok(manifest)
    }

    pub fn save(&self, dir: &Path) -> Result<()> {
        let text = serde_json::to_string_pretty(self)?;
        super::walk::write_file(&dir.join(MANIFEST_FILE), text.as_bytes())
    }
}

/// The secret values of a profile.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretsDoc {
    #[serde(default)]
    pub values: BTreeMap<String, String>,
    /// The `haryx ai export --full` envelope, when credentials were included.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credentials: Option<Value>,
}

impl SecretsDoc {
    pub fn is_empty(&self) -> bool {
        self.values.is_empty() && self.credentials.is_none()
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        Ok(serde_json::to_vec_pretty(self)?)
    }

    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        serde_json::from_slice(data)
            .map_err(|e| Error::Validation(format!("Damaged profile secrets: {e}")))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn sample() -> Manifest {
        Manifest {
            schema: SCHEMA,
            kind: KIND.into(),
            name: "work".into(),
            haryx_version: "0.1.0".into(),
            created_at: "2026-11-23T00:00:00Z".into(),
            source: Source::current(),
            options: Options {
                secrets: SecretsMode::Local,
                plugins: PluginsMode::Ref,
            },
            categories: vec!["skills".into()],
            plugins: vec![],
            include_dirs: vec![],
            requires: vec![],
            secrets: vec![],
            has_credentials: false,
            files: BTreeMap::new(),
            total_bytes: 0,
        }
    }

    #[test]
    fn round_trips() {
        let m = sample();
        let text = serde_json::to_string(&m).unwrap();
        assert_eq!(Manifest::parse(&text).unwrap(), m);
    }

    #[test]
    fn rejects_other_kinds_and_newer_schema() {
        let mut m = sample();
        m.kind = "other".into();
        assert!(Manifest::parse(&serde_json::to_string(&m).unwrap()).is_err());
        let mut m = sample();
        m.schema = SCHEMA + 1;
        assert!(Manifest::parse(&serde_json::to_string(&m).unwrap()).is_err());
    }
}

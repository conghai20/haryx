//! `haryx ai profile`: see, save, export, import and apply a Claude Code setup (skills, hooks, MCP servers,
//! plugins, history, ...). Plan: `docs/ai-profile-plan.md`.
//!
//! A *profile* is a directory with a [`Manifest`] and a `files/` tree in a portable form: home paths are
//! replaced by [`tokens::HOME_TOKEN`], and detected secrets by `${HARYX_SECRET_<NAME>}` placeholders whose
//! values live beside it (`secrets.json` for a profile saved on this machine, `secrets.age` for an
//! encrypted import). The profile named [`DEFAULT_PROFILE`] is not stored: it is the live `~/.claude`.
//!
//! Layout of a profile directory (and of the exported archive):
//!
//! ```text
//! manifest.json
//! files/claude/<rel>        ~/.claude/<rel>
//! files/json/<name>.json    a JSON document merged key by key (settings, mcpServers, ~/.claude.json, ...)
//! files/extra/<n>/...       an --include-dir folder
//! secrets.json | secrets.age
//! ```

pub mod apply;
pub mod archive;
pub mod catalog;
pub mod collect;
pub mod crypt;
pub mod diff;
pub mod inventory;
pub mod items;
pub mod manifest;
pub mod merge;
pub mod plan;
pub mod plugins;
pub mod secrets;
pub mod store;
pub mod tokens;
pub mod walk;

use std::collections::BTreeSet;
use std::path::PathBuf;

pub use catalog::Category;
pub use manifest::Manifest;

use crate::error::Result;
use crate::paths::{self, PathEnv};

/// The profile that is always the live `~/.claude`.
pub const DEFAULT_PROFILE: &str = "default";
/// File extension of an exported profile.
pub const FILE_EXTENSION: &str = "haryx-profile";

/// Where a profile operation reads and writes on this machine.
#[derive(Clone, Debug)]
pub struct ProfileEnv {
    pub home: PathBuf,
    /// `~/.claude` (or `CLAUDE_CONFIG_DIR`).
    pub claude_home: PathBuf,
    /// `~/.claude.json`.
    pub claude_json: PathBuf,
    /// The haryx store root.
    pub store: PathBuf,
    pub path_env: PathEnv,
}

impl ProfileEnv {
    pub fn from_path_env(env: &PathEnv) -> Result<Self> {
        Ok(ProfileEnv {
            home: env.home.clone(),
            claude_home: paths::claude_config_home_in(env),
            claude_json: paths::global_config_path_in(env),
            store: paths::store_root_in(env)?,
            path_env: env.clone(),
        })
    }

    pub fn from_process() -> Result<Self> {
        Self::from_path_env(&PathEnv::from_process()?)
    }

    /// `<store>/profiles`.
    pub fn profiles_dir(&self) -> PathBuf {
        self.store.join("profiles")
    }

    /// `<store>/profiles/.backups`.
    pub fn backups_dir(&self) -> PathBuf {
        self.profiles_dir().join(".backups")
    }

    /// The home directory as text, without a trailing slash.
    pub fn home_text(&self) -> String {
        tokens::normalize_home(&self.home.to_string_lossy())
    }
}

/// How secrets travel in an exported file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SecretsMode {
    /// Saved on this machine: values in a local `secrets.json`.
    Local,
    /// Values dropped; placeholders remain.
    Redact,
    /// Values encrypted with a passphrase in `secrets.age`.
    Encrypt,
}

impl SecretsMode {
    pub fn as_str(self) -> &'static str {
        match self {
            SecretsMode::Local => "local",
            SecretsMode::Redact => "redact",
            SecretsMode::Encrypt => "encrypt",
        }
    }
}

/// How plugins travel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PluginsMode {
    /// Name, marketplace and version; reinstalled with `claude plugin install`.
    #[default]
    Ref,
    /// The plugin files themselves.
    Bundle,
}

impl PluginsMode {
    pub fn as_str(self) -> &'static str {
        match self {
            PluginsMode::Ref => "ref",
            PluginsMode::Bundle => "bundle",
        }
    }
}

/// The categories an operation covers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Selection {
    pub categories: BTreeSet<Category>,
}

impl Default for Selection {
    fn default() -> Self {
        Selection {
            categories: Category::ALL
                .iter()
                .copied()
                .filter(|c| c.default_on())
                .collect(),
        }
    }
}

impl Selection {
    /// Every category, optional ones included.
    pub fn everything() -> Self {
        Selection {
            categories: Category::ALL.iter().copied().collect(),
        }
    }

    /// Build from the CLI flags: `only` replaces the defaults, `extra` adds optional parts, `exclude`
    /// removes.
    pub fn from_flags(only: Option<&[Category]>, extra: &[Category], exclude: &[Category]) -> Self {
        let base: BTreeSet<Category> = match only {
            Some(list) => list.iter().copied().collect(),
            None => Selection::default().categories,
        };
        let categories = base
            .into_iter()
            .chain(extra.iter().copied())
            .filter(|c| !exclude.contains(c))
            .collect();
        Selection { categories }
    }

    pub fn has(&self, category: Category) -> bool {
        self.categories.contains(&category)
    }

    /// Only the categories both selections cover.
    pub fn intersect(&self, other: &BTreeSet<Category>) -> Self {
        Selection {
            categories: self.categories.intersection(other).copied().collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_selection_has_history_but_not_optional_parts() {
        let sel = Selection::default();
        assert!(sel.has(Category::History));
        assert!(sel.has(Category::Other));
        assert!(!sel.has(Category::Credentials));
        assert!(!sel.has(Category::Cache));
    }

    #[test]
    fn flags_combine_only_extra_and_exclude() {
        let sel = Selection::from_flags(
            Some(&[Category::Skills, Category::History]),
            &[Category::Cache],
            &[Category::History],
        );
        let want: BTreeSet<_> = [Category::Skills, Category::Cache].into_iter().collect();
        assert_eq!(sel.categories, want);
    }
}

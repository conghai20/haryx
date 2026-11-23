//! The items of a setup, enumerated the same way for the live `~/.claude` and for a stored profile, so
//! the two sides can be matched by id.
//!
//! Ids: a path item is its path under `~/.claude` with the home slug tokenized (`skills/review`,
//! `projects/{{HARYX_HOME_SLUG}}-code-app`); a JSON item is `<doc>#<key>` (`settings.json#hooks`) or
//! `mcp:<server>`; a plugin is `plugin:<name@marketplace>`; an included folder is `extra:<origin>`.

use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use super::catalog::{self, Category, SETTINGS_FILES};
use super::manifest::{FILES_DIR, PluginRef};
use super::tokens::{self, SLUG_TOKEN};
use super::{ProfileEnv, Selection};
use crate::error::{Error, Result};

/// A JSON document merged key by key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum JsonDoc {
    Settings,
    SettingsLocal,
    Mcp,
    ClaudeJson,
    InstalledPlugins,
    KnownMarketplaces,
}

impl JsonDoc {
    pub const ALL: [JsonDoc; 6] = [
        JsonDoc::Settings,
        JsonDoc::SettingsLocal,
        JsonDoc::Mcp,
        JsonDoc::ClaudeJson,
        JsonDoc::InstalledPlugins,
        JsonDoc::KnownMarketplaces,
    ];

    /// File name inside `files/json/`.
    pub fn store_name(self) -> &'static str {
        match self {
            JsonDoc::Settings => SETTINGS_FILES[0],
            JsonDoc::SettingsLocal => SETTINGS_FILES[1],
            JsonDoc::Mcp => "mcpServers.json",
            JsonDoc::ClaudeJson => "claude.json",
            JsonDoc::InstalledPlugins => "installed_plugins.json",
            JsonDoc::KnownMarketplaces => "known_marketplaces.json",
        }
    }

    /// Where the document lives on this machine.
    pub fn live_path(self, env: &ProfileEnv) -> PathBuf {
        match self {
            JsonDoc::Settings | JsonDoc::SettingsLocal => env.claude_home.join(self.store_name()),
            JsonDoc::Mcp | JsonDoc::ClaudeJson => env.claude_json.clone(),
            JsonDoc::InstalledPlugins | JsonDoc::KnownMarketplaces => {
                env.claude_home.join("plugins").join(self.store_name())
            }
        }
    }

    /// The object holding the items (`mcpServers`, `plugins`, or the document itself).
    pub fn container(self) -> Option<&'static str> {
        match self {
            JsonDoc::Mcp => Some("mcpServers"),
            JsonDoc::InstalledPlugins => Some("plugins"),
            _ => None,
        }
    }

    /// The category of one key, or `None` for keys this document does not carry.
    pub fn category_of(self, key: &str) -> Option<Category> {
        match self {
            JsonDoc::Settings | JsonDoc::SettingsLocal if key == "hooks" => Some(Category::Hooks),
            JsonDoc::Settings | JsonDoc::SettingsLocal => Some(Category::Settings),
            JsonDoc::Mcp => Some(Category::Mcps),
            JsonDoc::ClaudeJson if key == "mcpServers" => None,
            JsonDoc::ClaudeJson => Some(Category::ClaudeJson),
            JsonDoc::InstalledPlugins | JsonDoc::KnownMarketplaces => Some(Category::Plugins),
        }
    }

    pub fn item_id(self, key: &str) -> String {
        match self {
            JsonDoc::Mcp => format!("mcp:{key}"),
            _ => format!("{}#{key}", self.store_name()),
        }
    }

    /// The item map of a document.
    pub fn entries(self, doc: &Value) -> Option<&Map<String, Value>> {
        match self.container() {
            Some(key) => doc.get(key)?.as_object(),
            None => doc.as_object(),
        }
    }
}

/// What an item points at.
#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    /// A file or folder under `~/.claude` (tokenized relative path).
    Path { rel: String },
    /// An `--include-dir` folder: `origin` (tokenized) and its folder inside the profile.
    Extra { origin: String, stored: String },
    /// One key of a JSON document.
    Json { doc: JsonDoc, key: String },
    /// A plugin carried by reference.
    Plugin(PluginRef),
    /// The accounts of the haryx store.
    Credentials,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub category: Category,
    pub id: String,
    pub target: Target,
}

/// The live path of a tokenized relative path under `~/.claude`.
pub fn live_path(env: &ProfileEnv, rel: &str) -> PathBuf {
    let home = env.home_text();
    rel.split('/').fold(env.claude_home.clone(), |acc, part| {
        if part.starts_with(SLUG_TOKEN) {
            acc.join(tokens::expand_name(part, &home))
        } else {
            acc.join(part)
        }
    })
}

/// The path of a tokenized relative path inside a stored profile.
pub fn stored_path(profile_dir: &Path, rel: &str) -> PathBuf {
    profile_dir.join(FILES_DIR).join("claude").join(rel)
}

/// The path of a JSON document inside a stored profile.
pub fn stored_json_path(profile_dir: &Path, doc: JsonDoc) -> PathBuf {
    profile_dir
        .join(FILES_DIR)
        .join("json")
        .join(doc.store_name())
}

/// The path items under `root` (live `~/.claude` or a profile's `files/claude`). `plugin_files` adds the
/// installed plugin folders (bundle mode).
pub fn path_items(
    root: &Path,
    selection: &Selection,
    home: &str,
    plugin_files: bool,
) -> Result<Vec<Item>> {
    let Ok(read) = std::fs::read_dir(root) else {
        return Ok(Vec::new());
    };
    let mut names: Vec<String> = read
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    let mut out = Vec::new();
    for name in names {
        let Some(category) = catalog::classify(&name) else {
            continue;
        };
        if SETTINGS_FILES.contains(&name.as_str()) || !selection.has(category) {
            continue;
        }
        if name == "plugins" {
            if plugin_files {
                out.extend(plugin_dir_items(&root.join("plugins"))?);
            }
            continue;
        }
        if catalog::splits_children(&name) && root.join(&name).is_dir() {
            for child in child_names(&root.join(&name))? {
                let child = if name == "projects" {
                    tokens::tokenize_name(&child, home)
                } else {
                    child
                };
                let rel = format!("{name}/{child}");
                out.push(path_item(category, rel));
            }
            continue;
        }
        out.push(path_item(category, name));
    }
    Ok(out)
}

fn path_item(category: Category, rel: String) -> Item {
    Item {
        category,
        id: rel.clone(),
        target: Target::Path { rel },
    }
}

fn child_names(dir: &Path) -> Result<Vec<String>> {
    let mut names: Vec<String> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| !catalog::ignored_child(n))
        .collect();
    names.sort();
    Ok(names)
}

/// `plugins/cache/<marketplace>` and `plugins/marketplaces/<marketplace>` folders.
fn plugin_dir_items(plugins: &Path) -> Result<Vec<Item>> {
    let mut out = Vec::new();
    for sub in ["cache", "marketplaces"] {
        let dir = plugins.join(sub);
        if !dir.is_dir() {
            continue;
        }
        for child in child_names(&dir)? {
            out.push(path_item(
                Category::Plugins,
                format!("plugins/{sub}/{child}"),
            ));
        }
    }
    Ok(out)
}

/// The items of one JSON document.
pub fn json_items(doc_kind: JsonDoc, doc: &Value, selection: &Selection) -> Vec<Item> {
    let Some(entries) = doc_kind.entries(doc) else {
        return Vec::new();
    };
    entries
        .keys()
        .filter_map(|key| {
            let category = doc_kind.category_of(key)?;
            selection.has(category).then(|| Item {
                category,
                id: doc_kind.item_id(key),
                target: Target::Json {
                    doc: doc_kind,
                    key: key.clone(),
                },
            })
        })
        .collect()
}

pub fn plugin_items(plugins: &[PluginRef], selection: &Selection) -> Vec<Item> {
    if !selection.has(Category::Plugins) {
        return Vec::new();
    }
    plugins
        .iter()
        .map(|p| Item {
            category: Category::Plugins,
            id: format!("plugin:{}", p.id),
            target: Target::Plugin(p.clone()),
        })
        .collect()
}

/// Read a JSON document; a missing file is `None`, a damaged one an error naming it.
pub fn read_json(path: &Path) -> Result<Option<Value>> {
    match std::fs::read_to_string(path) {
        Ok(text) if text.trim().is_empty() => Ok(None),
        Ok(text) => serde_json::from_str(&text)
            .map(Some)
            .map_err(|e| Error::Config(format!("{} is not valid JSON: {e}", path.display()))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// A live document; `Mcp` and `ClaudeJson` both read `~/.claude.json`.
pub fn read_live_json(env: &ProfileEnv, doc: JsonDoc) -> Result<Option<Value>> {
    read_json(&doc.live_path(env))
}

/// The value of one item key in a document.
pub fn json_value<'a>(doc_kind: JsonDoc, doc: &'a Value, key: &str) -> Option<&'a Value> {
    doc_kind.entries(doc)?.get(key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn enumerates_path_items_with_split_children_and_tokenized_projects() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("skills/review")).unwrap();
        std::fs::create_dir_all(root.join("projects/-Users-me-app")).unwrap();
        std::fs::create_dir_all(root.join("sessions")).unwrap();
        std::fs::write(root.join("CLAUDE.md"), "x").unwrap();
        std::fs::write(root.join("settings.json"), "{}").unwrap();
        let items = path_items(root, &Selection::default(), "/Users/me", false).unwrap();
        let ids: Vec<&str> = items.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(
            ids,
            vec![
                "CLAUDE.md",
                "projects/{{HARYX_HOME_SLUG}}-app",
                "skills/review"
            ]
        );
    }

    #[test]
    fn json_items_split_hooks_from_settings() {
        let doc = json!({"hooks": {}, "model": "opus"});
        let items = json_items(JsonDoc::Settings, &doc, &Selection::default());
        let got: Vec<(Category, &str)> =
            items.iter().map(|i| (i.category, i.id.as_str())).collect();
        assert_eq!(
            got,
            vec![
                (Category::Hooks, "settings.json#hooks"),
                (Category::Settings, "settings.json#model")
            ]
        );
    }

    #[test]
    fn claude_json_items_skip_mcp_servers() {
        let doc = json!({"mcpServers": {"a": {}}, "theme": "dark"});
        let all = Selection::everything();
        let ids: Vec<String> = json_items(JsonDoc::ClaudeJson, &doc, &all)
            .into_iter()
            .map(|i| i.id)
            .collect();
        assert_eq!(ids, vec!["claude.json#theme"]);
        let mcp: Vec<String> = json_items(JsonDoc::Mcp, &doc, &all)
            .into_iter()
            .map(|i| i.id)
            .collect();
        assert_eq!(mcp, vec!["mcp:a"]);
    }

    #[test]
    fn live_path_expands_project_slug() {
        let env = ProfileEnv {
            home: "/home/me".into(),
            claude_home: "/home/me/.claude".into(),
            claude_json: "/home/me/.claude.json".into(),
            store: "/s".into(),
            path_env: crate::paths::PathEnv::with_home("/home/me"),
        };
        assert_eq!(
            live_path(&env, "projects/{{HARYX_HOME_SLUG}}-app"),
            PathBuf::from("/home/me/.claude/projects/-home-me-app")
        );
    }
}

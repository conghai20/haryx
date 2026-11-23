//! `save`: copy the live setup into a profile directory in portable form.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use super::catalog::Category;
use super::items::{self, JsonDoc, Target};
use super::manifest::{
    FILES_DIR, FileRecord, IncludeDir, MANIFEST_FILE, Manifest, Options, PluginRef, SCHEMA,
    SECRETS_FILE, SecretsDoc, Source,
};
use super::secrets::SecretBook;
use super::{PluginsMode, ProfileEnv, SecretsMode, Selection, plugins, tokens, walk};
use crate::error::{Error, Result};

/// What to save.
#[derive(Clone, Debug)]
pub struct CollectRequest {
    pub name: String,
    pub selection: Selection,
    pub plugins: PluginsMode,
    pub include_dirs: Vec<PathBuf>,
    /// The `haryx ai export --full` envelope, when credentials are included.
    pub credentials: Option<Value>,
}

/// Copy the live setup into `dest` (created; must not exist) and write its manifest.
pub fn collect(env: &ProfileEnv, dest: &Path, req: &CollectRequest) -> Result<Manifest> {
    if dest.exists() {
        return Err(Error::General(format!("{} already exists", dest.display())));
    }
    walk::mkdirs(dest)?;
    let home = env.home_text();
    let mut book = SecretBook::default();
    let bundle = req.plugins == PluginsMode::Bundle && req.selection.has(Category::Plugins);

    for item in items::path_items(&env.claude_home, &req.selection, &home, bundle)? {
        if let Target::Path { rel } = &item.target {
            copy_portable(
                &items::live_path(env, rel),
                &items::stored_path(dest, rel),
                &home,
                &mut book,
            )?;
        }
    }
    for doc in JsonDoc::ALL {
        let plugin_doc = matches!(doc, JsonDoc::InstalledPlugins | JsonDoc::KnownMarketplaces);
        if plugin_doc && !bundle {
            continue;
        }
        if let Some(value) = filtered_live_doc(env, doc, &req.selection)? {
            let portable = book.redact_json(&tokenize_json(&value, &home));
            let text = serde_json::to_string_pretty(&portable)?;
            walk::write_file(&items::stored_json_path(dest, doc), text.as_bytes())?;
        }
    }
    let plugin_refs = if req.selection.has(Category::Plugins) {
        plugins::live_refs(env)?
            .into_iter()
            .map(|p| PluginRef {
                source: tokenize_json(&p.source, &home),
                ..p
            })
            .collect()
    } else {
        Vec::new()
    };
    let include_dirs = copy_include_dirs(dest, &req.include_dirs, &home, &mut book)?;

    let secrets = SecretsDoc {
        values: book.values().clone(),
        credentials: req.credentials.clone(),
    };
    if !secrets.is_empty() {
        walk::write_file(&dest.join(SECRETS_FILE), &secrets.to_bytes()?)?;
    }
    let manifest = Manifest {
        schema: SCHEMA,
        kind: super::manifest::KIND.to_owned(),
        name: req.name.clone(),
        haryx_version: crate::VERSION.to_owned(),
        created_at: jiff::Timestamp::now().to_string(),
        source: Source::current(),
        options: Options {
            secrets: SecretsMode::Local,
            plugins: req.plugins,
        },
        categories: req
            .selection
            .categories
            .iter()
            .map(|c| c.name().to_owned())
            .collect(),
        plugins: plugin_refs,
        include_dirs,
        requires: requires(env)?,
        secrets: secrets.values.keys().cloned().collect(),
        has_credentials: secrets.credentials.is_some(),
        files: BTreeMap::new(),
        total_bytes: 0,
    };
    finish_manifest(dest, manifest)
}

/// Record every stored file (but the manifest and the local secrets) and save the manifest.
pub fn finish_manifest(dir: &Path, manifest: Manifest) -> Result<Manifest> {
    let files: BTreeMap<String, FileRecord> = walk::files(dir)?
        .into_iter()
        .filter(|e| e.rel != MANIFEST_FILE && e.rel != SECRETS_FILE)
        .map(|e| {
            let data = std::fs::read(&e.abs)?;
            Ok((
                e.rel,
                FileRecord {
                    sha256: walk::sha256_hex(&data),
                    size: data.len() as u64,
                },
            ))
        })
        .collect::<Result<_>>()?;
    let total_bytes = files.values().map(|r| r.size).sum();
    let manifest = Manifest {
        files,
        total_bytes,
        ..manifest
    };
    manifest.save(dir)?;
    Ok(manifest)
}

/// Copy a file or tree, tokenizing the home and redacting secrets in UTF-8 files.
fn copy_portable(src: &Path, dst: &Path, home: &str, book: &mut SecretBook) -> Result<()> {
    for entry in walk::files(src)? {
        let target = if entry.rel.is_empty() {
            dst.to_path_buf()
        } else {
            dst.join(&entry.rel)
        };
        let data = std::fs::read(&entry.abs)?;
        let out = portable_bytes(data, home, book);
        walk::write_file(&target, &out)?;
        walk::copy_mode(&entry.abs, &target);
    }
    Ok(())
}

fn portable_bytes(data: Vec<u8>, home: &str, book: &mut SecretBook) -> Vec<u8> {
    match String::from_utf8(data) {
        Ok(text) => {
            let tokenized = tokens::tokenize_text(&text, home);
            book.redact_text(&tokenized).into_owned().into_bytes()
        }
        Err(e) => e.into_bytes(),
    }
}

fn copy_include_dirs(
    dest: &Path,
    dirs: &[PathBuf],
    home: &str,
    book: &mut SecretBook,
) -> Result<Vec<IncludeDir>> {
    dirs.iter()
        .enumerate()
        .map(|(index, dir)| {
            if !dir.is_dir() {
                return Err(Error::Validation(format!(
                    "--include-dir {} is not a folder",
                    dir.display()
                )));
            }
            let stored = format!("{FILES_DIR}/extra/{index}");
            copy_portable(dir, &dest.join(&stored), home, book)?;
            Ok(IncludeDir {
                origin: tokens::tokenize_text(&dir.to_string_lossy(), home).into_owned(),
                path: stored,
            })
        })
        .collect()
}

/// A live JSON document reduced to the keys of the selected categories, or `None` when nothing is left.
fn filtered_live_doc(env: &ProfileEnv, doc: JsonDoc, sel: &Selection) -> Result<Option<Value>> {
    let Some(value) = items::read_live_json(env, doc)? else {
        return Ok(None);
    };
    let Some(entries) = doc.entries(&value) else {
        return Ok(None);
    };
    let kept: Map<String, Value> = entries
        .iter()
        .filter(|(k, _)| doc.category_of(k).is_some_and(|c| sel.has(c)))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    if kept.is_empty() {
        return Ok(None);
    }
    let out = match doc.container() {
        Some(container) => {
            let mut wrapper = Map::new();
            wrapper.insert(container.to_owned(), Value::Object(kept));
            Value::Object(wrapper)
        }
        None => Value::Object(kept),
    };
    Ok(Some(out))
}

/// Every string of a JSON value with the home tokenized.
pub fn tokenize_json(value: &Value, home: &str) -> Value {
    map_strings(value, &|s| tokens::tokenize_text(s, home).into_owned())
}

/// Every string of a JSON value passed through `f` (keys too).
pub fn map_strings(value: &Value, f: &dyn Fn(&str) -> String) -> Value {
    match value {
        Value::String(s) => Value::String(f(s)),
        Value::Array(items) => Value::Array(items.iter().map(|v| map_strings(v, f)).collect()),
        Value::Object(map) => {
            Value::Object(map.iter().map(|(k, v)| (f(k), map_strings(v, f))).collect())
        }
        other => other.clone(),
    }
}

/// Executables the live hooks and MCP servers call.
fn requires(env: &ProfileEnv) -> Result<Vec<String>> {
    let settings = items::read_live_json(env, JsonDoc::Settings)?.unwrap_or(Value::Null);
    let claude = items::read_live_json(env, JsonDoc::Mcp)?.unwrap_or(Value::Null);
    let mut names: Vec<String> = hook_commands(&settings)
        .iter()
        .chain(mcp_commands(&claude).iter())
        .filter_map(|cmd| executable(cmd))
        .collect();
    names.sort();
    names.dedup();
    Ok(names)
}

/// The `command` strings of a settings document's hooks.
pub fn hook_commands(settings: &Value) -> Vec<String> {
    let Some(events) = settings.get("hooks").and_then(Value::as_object) else {
        return Vec::new();
    };
    events
        .values()
        .filter_map(Value::as_array)
        .flatten()
        .filter_map(|group| group.get("hooks").and_then(Value::as_array))
        .flatten()
        .filter_map(|hook| hook.get("command").and_then(Value::as_str))
        .map(str::to_owned)
        .collect()
}

/// `command args...` of each MCP server in a document holding `mcpServers`.
pub fn mcp_commands(doc: &Value) -> Vec<String> {
    let Some(servers) = doc.get("mcpServers").and_then(Value::as_object) else {
        return Vec::new();
    };
    servers.values().filter_map(mcp_command_line).collect()
}

/// One MCP server's command line, or its URL for a remote server.
pub fn mcp_command_line(server: &Value) -> Option<String> {
    if let Some(cmd) = server.get("command").and_then(Value::as_str) {
        let args: Vec<&str> = server
            .get("args")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        return Some(
            std::iter::once(cmd)
                .chain(args)
                .collect::<Vec<_>>()
                .join(" "),
        );
    }
    server.get("url").and_then(Value::as_str).map(str::to_owned)
}

/// Shell words that are not programs to install.
const BUILTINS: &[&str] = &[
    "[", "[[", "test", "if", "cd", "echo", "true", "false", "exec", "env", "sh", "bash", "zsh",
    "export", "source", ".",
];

/// The program a command starts with, when it is a bare name (not a path, not a shell builtin).
pub fn executable(command: &str) -> Option<String> {
    let first = command.split_whitespace().next()?;
    let name = first.trim_matches(|c| c == '\'' || c == '"');
    let bare = !name.is_empty()
        && !name.contains('/')
        && !name.contains('{')
        && !name.contains('$')
        && !name.starts_with("http")
        && !BUILTINS.contains(&name);
    bare.then(|| name.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn finds_hook_and_mcp_commands() {
        let settings =
            json!({"hooks": {"Stop": [{"hooks": [{"type": "command", "command": "jq -r ."}]}]}});
        assert_eq!(hook_commands(&settings), vec!["jq -r ."]);
        let doc = json!({"mcpServers": {"gh": {"command": "npx", "args": ["-y", "srv"]}, "r": {"url": "https://x"}}});
        assert_eq!(mcp_commands(&doc), vec!["npx -y srv", "https://x"]);
    }

    #[test]
    fn executable_skips_paths_and_builtins() {
        assert_eq!(executable("node a.js").as_deref(), Some("node"));
        assert_eq!(executable("/usr/bin/node"), None);
        assert_eq!(executable("[ -f x ] || y"), None);
        assert_eq!(executable("'uvx' tool").as_deref(), Some("uvx"));
    }
}

//! `show`: what a setup holds, with a status per item.

use std::collections::BTreeSet;
use std::path::Path;

use serde_json::Value;

use super::catalog::Category;
use super::collect;
use super::items::{self, Item, JsonDoc, Target};
use super::manifest::{FILES_DIR, Manifest};
use super::{ProfileEnv, Selection, plugins, secrets, tokens, walk};
use crate::error::Result;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Ok,
    /// A hook script, MCP command or path it needs is missing.
    Broken,
    /// Holds a secret in plain text (export will redact or encrypt it).
    Secret,
    /// Holds a placeholder with no value available.
    NeedsSecret,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Ok => "ok",
            Status::Broken => "broken",
            Status::Secret => "secret",
            Status::NeedsSecret => "needs-secret",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub category: Category,
    pub id: String,
    pub kind: &'static str,
    pub size: u64,
    pub files: usize,
    pub status: Status,
    pub detail: Vec<String>,
}

/// The live `~/.claude`, every category.
pub fn live(env: &ProfileEnv) -> Result<Vec<Row>> {
    let home = env.home_text();
    let all = Selection::everything();
    let mut rows = Vec::new();
    for item in items::path_items(&env.claude_home, &all, &home, false)? {
        if let Target::Path { rel } = &item.target {
            rows.push(path_row(&item, &items::live_path(env, rel))?);
        }
    }
    for doc in [
        JsonDoc::Settings,
        JsonDoc::SettingsLocal,
        JsonDoc::Mcp,
        JsonDoc::ClaudeJson,
    ] {
        let Some(value) = items::read_live_json(env, doc)? else {
            continue;
        };
        for item in items::json_items(doc, &value, &all) {
            let Target::Json { key, .. } = &item.target else {
                continue;
            };
            let v = items::json_value(doc, &value, key)
                .cloned()
                .unwrap_or(Value::Null);
            rows.push(json_row(&item, doc, key, &v, &home, None));
        }
    }
    for item in items::plugin_items(&plugins::live_refs(env)?, &all) {
        let Target::Plugin(p) = &item.target else {
            continue;
        };
        rows.push(Row {
            detail: p.version.iter().map(|v| format!("version {v}")).collect(),
            ..basic_row(&item, "plugin")
        });
    }
    Ok(rows)
}

/// A stored profile. `available` names the secrets whose values are at hand.
pub fn stored(
    env: &ProfileEnv,
    dir: &Path,
    manifest: &Manifest,
    available: &BTreeSet<String>,
) -> Result<Vec<Row>> {
    let ctx_values = Default::default();
    let ctx = super::plan::Ctx::new(env, dir, manifest, &ctx_values);
    let all = Selection::everything();
    let mut rows = Vec::new();
    for item in super::plan::profile_items(&ctx, &all)? {
        let row = match &item.target {
            Target::Path { rel } => path_row(&item, &items::stored_path(dir, rel))?,
            Target::Extra { stored, origin } => Row {
                detail: vec![format!("restores to {origin}")],
                ..path_row(&item, &dir.join(stored))?
            },
            Target::Json { doc, key } => {
                let stored_doc = items::read_json(&items::stored_json_path(dir, *doc))?;
                let v = stored_doc
                    .as_ref()
                    .and_then(|d| items::json_value(*doc, d, key))
                    .cloned()
                    .unwrap_or(Value::Null);
                json_row(&item, *doc, key, &v, tokens::HOME_TOKEN, Some(available))
            }
            Target::Plugin(p) => Row {
                detail: p.version.iter().map(|v| format!("version {v}")).collect(),
                ..basic_row(&item, "plugin")
            },
            Target::Credentials => basic_row(&item, "accounts"),
        };
        rows.push(mark_placeholders(row, &item, dir, available)?);
    }
    Ok(rows)
}

fn basic_row(item: &Item, kind: &'static str) -> Row {
    Row {
        category: item.category,
        id: item.id.clone(),
        kind,
        size: 0,
        files: 0,
        status: Status::Ok,
        detail: Vec::new(),
    }
}

fn path_row(item: &Item, path: &Path) -> Result<Row> {
    let is_dir = path.is_dir();
    let files = if path.exists() {
        walk::files(path)?
    } else {
        Vec::new()
    };
    Ok(Row {
        size: walk::total_size(&files),
        files: files.len(),
        ..basic_row(item, if is_dir { "folder" } else { "file" })
    })
}

fn json_row(
    item: &Item,
    doc: JsonDoc,
    key: &str,
    value: &Value,
    home: &str,
    available: Option<&BTreeSet<String>>,
) -> Row {
    let kind = if doc == JsonDoc::Mcp { "mcp" } else { "key" };
    let mut row = basic_row(item, kind);
    let broken: Vec<String> = match (doc, key) {
        (JsonDoc::Settings | JsonDoc::SettingsLocal, "hooks") => {
            let wrapper = serde_json::json!({ "hooks": value });
            let cmds = collect::hook_commands(&wrapper);
            row.detail = cmds.iter().map(|c| format!("hook: {c}")).collect();
            cmds.iter().flat_map(|c| missing_paths(c, home)).collect()
        }
        (JsonDoc::Mcp, _) => {
            let line = collect::mcp_command_line(value).unwrap_or_default();
            row.detail = vec![line.clone()];
            missing_program(&line, home).into_iter().collect()
        }
        _ => Vec::new(),
    };
    if available.is_some() {
        return row;
    }
    let secret = has_plain_secret(value);
    if secret {
        row.detail.push("holds a secret in plain text".into());
    }
    if !broken.is_empty() {
        row.status = Status::Broken;
        row.detail
            .extend(broken.into_iter().map(|p| format!("missing: {p}")));
    } else if secret {
        row.status = Status::Secret;
    }
    row
}

/// A stored item whose files or value hold placeholders without values is `needs-secret`.
fn mark_placeholders(
    row: Row,
    item: &Item,
    dir: &Path,
    available: &BTreeSet<String>,
) -> Result<Row> {
    let names: BTreeSet<String> = match &item.target {
        Target::Path { rel } => names_in_tree(&items::stored_path(dir, rel))?,
        Target::Extra { stored, .. } => names_in_tree(&dir.join(stored))?,
        Target::Json { doc, key } => items::read_json(&items::stored_json_path(dir, *doc))?
            .as_ref()
            .and_then(|d| items::json_value(*doc, d, key))
            .map(|v| {
                secrets::placeholder_names(&v.to_string())
                    .into_iter()
                    .collect()
            })
            .unwrap_or_default(),
        _ => BTreeSet::new(),
    };
    let missing: Vec<String> = names.difference(available).cloned().collect();
    if missing.is_empty() || item.category == Category::History {
        return Ok(row);
    }
    let mut detail = row.detail;
    detail.push(format!("needs: {}", missing.join(", ")));
    Ok(Row {
        status: Status::NeedsSecret,
        detail,
        ..row
    })
}

fn names_in_tree(path: &Path) -> Result<BTreeSet<String>> {
    if !path.exists() {
        return Ok(BTreeSet::new());
    }
    let mut names = BTreeSet::new();
    for entry in walk::files(path)? {
        if let Ok(text) = std::fs::read_to_string(&entry.abs) {
            names.extend(secrets::placeholder_names(&text));
        }
    }
    Ok(names)
}

/// Whether the JSON holds a secret in clear text (redaction would change it).
fn has_plain_secret(value: &Value) -> bool {
    let mut book = secrets::SecretBook::default();
    let wrapped = serde_json::json!({ "v": value });
    book.redact_json(&wrapped);
    !book.is_empty()
}

/// Absolute paths in a command that do not exist.
pub fn missing_paths(command: &str, home: &str) -> Vec<String> {
    let expanded = tokens::expand_text(command, home);
    expanded
        .split(|c: char| c.is_whitespace() || matches!(c, '\'' | '"' | ';' | '(' | ')'))
        .filter(|w| w.starts_with('/') && w.len() > 1)
        .filter(|w| !Path::new(w).exists())
        .map(str::to_owned)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// The program of an MCP command line when it is neither on PATH nor an existing path.
fn missing_program(line: &str, home: &str) -> Option<String> {
    let first = line.split_whitespace().next()?;
    if first.starts_with("http://") || first.starts_with("https://") {
        return None;
    }
    let program = tokens::expand_text(first, home).into_owned();
    let found = if program.contains('/') {
        Path::new(&program).exists()
    } else {
        crate::spawn::resolve(
            std::ffi::OsStr::new(&program),
            std::env::var_os("PATH").as_deref(),
        )
        .is_some()
    };
    (!found).then_some(program)
}

/// Rows of the history category folded to one row per top entry (`projects`, `history.jsonl`, ...).
pub fn fold_history(rows: Vec<Row>) -> Vec<Row> {
    let (history, rest): (Vec<Row>, Vec<Row>) = rows
        .into_iter()
        .partition(|r| r.category == Category::History);
    let mut folded: Vec<(Row, usize)> = Vec::new();
    for row in history {
        let top = row.id.split('/').next().unwrap_or(&row.id).to_owned();
        match folded.iter_mut().find(|(r, _)| r.id == top) {
            Some((existing, count)) => {
                existing.size += row.size;
                existing.files += row.files;
                *count += 1;
            }
            None => folded.push((
                Row {
                    id: top,
                    kind: "folder",
                    detail: Vec::new(),
                    ..row
                },
                1,
            )),
        }
    }
    let folded = folded.into_iter().map(|(row, count)| Row {
        detail: if count > 1 {
            vec![format!("{count} entries")]
        } else {
            row.detail.clone()
        },
        ..row
    });
    rest.into_iter().chain(folded).collect()
}

/// The stored `files/` folder of a profile exists.
pub fn has_files(dir: &Path) -> bool {
    dir.join(FILES_DIR).is_dir()
}

#[cfg(test)]
mod tests {
    use super::*;

    // `/bin/sh` exists only on Unix.
    #[cfg(unix)]
    #[test]
    fn finds_missing_absolute_paths() {
        let missing = missing_paths(
            "bash {{HARYX_HOME}}/nope.sh && /bin/sh -c x",
            "/tmp/haryx-none",
        );
        assert_eq!(missing, vec!["/tmp/haryx-none/nope.sh"]);
    }

    #[test]
    fn detects_plain_secrets_in_env() {
        let v = serde_json::json!({"command": "x", "env": {"API_TOKEN": "abc"}});
        assert!(has_plain_secret(&v));
        let v =
            serde_json::json!({"command": "x", "env": {"API_TOKEN": "${HARYX_SECRET_API_TOKEN}"}});
        assert!(!has_plain_secret(&v));
    }
}

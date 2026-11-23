//! What applying a profile would change, item by item.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::catalog::Category;
use super::collect::{self, map_strings};
use super::items::{self, Item, JsonDoc, Target};
use super::manifest::Manifest;
use super::merge;
use super::{ProfileEnv, Selection, plugins, secrets, tokens, walk};
use crate::error::Result;

/// Files of an item: (path relative to the item, bytes).
pub type ItemFiles = Vec<(String, Vec<u8>)>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Add,
    Update,
    /// History: files added or grown, nothing lost.
    Merge,
    Remove,
    Same,
    /// Left alone (reason in the note).
    Skip,
}

impl Action {
    pub fn as_str(self) -> &'static str {
        match self {
            Action::Add => "add",
            Action::Update => "update",
            Action::Merge => "merge",
            Action::Remove => "remove",
            Action::Same => "same",
            Action::Skip => "skip",
        }
    }

    pub fn changes(self) -> bool {
        matches!(
            self,
            Action::Add | Action::Update | Action::Merge | Action::Remove
        )
    }
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub item: Item,
    pub action: Action,
    pub note: Option<String>,
    /// Secret placeholders this item still needs.
    pub needs_secret: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct Plan {
    pub entries: Vec<Entry>,
    /// Hook commands, MCP servers and scripts the apply would install: shown for confirmation.
    pub runs_code: Vec<String>,
    /// Programs the profile calls that are not on this machine's PATH.
    pub missing_requires: Vec<String>,
    pub warnings: Vec<String>,
    /// `--prune`: JSON values are replaced, not merged.
    pub prune: bool,
}

impl Plan {
    pub fn changes(&self) -> impl Iterator<Item = &Entry> {
        self.entries.iter().filter(|e| e.action.changes())
    }
}

/// What to apply.
#[derive(Clone, Debug)]
pub struct ApplyRequest {
    pub selection: Selection,
    pub prune: bool,
}

/// Reads a stored profile and turns its portable content into this machine's form.
pub struct Ctx<'a> {
    pub env: &'a ProfileEnv,
    pub dir: &'a Path,
    pub manifest: &'a Manifest,
    pub home: String,
    pub values: &'a BTreeMap<String, String>,
}

impl<'a> Ctx<'a> {
    pub fn new(
        env: &'a ProfileEnv,
        dir: &'a Path,
        manifest: &'a Manifest,
        values: &'a BTreeMap<String, String>,
    ) -> Self {
        Ctx {
            env,
            dir,
            manifest,
            home: env.home_text(),
            values,
        }
    }

    /// File bytes for this machine: home expanded, known secrets filled in.
    pub fn materialize(&self, data: Vec<u8>) -> (Vec<u8>, Vec<String>) {
        match String::from_utf8(data) {
            Ok(text) => {
                let expanded = tokens::expand_text(&text, &self.home);
                let (resolved, missing) = secrets::resolve_text(&expanded, self.values);
                (resolved.into_owned().into_bytes(), missing)
            }
            Err(e) => (e.into_bytes(), Vec::new()),
        }
    }

    pub fn materialize_json(&self, value: &Value) -> (Value, Vec<String>) {
        let missing = std::cell::RefCell::new(BTreeSet::new());
        let out = map_strings(value, &|s| {
            let expanded = tokens::expand_text(s, &self.home);
            let (resolved, names) = secrets::resolve_text(&expanded, self.values);
            missing.borrow_mut().extend(names);
            resolved.into_owned()
        });
        (out, missing.into_inner().into_iter().collect())
    }

    /// The profile's files of a path item: (relative path inside the item, materialized bytes).
    pub fn item_files(&self, stored: &Path) -> Result<(ItemFiles, Vec<String>)> {
        let mut missing = BTreeSet::new();
        let mut out = Vec::new();
        for entry in walk::files(stored)? {
            let (data, names) = self.materialize(std::fs::read(&entry.abs)?);
            missing.extend(names);
            out.push((entry.rel, data));
        }
        Ok((out, missing.into_iter().collect()))
    }

    pub fn stored_json(&self, doc: JsonDoc) -> Result<Option<Value>> {
        items::read_json(&items::stored_json_path(self.dir, doc))
    }

    /// The live location of an included folder.
    pub fn extra_live_path(&self, origin: &str) -> PathBuf {
        PathBuf::from(tokens::expand_text(origin, &self.home).into_owned())
    }
}

/// The items a stored profile carries, restricted to `selection`.
pub fn profile_items(ctx: &Ctx<'_>, selection: &Selection) -> Result<Vec<Item>> {
    let bundle = ctx.manifest.options.plugins == super::PluginsMode::Bundle;
    let root = ctx.dir.join(super::manifest::FILES_DIR).join("claude");
    let mut out = items::path_items(&root, selection, &ctx.home, bundle)?;
    for doc in JsonDoc::ALL {
        if let Some(value) = ctx.stored_json(doc)? {
            out.extend(items::json_items(doc, &value, selection));
        }
    }
    if !bundle {
        out.extend(items::plugin_items(&ctx.manifest.plugins, selection));
    }
    out.extend(ctx.manifest.include_dirs.iter().map(|d| Item {
        category: Category::Other,
        id: format!("extra:{}", d.origin),
        target: Target::Extra {
            origin: d.origin.clone(),
            stored: d.path.clone(),
        },
    }));
    if ctx.manifest.has_credentials && selection.has(Category::Credentials) {
        out.push(Item {
            category: Category::Credentials,
            id: "credentials".into(),
            target: Target::Credentials,
        });
    }
    Ok(out)
}

/// Build the plan of applying the profile in `ctx` to this machine.
pub fn build(ctx: &Ctx<'_>, req: &ApplyRequest) -> Result<Plan> {
    let selection = req.selection.intersect(&ctx.manifest.category_set());
    let items = profile_items(ctx, &selection)?;
    let installed = plugins::installed_ids(ctx.env)?;
    let mut plan = Plan {
        prune: req.prune,
        ..Plan::default()
    };
    for item in &items {
        let entry = plan_item(ctx, item, req.prune, &installed)?;
        if entry.action.changes() {
            plan.runs_code.extend(code_notes(ctx, &entry)?);
        }
        plan.entries.push(entry);
    }
    if req.prune {
        plan.entries.extend(prune_entries(ctx, &selection, &items)?);
    }
    plan.missing_requires = ctx
        .manifest
        .requires
        .iter()
        .filter(|name| {
            crate::spawn::resolve(
                std::ffi::OsStr::new(name.as_str()),
                std::env::var_os("PATH").as_deref(),
            )
            .is_none()
        })
        .cloned()
        .collect();
    Ok(plan)
}

fn entry(item: &Item, action: Action, note: Option<String>, needs: Vec<String>) -> Entry {
    Entry {
        item: item.clone(),
        action,
        note,
        needs_secret: needs,
    }
}

fn plan_item(
    ctx: &Ctx<'_>,
    item: &Item,
    prune: bool,
    installed: &BTreeSet<String>,
) -> Result<Entry> {
    match &item.target {
        Target::Path { rel } => plan_path(
            ctx,
            item,
            &items::stored_path(ctx.dir, rel),
            &items::live_path(ctx.env, rel),
        ),
        Target::Extra { origin, stored } => plan_path(
            ctx,
            item,
            &ctx.dir.join(stored),
            &ctx.extra_live_path(origin),
        ),
        Target::Json { doc, key } => plan_json(ctx, item, *doc, key, prune),
        Target::Plugin(p) if installed.contains(&p.id) => {
            Ok(entry(item, Action::Same, Some("installed".into()), vec![]))
        }
        Target::Plugin(p) => Ok(entry(
            item,
            Action::Add,
            Some(format!("claude plugin install {}", p.id)),
            vec![],
        )),
        Target::Credentials => Ok(entry(
            item,
            Action::Add,
            Some("accounts imported into the haryx store; the active login is unchanged".into()),
            vec![],
        )),
    }
}

/// Hash of a list of (relative path, bytes).
fn content_hash(files: &[(String, Vec<u8>)]) -> String {
    walk::tree_hash(
        files
            .iter()
            .map(|(rel, data)| (rel.clone(), walk::sha256_hex(data)))
            .collect(),
    )
}

/// Hash of a live file or tree.
pub fn live_hash(path: &Path) -> Result<String> {
    let files: Vec<(String, Vec<u8>)> = walk::files(path)?
        .into_iter()
        .map(|e| Ok((e.rel, std::fs::read(&e.abs)?)))
        .collect::<Result<_>>()?;
    Ok(content_hash(&files))
}

fn plan_path(ctx: &Ctx<'_>, item: &Item, stored: &Path, live: &Path) -> Result<Entry> {
    if item.category == Category::History {
        let ops = history_ops(ctx, stored, live)?;
        let action = if !live.exists() {
            Action::Add
        } else if ops.is_empty() {
            Action::Same
        } else {
            Action::Merge
        };
        let note = (!ops.is_empty()).then(|| format!("{} file(s)", ops.len()));
        return Ok(entry(item, action, note, vec![]));
    }
    let (files, missing) = ctx.item_files(stored)?;
    if !live.exists() {
        return Ok(entry(item, Action::Add, None, missing));
    }
    if live_hash(live)? == content_hash(&files) {
        return Ok(entry(item, Action::Same, None, missing));
    }
    if !missing.is_empty() {
        let note = Some("needs secrets; the local copy is kept".to_owned());
        return Ok(entry(item, Action::Skip, note, missing));
    }
    Ok(entry(item, Action::Update, None, missing))
}

/// History writes that add or grow files, never shrink or delete: (live path, bytes).
pub fn history_ops(ctx: &Ctx<'_>, stored: &Path, live: &Path) -> Result<Vec<(PathBuf, Vec<u8>)>> {
    let (files, _) = ctx.item_files(stored)?;
    let mut ops = Vec::new();
    for (rel, incoming) in files {
        let target = if rel.is_empty() {
            live.to_path_buf()
        } else {
            live.join(&rel)
        };
        let Ok(local) = std::fs::read(&target) else {
            ops.push((target, incoming));
            continue;
        };
        if local == incoming {
            continue;
        }
        let is_prompt_log =
            rel.is_empty() && live.file_name().is_some_and(|n| n == "history.jsonl");
        if is_prompt_log {
            let merged = merge::merge_history_lines(
                &String::from_utf8_lossy(&local),
                &String::from_utf8_lossy(&incoming),
            )
            .into_bytes();
            if merged != local {
                ops.push((target, merged));
            }
        } else if rel.ends_with(".jsonl") && merge::take_incoming_transcript(&local, &incoming) {
            ops.push((target, incoming));
        }
    }
    Ok(ops)
}

/// The incoming value of a JSON item for this machine, and the secrets it still needs.
pub fn incoming_json(
    ctx: &Ctx<'_>,
    doc: JsonDoc,
    key: &str,
) -> Result<Option<(Value, Vec<String>)>> {
    let Some(stored) = ctx.stored_json(doc)? else {
        return Ok(None);
    };
    let Some(value) = items::json_value(doc, &stored, key) else {
        return Ok(None);
    };
    Ok(Some(ctx.materialize_json(value)))
}

fn plan_json(ctx: &Ctx<'_>, item: &Item, doc: JsonDoc, key: &str, prune: bool) -> Result<Entry> {
    let Some((incoming, missing)) = incoming_json(ctx, doc, key)? else {
        return Ok(entry(
            item,
            Action::Skip,
            Some("missing from profile".into()),
            vec![],
        ));
    };
    let live_doc = items::read_live_json(ctx.env, doc)?;
    let local = live_doc
        .as_ref()
        .and_then(|d| items::json_value(doc, d, key))
        .cloned();
    let Some(local) = local else {
        return Ok(entry(item, Action::Add, None, missing));
    };
    let merged = merge::merge_value(Some(&local), &incoming, prune);
    let action = if merged == local {
        Action::Same
    } else {
        Action::Update
    };
    Ok(entry(item, action, None, missing))
}

/// Local items of the pruned categories that the profile lacks.
fn prune_entries(ctx: &Ctx<'_>, selection: &Selection, profile: &[Item]) -> Result<Vec<Entry>> {
    let prunable = Selection {
        categories: selection
            .categories
            .iter()
            .copied()
            .filter(|c| c.prunable())
            .collect(),
    };
    let ids: BTreeSet<&str> = profile.iter().map(|i| i.id.as_str()).collect();
    let mut live = items::path_items(&ctx.env.claude_home, &prunable, &ctx.home, false)?;
    for doc in JsonDoc::ALL {
        if let Some(value) = items::read_live_json(ctx.env, doc)? {
            live.extend(items::json_items(doc, &value, &prunable));
        }
    }
    Ok(live
        .into_iter()
        .filter(|i| !ids.contains(i.id.as_str()))
        .map(|i| {
            entry(
                &i,
                Action::Remove,
                Some("not in the profile (--prune)".into()),
                vec![],
            )
        })
        .collect())
}

/// Lines describing code this entry installs, for the confirmation.
fn code_notes(ctx: &Ctx<'_>, entry: &Entry) -> Result<Vec<String>> {
    if entry.action == Action::Remove {
        return Ok(Vec::new());
    }
    match &entry.item.target {
        Target::Json { doc, key }
            if key == "hooks" && matches!(doc, JsonDoc::Settings | JsonDoc::SettingsLocal) =>
        {
            let value = incoming_json(ctx, *doc, key)?
                .map(|(v, _)| v)
                .unwrap_or(Value::Null);
            let wrapper = serde_json::json!({ "hooks": value });
            Ok(collect::hook_commands(&wrapper)
                .into_iter()
                .map(|c| format!("hook: {c}"))
                .collect())
        }
        Target::Json {
            doc: JsonDoc::Mcp,
            key,
        } => {
            let value = incoming_json(ctx, JsonDoc::Mcp, key)?
                .map(|(v, _)| v)
                .unwrap_or(Value::Null);
            let line = collect::mcp_command_line(&value).unwrap_or_else(|| "(no command)".into());
            Ok(vec![format!("mcp server {key}: {line}")])
        }
        Target::Json {
            doc: JsonDoc::ClaudeJson,
            key,
        } if key == "projects" => Ok(vec![
            "folder trust decisions and per-project settings (~/.claude.json projects)".into(),
        ]),
        Target::Path { rel } if entry.item.category == Category::Hooks => Ok(vec![format!(
            "script: {}",
            items::live_path(ctx.env, rel).display()
        )]),
        Target::Plugin(p) => Ok(vec![format!("plugin: {}", p.id)]),
        Target::Extra { origin, .. } => Ok(vec![format!(
            "folder: {}",
            ctx.extra_live_path(origin).display()
        )]),
        _ => Ok(Vec::new()),
    }
}

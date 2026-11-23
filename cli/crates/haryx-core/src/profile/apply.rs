//! Execute a plan: back up what changes, write, and keep a record for `rollback`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::items::{self, JsonDoc, Target};
use super::merge;
use super::plan::{self, Action, Ctx, Entry, Plan};
use super::plugins::{self, PluginInstaller};
use super::{ProfileEnv, walk};
use crate::error::{Error, Result};

const BACKUP_FILE: &str = "backup.json";
const CONFIG_LOCK_TIMEOUT: Duration = Duration::from_secs(10);

/// One path a backup can restore.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Record {
    /// The live path.
    path: PathBuf,
    /// Its copy inside the backup (`data/<n>`), or `None` when it did not exist (restore removes it).
    stored: Option<String>,
    /// When the path was a symlink: its target, restored as a link (never copied by value).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    link: Option<PathBuf>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct BackupDoc {
    created_at: String,
    profile: String,
    records: Vec<Record>,
}

/// Saves the live state of every path an apply touches, once.
struct Backup {
    dir: PathBuf,
    doc: BackupDoc,
}

impl Backup {
    fn new(env: &ProfileEnv, profile: &str) -> Result<Self> {
        let root = env.backups_dir();
        walk::mkdirs(&root)?;
        let stamp = jiff::Timestamp::now().strftime("%Y%m%d-%H%M%S").to_string();
        let id = (0..)
            .map(|n| {
                if n == 0 {
                    stamp.clone()
                } else {
                    format!("{stamp}-{n}")
                }
            })
            .find(|id| !root.join(id).exists())
            .unwrap_or(stamp);
        let dir = root.join(id);
        walk::mkdirs(&dir.join("data"))?;
        Ok(Backup {
            dir,
            doc: BackupDoc {
                created_at: jiff::Timestamp::now().to_string(),
                profile: profile.to_owned(),
                records: Vec::new(),
            },
        })
    }

    fn id(&self) -> String {
        self.dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    /// Copy `path` (if present) before it changes.
    fn keep(&mut self, path: &Path) -> Result<()> {
        if self.doc.records.iter().any(|r| r.path == path) {
            return Ok(());
        }
        let link = std::fs::symlink_metadata(path)
            .ok()
            .filter(|m| m.file_type().is_symlink())
            .and_then(|_| std::fs::read_link(path).ok());
        let stored = if link.is_none() && std::fs::metadata(path).is_ok() {
            let rel = format!("data/{}", self.doc.records.len());
            walk::copy_tree(path, &self.dir.join(&rel))?;
            Some(rel)
        } else {
            None
        };
        self.doc.records.push(Record {
            path: path.to_path_buf(),
            stored,
            link,
        });
        self.save()
    }

    fn save(&self) -> Result<()> {
        walk::write_file(
            &self.dir.join(BACKUP_FILE),
            serde_json::to_string_pretty(&self.doc)?.as_bytes(),
        )
    }
}

/// What an apply did.
#[derive(Debug, Default)]
pub struct Outcome {
    pub backup_id: String,
    pub applied: Vec<Entry>,
    /// Accounts to import (`haryx ai export --full` envelope), for the caller.
    pub credentials: Option<Value>,
    pub errors: Vec<String>,
}

/// Apply `plan` (built from `ctx`). `credentials` is the envelope from the profile's secrets.
pub fn apply(
    ctx: &Ctx<'_>,
    plan: &Plan,
    installer: &dyn PluginInstaller,
    credentials: Option<Value>,
) -> Result<Outcome> {
    let mut backup = Backup::new(ctx.env, &ctx.manifest.name)?;
    let mut outcome = Outcome {
        backup_id: backup.id(),
        ..Outcome::default()
    };
    let changes: Vec<&Entry> = plan.changes().collect();
    let mut json_changes: BTreeMap<JsonDoc, Vec<&Entry>> = BTreeMap::new();
    for entry in &changes {
        match &entry.item.target {
            Target::Path { rel } => {
                let stored = items::stored_path(ctx.dir, rel);
                apply_path(
                    ctx,
                    entry,
                    &stored,
                    &items::live_path(ctx.env, rel),
                    &mut backup,
                )?;
            }
            Target::Extra { origin, stored } => {
                // Re-checked here: an included folder never lands outside the allowed places.
                let live = ctx.check_extra(origin, stored).map_err(Error::Validation)?;
                apply_extra(ctx, &ctx.dir.join(stored), &live, &mut backup)?;
            }
            Target::Json { doc, .. } => json_changes.entry(*doc).or_default().push(entry),
            Target::Plugin(_) | Target::Credentials => {}
        }
    }
    for (doc, entries) in &json_changes {
        apply_json(ctx, *doc, entries, plan.prune, &mut backup)?;
    }
    install_plugins(ctx, &changes, installer, &mut outcome)?;
    if changes
        .iter()
        .any(|e| matches!(e.item.target, Target::Credentials))
    {
        outcome.credentials = credentials;
    }
    outcome.applied = changes.into_iter().cloned().collect();
    Ok(outcome)
}

/// Where writes go: through a symlink at the item root, so a linked skill stays linked.
fn write_target(live: &Path) -> PathBuf {
    let is_link = std::fs::symlink_metadata(live).is_ok_and(|m| m.file_type().is_symlink());
    if is_link {
        std::fs::canonicalize(live).unwrap_or_else(|_| live.to_path_buf())
    } else {
        live.to_path_buf()
    }
}

fn apply_path(
    ctx: &Ctx<'_>,
    entry: &Entry,
    stored: &Path,
    live: &Path,
    backup: &mut Backup,
) -> Result<()> {
    match entry.action {
        Action::Remove => {
            backup.keep(live)?;
            walk::remove_any(live)
        }
        Action::Merge => {
            let target = write_target(live);
            for (path, data) in plan::history_ops(ctx, stored, &target)? {
                backup.keep(&path)?;
                walk::write_file(&path, &data)?;
            }
            Ok(())
        }
        Action::Add | Action::Update => {
            let target = write_target(live);
            backup.keep(&target)?;
            let (files, _) = ctx.item_files(stored)?;
            if entry.action == Action::Update && target.is_dir() {
                clear_dir(&target)?;
            }
            for (rel, data) in files {
                let path = if rel.is_empty() {
                    target.clone()
                } else {
                    target.join(&rel)
                };
                walk::write_file(&path, &data)?;
                let source = if rel.is_empty() {
                    stored.to_path_buf()
                } else {
                    stored.join(&rel)
                };
                walk::copy_mode(&source, &path);
            }
            Ok(())
        }
        Action::Same | Action::Skip => Ok(()),
    }
}

/// An included folder: add and overwrite files, never delete what is already there.
fn apply_extra(ctx: &Ctx<'_>, stored: &Path, live: &Path, backup: &mut Backup) -> Result<()> {
    let (files, _) = ctx.item_files(stored)?;
    for (rel, data) in files {
        let path = live.join(&rel);
        backup.keep(&path)?;
        walk::write_file(&path, &data)?;
        walk::copy_mode(&stored.join(&rel), &path);
    }
    Ok(())
}

/// Empty a directory but keep it (and any symlink pointing at it).
fn clear_dir(dir: &Path) -> Result<()> {
    for entry in std::fs::read_dir(dir)? {
        walk::remove_any(&entry?.path())?;
    }
    Ok(())
}

fn apply_json(
    ctx: &Ctx<'_>,
    doc: JsonDoc,
    entries: &[&Entry],
    prune: bool,
    backup: &mut Backup,
) -> Result<()> {
    let path = doc.live_path(ctx.env);
    let _lock = if matches!(doc, JsonDoc::Mcp | JsonDoc::ClaudeJson) {
        Some(crate::claude_locks::claude_config_lock(
            &ctx.env.path_env,
            Some(CONFIG_LOCK_TIMEOUT),
        )?)
    } else {
        None
    };
    backup.keep(&path)?;
    let live = items::read_json(&path)?.unwrap_or_else(|| Value::Object(Map::new()));
    let updated = entries.iter().try_fold(live, |acc, entry| {
        let Target::Json { key, .. } = &entry.item.target else {
            return Ok(acc);
        };
        apply_json_key(ctx, doc, acc, key, entry.action, prune)
    })?;
    let text = format!("{}\n", serde_json::to_string_pretty(&updated)?);
    walk::write_file(&path, text.as_bytes())
}

fn apply_json_key(
    ctx: &Ctx<'_>,
    doc: JsonDoc,
    live: Value,
    key: &str,
    action: Action,
    prune: bool,
) -> Result<Value> {
    let Value::Object(mut root) = live else {
        return Err(Error::Config(format!(
            "{} is not a JSON object",
            doc.live_path(ctx.env).display()
        )));
    };
    let container = match doc.container() {
        Some(name) => match root.remove(name) {
            Some(Value::Object(m)) => m,
            _ => Map::new(),
        },
        None => std::mem::take(&mut root),
    };
    let mut entries = container;
    if action == Action::Remove {
        entries.remove(key);
    } else if let Some((incoming, _)) = plan::incoming_json(ctx, doc, key)? {
        let merged = merge::merge_value(entries.get(key), &incoming, prune);
        entries.insert(key.to_owned(), merged);
    }
    match doc.container() {
        Some(name) => {
            root.insert(name.to_owned(), Value::Object(entries));
            Ok(Value::Object(root))
        }
        None => Ok(Value::Object(entries)),
    }
}

fn install_plugins(
    ctx: &Ctx<'_>,
    changes: &[&Entry],
    installer: &dyn PluginInstaller,
    outcome: &mut Outcome,
) -> Result<()> {
    let mut known = plugins::known_marketplaces(ctx.env)?;
    for entry in changes {
        let Target::Plugin(p) = &entry.item.target else {
            continue;
        };
        if !known.contains(&p.marketplace) {
            let source = ctx.materialize_json(&p.source).0;
            match plugins::marketplace_arg(&source) {
                Some(arg) => match installer.add_marketplace(&arg) {
                    Ok(()) => {
                        known.insert(p.marketplace.clone());
                    }
                    Err(e) => {
                        outcome.errors.push(e.to_string());
                        continue;
                    }
                },
                None => {
                    outcome.errors.push(format!(
                        "{}: unknown marketplace source; add marketplace '{}' by hand",
                        p.id, p.marketplace
                    ));
                    continue;
                }
            }
        }
        if let Err(e) = installer.install(&p.id) {
            outcome.errors.push(e.to_string());
        }
    }
    Ok(())
}

/// Backups, newest first: (id, profile, created_at).
pub fn list_backups(env: &ProfileEnv) -> Result<Vec<(String, String, String)>> {
    let Ok(read) = std::fs::read_dir(env.backups_dir()) else {
        return Ok(Vec::new());
    };
    let mut out: Vec<(String, String, String)> = read
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let id = e.file_name().to_string_lossy().into_owned();
            let text = std::fs::read_to_string(e.path().join(BACKUP_FILE)).ok()?;
            let doc: BackupDoc = serde_json::from_str(&text).ok()?;
            Some((id, doc.profile, doc.created_at))
        })
        .collect();
    out.sort_by(|a, b| b.0.cmp(&a.0));
    Ok(out)
}

/// Restore a backup (the newest when `id` is `None`). The current state is backed up first, so a
/// rollback can itself be rolled back. Returns (restored id, new backup id).
pub fn rollback(env: &ProfileEnv, id: Option<&str>) -> Result<(String, String)> {
    let backups = list_backups(env)?;
    let chosen = match id {
        Some(id) => backups
            .iter()
            .find(|(b, _, _)| b == id)
            .map(|(b, _, _)| b.clone())
            .ok_or_else(|| Error::General(format!("No backup named '{id}'")))?,
        None => backups
            .first()
            .map(|(b, _, _)| b.clone())
            .ok_or_else(|| Error::General("No backups to roll back to".into()))?,
    };
    let dir = env.backups_dir().join(&chosen);
    let doc: BackupDoc = serde_json::from_str(&std::fs::read_to_string(dir.join(BACKUP_FILE))?)?;
    let mut current = Backup::new(env, &format!("rollback of {chosen}"))?;
    let _lock = crate::claude_locks::claude_config_lock(&env.path_env, Some(CONFIG_LOCK_TIMEOUT))?;
    for record in &doc.records {
        if !record.path.starts_with(&env.home) {
            return Err(Error::Validation(format!(
                "Backup {chosen} names {} outside your home folder; not restoring it",
                record.path.display()
            )));
        }
    }
    // TODO(windows): restore links as junctions once `fsutil` can make them. Refuse before touching
    // anything, so a rollback never removes a file it cannot put back.
    #[cfg(not(unix))]
    if let Some(record) = doc.records.iter().find(|r| r.link.is_some()) {
        return Err(Error::Validation(format!(
            "Backup {chosen} holds a link at {}; restoring links is not supported on Windows yet",
            record.path.display()
        )));
    }
    for record in &doc.records {
        current.keep(&record.path)?;
        walk::remove_any(&record.path)?;
        if let Some(_target) = &record.link {
            walk::ensure_parent(&record.path)?;
            #[cfg(unix)]
            std::os::unix::fs::symlink(_target, &record.path)?;
        } else if let Some(stored) = &record.stored {
            walk::copy_tree(&dir.join(stored), &record.path)?;
        }
    }
    Ok((chosen, current.id()))
}

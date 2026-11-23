//! Output of `haryx ai profile`: text for people, JSON (`--json`) for the macOS app. Shapes are in
//! `packages/contracts/haryx/profile-*.json`.

use std::path::Path;

use haryx_core::printer::{accent, bolded, dimmed, muted, reddened, warning, yellowed};
use haryx_core::profile::apply::Outcome;
use haryx_core::profile::diff::{Line, State};
use haryx_core::profile::inventory::{Row, Status};
use haryx_core::profile::manifest::Manifest;
use haryx_core::profile::plan::{Action, Plan};
use haryx_core::profile::store::Summary;
use haryx_core::profile::{Category, DEFAULT_PROFILE};
use serde_json::{Value, json};

pub fn print_json(value: &Value) {
    println!(
        "{}",
        serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".into())
    );
}

/// `1.2 MB` style sizes.
pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

// ------------------------------------------------------------------------------------------------ list

pub fn list_json(profiles: &[Summary]) -> Value {
    json!({
        "schema_version": 1,
        "default": DEFAULT_PROFILE,
        "profiles": profiles.iter().map(|p| json!({
            "name": p.name,
            "created_at": p.created_at,
            "categories": p.categories,
            "total_bytes": p.total_bytes,
            "secrets": p.secrets.as_str(),
            "plugins": p.plugins.as_str(),
            "has_credentials": p.has_credentials,
            "source_os": p.source_os,
            "last_applied": p.last_applied,
        })).collect::<Vec<_>>(),
    })
}

pub fn print_list(profiles: &[Summary]) {
    println!(
        "{}  {}",
        bolded(DEFAULT_PROFILE),
        muted("the live ~/.claude")
    );
    if profiles.is_empty() {
        println!(
            "{}",
            dimmed(&format!(
                "No saved profiles. Save one with '{} profile save NAME'.",
                haryx_core::AI_PROG
            ))
        );
        return;
    }
    for p in profiles {
        let marker = if p.last_applied {
            accent(" (last applied)")
        } else {
            String::new()
        };
        let creds = if p.has_credentials {
            ", credentials"
        } else {
            ""
        };
        println!(
            "{}{marker}  {}",
            bolded(&p.name),
            muted(&format!(
                "{}, {} categories, secrets {}{creds}, from {}, {}",
                human_size(p.total_bytes),
                p.categories.len(),
                p.secrets.as_str(),
                p.source_os,
                p.created_at
            ))
        );
    }
}

// ------------------------------------------------------------------------------------------------ show

pub fn show_json(name: &str, rows: &[Row]) -> Value {
    json!({
        "schema_version": 1,
        "profile": name,
        "items": rows.iter().map(|r| json!({
            "category": r.category.name(),
            "id": r.id,
            "kind": r.kind,
            "size": r.size,
            "files": r.files,
            "status": r.status.as_str(),
            "detail": r.detail,
        })).collect::<Vec<_>>(),
    })
}

fn status_label(status: Status) -> String {
    match status {
        Status::Ok => String::new(),
        Status::Broken => reddened(" [broken]"),
        Status::Secret => yellowed(" [secret]"),
        Status::NeedsSecret => yellowed(" [needs-secret]"),
    }
}

pub fn print_show(name: &str, rows: &[Row]) {
    let title = if name == DEFAULT_PROFILE {
        "default (live ~/.claude)".to_owned()
    } else {
        name.to_owned()
    };
    println!("{}", bolded(&title));
    for category in Category::ALL {
        let in_cat: Vec<&Row> = rows.iter().filter(|r| r.category == category).collect();
        if in_cat.is_empty() {
            continue;
        }
        let optional = if category.default_on() {
            ""
        } else {
            " (opt-in)"
        };
        println!("\n{}{}", accent(category.name()), muted(optional));
        for row in in_cat {
            let size = if row.files > 0 {
                muted(&format!(
                    "  {} in {} file(s)",
                    human_size(row.size),
                    row.files
                ))
            } else {
                String::new()
            };
            println!("  {}{}{size}", row.id, status_label(row.status));
            for detail in &row.detail {
                println!("    {}", dimmed(detail));
            }
        }
    }
}

// ------------------------------------------------------------------------------------------------ save/export/import

fn category_sizes(manifest: &Manifest) -> Vec<(String, u64)> {
    let mut sizes: std::collections::BTreeMap<String, u64> = Default::default();
    for (path, record) in &manifest.files {
        let label = path
            .strip_prefix("files/claude/")
            .and_then(|rest| rest.split('/').next())
            .and_then(haryx_core::profile::catalog::classify)
            .map(|c| c.name().to_owned())
            .unwrap_or_else(|| {
                if path.starts_with("files/json/") {
                    "settings & mcps".into()
                } else {
                    "other".into()
                }
            });
        *sizes.entry(label).or_default() += record.size;
    }
    sizes.into_iter().collect()
}

pub fn print_saved(manifest: &Manifest) {
    println!(
        "Saved profile '{}' ({}, {} file(s)).",
        manifest.name,
        human_size(manifest.total_bytes),
        manifest.files.len()
    );
    print_secret_note(manifest);
}

fn print_secret_note(manifest: &Manifest) {
    if !manifest.secrets.is_empty() {
        println!(
            "{}",
            dimmed(&format!("Secrets found: {}", manifest.secrets.join(", ")))
        );
    }
}

pub fn print_exported(out: &Path, manifest: &Manifest, warnings: &[String]) {
    println!(
        "Exported to {} ({}).",
        out.display(),
        human_size(std::fs::metadata(out).map(|m| m.len()).unwrap_or(0))
    );
    for (label, size) in category_sizes(manifest) {
        println!("  {label:<16} {}", muted(&human_size(size)));
    }
    match manifest.options.secrets.as_str() {
        "redact" if !manifest.secrets.is_empty() => println!(
            "{}",
            dimmed(&format!(
                "Secrets redacted: {}. On the other machine set HARYX_SECRET_<NAME> or re-enter them.",
                manifest.secrets.join(", ")
            ))
        ),
        "encrypt" => println!("{}", dimmed("Secrets are encrypted with your passphrase.")),
        _ => {}
    }
    if manifest.has_credentials {
        warning(
            "This file holds account logins. Refresh tokens rotate: using one login on two machines \
             can sign the other out (fix with 'haryx ai login N').",
        );
    }
    for w in warnings {
        warning(w);
    }
}

pub fn print_imported(name: &str, manifest: &Manifest) {
    println!(
        "Imported profile '{name}' ({}, from {} {}, made {}).",
        human_size(manifest.total_bytes),
        manifest.source.os,
        manifest.source.arch,
        manifest.created_at
    );
    if !manifest.requires.is_empty() {
        println!(
            "{}",
            dimmed(&format!("It calls: {}", manifest.requires.join(", ")))
        );
    }
}

// ------------------------------------------------------------------------------------------------ plan/apply

pub fn plan_json(name: &str, plan: &Plan) -> Value {
    json!({
        "schema_version": 1,
        "profile": name,
        "prune": plan.prune,
        "entries": plan.entries.iter().map(|e| json!({
            "category": e.item.category.name(),
            "id": e.item.id,
            "action": e.action.as_str(),
            "note": e.note,
            "needs_secret": e.needs_secret,
        })).collect::<Vec<_>>(),
        "runs_code": plan.runs_code,
        "missing_requires": plan.missing_requires,
        "warnings": plan.warnings,
    })
}

fn action_label(action: Action) -> String {
    match action {
        Action::Add => accent("add   "),
        Action::Update => yellowed("update"),
        Action::Merge => accent("merge "),
        Action::Remove => reddened("remove"),
        Action::Same => muted("same  "),
        Action::Skip => muted("skip  "),
    }
}

pub fn print_plan(plan: &Plan) {
    let changes: Vec<_> = plan
        .entries
        .iter()
        .filter(|e| e.action != Action::Same)
        .collect();
    let same = plan.entries.len() - changes.len();
    for e in &changes {
        let note = e
            .note
            .as_deref()
            .map(|n| muted(&format!("  {n}")))
            .unwrap_or_default();
        let needs = if e.needs_secret.is_empty() {
            String::new()
        } else {
            yellowed(&format!("  needs {}", e.needs_secret.join(", ")))
        };
        println!(
            "{} {:<14} {}{note}{needs}",
            action_label(e.action),
            e.item.category.name(),
            e.item.id
        );
    }
    if same > 0 {
        println!("{}", muted(&format!("{same} item(s) already the same")));
    }
    if !plan.missing_requires.is_empty() {
        warning(&format!(
            "Not on this machine's PATH: {} (hooks or MCP servers that call them will fail)",
            plan.missing_requires.join(", ")
        ));
    }
    for w in &plan.warnings {
        warning(w);
    }
}

pub fn print_confirmation(plan: &Plan, removes: usize) {
    if !plan.runs_code.is_empty() {
        println!(
            "\n{}",
            bolded("This profile installs code that will run on this machine:")
        );
        for line in &plan.runs_code {
            println!("  {line}");
        }
    }
    if removes > 0 {
        println!(
            "\n{}",
            bolded(&format!(
                "{removes} local item(s) will be removed (--prune)."
            ))
        );
    }
    println!(
        "{}",
        dimmed("A backup is made first; undo with 'haryx ai profile rollback'.")
    );
}

pub fn print_outcome(outcome: &Outcome, errors: &[String]) {
    println!(
        "Applied {} change(s). Backup: {}.",
        outcome.applied.len(),
        outcome.backup_id
    );
    for e in errors {
        warning(e);
    }
    println!(
        "{}",
        dimmed("Restart running Claude Code sessions to pick up the changes.")
    );
}

// ------------------------------------------------------------------------------------------------ diff/backups

pub fn diff_json(left: &str, right: &str, lines: &[Line]) -> Value {
    json!({
        "schema_version": 1,
        "left": left,
        "right": right,
        "items": lines.iter().map(|l| json!({
            "category": l.category.name(),
            "id": l.id,
            "state": l.state.as_str(),
        })).collect::<Vec<_>>(),
    })
}

pub fn print_diff(left: &str, right: &str, lines: &[Line]) {
    let differing: Vec<&Line> = lines.iter().filter(|l| l.state != State::Same).collect();
    if differing.is_empty() {
        println!("'{left}' and '{right}' hold the same items.");
        return;
    }
    for l in differing {
        let label = match l.state {
            State::OnlyLeft => accent(&format!("only in {left}")),
            State::OnlyRight => yellowed(&format!("only in {right}")),
            State::Differs => muted("differs"),
            State::Same => String::new(),
        };
        println!("{:<14} {}  {label}", l.category.name(), l.id);
    }
}

pub fn print_backups(backups: &[(String, String, String)]) {
    if backups.is_empty() {
        println!("No backups.");
        return;
    }
    for (id, profile, at) in backups {
        println!(
            "{}  {}",
            bolded(id),
            muted(&format!("before applying {profile}, {at}"))
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn human_sizes() {
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(1536), "1.5 KB");
        assert_eq!(human_size(73 * 1024 * 1024), "73.0 MB");
    }
}

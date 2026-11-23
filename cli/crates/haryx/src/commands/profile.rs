//! `haryx ai profile`: the command side of [`haryx_core::profile`]. Prompts, confirmation and output
//! live here; every file operation is in core.

use std::collections::{BTreeMap, BTreeSet};
use std::io::IsTerminal;
use std::path::{Path, PathBuf};

use haryx_core::Error;
use haryx_core::printer::{dimmed, warning};
use haryx_core::profile::collect::CollectRequest;
use haryx_core::profile::manifest::{Manifest, SECRETS_AGE_FILE, SecretsDoc};
use haryx_core::profile::plan::{self, ApplyRequest, Ctx, Plan};
use haryx_core::profile::store::{self, ExportRequest, ProfileStore};
use haryx_core::profile::{
    Category, DEFAULT_PROFILE, PluginsMode, ProfileEnv, SecretsMode, Selection, apply, diff,
    inventory, plugins, secrets,
};
use haryx_core::switcher::{Switcher, prompt, prompt_secret};

use super::accounts::fail;
use super::profile_view as view;
use super::{CommandError, guard};
use crate::cli::{PartFlags, PluginsChoice, ProfileAction, SecretsChoice};

/// Environment variable that supplies the passphrase without a prompt.
const PASSPHRASE_ENV: &str = "HARYX_PROFILE_PASSPHRASE";
/// Export size above which a warning is printed.
const LARGE_EXPORT_BYTES: u64 = 500 * 1024 * 1024;

pub fn run(action: &ProfileAction, debug: bool) -> Result<(), CommandError> {
    guard::refuse_root()?;
    let env = ProfileEnv::from_process().map_err(fail)?;
    match action {
        ProfileAction::List { json } => list(&env, *json),
        ProfileAction::Show { name, json } => show(&env, name, *json),
        ProfileAction::Save {
            name,
            parts,
            plugins,
            force,
        } => save(&env, name, parts, *plugins, *force, debug),
        ProfileAction::Export {
            name,
            file,
            parts,
            secrets,
            plugins,
        } => export(&env, name, file, parts, *secrets, *plugins, debug),
        ProfileAction::Import {
            name,
            file,
            dry_run,
            apply,
            force,
            yes,
            prune,
        } => import(
            &env, name, file, *dry_run, *apply, *force, *yes, *prune, debug,
        ),
        ProfileAction::Apply {
            name,
            dry_run,
            yes,
            prune,
            parts,
            json,
        } => {
            let opts = ApplyOpts {
                dry_run: *dry_run,
                yes: *yes,
                prune: *prune,
                json: *json,
                selection: selection(parts),
            };
            apply_saved(&env, name, &opts, debug)
        }
        ProfileAction::Diff { name, other, json } => diff_cmd(&env, name, other, *json),
        ProfileAction::Delete { name, yes } => delete(&env, name, *yes),
        ProfileAction::Rollback { backup, list } => rollback(&env, backup.as_deref(), *list),
    }
}

fn selection(parts: &PartFlags) -> Selection {
    Selection::from_flags(parts.only.as_deref(), &parts.extra, &parts.exclude)
}

fn plugins_mode(choice: PluginsChoice) -> PluginsMode {
    match choice {
        PluginsChoice::Ref => PluginsMode::Ref,
        PluginsChoice::Bundle => PluginsMode::Bundle,
    }
}

fn not_default(name: &str, verb: &str) -> Result<(), CommandError> {
    if name == DEFAULT_PROFILE {
        return Err(fail(Error::Validation(format!(
            "'{DEFAULT_PROFILE}' is the live ~/.claude; {verb} needs a saved profile"
        ))));
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------- list/show

fn list(env: &ProfileEnv, json: bool) -> Result<(), CommandError> {
    let profiles = ProfileStore::new(env).list().map_err(fail)?;
    if json {
        view::print_json(&view::list_json(&profiles));
    } else {
        view::print_list(&profiles);
    }
    Ok(())
}

fn show(env: &ProfileEnv, name: &str, json: bool) -> Result<(), CommandError> {
    let rows = if name == DEFAULT_PROFILE {
        inventory::fold_history(inventory::live(env).map_err(fail)?)
    } else {
        let store = ProfileStore::new(env);
        let dir = store.require(name).map_err(fail)?;
        let manifest = Manifest::load(&dir).map_err(fail)?;
        let available = available_secret_names(&dir, &manifest);
        inventory::fold_history(inventory::stored(env, &dir, &manifest, &available).map_err(fail)?)
    };
    if json {
        view::print_json(&view::show_json(name, &rows));
    } else {
        view::print_show(name, &rows);
    }
    Ok(())
}

/// Secret names whose values are at hand without a passphrase prompt (local values, encrypted ones,
/// or the environment).
fn available_secret_names(dir: &Path, manifest: &Manifest) -> BTreeSet<String> {
    let mut names: BTreeSet<String> = store::load_secrets(dir, &mut || None)
        .map(|doc| doc.values.into_keys().collect())
        .unwrap_or_default();
    if dir.join(SECRETS_AGE_FILE).is_file() {
        names.extend(manifest.secrets.iter().cloned());
    }
    names.extend(
        manifest
            .secrets
            .iter()
            .filter(|n| std::env::var_os(format!("{}{n}", secrets::VAR_PREFIX)).is_some())
            .cloned(),
    );
    names
}

// ---------------------------------------------------------------------------------------- save/export

fn credentials_envelope(
    sel: &Selection,
    debug: bool,
) -> Result<Option<serde_json::Value>, CommandError> {
    if !sel.has(Category::Credentials) {
        return Ok(None);
    }
    let switcher = Switcher::from_process(debug).map_err(fail)?;
    haryx_core::transfer::build_export(&switcher, None, true)
        .map(Some)
        .map_err(fail)
}

fn collect_request(
    name: &str,
    parts: &PartFlags,
    plugins: PluginsChoice,
    debug: bool,
) -> Result<CollectRequest, CommandError> {
    let sel = selection(parts);
    Ok(CollectRequest {
        name: name.to_owned(),
        credentials: credentials_envelope(&sel, debug)?,
        selection: sel,
        plugins: plugins_mode(plugins),
        include_dirs: parts.include_dirs.iter().map(PathBuf::from).collect(),
    })
}

fn save(
    env: &ProfileEnv,
    name: &str,
    parts: &PartFlags,
    plugins: PluginsChoice,
    force: bool,
    debug: bool,
) -> Result<(), CommandError> {
    not_default(name, "save")?;
    let req = collect_request(name, parts, plugins, debug)?;
    let manifest = ProfileStore::new(env)
        .save(env, &req, force)
        .map_err(fail)?;
    view::print_saved(&manifest);
    Ok(())
}

fn export(
    env: &ProfileEnv,
    name: &str,
    file: &str,
    parts: &PartFlags,
    secrets_choice: SecretsChoice,
    plugins: PluginsChoice,
    debug: bool,
) -> Result<(), CommandError> {
    let out = PathBuf::from(file);
    let sel = selection(parts);
    let secrets_mode = match secrets_choice {
        SecretsChoice::Redact => SecretsMode::Redact,
        SecretsChoice::Encrypt => SecretsMode::Encrypt,
    };
    if sel.has(Category::Credentials) && secrets_mode != SecretsMode::Encrypt {
        return Err(fail(Error::Validation(
            "--credentials needs --secrets encrypt (credentials are never written in plain text)"
                .into(),
        )));
    }
    let store = ProfileStore::new(env);
    let work_root = env.profiles_dir();
    let snapshot = work_root.join(format!(".tmp-export-src-{}", std::process::id()));
    let src = if name == DEFAULT_PROFILE {
        let req = collect_request(name, parts, plugins, debug)?;
        let _ = haryx_core::profile::walk::remove_any(&snapshot);
        haryx_core::profile::collect::collect(env, &snapshot, &req).map_err(fail)?;
        snapshot.clone()
    } else {
        store.require(name).map_err(fail)?
    };
    let result = export_from(&src, &sel, secrets_mode, plugins, &out, &work_root);
    let _ = haryx_core::profile::walk::remove_any(&snapshot);
    let exported = result?;
    view::print_exported(&out, &exported.manifest, &exported.warnings);
    if exported.manifest.total_bytes > LARGE_EXPORT_BYTES {
        warning("This export is large; leave history out with --no-history to shrink it.");
    }
    Ok(())
}

fn export_from(
    src: &Path,
    sel: &Selection,
    secrets_mode: SecretsMode,
    plugins: PluginsChoice,
    out: &Path,
    work_root: &Path,
) -> Result<store::Exported, CommandError> {
    let doc = store::load_secrets(src, &mut || {
        ask_passphrase("Passphrase of this profile: ", false)
    })
    .map_err(fail)?;
    let passphrase = match secrets_mode {
        SecretsMode::Encrypt => Some(
            ask_passphrase("Passphrase to encrypt the secrets: ", true).ok_or_else(|| {
                fail(Error::Validation(format!(
                    "--secrets encrypt needs a passphrase (prompt, or {PASSPHRASE_ENV})"
                )))
            })?,
        ),
        _ => None,
    };
    let req = ExportRequest {
        selection: sel.clone(),
        secrets: secrets_mode,
        plugins: plugins_mode(plugins),
        passphrase,
    };
    let work = work_root.join(format!(".tmp-export-{}", std::process::id()));
    store::export(src, &doc, &req, &work, out).map_err(fail)
}

/// The passphrase from [`PASSPHRASE_ENV`], or a prompt at a terminal (asked twice when `confirm`).
fn ask_passphrase(question: &str, confirm: bool) -> Option<String> {
    if let Some(pass) = std::env::var(PASSPHRASE_ENV).ok().filter(|p| !p.is_empty()) {
        return Some(pass);
    }
    if !std::io::stdin().is_terminal() {
        return None;
    }
    let first = prompt_secret(question)?;
    if confirm {
        let again = prompt_secret("Again: ")?;
        if again != first {
            warning("The passphrases differ.");
            return None;
        }
    }
    Some(first)
}

// ---------------------------------------------------------------------------------------- import/apply

struct ApplyOpts {
    dry_run: bool,
    yes: bool,
    prune: bool,
    json: bool,
    selection: Selection,
}

#[allow(clippy::too_many_arguments)]
fn import(
    env: &ProfileEnv,
    name: &str,
    file: &str,
    dry_run: bool,
    then_apply: bool,
    force: bool,
    yes: bool,
    prune: bool,
    debug: bool,
) -> Result<(), CommandError> {
    let store = ProfileStore::new(env);
    let opts = ApplyOpts {
        dry_run,
        yes,
        prune,
        json: false,
        selection: Selection::everything(),
    };
    if dry_run {
        store::validate_name(name).map_err(fail)?;
        let tmp = env
            .profiles_dir()
            .join(format!(".tmp-dryrun-{}", std::process::id()));
        let _ = haryx_core::profile::walk::remove_any(&tmp);
        haryx_core::profile::walk::mkdirs(&env.profiles_dir()).map_err(fail)?;
        let result = haryx_core::profile::archive::extract(Path::new(file), &tmp)
            .map_err(fail)
            .and_then(|manifest| apply_dir(env, &tmp, &manifest, &opts, debug));
        let _ = haryx_core::profile::walk::remove_any(&tmp);
        return result;
    }
    let manifest = store.import(name, Path::new(file), force).map_err(fail)?;
    view::print_imported(name, &manifest);
    if then_apply {
        return apply_dir(env, &store.dir(name), &manifest, &opts, debug);
    }
    println!(
        "{}",
        dimmed(&format!(
            "Preview with '{} profile apply {name} --dry-run', then apply without --dry-run.",
            haryx_core::AI_PROG
        ))
    );
    Ok(())
}

fn apply_saved(
    env: &ProfileEnv,
    name: &str,
    opts: &ApplyOpts,
    debug: bool,
) -> Result<(), CommandError> {
    not_default(name, "apply")?;
    let store = ProfileStore::new(env);
    let dir = store.require(name).map_err(fail)?;
    let manifest = Manifest::load(&dir).map_err(fail)?;
    apply_dir(env, &dir, &manifest, opts, debug)
}

/// Secret values for an apply: the profile's own (passphrase asked only when the dry run is not
/// enough), then the environment.
fn secret_values(
    dir: &Path,
    manifest: &Manifest,
    dry_run: bool,
) -> Result<SecretsDoc, CommandError> {
    let sealed = dir.join(SECRETS_AGE_FILE).is_file();
    let doc = if sealed && dry_run && std::env::var_os(PASSPHRASE_ENV).is_none() {
        SecretsDoc::default()
    } else {
        store::load_secrets(dir, &mut || {
            ask_passphrase("Passphrase of this profile: ", false)
        })
        .map_err(fail)?
    };
    let values = secrets::with_env(&doc.values, &manifest.secrets, &|k| std::env::var(k).ok());
    Ok(SecretsDoc { values, ..doc })
}

fn apply_dir(
    env: &ProfileEnv,
    dir: &Path,
    manifest: &Manifest,
    opts: &ApplyOpts,
    debug: bool,
) -> Result<(), CommandError> {
    let doc = secret_values(dir, manifest, opts.dry_run)?;
    let ctx = Ctx::new(env, dir, manifest, &doc.values);
    let req = ApplyRequest {
        selection: opts.selection.clone(),
        prune: opts.prune,
    };
    let built = plan::build(&ctx, &req).map_err(fail)?;
    if opts.json {
        view::print_json(&view::plan_json(&manifest.name, &built));
        return Ok(());
    }
    view::print_plan(&built);
    if opts.dry_run {
        return Ok(());
    }
    if built.changes().next().is_none() {
        println!("Nothing to change.");
        return Ok(());
    }
    confirm(&built, opts.yes)?;
    let installer: Box<dyn plugins::PluginInstaller> = match plugins::ClaudeCli::from_path() {
        Ok(cli) => Box::new(cli),
        Err(e) => Box::new(Unavailable(e.to_string())),
    };
    let outcome =
        apply::apply(&ctx, &built, installer.as_ref(), doc.credentials.clone()).map_err(fail)?;
    let mut errors = outcome.errors.clone();
    if let Some(envelope) = &outcome.credentials {
        let switcher = Switcher::from_process(debug).map_err(fail)?;
        let text = serde_json::to_string(envelope).unwrap_or_default();
        if let Err(e) = haryx_core::transfer::import_envelope_text(&switcher, &text, false) {
            errors.push(format!("accounts: {e}"));
        }
    }
    if manifest.name != DEFAULT_PROFILE {
        let _ = ProfileStore::new(env).write_state(Some(&manifest.name));
    }
    view::print_outcome(&outcome, &errors);
    Ok(())
}

/// Ask before writing anything that runs code or removes items.
fn confirm(built: &Plan, yes: bool) -> Result<(), CommandError> {
    let removes = built
        .changes()
        .filter(|e| e.action == plan::Action::Remove)
        .count();
    if built.runs_code.is_empty() && removes == 0 {
        return Ok(());
    }
    view::print_confirmation(built, removes);
    if yes {
        return Ok(());
    }
    if !std::io::stdin().is_terminal() {
        return Err(fail(Error::Validation(
            "This profile installs code or removes items; re-run with --yes to accept".into(),
        )));
    }
    match prompt("Apply? [y/N] ") {
        Some(answer) if matches!(answer.trim(), "y" | "Y" | "yes") => Ok(()),
        _ => Err(CommandError {
            message: "Cancelled; nothing was changed.".into(),
            exit_code: 1,
        }),
    }
}

/// Plugin installs when `claude` is missing: every call reports why.
struct Unavailable(String);

impl plugins::PluginInstaller for Unavailable {
    fn add_marketplace(&self, _source: &str) -> haryx_core::Result<()> {
        Err(Error::General(self.0.clone()))
    }
    fn install(&self, _id: &str) -> haryx_core::Result<()> {
        Err(Error::General(self.0.clone()))
    }
}

// ---------------------------------------------------------------------------------------- diff/delete/rollback

fn diff_cmd(env: &ProfileEnv, name: &str, other: &str, json: bool) -> Result<(), CommandError> {
    not_default(name, "diff")?;
    let store = ProfileStore::new(env);
    let dir = store.require(name).map_err(fail)?;
    let manifest = Manifest::load(&dir).map_err(fail)?;
    let values = BTreeMap::new();
    let left = Ctx::new(env, &dir, &manifest, &values);
    let lines = if other == DEFAULT_PROFILE {
        diff::against_live(&left).map_err(fail)?
    } else {
        let other_dir = store.require(other).map_err(fail)?;
        let other_manifest = Manifest::load(&other_dir).map_err(fail)?;
        let right = Ctx::new(env, &other_dir, &other_manifest, &values);
        diff::between(&left, &right).map_err(fail)?
    };
    if json {
        view::print_json(&view::diff_json(name, other, &lines));
    } else {
        view::print_diff(name, other, &lines);
    }
    Ok(())
}

fn delete(env: &ProfileEnv, name: &str, yes: bool) -> Result<(), CommandError> {
    let store = ProfileStore::new(env);
    store::validate_name(name).map_err(fail)?;
    store.require(name).map_err(fail)?;
    if !yes {
        if !std::io::stdin().is_terminal() {
            return Err(fail(Error::Validation(format!(
                "Deleting profile '{name}' needs --yes when not at a terminal"
            ))));
        }
        match prompt(&format!("Delete profile '{name}'? [y/N] ")) {
            Some(a) if matches!(a.trim(), "y" | "Y" | "yes") => {}
            _ => {
                println!("{}", dimmed("Cancelled"));
                return Ok(());
            }
        }
    }
    store.delete(name).map_err(fail)?;
    println!("Deleted profile '{name}'.");
    Ok(())
}

fn rollback(env: &ProfileEnv, backup: Option<&str>, list: bool) -> Result<(), CommandError> {
    if list {
        view::print_backups(&apply::list_backups(env).map_err(fail)?);
        return Ok(());
    }
    let (restored, saved) = apply::rollback(env, backup).map_err(fail)?;
    println!("Restored backup {restored}. The state before the rollback is backup {saved}.");
    println!(
        "{}",
        dimmed("Plugins installed and accounts imported by that apply are not undone.")
    );
    Ok(())
}

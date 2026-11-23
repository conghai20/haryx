//! `haryx ai profile`: list, show, save, export, import, apply, diff, delete and roll back Claude Code
//! setup profiles (plan: `docs/ai-profile-plan.md`).

use clap::error::ErrorKind;
use clap::{Args, CommandFactory, Parser, Subcommand, ValueEnum};
use haryx_core::profile::Category;

use super::help::{self, p, sub_bin};
use super::{Command, Error, Invocation};

/// Which parts an operation covers, as typed.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PartFlags {
    pub only: Option<Vec<Category>>,
    pub exclude: Vec<Category>,
    pub extra: Vec<Category>,
    pub include_dirs: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum SecretsChoice {
    Redact,
    Encrypt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum PluginsChoice {
    Ref,
    Bundle,
}

/// `profile` verbs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProfileAction {
    List {
        json: bool,
    },
    Show {
        name: String,
        json: bool,
    },
    Save {
        name: String,
        parts: PartFlags,
        plugins: PluginsChoice,
        force: bool,
    },
    Export {
        name: String,
        file: String,
        parts: PartFlags,
        secrets: SecretsChoice,
        plugins: PluginsChoice,
    },
    Import {
        name: String,
        file: String,
        dry_run: bool,
        apply: bool,
        force: bool,
        yes: bool,
        prune: bool,
    },
    Apply {
        name: String,
        dry_run: bool,
        yes: bool,
        prune: bool,
        parts: PartFlags,
        json: bool,
    },
    Diff {
        name: String,
        other: String,
        json: bool,
    },
    Delete {
        name: String,
        yes: bool,
    },
    Rollback {
        backup: Option<String>,
        list: bool,
    },
}

#[derive(Parser, Debug)]
#[command(
    name = "profile",
    bin_name = sub_bin("profile"),
    about = p("See, save, export and import your Claude Code setup (skills, hooks, MCP servers, \
               plugins, history, ...) to move it between machines. The profile 'default' is the \
               live ~/.claude."),
    after_help = profile_epilog(),
    help_template = help::HELP_TEMPLATE,
    disable_help_subcommand = true,
    subcommand_value_name = "{list,show,save,export,import,apply,diff,delete,rollback}",
    subcommand_help_heading = "Actions",
)]
struct ProfileArgs {
    /// Enable debug logging
    #[arg(long, global = true)]
    debug: bool,

    #[command(subcommand)]
    action: Option<Verb>,
}

#[derive(Args, Debug, Clone)]
struct Parts {
    /// Only these categories (comma list)
    #[arg(long, value_name = "CATS", value_parser = parse_categories)]
    only: Option<CategoryList>,
    /// Leave these categories out (comma list)
    #[arg(long, value_name = "CATS", value_parser = parse_categories)]
    exclude: Vec<CategoryList>,
    /// Leave history out (conversations, prompt history, checkpoints, todos, plans)
    #[arg(long)]
    no_history: bool,
}

#[derive(Args, Debug, Clone)]
struct OptionalParts {
    /// Include the accounts of the haryx store (needs --secrets encrypt to export)
    #[arg(long)]
    credentials: bool,
    /// Include Anthropic-managed signed files (policy-limits, remote-settings)
    #[arg(long)]
    signed: bool,
    /// Include the whole ~/.claude.json, not only its MCP servers
    #[arg(long)]
    claude_json: bool,
    /// Include caches (cache/, paste-cache/, statsig/)
    #[arg(long)]
    cache: bool,
    /// Also carry this folder (repeatable)
    #[arg(long, value_name = "PATH")]
    include_dir: Vec<String>,
}

#[derive(Subcommand, Debug)]
enum Verb {
    /// List saved profiles (the default)
    List {
        #[arg(long)]
        json: bool,
    },
    /// Show what a profile holds (default: the live ~/.claude)
    Show {
        #[arg(value_name = "NAME", default_value = "default")]
        name: String,
        #[arg(long)]
        json: bool,
    },
    /// Snapshot the live ~/.claude as NAME
    Save {
        #[arg(value_name = "NAME")]
        name: String,
        #[command(flatten)]
        parts: Parts,
        #[command(flatten)]
        optional: OptionalParts,
        /// Keep plugin files (bundle) or only references (ref)
        #[arg(long, value_enum, default_value = "ref")]
        plugins: PluginsChoice,
        /// Replace an existing profile of that name
        #[arg(long)]
        force: bool,
    },
    /// Write a portable .haryx-profile file: export [NAME] FILE
    Export {
        #[arg(value_name = "[NAME] FILE", num_args = 1..=2, required = true)]
        args: Vec<String>,
        #[command(flatten)]
        parts: Parts,
        #[command(flatten)]
        optional: OptionalParts,
        /// Drop secret values (redact) or encrypt them with a passphrase (encrypt)
        #[arg(long, value_enum, default_value = "redact")]
        secrets: SecretsChoice,
        /// Plugin references (ref) or plugin files too (bundle)
        #[arg(long, value_enum, default_value = "ref")]
        plugins: PluginsChoice,
    },
    /// Store an exported file as NAME
    Import {
        #[arg(value_name = "NAME")]
        name: String,
        #[arg(value_name = "FILE")]
        file: String,
        /// Show what applying it would change; store nothing
        #[arg(long)]
        dry_run: bool,
        /// Apply it right after storing
        #[arg(long)]
        apply: bool,
        /// Replace an existing profile of that name
        #[arg(long)]
        force: bool,
        /// With --apply: do not ask for confirmation
        #[arg(long, short = 'y')]
        yes: bool,
        /// With --apply or --dry-run: also remove local items the profile does not have
        #[arg(long)]
        prune: bool,
    },
    /// Write a saved profile into ~/.claude (merging; backup first)
    Apply {
        #[arg(value_name = "NAME")]
        name: String,
        /// Show the plan only
        #[arg(long)]
        dry_run: bool,
        /// Do not ask for confirmation
        #[arg(long, short = 'y')]
        yes: bool,
        /// Also remove local items the profile does not have (never history)
        #[arg(long)]
        prune: bool,
        #[command(flatten)]
        parts: Parts,
        /// With --dry-run: print the plan as JSON
        #[arg(long)]
        json: bool,
    },
    /// Compare a saved profile with the live ~/.claude or another profile
    Diff {
        #[arg(value_name = "NAME")]
        name: String,
        #[arg(value_name = "OTHER", default_value = "default")]
        other: String,
        #[arg(long)]
        json: bool,
    },
    /// Delete a saved profile
    Delete {
        #[arg(value_name = "NAME")]
        name: String,
        #[arg(long, short = 'y')]
        yes: bool,
    },
    /// Undo the last apply (or the given backup)
    Rollback {
        #[arg(value_name = "BACKUP")]
        backup: Option<String>,
        /// List the backups instead
        #[arg(long)]
        list: bool,
    },
}

/// One `--only`/`--exclude` value: a comma list of categories.
#[derive(Debug, Clone)]
struct CategoryList(Vec<Category>);

fn parse_categories(text: &str) -> Result<CategoryList, String> {
    Category::parse_list(text).map(CategoryList)
}

fn profile_epilog() -> String {
    p("Categories: settings, hooks, skills, agents, commands, rules, output-styles, plugins, mcps,
memory, other, history (all on by default); credentials, signed, claude-json, cache (opt-in).

Examples:
  {ai} profile show                          what ~/.claude holds now
  {ai} profile export work.haryx-profile     export the live setup (secrets redacted)
  {ai} profile export work.haryx-profile --secrets encrypt --credentials
  {ai} profile import work work.haryx-profile --dry-run
  {ai} profile import work work.haryx-profile --apply
  {ai} profile apply work --prune            make ~/.claude match 'work' (history kept)
  {ai} profile rollback                      undo the last apply

Secrets: placeholders ${HARYX_SECRET_NAME} are filled from the profile, then from the environment
variable of the same name. HARYX_PROFILE_PASSPHRASE supplies the passphrase without a prompt.")
}

impl Parts {
    fn into_flags(self, optional: Option<OptionalParts>) -> PartFlags {
        let mut exclude: Vec<Category> = self.exclude.into_iter().flat_map(|l| l.0).collect();
        if self.no_history {
            exclude.push(Category::History);
        }
        let (extra, include_dirs) = match optional {
            Some(o) => {
                let extra = [
                    (o.credentials, Category::Credentials),
                    (o.signed, Category::Signed),
                    (o.claude_json, Category::ClaudeJson),
                    (o.cache, Category::Cache),
                ]
                .into_iter()
                .filter_map(|(on, c)| on.then_some(c))
                .collect();
                (extra, o.include_dir)
            }
            None => (Vec::new(), Vec::new()),
        };
        PartFlags {
            only: self.only.map(|l| l.0),
            exclude,
            extra,
            include_dirs,
        }
    }
}

fn usage_error(message: &str) -> Error {
    ProfileArgs::command().error(ErrorKind::ArgumentConflict, message)
}

pub(super) fn parse(rest: &[String]) -> Result<Invocation, Error> {
    let args = ProfileArgs::try_parse_from(
        std::iter::once(haryx_core::PROG.to_owned()).chain(rest.iter().cloned()),
    )?;
    let action = match args.action.unwrap_or(Verb::List { json: false }) {
        Verb::List { json } => ProfileAction::List { json },
        Verb::Show { name, json } => ProfileAction::Show { name, json },
        Verb::Save {
            name,
            parts,
            optional,
            plugins,
            force,
        } => ProfileAction::Save {
            name,
            parts: parts.into_flags(Some(optional)),
            plugins,
            force,
        },
        Verb::Export {
            args: positional,
            parts,
            optional,
            secrets,
            plugins,
        } => {
            let (name, file) = match positional.as_slice() {
                [file] => (
                    haryx_core::profile::DEFAULT_PROFILE.to_owned(),
                    file.clone(),
                ),
                [name, file] => (name.clone(), file.clone()),
                _ => return Err(usage_error("export takes [NAME] FILE")),
            };
            ProfileAction::Export {
                name,
                file,
                parts: parts.into_flags(Some(optional)),
                secrets,
                plugins,
            }
        }
        Verb::Import {
            name,
            file,
            dry_run,
            apply,
            force,
            yes,
            prune,
        } => {
            if dry_run && apply {
                return Err(usage_error("--dry-run and --apply cannot be used together"));
            }
            ProfileAction::Import {
                name,
                file,
                dry_run,
                apply,
                force,
                yes,
                prune,
            }
        }
        Verb::Apply {
            name,
            dry_run,
            yes,
            prune,
            parts,
            json,
        } => {
            if json && !dry_run {
                return Err(usage_error("--json is only for --dry-run"));
            }
            ProfileAction::Apply {
                name,
                dry_run,
                yes,
                prune,
                parts: parts.into_flags(None),
                json,
            }
        }
        Verb::Diff { name, other, json } => ProfileAction::Diff { name, other, json },
        Verb::Delete { name, yes } => ProfileAction::Delete { name, yes },
        Verb::Rollback { backup, list } => ProfileAction::Rollback { backup, list },
    };
    Ok(Invocation {
        debug: args.debug,
        command: Command::Profile(action),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_ok(argv: &[&str]) -> ProfileAction {
        let args: Vec<String> = argv.iter().map(|s| (*s).to_owned()).collect();
        match parse(&args).unwrap().command {
            Command::Profile(action) => action,
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn definition_is_consistent() {
        ProfileArgs::command().debug_assert();
    }

    #[test]
    fn bare_profile_lists() {
        assert_eq!(parse_ok(&[]), ProfileAction::List { json: false });
    }

    #[test]
    fn export_with_one_positional_uses_default() {
        let ProfileAction::Export {
            name,
            file,
            parts,
            secrets,
            ..
        } = parse_ok(&[
            "export",
            "out.haryx-profile",
            "--no-history",
            "--credentials",
        ])
        else {
            panic!()
        };
        assert_eq!(
            (name.as_str(), file.as_str()),
            ("default", "out.haryx-profile")
        );
        assert_eq!(parts.exclude, vec![Category::History]);
        assert_eq!(parts.extra, vec![Category::Credentials]);
        assert_eq!(secrets, SecretsChoice::Redact);
    }

    #[test]
    fn export_with_name_and_categories() {
        let ProfileAction::Export { name, parts, .. } = parse_ok(&[
            "export",
            "work",
            "f",
            "--only",
            "skills,mcps",
            "--secrets",
            "encrypt",
        ]) else {
            panic!()
        };
        assert_eq!(name, "work");
        assert_eq!(parts.only, Some(vec![Category::Skills, Category::Mcps]));
    }

    #[test]
    fn rejects_unknown_category_and_conflicts() {
        let args: Vec<String> = ["export", "f", "--only", "nope"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(parse(&args).is_err());
        let args: Vec<String> = ["import", "w", "f", "--dry-run", "--apply"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(parse(&args).is_err());
    }

    #[test]
    fn show_defaults_to_live_profile() {
        assert_eq!(
            parse_ok(&["show"]),
            ProfileAction::Show {
                name: "default".into(),
                json: false
            }
        );
    }
}

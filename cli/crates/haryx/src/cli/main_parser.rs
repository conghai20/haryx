//! The main parser: modifier flags plus the mutually exclusive group of legacy action flags, and the
//! post-parse checks `main()` runs before dispatch (same order, same messages).

use clap::error::ErrorKind;
use clap::{ArgGroup, CommandFactory, Parser};
use haryx_core::{AI_PROG, PROG};

use super::help;
use super::{Command, Error, ImportSource, Invocation, SwitchStrategy};

#[derive(Parser, Debug)]
#[command(
    name = PROG,
    bin_name = PROG,
    version = help::version(),
    override_usage = help::main_usage(),
    about = help::main_description(),
    after_help = help::main_epilog(),
    help_template = help::HELP_TEMPLATE,
    args_override_self = true,
    infer_long_args = true,
    allow_negative_numbers = true,
    disable_help_subcommand = true,
)]
#[command(group(ArgGroup::new("action").multiple(false)))]
struct MainArgs {
    /// Enable debug logging
    #[arg(long)]
    debug: bool,

    /// Show source-labelled OAuth token diagnostics (use with 'list')
    #[arg(long)]
    token_status: bool,

    /// Emit machine-readable JSON to stdout (use with 'list', 'status', or 'switch'). See README
    /// 'JSON output for scripting'.
    #[arg(long)]
    json: bool,

    /// With bare 'switch': pick the target by remaining 5h/7d quota. 'best' jumps to the account
    /// with the most headroom; 'next-available' rotates to the next account, skipping any at their
    /// limit
    #[arg(
        long,
        value_enum,
        value_name = "{best,next-available}",
        hide_possible_values = true
    )]
    strategy: Option<SwitchStrategy>,

    /// With 'switch --strategy': also count these models' per-model weekly limits when comparing
    /// accounts (comma-separated display names, or 'all'). Defaults to the autoswitch.model setting
    #[arg(long, value_name = "NAMES")]
    model: Option<String>,

    /// Specify slot number when adding account (use with 'add' or 'add-token')
    #[arg(long, value_name = "NUM")]
    slot: Option<i64>,

    /// Email address for the account. Optional with 'add-token'; defaults to
    /// setup-token-{slot}@token.local (or api-key-{slot}@token.local for API keys) since these
    /// tokens carry no real email metadata.
    #[arg(long, value_name = "EMAIL")]
    email: Option<String>,

    /// Limit export to one account (use with 'export')
    #[arg(long, value_name = "NUM|EMAIL")]
    account: Option<String>,

    /// Set a short display alias for the account (use with 'add')
    #[arg(long, value_name = "NAME")]
    alias: Option<String>,

    /// Overwrite existing accounts during import; with 'switch <num|email>', activate the stored
    /// credentials without backing up the current login first
    #[arg(long)]
    force: bool,

    /// Include full ~/.claude.json in export (default: oauthAccount only)
    #[arg(long)]
    full: bool,

    /// With 'import-usage': keep this machine from fetching the imported accounts for this many
    /// seconds (0 lifts an earlier hold)
    #[arg(long, value_name = "SECONDS")]
    hold: Option<f64>,

    /// With 'import': copy accounts from an existing cswap store instead of an export file
    #[arg(long)]
    from_cswap: bool,

    // Legacy `--flag` interface: still fully supported (subcommands rewrite into these), hidden from
    // --help so the subcommands in the description are the one documented interface.
    #[arg(long, hide = true, group = "action")]
    add_account: bool,
    #[arg(long, hide = true, group = "action", value_name = "NUM|EMAIL")]
    remove_account: Option<String>,
    #[arg(long, hide = true, group = "action", value_name = "NUM|EMAIL")]
    disable_account: Option<String>,
    #[arg(long, hide = true, group = "action", value_name = "NUM|EMAIL")]
    enable_account: Option<String>,
    #[arg(long, hide = true, group = "action")]
    list: bool,
    #[arg(long, hide = true, group = "action")]
    switch: bool,
    #[arg(long, hide = true, group = "action", value_name = "NUM|EMAIL")]
    switch_to: Option<String>,
    #[arg(long, hide = true, group = "action")]
    status: bool,
    #[arg(long, hide = true, group = "action")]
    purge: bool,
    #[arg(long, hide = true, group = "action", value_name = "PATH")]
    export: Option<String>,
    /// The value is optional only so `import --from-cswap` parses; plain `--import` still needs one.
    #[arg(long = "import", hide = true, group = "action", value_name = "PATH", num_args = 0..=1)]
    import: Option<Option<String>>,
    #[arg(long, hide = true, group = "action", value_name = "PATH")]
    import_usage: Option<String>,
    #[arg(long, hide = true, group = "action")]
    tui: bool,
    #[arg(long, hide = true, group = "action")]
    watch: bool,
    #[arg(long, hide = true, group = "action")]
    upgrade: bool,
    #[arg(long, hide = true, group = "action", value_name = "TOKEN|-", num_args = 0..=1)]
    add_token: Option<Option<String>>,
}

/// Parse the flag argv (subcommands already translated) into an [`Invocation`].
pub(super) fn parse(argv: &[String]) -> Result<Invocation, Error> {
    let args =
        MainArgs::try_parse_from(std::iter::once(PROG.to_owned()).chain(argv.iter().cloned()))?;
    let command = args
        .validate()
        .map_err(|(kind, msg)| MainArgs::command().error(kind, msg))?;
    Ok(Invocation {
        debug: args.debug,
        command,
    })
}

type Usage = (ErrorKind, String);

fn conflict(message: &str) -> Usage {
    (ErrorKind::ArgumentConflict, message.to_owned())
}

impl MainArgs {
    fn has_action(&self) -> bool {
        self.add_account
            || self.list
            || self.switch
            || self.status
            || self.purge
            || self.tui
            || self.watch
            || self.upgrade
            || self.remove_account.is_some()
            || self.disable_account.is_some()
            || self.enable_account.is_some()
            || self.switch_to.is_some()
            || self.export.is_some()
            || self.import.is_some()
            || self.import_usage.is_some()
            || self.add_token.is_some()
    }

    /// The checks `main()` runs after parsing, in upstream order, then the action.
    fn validate(&self) -> Result<Command, Usage> {
        if !self.has_action() {
            return Err((
                ErrorKind::MissingRequiredArgument,
                format!("no command given — try '{AI_PROG} help'"),
            ));
        }
        self.check_empty_values()?;
        self.check_modifiers()?;
        Ok(self.action())
    }

    /// An empty value would match no dispatch branch and exit 0 having done nothing
    /// (`haryx export "$DEST"` with DEST unset). `--add-token` is exempt: empty means "prompt".
    fn check_empty_values(&self) -> Result<(), Usage> {
        let import_path = self.import.as_ref().and_then(|value| value.as_ref());
        let values = [
            ("remove", "NUM|EMAIL", self.remove_account.as_ref()),
            ("disable", "NUM|EMAIL", self.disable_account.as_ref()),
            ("enable", "NUM|EMAIL", self.enable_account.as_ref()),
            ("switch", "NUM|EMAIL", self.switch_to.as_ref()),
            ("export", "PATH", self.export.as_ref()),
            ("import", "PATH", import_path),
            ("import-usage", "PATH", self.import_usage.as_ref()),
        ];
        if let Some((sub, metavar, _)) = values
            .iter()
            .find(|(_, _, value)| value.is_some_and(|v| v.is_empty()))
        {
            return Err((
                ErrorKind::InvalidValue,
                format!("'{sub}' requires a non-empty {metavar}"),
            ));
        }
        match (&self.import, self.from_cswap) {
            (Some(None), false) => Err((
                ErrorKind::MissingRequiredArgument,
                "'import' requires a PATH (or --from-cswap)".to_owned(),
            )),
            (Some(Some(_)), true) => {
                Err(conflict("'import' takes a PATH or --from-cswap, not both"))
            }
            (None, true) => Err(conflict("--from-cswap can only be used with 'import'")),
            _ => Ok(()),
        }
    }

    fn check_modifiers(&self) -> Result<(), Usage> {
        let switching = self.switch || self.switch_to.is_some();
        let rules: [(bool, &str); 13] = [
            (
                self.token_status && !self.list,
                "--token-status can only be used with 'list'",
            ),
            (
                self.json && !(self.list || self.status || switching),
                "--json can only be used with 'list', 'status', or 'switch'",
            ),
            (
                self.json && self.token_status,
                "--token-status cannot be combined with --json",
            ),
            (
                self.strategy.is_some() && !self.switch,
                "--strategy can only be used with bare 'switch'",
            ),
            (
                self.model.is_some() && self.strategy.is_none(),
                "--model can only be used with 'switch --strategy best' or \
                 'switch --strategy next-available'",
            ),
            (
                self.slot.is_some() && !(self.add_account || self.add_token.is_some()),
                "--slot can only be used with 'add' or 'add-token'",
            ),
            (
                self.email.is_some() && self.add_token.is_none(),
                "--email can only be used with 'add-token'",
            ),
            (
                self.account.is_some() && self.export.is_none(),
                "--account can only be used with 'export'",
            ),
            (
                self.alias.is_some() && !self.add_account,
                "--alias can only be used with 'add'",
            ),
            (
                self.force && !(self.import.is_some() || self.switch_to.is_some()),
                "--force can only be used with 'import' or 'switch <num|email>'",
            ),
            (
                self.full && self.export.is_none(),
                "--full can only be used with 'export'",
            ),
            (
                self.hold.is_some() && self.import_usage.is_none(),
                "--hold can only be used with 'import-usage'",
            ),
            (
                self.hold
                    .is_some_and(|hold| !(hold.is_finite() && hold >= 0.0)),
                "--hold must be a non-negative number of seconds",
            ),
        ];
        match rules.iter().find(|(broken, _)| *broken) {
            Some((_, message)) => Err(conflict(message)),
            None => Ok(()),
        }
    }

    /// The selected action. Only called after [`Self::validate`]'s checks, so exactly one is set.
    fn action(&self) -> Command {
        let cloned = |value: &Option<String>| value.clone().unwrap_or_default();
        if self.add_account {
            Command::Add {
                slot: self.slot,
                alias: self.alias.clone(),
            }
        } else if let Some(token) = &self.add_token {
            Command::AddToken {
                token: token.clone().filter(|t| !t.is_empty()),
                email: self.email.clone(),
                slot: self.slot,
            }
        } else if self.remove_account.is_some() {
            Command::Remove {
                account: cloned(&self.remove_account),
            }
        } else if self.disable_account.is_some() {
            Command::Disable {
                account: cloned(&self.disable_account),
            }
        } else if self.enable_account.is_some() {
            Command::Enable {
                account: cloned(&self.enable_account),
            }
        } else if self.list {
            Command::List {
                token_status: self.token_status,
                json: self.json,
            }
        } else if self.switch {
            Command::Switch {
                strategy: self.strategy,
                model: self.model.clone(),
                json: self.json,
            }
        } else if self.switch_to.is_some() {
            Command::SwitchTo {
                target: cloned(&self.switch_to),
                json: self.json,
                force: self.force,
            }
        } else if self.status {
            Command::Status { json: self.json }
        } else if self.purge {
            Command::Purge
        } else if self.export.is_some() {
            Command::Export {
                path: cloned(&self.export),
                account: self.account.clone(),
                full: self.full,
            }
        } else if let Some(path) = &self.import {
            let source = match path {
                Some(path) => ImportSource::File(path.clone()),
                None => ImportSource::Cswap,
            };
            Command::Import {
                source,
                force: self.force,
            }
        } else if self.import_usage.is_some() {
            Command::ImportUsage {
                path: cloned(&self.import_usage),
                hold_seconds: self.hold,
            }
        } else if self.tui {
            Command::Tui
        } else if self.watch {
            Command::Watch
        } else {
            Command::Upgrade
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clap_definition_is_consistent() {
        MainArgs::command().debug_assert();
    }
}

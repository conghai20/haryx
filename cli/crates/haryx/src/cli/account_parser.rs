//! `haryx login`, `haryx logout`, `haryx whoami`, `haryx account …`: the HaryX account itself,
//! outside the `ai` section. Plan: `docs/sso-platform-plan.md`.

use std::path::PathBuf;

use clap::error::ErrorKind;
use clap::{CommandFactory, Parser, ValueEnum};
use haryx_core::account::Provider;
use haryx_core::account::api::Source;
use haryx_core::contracts::Purpose;

use super::help::{HELP_TEMPLATE, p};
use super::{Command, Error, Invocation};

/// The top-level verbs this parser owns.
pub const ACCOUNT_VERBS: &[&str] = &["login", "logout", "whoami", "account"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccountAction {
    Login {
        /// `None`: the WeCoAp page, with every method.
        provider: Option<Provider>,
        no_browser: bool,
        json: bool,
    },
    Logout {
        json: bool,
    },
    Whoami {
        json: bool,
    },
    /// `account delete`: answered with where a WeCoAp ID is deleted (plan A10).
    Delete {
        yes: bool,
        json: bool,
    },
    /// `account consents`.
    Consents {
        json: bool,
    },
    /// `account consent grant|withdraw PURPOSE`.
    Consent {
        grant: bool,
        purpose: Purpose,
        source: Source,
        yes: bool,
        json: bool,
    },
    /// `account export [PATH]`.
    Export {
        path: Option<PathBuf>,
        json: bool,
    },
    /// `account machines`.
    Machines {
        json: bool,
    },
    /// `account machine rename ID NAME`.
    RenameMachine {
        id: String,
        name: String,
        json: bool,
    },
}

impl AccountAction {
    pub fn wants_json(&self) -> bool {
        match self {
            AccountAction::Login { json, .. }
            | AccountAction::Logout { json }
            | AccountAction::Whoami { json }
            | AccountAction::Delete { json, .. }
            | AccountAction::Consents { json }
            | AccountAction::Consent { json, .. }
            | AccountAction::Export { json, .. }
            | AccountAction::Machines { json }
            | AccountAction::RenameMachine { json, .. } => *json,
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum ProviderArg {
    Github,
    Google,
    Apple,
    Email,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum ChangeArg {
    Grant,
    Withdraw,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum PurposeArg {
    /// End-to-end encrypted sync between your machines
    Sync,
    /// Showing your machines on HaryX web
    Machines,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum SourceArg {
    Cli,
    Macos,
    Windows,
}

#[derive(Parser, Debug)]
#[command(
    name = "login",
    bin_name = format!("{} login", haryx_core::PROG),
    about = p("Sign this machine in to your HaryX account with your WeCoAp ID: GitHub, Google, \
               Apple, or email and password. Your browser opens; approve, and it comes back to HaryX."),
    after_help = login_epilog(),
    help_template = HELP_TEMPLATE,
)]
struct LoginArgs {
    /// Who to sign in with; without it, the WeCoAp page offers every method
    #[arg(long, value_enum)]
    provider: Option<ProviderArg>,
    /// No browser here (over SSH): print a link and a code to approve on any other device
    #[arg(long)]
    no_browser: bool,
    /// One JSON object per line: browser, then signed_in or error
    #[arg(long)]
    json: bool,
    /// Enable debug logging
    #[arg(long)]
    debug: bool,
    /// Not accepted: a Claude account is signed in with `haryx ai login`
    #[arg(hide = true)]
    account: Option<String>,
}

#[derive(Parser, Debug)]
#[command(
    name = "logout",
    bin_name = format!("{} logout", haryx_core::PROG),
    about = p("Sign this machine out of your HaryX account. Claude accounts are not touched."),
    help_template = HELP_TEMPLATE,
)]
struct LogoutArgs {
    /// Print the result as JSON
    #[arg(long)]
    json: bool,
    /// Enable debug logging
    #[arg(long)]
    debug: bool,
}

#[derive(Parser, Debug)]
#[command(
    name = "whoami",
    bin_name = format!("{} whoami", haryx_core::PROG),
    about = p("Show which HaryX account this machine is signed in to, and its consents. Works \
               offline, without the consents."),
    help_template = HELP_TEMPLATE,
)]
struct WhoamiArgs {
    /// Print the result as JSON
    #[arg(long)]
    json: bool,
    /// Enable debug logging
    #[arg(long)]
    debug: bool,
}

#[derive(Parser, Debug)]
#[command(
    name = "account",
    bin_name = format!("{} account", haryx_core::PROG),
    about = p("Manage the HaryX account itself."),
    help_template = HELP_TEMPLATE,
    disable_help_subcommand = true,
)]
struct AccountArgs {
    #[command(subcommand)]
    verb: AccountVerb,
}

#[derive(clap::Subcommand, Debug)]
enum AccountVerb {
    /// Where to delete your WeCoAp ID, which is your HaryX account. HaryX does not delete it.
    Delete {
        /// Do not ask for confirmation
        #[arg(long, short = 'y')]
        yes: bool,
        /// Print the result as JSON
        #[arg(long)]
        json: bool,
        /// Enable debug logging
        #[arg(long)]
        debug: bool,
    },
    /// Your HaryX consents: Sync, and showing your machines on HaryX web
    Consents {
        /// Print the result as JSON
        #[arg(long)]
        json: bool,
        /// Enable debug logging
        #[arg(long)]
        debug: bool,
    },
    /// Grant or withdraw a consent. Withdrawing erases that purpose's data on the server
    Consent {
        #[arg(value_enum)]
        change: ChangeArg,
        #[arg(value_enum)]
        purpose: PurposeArg,
        /// Withdraw without asking for confirmation
        #[arg(long, short = 'y')]
        yes: bool,
        /// Where the user answered, when an app runs this
        #[arg(long, value_enum, default_value = "cli")]
        source: SourceArg,
        /// Print the result as JSON
        #[arg(long)]
        json: bool,
        /// Enable debug logging
        #[arg(long)]
        debug: bool,
    },
    /// Download everything HaryX keeps about you, as one JSON file
    Export {
        /// Where to write it (default: ./haryx-export-DATE.json)
        path: Option<PathBuf>,
        /// Print the result as JSON
        #[arg(long)]
        json: bool,
        /// Enable debug logging
        #[arg(long)]
        debug: bool,
    },
    /// The machines shown on HaryX web
    Machines {
        /// Print the result as JSON
        #[arg(long)]
        json: bool,
        /// Enable debug logging
        #[arg(long)]
        debug: bool,
    },
    /// Manage one machine shown on HaryX web
    Machine {
        #[command(subcommand)]
        verb: MachineVerb,
    },
}

#[derive(clap::Subcommand, Debug)]
enum MachineVerb {
    /// Rename a machine; its heartbeats never change the name back
    Rename {
        /// The machine id, as 'account machines' lists it
        id: String,
        name: String,
        /// Print the result as JSON
        #[arg(long)]
        json: bool,
        /// Enable debug logging
        #[arg(long)]
        debug: bool,
    },
}

fn account_action(verb: AccountVerb) -> (bool, AccountAction) {
    match verb {
        AccountVerb::Delete { yes, json, debug } => (debug, AccountAction::Delete { yes, json }),
        AccountVerb::Consents { json, debug } => (debug, AccountAction::Consents { json }),
        AccountVerb::Consent {
            change,
            purpose,
            yes,
            source,
            json,
            debug,
        } => {
            let action = AccountAction::Consent {
                grant: matches!(change, ChangeArg::Grant),
                purpose: match purpose {
                    PurposeArg::Sync => Purpose::Sync,
                    PurposeArg::Machines => Purpose::Machines,
                },
                source: match source {
                    SourceArg::Cli => Source::Cli,
                    SourceArg::Macos => Source::Macos,
                    SourceArg::Windows => Source::Windows,
                },
                yes,
                json,
            };
            (debug, action)
        }
        AccountVerb::Export { path, json, debug } => (debug, AccountAction::Export { path, json }),
        AccountVerb::Machines { json, debug } => (debug, AccountAction::Machines { json }),
        AccountVerb::Machine {
            verb:
                MachineVerb::Rename {
                    id,
                    name,
                    json,
                    debug,
                },
        } => (debug, AccountAction::RenameMachine { id, name, json }),
    }
}

fn login_epilog() -> String {
    p(
        "This is the HaryX account, used to sync between your machines. To sign in a Claude
account, use '{ai} login'.

Examples:
  {prog} login
  {prog} login --provider google
  {prog} login --provider email
  {prog} login --no-browser",
    )
}

fn argv(verb: &str, rest: &[String]) -> Vec<String> {
    std::iter::once(verb.to_owned())
        .chain(rest.iter().cloned())
        .collect()
}

/// Parse `verb` (one of [`ACCOUNT_VERBS`]) and its arguments.
pub(super) fn parse(verb: &str, rest: &[String]) -> Result<Invocation, Error> {
    let (debug, action) =
        match verb {
            "login" => {
                let args = LoginArgs::try_parse_from(argv(verb, rest))?;
                if let Some(account) = args.account {
                    return Err(LoginArgs::command().error(
                    ErrorKind::UnknownArgument,
                    format!(
                        "{} {account}'",
                        p("'{prog} login' signs in to HaryX — to sign in a Claude account, use \
                           '{ai} login")
                    ),
                ));
                }
                let provider = args.provider.map(|arg| match arg {
                    ProviderArg::Github => Provider::Github,
                    ProviderArg::Google => Provider::Google,
                    ProviderArg::Apple => Provider::Apple,
                    ProviderArg::Email => Provider::Email,
                });
                (
                    args.debug,
                    AccountAction::Login {
                        provider,
                        no_browser: args.no_browser,
                        json: args.json,
                    },
                )
            }
            "account" => {
                let args = AccountArgs::try_parse_from(argv(verb, rest))?;
                account_action(args.verb)
            }
            "logout" => {
                let args = LogoutArgs::try_parse_from(argv(verb, rest))?;
                (args.debug, AccountAction::Logout { json: args.json })
            }
            _ => {
                let args = WhoamiArgs::try_parse_from(argv(verb, rest))?;
                (args.debug, AccountAction::Whoami { json: args.json })
            }
        };
    Ok(Invocation {
        debug,
        command: Command::Account(action),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn action(verb: &str, rest: &[&str]) -> Result<AccountAction, Error> {
        let rest: Vec<String> = rest.iter().map(|s| (*s).to_owned()).collect();
        parse(verb, &rest).map(|inv| match inv.command {
            Command::Account(action) => action,
            other => panic!("unexpected {other:?}"),
        })
    }

    #[test]
    fn definitions_are_consistent() {
        LoginArgs::command().debug_assert();
        LogoutArgs::command().debug_assert();
        WhoamiArgs::command().debug_assert();
        AccountArgs::command().debug_assert();
    }

    #[test]
    fn login_defaults_to_the_wecoap_page_and_opens_a_browser() {
        assert_eq!(
            action("login", &[]).unwrap(),
            AccountAction::Login {
                provider: None,
                no_browser: false,
                json: false
            }
        );
        assert_eq!(
            action("login", &["--provider", "google", "--no-browser", "--json"]).unwrap(),
            AccountAction::Login {
                provider: Some(Provider::Google),
                no_browser: true,
                json: true
            }
        );
        for (arg, provider) in [
            ("github", Provider::Github),
            ("apple", Provider::Apple),
            ("email", Provider::Email),
        ] {
            assert!(matches!(
                action("login", &["--provider", arg]).unwrap(),
                AccountAction::Login { provider: Some(p), .. } if p == provider
            ));
        }
        assert!(action("login", &["--provider", "gitlab"]).is_err());
    }

    #[test]
    fn login_with_an_account_points_at_the_ai_section() {
        let err = action("login", &["3"]).unwrap_err().to_string();
        assert!(err.contains("haryx ai login 3"), "{err}");
    }

    #[test]
    fn logout_and_whoami_take_json() {
        assert_eq!(
            action("logout", &["--json"]).unwrap(),
            AccountAction::Logout { json: true }
        );
        assert_eq!(
            action("whoami", &[]).unwrap(),
            AccountAction::Whoami { json: false }
        );
        assert!(action("whoami", &["extra"]).is_err());
    }

    #[test]
    fn consent_verbs_parse_with_their_flags() {
        assert_eq!(
            action("account", &["consents", "--json"]).unwrap(),
            AccountAction::Consents { json: true }
        );
        assert_eq!(
            action("account", &["consent", "grant", "sync"]).unwrap(),
            AccountAction::Consent {
                grant: true,
                purpose: Purpose::Sync,
                source: Source::Cli,
                yes: false,
                json: false,
            }
        );
        assert_eq!(
            action(
                "account",
                &[
                    "consent", "withdraw", "machines", "--yes", "--source", "windows", "--json"
                ]
            )
            .unwrap(),
            AccountAction::Consent {
                grant: false,
                purpose: Purpose::Machines,
                source: Source::Windows,
                yes: true,
                json: true,
            }
        );
        assert!(action("account", &["consent", "grant", "usage"]).is_err());
        assert!(action("account", &["consent", "grant", "sync", "--source", "web"]).is_err());
    }

    #[test]
    fn export_and_machine_verbs_parse() {
        assert_eq!(
            action("account", &["export"]).unwrap(),
            AccountAction::Export {
                path: None,
                json: false
            }
        );
        assert_eq!(
            action("account", &["export", "out.json", "--json"]).unwrap(),
            AccountAction::Export {
                path: Some("out.json".into()),
                json: true
            }
        );
        assert_eq!(
            action("account", &["machines"]).unwrap(),
            AccountAction::Machines { json: false }
        );
        assert_eq!(
            action("account", &["machine", "rename", "abc", "Work Mac"]).unwrap(),
            AccountAction::RenameMachine {
                id: "abc".into(),
                name: "Work Mac".into(),
                json: false
            }
        );
        assert!(action("account", &["machine", "rename", "abc"]).is_err());
    }
}

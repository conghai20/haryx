//! `haryx login`, `haryx logout`, `haryx whoami`: the HaryX account itself, outside the `ai` section.
//! Plan: `docs/haryx-account-plan.md`.

use clap::error::ErrorKind;
use clap::{CommandFactory, Parser, ValueEnum};
use haryx_core::account::Provider;

use super::help::{HELP_TEMPLATE, p};
use super::{Command, Error, Invocation};

/// The top-level verbs this parser owns.
pub const ACCOUNT_VERBS: &[&str] = &["login", "logout", "whoami"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccountAction {
    Login {
        provider: Provider,
        no_browser: bool,
        json: bool,
    },
    Logout {
        json: bool,
    },
    Whoami {
        json: bool,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum ProviderArg {
    Github,
    Google,
}

#[derive(Parser, Debug)]
#[command(
    name = "login",
    bin_name = format!("{} login", haryx_core::PROG),
    about = p("Sign this machine in to your HaryX account, with GitHub or Google. \
               Your browser opens; approve, and it comes back to HaryX."),
    after_help = login_epilog(),
    help_template = HELP_TEMPLATE,
)]
struct LoginArgs {
    /// Who to sign in with
    #[arg(long, value_enum, default_value = "github")]
    provider: ProviderArg,
    /// Print the sign-in link instead of opening a browser; open it in a browser on this machine
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
    about = p("Show which HaryX account this machine is signed in to. Works offline."),
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

fn login_epilog() -> String {
    p(
        "This is the HaryX account, used to sync between your machines. To sign in a Claude
account, use '{ai} login'.

Examples:
  {prog} login
  {prog} login --provider google
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
                let provider = match args.provider {
                    ProviderArg::Github => Provider::Github,
                    ProviderArg::Google => Provider::Google,
                };
                (
                    args.debug,
                    AccountAction::Login {
                        provider,
                        no_browser: args.no_browser,
                        json: args.json,
                    },
                )
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
    }

    #[test]
    fn login_defaults_to_github_and_opens_a_browser() {
        assert_eq!(
            action("login", &[]).unwrap(),
            AccountAction::Login {
                provider: Provider::Github,
                no_browser: false,
                json: false
            }
        );
        assert_eq!(
            action("login", &["--provider", "google", "--no-browser", "--json"]).unwrap(),
            AccountAction::Login {
                provider: Provider::Google,
                no_browser: true,
                json: true
            }
        );
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
}

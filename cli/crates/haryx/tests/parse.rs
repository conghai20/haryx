//! Argument parsing: subcommand and flag spellings, pre-dispatched parsers, upstream's usage errors.

use clap::error::ErrorKind;
use haryx::cli::{
    AccountSetting, AutoOptions, AutoStrategy, Command, ConfigAction, ImportSource, Invocation,
    RunOptions, SwitchStrategy, parse, translate_subcommand,
};

fn args(argv: &[&str]) -> Vec<String> {
    argv.iter().map(|arg| (*arg).to_owned()).collect()
}

fn ok(argv: &[&str]) -> Invocation {
    parse(&args(argv), false).unwrap_or_else(|err| panic!("{argv:?} failed to parse: {err}"))
}

fn cmd(argv: &[&str]) -> Command {
    ok(argv).command
}

fn usage_error(argv: &[&str]) -> String {
    let err = match parse(&args(argv), false) {
        Ok(parsed) => panic!("{argv:?} parsed to {parsed:?}; expected a usage error"),
        Err(err) => err,
    };
    assert_eq!(err.exit_code(), 2, "{argv:?}: {err}");
    err.to_string()
}

fn assert_usage_error(argv: &[&str], message: &str) {
    let rendered = usage_error(argv);
    assert!(
        rendered.contains(message),
        "{argv:?}: expected {message:?} in {rendered:?}"
    );
}

// --- the two interface styles agree -----------------------------------------------------------

#[test]
fn every_subcommand_parses_like_its_flag() {
    let pairs: &[(&[&str], &[&str])] = &[
        (&["list"], &["--list"]),
        (&["ls"], &["--list"]),
        (&["list", "--json"], &["--list", "--json"]),
        (&["list", "--token-status"], &["--list", "--token-status"]),
        (&["status"], &["--status"]),
        (&["status", "--json"], &["--json", "--status"]),
        (&["add"], &["--add-account"]),
        (
            &["add", "--slot", "3", "--alias", "dev"],
            &["--add-account", "--slot", "3", "--alias", "dev"],
        ),
        (&["add-token"], &["--add-token"]),
        (&["add-token", "-"], &["--add-token", "-"]),
        (
            &["add-token", "sk-ant-oat01-x", "--email", "me@example.com"],
            &["--add-token", "sk-ant-oat01-x", "--email", "me@example.com"],
        ),
        (&["remove", "2"], &["--remove-account", "2"]),
        (&["rm", "2"], &["--remove-account", "2"]),
        (&["disable", "a@b.c"], &["--disable-account", "a@b.c"]),
        (&["enable", "a@b.c"], &["--enable-account", "a@b.c"]),
        (
            &["export", "out.json", "--full"],
            &["--export", "out.json", "--full"],
        ),
        (
            &["export", "out.json", "--account", "1"],
            &["--export", "out.json", "--account", "1"],
        ),
        (
            &["import", "in.json", "--force"],
            &["--import", "in.json", "--force"],
        ),
        (&["import", "--from-cswap"], &["--import", "--from-cswap"]),
        (
            &["import-usage", "-", "--hold", "600"],
            &["--import-usage", "-", "--hold", "600"],
        ),
        (&["purge"], &["--purge"]),
        (&["upgrade"], &["--upgrade"]),
        (&["update"], &["--upgrade"]),
        (&["tui"], &["--tui"]),
        (&["watch"], &["--watch"]),
        (&["switch"], &["--switch"]),
        (&["switch", "--json"], &["--switch", "--json"]),
        (
            &["switch", "--strategy", "best"],
            &["--switch", "--strategy", "best"],
        ),
        (&["switch", "2"], &["--switch-to", "2"]),
        (
            &["switch", "2", "--force", "--json"],
            &["--switch-to", "2", "--force", "--json"],
        ),
        (&["--debug", "--list"], &["list", "--debug"]),
    ];
    for (subcommand, flags) in pairs {
        assert_eq!(ok(subcommand), ok(flags), "{subcommand:?} vs {flags:?}");
    }
}

#[test]
fn translate_matches_upstream_rules() {
    assert_eq!(translate_subcommand(&args(&["--list"])), args(&["--list"]));
    assert_eq!(
        translate_subcommand(&args(&["switch"])),
        args(&["--switch"])
    );
    assert_eq!(
        translate_subcommand(&args(&["switch", "--strategy", "best"])),
        args(&["--switch", "--strategy", "best"])
    );
    assert_eq!(
        translate_subcommand(&args(&["switch", "a@b.c"])),
        args(&["--switch-to", "a@b.c"])
    );
    assert_eq!(translate_subcommand(&args(&["help"])), args(&["--help"]));
    assert_eq!(
        translate_subcommand(&args(&["bogus", "x"])),
        args(&["bogus", "x"])
    );
    assert!(translate_subcommand(&[]).is_empty());
}

// --- actions and their options --------------------------------------------------------------

#[test]
fn bare_switch_rotates_and_switch_with_target_jumps() {
    assert_eq!(
        cmd(&["switch"]),
        Command::Switch {
            strategy: None,
            model: None,
            json: false
        }
    );
    assert_eq!(
        cmd(&[
            "switch",
            "--strategy",
            "next-available",
            "--model",
            "Fable,Opus"
        ]),
        Command::Switch {
            strategy: Some(SwitchStrategy::NextAvailable),
            model: Some("Fable,Opus".into()),
            json: false,
        }
    );
    assert_eq!(
        cmd(&["switch", "2"]),
        Command::SwitchTo {
            target: "2".into(),
            json: false,
            force: false
        }
    );
    assert_eq!(
        cmd(&["switch", "user@example.com", "--force"]),
        Command::SwitchTo {
            target: "user@example.com".into(),
            json: false,
            force: true
        }
    );
}

#[test]
fn add_token_without_a_value_prompts() {
    let prompt = Command::AddToken {
        token: None,
        email: None,
        slot: None,
    };
    assert_eq!(cmd(&["add-token"]), prompt);
    assert_eq!(cmd(&["--add-token", ""]), prompt);
    assert_eq!(
        cmd(&["add-token", "-", "--slot", "4"]),
        Command::AddToken {
            token: Some("-".into()),
            email: None,
            slot: Some(4)
        }
    );
}

#[test]
fn import_reads_a_file_or_the_cswap_store() {
    assert_eq!(
        cmd(&["import", "backup.json"]),
        Command::Import {
            source: ImportSource::File("backup.json".into()),
            force: false
        }
    );
    assert_eq!(
        cmd(&["import", "--from-cswap", "--force"]),
        Command::Import {
            source: ImportSource::Cswap,
            force: true
        }
    );
}

#[test]
fn import_from_cswap_misuse_is_rejected() {
    assert_usage_error(&["import"], "'import' requires a PATH (or --from-cswap)");
    assert_usage_error(&["import", "x.json", "--from-cswap"], "not both");
    assert_usage_error(
        &["list", "--from-cswap"],
        "--from-cswap can only be used with 'import'",
    );
}

#[test]
fn import_usage_keeps_its_hold() {
    assert_eq!(
        cmd(&["import-usage", "-", "--hold", "0"]),
        Command::ImportUsage {
            path: "-".into(),
            hold_seconds: Some(0.0)
        }
    );
    assert_eq!(
        cmd(&["import-usage", "/tmp/u.json"]),
        Command::ImportUsage {
            path: "/tmp/u.json".into(),
            hold_seconds: None
        }
    );
}

#[test]
fn a_whitespace_only_path_is_still_a_path() {
    assert_eq!(
        cmd(&["export", " "]),
        Command::Export {
            path: " ".into(),
            account: None,
            full: false
        }
    );
}

#[test]
fn bare_haryx_opens_the_tui_only_at_a_terminal() {
    assert_eq!(
        parse(&[], true).map(|inv| inv.command).ok(),
        Some(Command::Tui)
    );
    assert_usage_error(&[], "no command given — try 'haryx help'");
}

#[test]
fn ai_section_parses_like_the_bare_commands() {
    for argv in [
        &["list", "--json"][..],
        &["switch", "2"],
        &["run", "2", "--", "--resume"],
        &["config", "get", "autoswitch.threshold"],
        &["tui"],
        &["watch"],
        &["--list"],
    ] {
        let sectioned: Vec<&str> = std::iter::once("ai").chain(argv.iter().copied()).collect();
        assert_eq!(ok(&sectioned), ok(argv), "{argv:?}");
    }
}

#[test]
fn login_lives_only_in_the_ai_section() {
    assert!(matches!(
        ok(&["ai", "login", "3"]).command,
        Command::Login(_)
    ));
    assert_usage_error(&["login", "3"], "did you mean 'haryx ai login'?");
    assert_usage_error(&["login"], "did you mean 'haryx ai login'?");
}

#[test]
fn bare_ai_section_opens_the_tui_only_at_a_terminal() {
    assert_eq!(
        parse(&args(&["ai"]), true).map(|inv| inv.command).ok(),
        Some(Command::Tui)
    );
    assert_usage_error(&["ai"], "no command given — try 'haryx ai help'");
}

#[test]
fn top_help_and_ai_help_are_both_help_requests() {
    for argv in [
        &["help"][..],
        &["--help"],
        &["-h"],
        &["ai", "help"],
        &["ai", "--help"],
    ] {
        let err = parse(&args(argv), false).expect_err("help is reported as an error");
        assert_eq!(err.kind(), ErrorKind::DisplayHelp, "{argv:?}");
    }
}

#[test]
fn debug_is_carried_by_every_parser() {
    assert!(ok(&["list", "--debug"]).debug);
    assert!(!ok(&["list"]).debug);
    for argv in [
        &["run", "2", "--debug"][..],
        &["auto", "--debug"],
        &["config", "--debug"],
        &["map", "--debug"],
        &["unmap", "--debug"],
        &["unclaimed", "--debug"],
        &["alias", "--debug"],
        &["threshold", "--debug"],
        &["priority", "--debug"],
        &["swap", "1", "2", "--debug"],
        &["move", "a", "1", "--debug"],
    ] {
        assert!(ok(argv).debug, "{argv:?}");
    }
}

// --- pre-dispatched parsers -----------------------------------------------------------------

#[test]
fn run_forwards_everything_after_the_separator() {
    assert_eq!(
        cmd(&["run", "2", "--", "--resume"]),
        Command::Run(RunOptions {
            account: Some("2".into()),
            claude_args: args(&["--resume"]),
            ..RunOptions::default()
        })
    );
    assert_eq!(
        cmd(&["run", "2", "--no-share", "--", "-p", "hi", "--", "--debug"]),
        Command::Run(RunOptions {
            account: Some("2".into()),
            no_share: true,
            claude_args: args(&["-p", "hi", "--", "--debug"]),
            ..RunOptions::default()
        })
    );
    assert!(!ok(&["run", "2", "--", "--debug"]).debug);
}

#[test]
fn run_options() {
    assert_eq!(cmd(&["run"]), Command::Run(RunOptions::default()));
    let Command::Run(run) = cmd(&["run", "dev", "--no-share-history", "--require-session"]) else {
        panic!("not run");
    };
    assert_eq!(run.share_history, Some(false));
    assert!(run.require_session);
    let Command::Run(run) = cmd(&["run", "dev", "--no-share-history", "--share-history"]) else {
        panic!("not run");
    };
    assert_eq!(run.share_history, Some(true), "the last of the pair wins");
    let Command::Run(run) = cmd(&["run", "dev", "--no-share"]) else {
        panic!("not run")
    };
    assert!(run.no_share);
    assert_eq!(
        run.share_history, None,
        "--no-share is not a prefix of --no-share-history"
    );
}

#[test]
fn run_must_come_first() {
    usage_error(&["--debug", "run", "2"]);
}

#[test]
fn auto_options() {
    assert_eq!(cmd(&["auto"]), Command::Auto(AutoOptions::default()));
    assert_eq!(
        cmd(&[
            "auto",
            "--once",
            "--json",
            "--interval",
            "30",
            "--threshold",
            "80",
            "--weekly-threshold",
            "97",
            "--cooldown",
            "0",
            "--model",
            "Fable",
            "--no-include-api-key-accounts",
            "--strategy",
            "priority-consume-first",
            "--dry-run",
        ]),
        Command::Auto(AutoOptions {
            once: true,
            json: true,
            interval: Some(30.0),
            threshold: Some(80.0),
            weekly_threshold: Some(97.0),
            cooldown: Some(0.0),
            model: Some("Fable".into()),
            include_api_key_accounts: Some(false),
            strategy: Some(AutoStrategy::PriorityConsumeFirst),
            dry_run: true,
        })
    );
}

#[test]
fn choices_are_validated() {
    assert_usage_error(
        &["auto", "--strategy", "fastest"],
        "invalid value 'fastest'",
    );
    assert_usage_error(
        &["switch", "--strategy", "weekly-first"],
        "invalid value 'weekly-first'",
    );
    assert_usage_error(&["auto", "--threshold", "high"], "invalid value 'high'");
    assert_usage_error(&["add", "--slot", "three"], "invalid value 'three'");
    assert_usage_error(&["config", "frob"], "unrecognized subcommand 'frob'");
}

#[test]
fn config_actions() {
    let config = |json, action| Command::Config { json, action };
    assert_eq!(cmd(&["config"]), config(false, ConfigAction::List));
    assert_eq!(cmd(&["config", "--json"]), config(true, ConfigAction::List));
    assert_eq!(
        cmd(&["config", "list", "--json"]),
        config(true, ConfigAction::List)
    );
    assert_eq!(
        cmd(&["config", "get", "ui.theme", "--json"]),
        config(
            true,
            ConfigAction::Get {
                key: "ui.theme".into()
            }
        )
    );
    assert_eq!(
        cmd(&["config", "set", "autoswitch.threshold", "80"]),
        config(
            false,
            ConfigAction::Set {
                key: "autoswitch.threshold".into(),
                value: "80".into()
            }
        )
    );
    assert_eq!(
        cmd(&["config", "unset", "autoswitch.threshold"]),
        config(
            false,
            ConfigAction::Unset {
                key: "autoswitch.threshold".into()
            }
        )
    );
    assert_eq!(cmd(&["config", "path"]), config(false, ConfigAction::Path));
    assert_usage_error(
        &["config", "--json", "path"],
        "--json can only be used with list or get",
    );
    assert_usage_error(
        &["config", "set", "k", "v", "--json"],
        "--json can only be used with list or get",
    );
}

#[test]
fn map_unmap_unclaimed_swap_move() {
    assert_eq!(
        cmd(&["map"]),
        Command::Map {
            account: None,
            path: None
        }
    );
    assert_eq!(
        cmd(&["map", "2", "~/work"]),
        Command::Map {
            account: Some("2".into()),
            path: Some("~/work".into())
        }
    );
    assert_eq!(cmd(&["unmap"]), Command::Unmap { path: None });
    assert_eq!(
        cmd(&["unmap", "/x"]),
        Command::Unmap {
            path: Some("/x".into())
        }
    );
    assert_eq!(cmd(&["unclaimed"]), Command::Unclaimed { purge: None });
    assert_eq!(
        cmd(&["unclaimed", "--purge", "abc"]),
        Command::Unclaimed {
            purge: Some("abc".into())
        }
    );
    assert_eq!(
        cmd(&["unclaimed", "--purge", ""]),
        Command::Unclaimed { purge: None }
    );
    assert_eq!(
        cmd(&["swap", "1", "dev"]),
        Command::Swap {
            first: "1".into(),
            second: "dev".into()
        }
    );
    assert_eq!(
        cmd(&["move", "dev", "1"]),
        Command::Move {
            account: "dev".into(),
            slot: "1".into()
        }
    );
    usage_error(&["swap", "1"]);
    usage_error(&["move", "dev"]);
    usage_error(&["map", "1", "/a", "/b"]);
}

#[test]
fn alias_threshold_priority_shapes() {
    assert_eq!(cmd(&["alias"]), Command::Alias(AccountSetting::List));
    assert_eq!(
        cmd(&["alias", "2", "dev"]),
        Command::Alias(AccountSetting::Set {
            account: "2".into(),
            value: "dev".into()
        })
    );
    assert_eq!(
        cmd(&["alias", "2", "--unset"]),
        Command::Alias(AccountSetting::Unset {
            account: "2".into()
        })
    );
    assert_eq!(
        cmd(&["threshold"]),
        Command::Threshold(AccountSetting::List)
    );
    assert_eq!(
        cmd(&["threshold", "a@b.c", "99.9"]),
        Command::Threshold(AccountSetting::Set {
            account: "a@b.c".into(),
            value: "99.9".into()
        })
    );
    assert_eq!(
        cmd(&["threshold", "2", "--unset"]),
        Command::Threshold(AccountSetting::Unset {
            account: "2".into()
        })
    );
    assert_eq!(
        cmd(&["priority", "2", "-1"]),
        Command::Priority(AccountSetting::Set {
            account: "2".into(),
            value: "-1".into()
        }),
        "a negative TIER reaches core's validator, as upstream"
    );
    assert_eq!(cmd(&["priority"]), Command::Priority(AccountSetting::List));
}

#[test]
fn alias_threshold_priority_errors() {
    assert_usage_error(
        &["alias", "2", "dev", "--unset"],
        "--unset does not take a NAME argument",
    );
    assert_usage_error(&["alias", "--unset"], "NUM|EMAIL is required with --unset");
    assert_usage_error(
        &["alias", "2"],
        "NAME is required (or pass --unset to remove the alias)",
    );
    assert_usage_error(
        &["threshold", "2", "85", "--unset"],
        "--unset does not take a PCT argument",
    );
    assert_usage_error(
        &["threshold", "--unset"],
        "NUM|EMAIL is required with --unset",
    );
    assert_usage_error(&["threshold", "2"], "PCT is required");
    assert_usage_error(
        &["priority", "2", "5", "--unset"],
        "--unset does not take a TIER argument",
    );
    assert_usage_error(
        &["priority", "--unset"],
        "NUM|EMAIL|ALIAS is required with --unset",
    );
    assert_usage_error(&["priority", "2"], "TIER is required (or pass --unset)");
}

// --- main parser usage errors ---------------------------------------------------------------

#[test]
fn actions_are_mutually_exclusive() {
    assert_usage_error(&["list", "--status"], "cannot be used with");
    assert_usage_error(&["--switch", "--switch-to", "2"], "cannot be used with");
    assert_usage_error(&["add", "--purge"], "cannot be used with");
}

#[test]
fn empty_values_are_rejected_before_their_modifiers() {
    for (argv, message) in [
        (
            &["remove", ""][..],
            "'remove' requires a non-empty NUM|EMAIL",
        ),
        (&["disable", ""], "'disable' requires a non-empty NUM|EMAIL"),
        (&["enable", ""], "'enable' requires a non-empty NUM|EMAIL"),
        (&["switch", ""], "'switch' requires a non-empty NUM|EMAIL"),
        (
            &["--switch-to", ""],
            "'switch' requires a non-empty NUM|EMAIL",
        ),
        (&["export", ""], "'export' requires a non-empty PATH"),
        (
            &["export", "", "--account", "1"],
            "'export' requires a non-empty PATH",
        ),
        (&["import", ""], "'import' requires a non-empty PATH"),
        (
            &["import-usage", ""],
            "'import-usage' requires a non-empty PATH",
        ),
    ] {
        assert_usage_error(argv, message);
    }
}

#[test]
fn modifiers_need_their_action() {
    for (argv, message) in [
        (
            &["status", "--token-status"][..],
            "--token-status can only be used with 'list'",
        ),
        (
            &["add", "--json"],
            "--json can only be used with 'list', 'status', 'switch' or 'upgrade --check'",
        ),
        (
            &["list", "--json", "--token-status"],
            "--token-status cannot be combined with --json",
        ),
        (
            &["switch", "2", "--strategy", "best"],
            "--strategy can only be used with bare 'switch'",
        ),
        (
            &["switch", "--model", "Fable"],
            "--model can only be used with 'switch --strategy best'",
        ),
        (
            &["list", "--slot", "1"],
            "--slot can only be used with 'add' or 'add-token'",
        ),
        (
            &["add", "--email", "a@b.c"],
            "--email can only be used with 'add-token'",
        ),
        (
            &["list", "--account", "1"],
            "--account can only be used with 'export'",
        ),
        (
            &["add-token", "--alias", "x"],
            "--alias can only be used with 'add'",
        ),
        (
            &["switch", "--force"],
            "--force can only be used with 'import' or 'switch <num|email>'",
        ),
        (
            &["import", "a", "--full"],
            "--full can only be used with 'export'",
        ),
        (
            &["list", "--hold", "60"],
            "--hold can only be used with 'import-usage'",
        ),
        (
            &["import-usage", "-", "--hold", "-1"],
            "--hold must be a non-negative",
        ),
        (
            &["import-usage", "-", "--hold", "inf"],
            "--hold must be a non-negative",
        ),
        (
            &["import-usage", "-", "--hold", "NaN"],
            "--hold must be a non-negative",
        ),
    ] {
        assert_usage_error(argv, message);
    }
}

#[test]
fn unknown_arguments_and_dropped_menubar_are_usage_errors() {
    usage_error(&["switch", "2", "3"]);
    usage_error(&["menubar"]);
    usage_error(&["--menubar"]);
    usage_error(&["list", "--install-service"]);
}

#[test]
fn help_and_version_are_not_errors() {
    for argv in [
        &["help"][..],
        &["--help"],
        &["-h"],
        &["run", "--help"],
        &["config", "-h"],
    ] {
        let err = parse(&args(argv), false).expect_err("help is reported through clap");
        assert_eq!(err.kind(), ErrorKind::DisplayHelp, "{argv:?}");
        assert_eq!(err.exit_code(), 0);
    }
    let err = parse(&args(&["--version"]), false).expect_err("version is reported through clap");
    assert_eq!(err.kind(), ErrorKind::DisplayVersion);
    assert_eq!(
        parse(&args(&["run", "--version"]), false)
            .map_err(|e| e.kind())
            .err(),
        Some(ErrorKind::UnknownArgument),
        "only the main parser has --version"
    );
}

#[test]
fn every_command_has_a_name() {
    assert_eq!(cmd(&["switch", "2"]).name(), "switch");
    assert_eq!(cmd(&["add-token"]).name(), "add-token");
    assert_eq!(cmd(&["import", "--from-cswap"]).name(), "import");
    assert_eq!(cmd(&["config", "path"]).name(), "config");
}

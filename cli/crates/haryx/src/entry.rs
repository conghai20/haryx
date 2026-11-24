//! The shared program entry: `haryx` (console) and `haryxw` (windowless, for the Windows wake task)
//! both run [`run`].

use std::io::IsTerminal;
use std::process::ExitCode;

use crate::{cli, commands, logging};

/// Parse `argv` (without the program name), dispatch, and turn the outcome into an exit code.
pub fn run(argv: &[String]) -> ExitCode {
    // Bare `haryx` opens the TUI only when a person is at both ends; scripts get the usage error.
    let interactive = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
    let invocation = match cli::parse(argv, interactive) {
        Ok(invocation) => invocation,
        // Help and version go to stdout (exit 0); usage errors to stderr (exit 2).
        Err(err) => err.exit(),
    };
    logging::init(invocation.debug);
    match commands::dispatch(invocation.command, invocation.debug) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            if !err.message.is_empty() {
                eprintln!("{err}");
            }
            ExitCode::from(err.exit_code)
        }
    }
}

/// The process arguments after the program name, lossily as text.
pub fn args() -> Vec<String> {
    std::env::args_os()
        .skip(1)
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect()
}

/// `(NAME, VALUE)` pairs to set, then the remaining argv.
pub type EnvSplit = (Vec<(String, String)>, Vec<String>);

/// Split leading `--env NAME=VALUE` pairs off `argv` (how the Windows wake task, which has no
/// per-task environment, hands over `HARYX_HOME` and friends). Only the variables haryx reads are
/// accepted, so the arguments cannot reach anything else in the environment.
pub fn split_env_args(argv: &[String]) -> Result<EnvSplit, String> {
    const ALLOWED: &[&str] = &["HARYX_HOME", "CLAUDE_CONFIG_DIR", "XDG_DATA_HOME"];
    let mut env = Vec::new();
    let mut rest = argv;
    while let [flag, pair, tail @ ..] = rest {
        if flag != "--env" {
            break;
        }
        let (name, value) = pair
            .split_once('=')
            .ok_or_else(|| format!("--env expects NAME=VALUE, got {pair:?}"))?;
        if !ALLOWED.contains(&name) {
            return Err(format!("--env does not accept {name}"));
        }
        env.push((name.to_string(), value.to_string()));
        rest = tail;
    }
    if rest.first().is_some_and(|a| a == "--env") {
        return Err("--env expects NAME=VALUE".into());
    }
    Ok((env, rest.to_vec()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn leading_env_pairs_are_split_off() {
        let argv = strings(&["--env", r"HARYX_HOME=D:\s=1", "ai", "wake", "run", "--once"]);
        let (env, rest) = split_env_args(&argv).unwrap();
        assert_eq!(env, vec![("HARYX_HOME".into(), r"D:\s=1".into())]);
        assert_eq!(rest, strings(&["ai", "wake", "run", "--once"]));
    }

    #[test]
    fn no_env_leaves_argv_alone() {
        let argv = strings(&["list", "--env", "X=1"]);
        assert_eq!(split_env_args(&argv).unwrap(), (vec![], argv));
    }

    #[test]
    fn rejects_unknown_names_and_bad_pairs() {
        assert!(split_env_args(&strings(&["--env", "PATH=/x", "list"])).is_err());
        assert!(split_env_args(&strings(&["--env", "HARYX_HOME"])).is_err());
        assert!(split_env_args(&strings(&["--env"])).is_err());
    }
}

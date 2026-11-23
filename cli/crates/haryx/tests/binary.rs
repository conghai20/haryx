//! The `haryx` binary end to end: streams and exit codes, as argparse would produce them.

use std::process::{Command, Output, Stdio};

fn haryx(argv: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_haryx"))
        .args(argv)
        .stdin(Stdio::null())
        .output()
        .expect("the haryx binary runs")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn version_names_haryx_and_the_upstream_release() {
    let output = haryx(&["--version"]);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        stdout(&output).trim(),
        format!(
            "haryx {} (claude-swap {})",
            haryx_core::VERSION,
            haryx_core::UPSTREAM_VERSION
        )
    );
}

#[test]
fn help_goes_to_stdout_with_exit_zero() {
    for argv in [
        &["help"][..],
        &["--help"],
        &["run", "--help"],
        &["ai", "run", "--help"],
    ] {
        let output = haryx(argv);
        assert_eq!(output.status.code(), Some(0), "{argv:?}");
        assert!(stdout(&output).contains("Usage: haryx"), "{argv:?}");
        assert!(stderr(&output).is_empty(), "{argv:?}");
    }
    let top_help = stdout(&haryx(&["help"]));
    assert!(
        top_help.contains("haryx ai <command>"),
        "the overview lists the sections"
    );
    assert!(
        !top_help.contains("import --from-cswap"),
        "the overview leaves commands to 'ai'"
    );
    for argv in [&["ai", "help"][..], &["ai", "--help"], &["list", "--help"]] {
        let ai_help = stdout(&haryx(argv));
        assert!(ai_help.contains("haryx ai import --from-cswap"), "{argv:?}");
        assert!(!ai_help.contains("menubar"), "{argv:?}");
        assert!(
            !ai_help.contains("cswap run"),
            "examples say haryx: {argv:?}"
        );
    }
    assert!(stdout(&haryx(&["config", "--help"])).contains("autoswitch.threshold"));
}

#[test]
fn usage_errors_go_to_stderr_with_exit_two() {
    let output = haryx(&["list", "--status"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(stdout(&output).is_empty());
    assert!(stderr(&output).contains("Usage: haryx"));
}

#[test]
fn bare_haryx_without_a_terminal_is_a_usage_error() {
    let output = haryx(&[]);
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("no command given — try 'haryx help'"));
}

#[test]
fn watch_without_a_terminal_points_at_list() {
    // Test stdout is a pipe, never a terminal: the dashboard refuses before touching any store.
    let output = haryx(&["watch"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).contains("the dashboard needs a terminal"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn upgrade_from_a_source_build_explains_the_manual_routes() {
    // The test binary lives under target/, which matches no package manager.
    let output = haryx(&["upgrade"]);
    assert_eq!(output.status.code(), Some(1));
    let err = stderr(&output);
    assert!(err.contains("Could not detect install method"), "{err}");
    assert!(err.contains("brew upgrade haryx"), "{err}");
}

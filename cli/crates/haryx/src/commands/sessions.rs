//! `haryx ai sessions`: the command side of [`haryx_core::running`].

use std::path::{Path, PathBuf};

use haryx_core::AI_PROG;
use haryx_core::handoff;
use haryx_core::paths;
use haryx_core::printer::{accent, dimmed, warning_stderr};
use haryx_core::running::{self, RunningSession, Scan};
use haryx_core::session::session_dir_for;
use haryx_core::session_manager::{RunRequest, SessionManager};
use haryx_core::switcher::Switcher;
use serde_json::{Value, json};

use super::accounts::fail;
use super::profile_view::print_json;
use super::{CommandError, guard};

/// `ai sessions continue`: end a live session and resume its conversation on another account.
pub fn continue_session(
    key: &str,
    to: &str,
    json: bool,
    no_launch: bool,
    debug: bool,
) -> Result<(), CommandError> {
    guard::refuse_root()?;
    let switcher = Switcher::from_process(debug).map_err(fail)?;
    let roster = switcher
        .get_sequence_data_migrated()
        .map_err(fail)?
        .unwrap_or_default();
    let default_profile = paths::claude_config_home_in(switcher.path_env());
    let scan = running::scan(switcher.backup_dir(), &default_profile, &roster);
    let result = prepare(&switcher, &roster, &default_profile, &scan, key, to);
    let plan = match result {
        Ok(plan) => plan,
        Err(message) if json => {
            print_json(&json!({"error": {"message": message}}));
            return Err(CommandError::silent(1));
        }
        Err(message) => {
            return Err(CommandError {
                message: format!("Error: {message}"),
                exit_code: 1,
            });
        }
    };
    log::info!(
        "Continued session {} from account {} on account {}",
        plan.session_id,
        plan.from.as_deref().unwrap_or("-"),
        plan.to
    );
    if json {
        print_json(&plan.to_json());
        return Ok(());
    }
    if no_launch {
        println!("{}", plan.command_line());
        return Ok(());
    }
    println!(
        "{} session {} on account {} ({})",
        accent("Continuing"),
        short(&plan.session_id),
        plan.to,
        plan.to_email
    );
    std::env::set_current_dir(&plan.cwd).map_err(|e| CommandError {
        message: format!("Error: cannot enter {}: {e}", plan.cwd.display()),
        exit_code: 1,
    })?;
    haryx_core::session_manager::redirect_stdout_until_exec();
    let mut manager = SessionManager::new(&switcher);
    let request = RunRequest {
        share: true,
        share_history: None,
        require_session: false,
    };
    manager
        .run(
            &plan.to,
            &["--resume".to_owned(), plan.session_id.clone()],
            &request,
        )
        .map_err(fail)
}

/// Everything decided before the launch.
struct Plan {
    session_id: String,
    from: Option<String>,
    to: String,
    to_email: String,
    cwd: PathBuf,
    copied: Option<PathBuf>,
}

impl Plan {
    fn command_line(&self) -> String {
        format!(
            "{} run {} -- --resume {}",
            AI_PROG, self.to, self.session_id
        )
    }

    fn to_json(&self) -> Value {
        json!({
            "sessionId": self.session_id,
            "from": self.from.as_deref().and_then(|n| n.parse::<i64>().ok()),
            "to": self.to.parse::<i64>().ok(),
            "toEmail": self.to_email,
            "cwd": self.cwd,
            "copiedTranscript": self.copied,
            "command": self.command_line(),
        })
    }
}

fn short(id: &str) -> &str {
    id.get(..8).unwrap_or(id)
}

/// Find, choose, stop, copy: everything but the launch. Nothing is stopped unless a target exists.
fn prepare(
    switcher: &Switcher,
    roster: &serde_json::Map<String, Value>,
    default_profile: &Path,
    scan: &Scan,
    key: &str,
    to: &str,
) -> Result<Plan, String> {
    let live = handoff::find(scan, key)?.clone();
    let from = live.account.map(|n| n.to_string());
    let target = choose_target(switcher, from.as_deref(), to)?;
    let (to_num, to_email, _) = switcher
        .resolve_account(&target)
        .map_err(|e| e.to_string())?;
    // A default-profile session runs as the active account and picks up a switch while it runs, so
    // continuing it there would only restart it. Moving it takes another account, as for a session
    // profile, whose account is exact.
    if Some(&to_num) == from.as_ref() {
        return Err(format!("the session already runs as account {to_num}"));
    }
    let source_profile = profile_of(switcher, default_profile, &live);
    let transcript = handoff::transcript(&source_profile, &live.session.session_id);
    let active = roster
        .get("activeAccountNumber")
        .and_then(Value::as_i64)
        .map(|n| n.to_string());
    let target_profile = if active.as_deref() == Some(to_num.as_str()) {
        default_profile.to_path_buf()
    } else {
        session_dir_for(switcher.backup_dir(), &to_num, &to_email)
    };
    // Everything that can fail is checked before the old session is stopped.
    let cwd = PathBuf::from(&live.session.cwd);
    if !cwd.is_dir() {
        return Err(format!(
            "the session's folder {} no longer exists; nothing was changed",
            cwd.display()
        ));
    }
    // A profile that does not exist yet is set up by `run`, which shares history from ~/.claude.
    let copy_needed = transcript.is_some() && target_profile.join("projects").exists();
    if copy_needed && !handoff::can_write_into(&target_profile) {
        return Err(format!(
            "cannot write to {}; nothing was changed",
            target_profile.join("projects").display()
        ));
    }
    handoff::stop(live.session.pid, handoff::STOP_TIMEOUT)?;
    let copied = match &transcript {
        Some(file) if copy_needed => handoff::ensure_transcript(file, &target_profile)?,
        _ => None,
    };
    // The copy's replies were counted where they were made; from now on the session runs as the
    // target account.
    let store = switcher.backup_dir();
    if let Some(copy) = &copied
        && let Err(e) = haryx_core::usage_history::index::mark_read(store, copy)
    {
        log::debug!("usage history: could not mark the copy as read: {e}");
    }
    if let Ok(n) = to_num.parse::<i64>() {
        let profile = if target_profile == default_profile {
            "default"
        } else {
            "session"
        };
        haryx_core::usage_history::attribution::record_sessions(
            store,
            &[(live.session.session_id.clone(), n, profile)],
        );
    }
    Ok(Plan {
        session_id: live.session.session_id.clone(),
        from,
        to: to_num,
        to_email,
        cwd,
        copied,
    })
}

/// The profile folder a live session runs in.
fn profile_of(switcher: &Switcher, default_profile: &Path, live: &RunningSession) -> PathBuf {
    match (live.profile, live.account) {
        ("session", Some(num)) => {
            session_dir_for(switcher.backup_dir(), &num.to_string(), &live.email)
        }
        _ => default_profile.to_path_buf(),
    }
}

/// `best` picks an account with headroom other than `from`, preferring `from`'s group
/// ([`Switcher::best_handoff_target`]); anything else is an account number or email.
fn choose_target(switcher: &Switcher, from: Option<&str>, to: &str) -> Result<String, String> {
    if !to.eq_ignore_ascii_case("best") {
        return Ok(to.to_owned());
    }
    let (target, note) = switcher
        .best_handoff_target(from)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "no other account has headroom left; pick one with --to".to_owned())?;
    if let Some(note) = note {
        warning_stderr(&note);
    }
    Ok(target)
}

pub fn run(json: bool, debug: bool) -> Result<(), CommandError> {
    guard::refuse_root()?;
    let switcher = Switcher::from_process(debug).map_err(fail)?;
    let roster = switcher
        .get_sequence_data_migrated()
        .map_err(fail)?
        .unwrap_or_default();
    let default_profile = paths::claude_config_home_in(switcher.path_env());
    let scan = running::scan(switcher.backup_dir(), &default_profile, &roster);
    let live: Vec<(String, i64, &str)> = scan
        .sessions
        .iter()
        .filter_map(|s| Some((s.session.session_id.clone(), s.account?, s.profile)))
        .collect();
    haryx_core::usage_history::attribution::record_sessions(switcher.backup_dir(), &live);
    if json {
        print_json(&running::to_json(&scan));
    } else {
        print_text(&scan);
    }
    Ok(())
}

fn print_text(scan: &Scan) {
    if scan.sessions.is_empty() {
        println!("No Claude Code sessions running.");
    }
    for s in &scan.sessions {
        let who = match s.account {
            Some(n) => format!("{n:>3}  {}", s.email),
            None => "  -  (no active account)".to_string(),
        };
        let status = s.session.status.as_deref().unwrap_or("");
        println!("{:>7}  {who}  {status}", s.session.pid);
        println!(
            "         {}",
            dimmed(&format!("{}  ({})", s.session.cwd, s.profile))
        );
    }
    if scan.unreadable > 0 {
        println!(
            "{}",
            dimmed(&format!(
                "{} session record(s) could not be read; more may be running.",
                scan.unreadable
            ))
        );
    }
}

#[cfg(test)]
mod continue_tests {
    use super::*;

    #[test]
    fn plan_json_matches_the_contract() {
        let contract: Value = serde_json::from_str(include_str!(
            "../../../../../packages/contracts/haryx/session-continue.json"
        ))
        .unwrap();
        let plan = Plan {
            session_id: "4f6c2a1e-9b7d-4e3a-8c21-0d5e7f9a1b2c".into(),
            from: Some("1".into()),
            to: "3".into(),
            to_email: "account3@example.com".into(),
            cwd: PathBuf::from("/Users/me/work/api"),
            copied: None,
        };
        assert_eq!(plan.to_json(), contract);
        assert_eq!(short(&plan.session_id), "4f6c2a1e");
    }
}

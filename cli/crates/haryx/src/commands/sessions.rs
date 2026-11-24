//! `haryx ai sessions`: the command side of [`haryx_core::running`].

use haryx_core::paths;
use haryx_core::printer::dimmed;
use haryx_core::running::{self, Scan};
use haryx_core::switcher::Switcher;

use super::accounts::fail;
use super::profile_view::print_json;
use super::{CommandError, guard};

pub fn run(json: bool, debug: bool) -> Result<(), CommandError> {
    guard::refuse_root()?;
    let switcher = Switcher::from_process(debug).map_err(fail)?;
    let roster = switcher
        .get_sequence_data_migrated()
        .map_err(fail)?
        .unwrap_or_default();
    let default_profile = paths::claude_config_home_in(switcher.path_env());
    let scan = running::scan(switcher.backup_dir(), &default_profile, &roster);
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

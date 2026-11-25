//! `haryx ai doctor`: the command side of [`haryx_core::claude_setup`].

use haryx_core::claude_setup::{self, Readiness};
use haryx_core::switcher::Switcher;
use serde_json::{Value, json};

use super::CommandError;
use super::accounts::fail;
use super::profile_view::print_json;

/// Reports and exits 0 either way: the result is the answer, not a failure of the command.
pub fn run(json: bool, debug: bool) -> Result<(), CommandError> {
    let switcher = Switcher::from_process(debug).map_err(fail)?;
    let readiness = Readiness {
        claude: claude_setup::claude_bin().ok(),
        signed_in_email: switcher.get_current_account().map(|(email, _)| email),
    };
    let config = switcher.claude_config_path();
    if json {
        print_json(&report_json(&readiness, &config.display().to_string()));
    } else {
        print_text(&readiness, &config.display().to_string());
    }
    Ok(())
}

fn report_json(r: &Readiness, config_path: &str) -> Value {
    json!({
        "ready": r.is_ready(),
        "claude": r.claude.as_ref().map(|p| p.display().to_string()),
        "signedIn": r.signed_in_email.is_some(),
        "email": r.signed_in_email,
        "configPath": config_path,
        "installCommand": claude_setup::INSTALL_COMMAND,
        "problems": r.problems().iter().map(|p| json!({
            "code": p.code(),
            "message": p.message(),
            "fix": p.fix(),
        })).collect::<Vec<_>>(),
    })
}

fn print_text(r: &Readiness, config_path: &str) {
    let claude = r
        .claude
        .as_ref()
        .map_or("not found".into(), |p| p.display().to_string());
    let signed_in = r.signed_in_email.as_deref().unwrap_or("no");
    println!("Claude Code  {claude}");
    println!("Signed in    {signed_in}  ({config_path})");
    let problems = r.problems();
    if problems.is_empty() {
        println!("\nReady.");
        return;
    }
    for p in problems {
        println!("\n{}\n  {}", p.message(), p.fix());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_lists_each_problem_with_its_fix() {
        let r = Readiness {
            claude: None,
            signed_in_email: None,
        };
        let v = report_json(&r, "/h/.claude.json");
        assert_eq!(v["ready"], false);
        assert_eq!(v["signedIn"], false);
        assert_eq!(v["problems"][0]["code"], "claude_missing");
        assert_eq!(v["problems"][1]["code"], "not_signed_in");
        assert!(
            v["problems"][0]["fix"]
                .as_str()
                .unwrap()
                .contains(claude_setup::INSTALL_COMMAND)
        );
    }

    #[test]
    fn json_when_ready() {
        let r = Readiness {
            claude: Some("/x/claude".into()),
            signed_in_email: Some("a@x.com".into()),
        };
        let v = report_json(&r, "/h/.claude.json");
        assert_eq!(v["ready"], true);
        assert_eq!(v["email"], "a@x.com");
        assert_eq!(v["problems"].as_array().unwrap().len(), 0);
    }
}

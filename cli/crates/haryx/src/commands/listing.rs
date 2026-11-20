//! `list` and `status` (upstream `main()` arms for `--list` / `--status`). The switcher renders the
//! human view itself; with `--json` the payload is printed here, as the single place JSON is written.

use haryx_core::json_output::error_envelope;
use haryx_core::models::pyfmt::json_dumps_pretty;
use haryx_core::switcher::Switcher;
use haryx_core::{Error, printer, settings};
use serde_json::Value;

use super::{CommandError, guard};
use crate::cli::SwitchStrategy;

/// Run a JSON-capable command: on a handled error, JSON mode prints the envelope on stdout (exit 1);
/// otherwise `Error: ...` on stderr. A returned payload is printed with `indent=2`.
fn run_json_capable(
    json: bool,
    debug: bool,
    body: impl FnOnce(&Switcher) -> Result<Option<Value>, Error>,
) -> Result<(), CommandError> {
    guard::refuse_root()?;
    let outcome = Switcher::from_process(debug).and_then(|s| body(&s));
    match outcome {
        Ok(Some(payload)) if json => {
            println!("{}", json_dumps_pretty(&payload));
            Ok(())
        }
        Ok(_) => Ok(()),
        Err(err) if json => {
            println!("{}", json_dumps_pretty(&error_envelope(&err)));
            Err(CommandError::silent(1))
        }
        Err(err) => {
            printer::error(&format!("Error: {err}"));
            Err(CommandError::silent(1))
        }
    }
}

pub fn list(token_status: bool, json: bool, debug: bool) -> Result<(), CommandError> {
    run_json_capable(json, debug, |s| {
        Ok(s.list_accounts(token_status, json, None))
    })
}

pub fn status(json: bool, debug: bool) -> Result<(), CommandError> {
    run_json_capable(json, debug, |s| Ok(s.status(json)))
}

pub fn switch(
    strategy: Option<SwitchStrategy>,
    model: Option<&str>,
    json: bool,
    debug: bool,
) -> Result<(), CommandError> {
    run_json_capable(json, debug, |s| {
        let strategy = strategy.map(|st| match st {
            SwitchStrategy::Best => "best",
            SwitchStrategy::NextAvailable => "next-available",
        });
        // Only the usage-aware strategies read model limits: --model wins, else `autoswitch.model`.
        let (models, source) = match (strategy, model) {
            (None, _) => (Vec::new(), None),
            (Some(_), Some(m)) => (settings::parse_model_names(Some(m)), Some("cli")),
            (Some(_), None) => {
                let models = settings::parse_model_names(
                    settings::load_settings(s.backup_dir()).model.as_deref(),
                );
                let source = (!models.is_empty()).then_some("autoswitch.model");
                (models, source)
            }
        };
        let mut payload = s.switch(strategy, json, &models, source)?;
        if let Some(Value::Object(p)) = payload.as_mut()
            && !models.is_empty()
        {
            p.insert("models".into(), serde_json::json!(models));
            p.insert("modelSource".into(), serde_json::json!(source));
        }
        Ok(payload)
    })
}

pub fn switch_to(target: &str, json: bool, force: bool, debug: bool) -> Result<(), CommandError> {
    run_json_capable(json, debug, |s| s.switch_to(target, json, force))
}

//! `list` and `status` (upstream `main()` arms for `--list` / `--status`). The switcher renders the
//! human view itself; with `--json` the payload is printed here, as the single place JSON is written.

use haryx_core::json_output::error_envelope;
use haryx_core::models::pyfmt::json_dumps_pretty;
use haryx_core::switcher::Switcher;
use haryx_core::{Error, printer};
use serde_json::Value;

use super::{CommandError, guard};

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

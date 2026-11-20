//! `haryx config [--json] [list|get KEY|set KEY VALUE|unset KEY|path]`: port of upstream
//! `cli._config_command`.
//!
//! Edits settings.json in the store root with strict validation (unlike loading, which forgivingly
//! clamps), so a typo'd key or out-of-range value errors loudly here instead of silently degrading at
//! `haryx auto` time. `list` also names the keys a hand edit left in the file that nothing reads.
//!
//! Exit codes: 0 success, 1 a handled error (`Error: ...` on stderr, or a JSON error envelope on stdout
//! with `--json`), 2 usage errors (raised by the parser before this runs).

use std::path::Path;

use haryx_core::json_output::error_envelope;

use haryx_core::models::pyfmt::json_dumps_pretty;
use haryx_core::settings::{
    self, EffectiveSetting, SettingValue, format_setting_value, setting_spec, settings_path,
};
use haryx_core::{Error, paths};
use serde_json::json;

use super::{CommandError, guard};
use crate::cli::ConfigAction;

/// Exit code of a handled error.
const ERROR_EXIT: u8 = 1;

/// Run `haryx config`.
pub fn run(json_mode: bool, action: &ConfigAction) -> Result<(), CommandError> {
    guard::refuse_root()?;
    paths::store_root()
        .and_then(|root| execute(&root, json_mode, action))
        .map_err(|err| report(&err, json_mode))
}

fn report(err: &Error, json_mode: bool) -> CommandError {
    if json_mode {
        println!("{}", json_dumps_pretty(&error_envelope(err)));
        CommandError::silent(ERROR_EXIT)
    } else {
        // TODO(printer): upstream prints this in the palette's red.
        CommandError {
            message: format!("Error: {err}"),
            exit_code: ERROR_EXIT,
        }
    }
}

fn execute(root: &Path, json_mode: bool, action: &ConfigAction) -> haryx_core::Result<()> {
    match action {
        ConfigAction::Path => println!("{}", settings_path(root).display()),
        ConfigAction::List => list(root, json_mode),
        ConfigAction::Get { key } => get(root, json_mode, key)?,
        ConfigAction::Set { key, value } => set(root, key, value)?,
        ConfigAction::Unset { key } => unset(root, key)?,
    }
    Ok(())
}

fn list(root: &Path, json_mode: bool) {
    let rows = settings::effective_settings(root);
    let ignored = settings::unrecognized_keys(root);
    if json_mode {
        let payload = json!({
            "schemaVersion": 1,
            "path": settings_path(root).to_string_lossy(),
            "settings": rows.iter().map(|row| json!({
                "key": row.spec.dotted(),
                "value": row.value.to_json(),
                "isSet": row.is_set,
            })).collect::<Vec<_>>(),
            "unrecognized": ignored.iter().map(|item| json!({
                "key": item.dotted,
                "suggestion": item.suggestion,
            })).collect::<Vec<_>>(),
        });
        println!("{}", json_dumps_pretty(&payload));
        return;
    }
    for line in list_lines(&rows) {
        println!("{line}");
    }
    for item in ignored {
        // TODO(printer): upstream prints this in the palette's yellow.
        eprintln!("Warning: settings.json: {}", item.describe());
    }
}

/// The text table: key and value columns padded to their widest entry, unset rows marked `(default)`.
fn list_lines(rows: &[EffectiveSetting]) -> Vec<String> {
    let width = |f: &dyn Fn(&EffectiveSetting) -> usize| rows.iter().map(f).max().unwrap_or(0);
    let key_w = width(&|row| row.spec.dotted().chars().count());
    let val_w = width(&|row| format_setting_value(&row.value).chars().count());
    rows.iter()
        .map(|row| {
            let line = format!(
                "{:<key_w$}  {:<val_w$}",
                row.spec.dotted(),
                format_setting_value(&row.value)
            );
            // TODO(printer): upstream dims the "(default)" marker.
            if row.is_set {
                line
            } else {
                format!("{line}  (default)")
            }
        })
        .collect()
}

fn get(root: &Path, json_mode: bool, key: &str) -> haryx_core::Result<()> {
    let spec = setting_spec(key)?;
    let row = settings::effective_settings(root)
        .into_iter()
        .find(|row| std::ptr::eq(row.spec, spec))
        .ok_or_else(|| Error::Config(format!("unknown setting '{key}'")))?;
    if json_mode {
        let payload = json!({
            "schemaVersion": 1,
            "key": spec.dotted(),
            "value": row.value.to_json(),
            "isSet": row.is_set,
        });
        println!("{}", json_dumps_pretty(&payload));
    } else {
        println!("{}", format_setting_value(&row.value));
    }
    Ok(())
}

fn set(root: &Path, key: &str, value: &str) -> haryx_core::Result<()> {
    let value = settings::set_setting(root, key, value)?;
    println!("{key} = {}", format_setting_value(&value));
    let usage = settings::load_usage_settings(root);
    if key.starts_with("usage.") && usage.fable_probe && !usage.header_probe {
        // The Fable probe rides on the header probe's request cycle.
        // TODO(printer): upstream prints this muted.
        eprintln!("usage.fableProbe does nothing until usage.headerProbe is true");
    }
    Ok(())
}

fn unset(root: &Path, key: &str) -> haryx_core::Result<()> {
    if settings::unset_setting(root, key)? {
        let default: SettingValue = setting_spec(key)?.default();
        println!("{key} unset (default: {})", format_setting_value(&default));
    } else {
        // TODO(printer): upstream prints this muted.
        eprintln!("{key} is not set; nothing to do");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_pads_columns_and_marks_defaults() {
        let dir = tempfile::tempdir().unwrap();
        settings::set_setting(dir.path(), "autoswitch.cooldownSeconds", "600").unwrap();
        let lines = list_lines(&settings::effective_settings(dir.path()));
        assert_eq!(lines.len(), 15);
        let key_w = "autoswitch.includeApiKeyAccounts".len();
        assert_eq!(
            lines[0],
            format!("{:<key_w$}  {:<6}  (default)", "autoswitch.threshold", "90")
        );
        assert_eq!(
            lines[3],
            format!("{:<key_w$}  {:<6}", "autoswitch.cooldownSeconds", "600")
        );
    }

    #[test]
    fn envelope_names_the_upstream_exception() {
        let envelope = error_envelope(&Error::Config("bad".into()));
        assert_eq!(
            envelope,
            json!({"schemaVersion": 1, "error": {"type": "ConfigError", "message": "bad"}})
        );
    }
}

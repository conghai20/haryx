//! `haryx ai desktop`: the command side of [`haryx_core::desktop`].

use haryx_core::Error;
use haryx_core::desktop::{self, AccountFolder};
use haryx_core::printer::dimmed;
use haryx_core::switcher::Switcher;
use serde_json::{Value, json};

use super::accounts::fail;
use super::profile_view::print_json;
use super::{CommandError, guard};
use crate::cli::DesktopAction;

pub fn run(action: &DesktopAction, debug: bool) -> Result<(), CommandError> {
    guard::refuse_root()?;
    let switcher = Switcher::from_process(debug).map_err(fail)?;
    let roster = switcher
        .get_sequence_data_migrated()
        .map_err(fail)?
        .unwrap_or_default();
    let rows = desktop::account_folders(switcher.backup_dir(), &roster);
    match action {
        DesktopAction::Status { json } => {
            status(&rows, *json);
            Ok(())
        }
        DesktopAction::Path { account } => path(&switcher, &rows, account),
    }
}

fn row_json(row: &AccountFolder) -> Value {
    json!({
        "number": row.number,
        "email": row.email,
        "alias": row.alias,
        "uuid": row.uuid,
        "folder": row.folder.as_ref().map(|f| f.display().to_string()),
        "folderExists": row.exists,
        "signedInUuid": row.signed_in_uuid,
        "mismatch": row.is_mismatch(),
    })
}

/// One word for a folder's state in the text listing.
fn state(row: &AccountFolder) -> String {
    if row.folder.is_none() {
        "no account UUID".into()
    } else if !row.exists {
        "no folder".into()
    } else if row.is_mismatch() {
        format!(
            "signed in as another account ({})",
            row.signed_in_uuid.as_deref().unwrap_or_default()
        )
    } else if row.is_signed_in() {
        "signed in".into()
    } else {
        "not signed in yet".into()
    }
}

fn status(rows: &[AccountFolder], json: bool) {
    if json {
        let accounts: Vec<Value> = rows.iter().map(row_json).collect();
        print_json(&json!({ "accounts": accounts }));
        return;
    }
    if rows.is_empty() {
        println!("No accounts.");
        return;
    }
    for row in rows {
        println!("{:>3}  {:<32}  {}", row.number, row.email, state(row));
        if let Some(folder) = row.folder.as_ref().filter(|_| row.exists) {
            println!("     {}", dimmed(&folder.display().to_string()));
        }
    }
}

fn path(switcher: &Switcher, rows: &[AccountFolder], account: &str) -> Result<(), CommandError> {
    let (num, email, _) = switcher.resolve_account(account).map_err(fail)?;
    let row = rows
        .iter()
        .find(|r| r.number.to_string() == num)
        .ok_or_else(|| {
            fail(Error::AccountNotFound(format!(
                "Account-{num} does not exist"
            )))
        })?;
    let folder = row.folder.as_ref().ok_or_else(|| {
        fail(Error::General(format!(
            "{email} has no account UUID, so it has no Claude Desktop folder"
        )))
    })?;
    println!("{}", folder.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn row(folder: Option<&str>, exists: bool, signed_in: Option<&str>) -> AccountFolder {
        AccountFolder {
            number: 1,
            email: "a@x.com".into(),
            alias: String::new(),
            uuid: "a".into(),
            folder: folder.map(PathBuf::from),
            exists,
            signed_in_uuid: signed_in.map(str::to_owned),
        }
    }

    #[test]
    fn state_words() {
        assert_eq!(state(&row(None, false, None)), "no account UUID");
        assert_eq!(state(&row(Some("/f"), false, None)), "no folder");
        assert_eq!(state(&row(Some("/f"), true, None)), "not signed in yet");
        assert_eq!(state(&row(Some("/f"), true, Some("a"))), "signed in");
        assert_eq!(
            state(&row(Some("/f"), true, Some("b"))),
            "signed in as another account (b)"
        );
    }

    #[test]
    fn json_row_shape() {
        let v = row_json(&row(Some("/f"), true, Some("b")));
        assert_eq!(v["folder"], "/f");
        assert_eq!(v["folderExists"], true);
        assert_eq!(v["signedInUuid"], "b");
        assert_eq!(v["mismatch"], true);
        assert_eq!(row_json(&row(None, false, None))["folder"], Value::Null);
    }
}

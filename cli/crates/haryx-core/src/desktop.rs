//! Claude Desktop following the Use button (plan: `docs/desktop-follow-plan.md`).
//!
//! Each account gets its own Desktop data folder, `<store>/desktop/<accountUuid>/`, which the macOS
//! app hands to Claude Desktop as `--user-data-dir`. This module only names those folders and reads
//! which account a folder last signed into. It never creates, moves, copies or deletes a folder, and
//! never reads the login inside one.

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::switcher::JsonObject;

/// The folder under the store root that holds one Desktop data folder per account.
pub const DIRNAME: &str = "desktop";
/// Claude Desktop's own settings file inside a data folder.
const CONFIG_FILE: &str = "config.json";
/// The key Desktop writes when an account signs in. Undocumented: its absence means "unknown".
const SIGNED_IN_KEY: &str = "lastKnownAccountUuid";
/// Group lengths of a canonical 8-4-4-4-12 UUID.
const UUID_GROUPS: [usize; 5] = [8, 4, 4, 4, 12];

/// Whether `s` is a canonical hex UUID. Folder names are built from it, so anything else (an empty
/// field, `..`, a slash) is refused rather than joined into a path.
pub fn is_uuid(s: &str) -> bool {
    let groups: Vec<&str> = s.split('-').collect();
    groups.len() == UUID_GROUPS.len()
        && groups
            .iter()
            .zip(UUID_GROUPS)
            .all(|(g, n)| g.len() == n && g.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// The Desktop data folder of the account with this UUID; `None` when the UUID is not one. Keyed by
/// UUID, not slot number or email, because slot numbers can be reassigned.
pub fn folder(store_root: &Path, account_uuid: &str) -> Option<PathBuf> {
    is_uuid(account_uuid).then(|| {
        store_root
            .join(DIRNAME)
            .join(account_uuid.to_ascii_lowercase())
    })
}

/// The account a Desktop data folder last signed into. `None` ("unknown") when `config.json` is
/// missing or not JSON, or holds no UUID under the key.
pub fn signed_in_uuid(folder: &Path) -> Option<String> {
    let text = std::fs::read_to_string(folder.join(CONFIG_FILE)).ok()?;
    let config: Value = serde_json::from_str(&text).ok()?;
    config
        .get(SIGNED_IN_KEY)?
        .as_str()
        .filter(|u| is_uuid(u))
        .map(str::to_ascii_lowercase)
}

/// One managed account and its Desktop data folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountFolder {
    pub number: i64,
    pub email: String,
    pub alias: String,
    /// The account's UUID as stored, lowercased; "" when the roster has none.
    pub uuid: String,
    /// `None` when the account has no usable UUID, so it cannot have a folder.
    pub folder: Option<PathBuf>,
    /// The folder exists as a directory.
    pub exists: bool,
    /// See [`signed_in_uuid`]; `None` also when there is no folder.
    pub signed_in_uuid: Option<String>,
}

impl AccountFolder {
    /// The folder is signed into a different account. An unknown sign-in is never a mismatch.
    pub fn is_mismatch(&self) -> bool {
        self.signed_in_uuid
            .as_deref()
            .is_some_and(|signed| signed != self.uuid)
    }

    /// The folder exists and is signed into this account.
    pub fn is_signed_in(&self) -> bool {
        self.signed_in_uuid.as_deref() == Some(self.uuid.as_str())
    }
}

/// Every account in `sequence.json` order, with the state of its Desktop data folder. Reads only the
/// roster and each folder's `config.json`.
pub fn account_folders(store_root: &Path, roster: &JsonObject) -> Vec<AccountFolder> {
    let empty = JsonObject::new();
    let accounts = roster
        .get("accounts")
        .and_then(Value::as_object)
        .unwrap_or(&empty);
    let field = |rec: Option<&Value>, key: &str| {
        rec.and_then(|r| r.get(key))
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_owned()
    };
    roster
        .get("sequence")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_i64)
        .map(|number| {
            let rec = accounts.get(&number.to_string());
            let uuid = field(rec, "uuid").to_ascii_lowercase();
            let folder = folder(store_root, &uuid);
            let exists = folder.as_deref().is_some_and(Path::is_dir);
            let signed_in_uuid = folder
                .as_deref()
                .filter(|_| exists)
                .and_then(signed_in_uuid);
            AccountFolder {
                number,
                email: field(rec, "email"),
                alias: field(rec, "alias"),
                uuid,
                folder,
                exists,
                signed_in_uuid,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const A: &str = "7c612f5a-e7d1-43ab-ac37-06c2028bea7a";
    const B: &str = "4349fb27-5afa-4766-8a79-3b16c4ddca6f";

    fn roster(value: Value) -> JsonObject {
        value.as_object().cloned().unwrap()
    }

    fn make_folder(root: &Path, uuid: &str, config: Option<&str>) {
        let dir = root.join(DIRNAME).join(uuid);
        std::fs::create_dir_all(&dir).unwrap();
        if let Some(text) = config {
            std::fs::write(dir.join(CONFIG_FILE), text).unwrap();
        }
    }

    #[test]
    fn uuid_shape_is_strict() {
        assert!(is_uuid(A));
        assert!(is_uuid(&A.to_uppercase()));
        assert!(!is_uuid(""));
        assert!(!is_uuid(".."));
        assert!(!is_uuid("7c612f5a-e7d1-43ab-ac37-06c2028bea7"));
        assert!(!is_uuid("7c612f5a-e7d1-43ab-ac37/06c2028bea7a"));
        assert!(!is_uuid("7c612f5a-e7d1-43ab-ac37-06c2028bea7a-0000"));
        assert!(!is_uuid("zc612f5a-e7d1-43ab-ac37-06c2028bea7a"));
    }

    #[test]
    fn folder_is_keyed_by_lowercased_uuid() {
        let root = Path::new("/store");
        assert_eq!(
            folder(root, &A.to_uppercase()),
            Some(root.join("desktop").join(A))
        );
        assert_eq!(folder(root, "../../etc"), None);
        assert_eq!(folder(root, ""), None);
    }

    #[test]
    fn signed_in_uuid_reads_config_and_treats_anything_odd_as_unknown() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        assert_eq!(signed_in_uuid(&root.join("missing")), None);

        make_folder(
            root,
            A,
            Some(&format!(
                r#"{{"lastKnownAccountUuid":"{}"}}"#,
                A.to_uppercase()
            )),
        );
        assert_eq!(
            signed_in_uuid(&root.join(DIRNAME).join(A)).as_deref(),
            Some(A)
        );

        for bad in [
            "not json",
            "{}",
            r#"{"lastKnownAccountUuid":42}"#,
            r#"{"lastKnownAccountUuid":"x"}"#,
            "[]",
        ] {
            make_folder(root, B, Some(bad));
            assert_eq!(signed_in_uuid(&root.join(DIRNAME).join(B)), None, "{bad}");
        }
    }

    #[test]
    fn account_folders_report_none_signed_in_unknown_and_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let c = "27bcca23-8425-425a-ae92-3704caa54b77";
        make_folder(
            root,
            A,
            Some(&format!(r#"{{"lastKnownAccountUuid":"{A}"}}"#)),
        );
        make_folder(
            root,
            B,
            Some(&format!(r#"{{"lastKnownAccountUuid":"{A}"}}"#)),
        );
        make_folder(root, c, None);
        let data = roster(json!({
            "sequence": [2, 1, 3, 4, 5, 9],
            "accounts": {
                "1": {"email": "a@x.com", "uuid": A, "alias": "work"},
                "2": {"email": "b@x.com", "uuid": B.to_uppercase()},
                "3": {"email": "c@x.com", "uuid": c},
                "4": {"email": "d@x.com", "uuid": "ced206c6-368d-4657-8020-a9f83410f06a"},
                "5": {"email": "e@x.com"}
            }
        }));

        let rows = account_folders(root, &data);
        let numbers: Vec<i64> = rows.iter().map(|r| r.number).collect();
        assert_eq!(
            numbers,
            [2, 1, 3, 4, 5, 9],
            "roster order, unknown slots kept"
        );

        let [b, a, c_row, d, e, ghost] = rows.as_slice() else {
            panic!()
        };
        assert!(a.exists && a.is_signed_in() && !a.is_mismatch());
        assert_eq!(a.alias, "work");
        assert_eq!(b.uuid, B, "uuid lowercased");
        assert!(b.exists && b.is_mismatch() && !b.is_signed_in());
        assert!(c_row.exists && c_row.signed_in_uuid.is_none() && !c_row.is_mismatch());
        assert!(!d.exists && d.folder.is_some() && !d.is_mismatch());
        assert!(e.folder.is_none() && !e.exists && !e.is_mismatch());
        assert!(ghost.email.is_empty() && ghost.folder.is_none());
    }

    #[test]
    fn a_file_where_the_folder_should_be_is_not_a_folder() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(DIRNAME)).unwrap();
        std::fs::write(dir.path().join(DIRNAME).join(A), "").unwrap();
        let data =
            roster(json!({"sequence": [1], "accounts": {"1": {"email": "a@x.com", "uuid": A}}}));
        let rows = account_folders(dir.path(), &data);
        assert!(!rows[0].exists);
        assert!(rows[0].signed_in_uuid.is_none());
    }
}

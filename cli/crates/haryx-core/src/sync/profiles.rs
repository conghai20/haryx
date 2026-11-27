//! Setup profiles carried between machines: `haryx sync profile push|pull|list`.
//!
//! A profile travels as its export file (secrets redacted, no history, no credentials), encrypted
//! whole to the sync key, as the API's profile file `/profiles/<HMAC of the name>` (the API scopes
//! it to the caller). Names live only in the encrypted `profiles` index row. A pull hands back the file for
//! `ai profile import`; applying it stays the explicit `ai profile apply`.

use std::time::Duration;

use serde_json::{Value, json};

use super::crypto::SyncKey;
use super::merge::Values;
use super::remote::{Api, Method, Row, encode};
use super::{Context, KIND_PROFILES, Opened, SyncError, open, require_key, write_row};

/// Profiles one account may keep on the server.
pub const MAX_PROFILES: usize = 3;
/// Largest encrypted profile file; the API enforces the same limit.
pub const MAX_BYTES: usize = 10 * 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(120);

/// Where the encrypted files live. Names are object keys (lowercase hex).
pub trait Objects {
    fn put(&self, name: &str, bytes: &[u8]) -> Result<(), SyncError>;
    fn get(&self, name: &str) -> Result<Option<Vec<u8>>, SyncError>;
    fn delete(&self, name: &str) -> Result<(), SyncError>;
    /// The names of every file this user has.
    fn list(&self) -> Result<Vec<String>, SyncError>;
}

fn object_path(name: &str) -> String {
    format!("/profiles/{}", encode(name))
}

/// The API's `/profiles`.
impl Objects for Api {
    fn put(&self, name: &str, bytes: &[u8]) -> Result<(), SyncError> {
        let headers = [("content-type", "application/octet-stream")];
        self.call(
            Method::Put,
            &object_path(name),
            &headers,
            bytes,
            TIMEOUT,
            &[],
        )
        .map(drop)
    }

    fn get(&self, name: &str) -> Result<Option<Vec<u8>>, SyncError> {
        let resp = self.call(Method::Get, &object_path(name), &[], &[], TIMEOUT, &[404])?;
        Ok(resp.map(|r| r.body))
    }

    fn delete(&self, name: &str) -> Result<(), SyncError> {
        self.call(Method::Delete, &object_path(name), &[], &[], TIMEOUT, &[])
            .map(drop)
    }

    fn list(&self) -> Result<Vec<String>, SyncError> {
        let bad = || SyncError::Server("unexpected profiles response".into());
        let resp = self
            .call(Method::Get, "/profiles", &[], &[], TIMEOUT, &[])?
            .ok_or_else(bad)?;
        let value = resp.json().map_err(|_| bad())?;
        value
            .as_array()
            .ok_or_else(bad)?
            .iter()
            .map(|p| {
                p.get("name")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .ok_or_else(bad)
            })
            .collect()
    }
}

/// Objects in memory, for tests.
#[derive(Default)]
pub struct MemoryObjects {
    pub files: std::sync::Mutex<std::collections::BTreeMap<String, Vec<u8>>>,
}

impl Objects for MemoryObjects {
    fn put(&self, name: &str, bytes: &[u8]) -> Result<(), SyncError> {
        self.files
            .lock()
            .expect("files lock")
            .insert(name.to_owned(), bytes.to_vec());
        Ok(())
    }

    fn get(&self, name: &str) -> Result<Option<Vec<u8>>, SyncError> {
        Ok(self.files.lock().expect("files lock").get(name).cloned())
    }

    fn delete(&self, name: &str) -> Result<(), SyncError> {
        self.files.lock().expect("files lock").remove(name);
        Ok(())
    }

    fn list(&self) -> Result<Vec<String>, SyncError> {
        Ok(self
            .files
            .lock()
            .expect("files lock")
            .keys()
            .cloned()
            .collect())
    }
}

/// A profile on the server.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub name: String,
    pub bytes: u64,
    /// Unix seconds.
    pub pushed_at: i64,
}

fn object_name(key: &SyncKey, name: &str) -> String {
    key.row_key(&format!("profile:{name}"))
}

/// The index row and its entries, by object key. An index that does not open reads as empty, and
/// the next push replaces it.
fn index(ctx: &Context<'_>, key: &SyncKey) -> Result<(Option<Row>, Values), SyncError> {
    let row = ctx
        .remote
        .list()?
        .into_iter()
        .find(|r| r.kind == KIND_PROFILES && r.key.is_empty());
    let values = match row.as_ref().map(|row| open(key, row)) {
        Some(Opened::Doc(doc)) => doc.values,
        _ => Values::new(),
    };
    Ok((row, values))
}

/// What is encrypted for profile `name`: the file, behind a header naming it, so the server cannot
/// hand back one profile's file as another's.
fn framed(name: &str, file: &[u8]) -> Vec<u8> {
    let mut plain = format!("haryx-profile:{name}\n").into_bytes();
    plain.extend_from_slice(file);
    plain
}

fn entries(values: &Values) -> Vec<Entry> {
    let mut list: Vec<Entry> = values
        .values()
        .filter_map(|v| {
            Some(Entry {
                name: v.get("name")?.as_str()?.to_owned(),
                bytes: v.get("bytes")?.as_u64()?,
                pushed_at: v.get("pushedAt")?.as_i64()?,
            })
        })
        .collect();
    list.sort_by(|a, b| a.name.cmp(&b.name));
    list
}

/// Write the index, at the version it was read. `false`: another machine changed it first.
fn save_index(
    ctx: &Context<'_>,
    key: &SyncKey,
    row: Option<&Row>,
    values: Values,
) -> Result<bool, SyncError> {
    let current = row.map(|r| r.version);
    Ok(write_row(ctx, key, KIND_PROFILES, "", values, ctx.now, current)?.is_some())
}

pub fn list(ctx: &Context<'_>) -> Result<Vec<Entry>, SyncError> {
    let key = require_key(ctx)?;
    Ok(entries(&index(ctx, &key)?.1))
}

/// Upload the export file of profile `name`, replacing an earlier one of that name.
pub fn push(ctx: &Context<'_>, name: &str, file: &[u8]) -> Result<Entry, SyncError> {
    let key = require_key(ctx)?;
    let cipher = key.seal_bytes(&framed(name, file))?;
    if cipher.len() > MAX_BYTES {
        return Err(SyncError::Invalid(format!(
            "Profile '{name}' is {} MB encrypted; the limit is {} MB. Leave plugins as references \
             and history out.",
            cipher.len() / (1024 * 1024),
            MAX_BYTES / (1024 * 1024)
        )));
    }
    let object = object_name(&key, name);
    for _ in 0..3 {
        let (row, mut values) = index(ctx, &key)?;
        if !values.contains_key(&object) && values.len() >= MAX_PROFILES {
            return Err(SyncError::Invalid(format!(
                "At most {MAX_PROFILES} profiles are kept on the server; remove one with \
                 'haryx sync profile remove NAME' first"
            )));
        }
        ctx.objects.put(&object, &cipher)?;
        let entry = Entry {
            name: name.to_owned(),
            bytes: file.len() as u64,
            pushed_at: ctx.now,
        };
        values.insert(
            object.clone(),
            json!({"name": entry.name, "bytes": entry.bytes, "pushedAt": entry.pushed_at}),
        );
        if save_index(ctx, &key, row.as_ref(), values)? {
            return Ok(entry);
        }
    }
    Err(SyncError::Busy)
}

/// The export file of profile `name`, decrypted. `None`: no such profile on the server.
pub fn pull(ctx: &Context<'_>, name: &str) -> Result<Option<Vec<u8>>, SyncError> {
    let key = require_key(ctx)?;
    let object = object_name(&key, name);
    let Some(cipher) = ctx.objects.get(&object)? else {
        return Ok(None);
    };
    let header = framed(name, b"");
    match key.open_bytes(&cipher) {
        Some(plain) if plain.starts_with(&header) => Ok(Some(plain[header.len()..].to_vec())),
        _ => Err(SyncError::Server(format!(
            "the server's copy of profile '{name}' does not open with this sync key; push it again"
        ))),
    }
}

/// Remove profile `name` from the server. `false`: there was none.
pub fn remove(ctx: &Context<'_>, name: &str) -> Result<bool, SyncError> {
    let key = require_key(ctx)?;
    let object = object_name(&key, name);
    for _ in 0..3 {
        let (row, mut values) = index(ctx, &key)?;
        if values.remove(&object).is_none() {
            return Ok(false);
        }
        ctx.objects.delete(&object)?;
        if save_index(ctx, &key, row.as_ref(), values)? {
            return Ok(true);
        }
    }
    Err(SyncError::Busy)
}

/// Delete every profile file this account has (`sync reset`). The server lists them, so this
/// works without the sync key too, and leaves no orphaned files to count against its limit.
pub fn delete_all_objects(ctx: &Context<'_>) -> Result<(), SyncError> {
    for name in ctx.objects.list()? {
        ctx.objects.delete(&name)?;
    }
    Ok(())
}

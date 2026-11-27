//! Usage that syncs (`docs/sso-platform-plan.md`, task 12): one `usage` row per (machine, month),
//! key `row_key("usage:<machineId>:<yyyy-mm>")`, body `packages/contracts/haryx/usage-sync.json`.
//!
//! Each machine writes only its own rows, so there is nothing to merge: the row is rebuilt from
//! this machine's usage history and replaced when it changed. Other machines and the web read
//! every machine's rows. The plaintext carries day, account email, model, token counts and the
//! 5h/7d readings, never a project path, a session id or conversation text.
//!
//! The `usage` kind also holds the usage-share rows (`crate::usage_share`), keyed per account.
//! The two never meet: each side only reads the keys it derives itself.

use std::collections::BTreeMap;

use jiff::tz::TimeZone;
use jiff::{Timestamp, ToSpan};
use log::debug;
use serde_json::Value;

use super::crypto::SyncKey;
use super::remote::Row;
use super::{Context, KIND_USAGE, Opened, SyncError, open, sources, write_row};
use crate::contracts::{UsageDay, UsageQuota, UsageSync};
use crate::usage_history::{Tokens, history_dir, quota, report};

pub const SCHEMA: u32 = 1;
/// Months of this machine's rows kept on the server, the current one included.
pub const KEEP_MONTHS: i32 = 13;
/// Plaintext budget of one row. Sealed and base64-encoded it stays under the server's 256 KiB.
const MAX_PLAIN: usize = 160_000;
/// The finest spacing of the 5h/7d readings kept in a row; widened until the row fits.
const QUOTA_STEP_S: i64 = 3_600;

/// The label the row key is derived from.
pub fn label(machine_id: &str, month: &str) -> String {
    format!("usage:{machine_id}:{month}")
}

/// `yyyy-mm` of `at` in `tz`.
fn month_of(at: i64, tz: &TimeZone) -> String {
    zoned(at, tz).strftime("%Y-%m").to_string()
}

fn day_of(at: i64, tz: &TimeZone) -> String {
    zoned(at, tz).strftime("%Y-%m-%d").to_string()
}

fn zoned(at: i64, tz: &TimeZone) -> jiff::Zoned {
    Timestamp::from_second(at)
        .unwrap_or_default()
        .to_zoned(tz.clone())
}

/// Unix seconds of the first instant of `at`'s month, `back` months earlier.
fn month_start(at: i64, back: i32, tz: &TimeZone) -> i64 {
    let first = zoned(at, tz)
        .first_of_month()
        .and_then(|z| z.start_of_day())
        .and_then(|z| z.checked_sub(back.months()));
    first.map_or(0, |z| z.timestamp().as_second())
}

/// Account number → email, from this machine's roster.
fn emails(ctx: &Context<'_>) -> Result<BTreeMap<i64, String>, SyncError> {
    let (accounts, _) = sources::read_accounts(ctx.switcher)?;
    Ok(accounts
        .into_iter()
        .filter_map(|a| Some((a.num.parse().ok()?, a.email)))
        .collect())
}

/// This machine's usage in `month`, or `None` when it has none.
pub fn summary(
    rows: &[report::Row],
    readings: &[quota::Reading],
    emails: &BTreeMap<i64, String>,
    machine_id: &str,
    month: &str,
    now: i64,
    tz: &TimeZone,
) -> Option<UsageSync> {
    let email = |n: Option<i64>| n.and_then(|n| emails.get(&n).cloned());
    let mut days: BTreeMap<(String, Option<String>, String), (u64, Tokens)> = BTreeMap::new();
    for row in rows.iter().filter(|r| month_of(r.hour, tz) == month) {
        let at = (day_of(row.hour, tz), email(row.account), row.model.clone());
        let entry = days.entry(at).or_default();
        entry.0 += row.replies;
        entry.1.add(&row.tokens);
    }
    let days: Vec<UsageDay> = days
        .into_iter()
        .map(|((day, account, model), (replies, t))| UsageDay {
            day,
            account,
            model,
            replies,
            input: t.input,
            output: t.output,
            cache_read: t.cache_read,
            cache_write5m: t.cache_write_5m,
            cache_write1h: t.cache_write_1h,
        })
        .collect();
    let readings: Vec<&quota::Reading> = readings
        .iter()
        .filter(|r| month_of(r.at, tz) == month)
        .collect();
    if days.is_empty() && readings.is_empty() {
        return None;
    }
    let mut out = UsageSync {
        schema: SCHEMA,
        machine_id: machine_id.to_owned(),
        month: month.to_owned(),
        updated_at: Timestamp::from_second(now)
            .unwrap_or_default()
            .strftime("%Y-%m-%dT%H:%M:%SZ")
            .to_string(),
        days,
        quota: Vec::new(),
    };
    // Thin the readings until the row fits; the token counts always stay.
    let mut step = QUOTA_STEP_S;
    loop {
        out.quota = thinned(&readings, step, &email);
        if size(&out) <= MAX_PLAIN || out.quota.is_empty() {
            break;
        }
        step *= 2;
        if step > 31 * 86_400 {
            out.quota.clear();
        }
    }
    Some(out)
}

/// The last reading of each account in each `step`-second slot, oldest first.
fn thinned(
    readings: &[&quota::Reading],
    step: i64,
    email: &dyn Fn(Option<i64>) -> Option<String>,
) -> Vec<UsageQuota> {
    let mut last: BTreeMap<(i64, i64), &quota::Reading> = BTreeMap::new();
    for r in readings {
        last.insert((r.at.div_euclid(step), r.account), r);
    }
    let mut out: Vec<UsageQuota> = last
        .into_values()
        .map(|r| UsageQuota {
            at: r.at,
            account: email(Some(r.account)),
            five_hour_pct: r.five_hour,
            seven_day_pct: r.seven_day,
        })
        .collect();
    out.sort_by_key(|q| q.at);
    out
}

fn size(body: &UsageSync) -> usize {
    serde_json::to_vec(body).map_or(usize::MAX, |v| v.len())
}

fn to_values(body: &UsageSync) -> Result<super::Values, SyncError> {
    match serde_json::to_value(body) {
        Ok(Value::Object(map)) => Ok(map.into_iter().collect()),
        _ => Err(SyncError::Local("usage row did not serialize".into())),
    }
}

/// A usage-sync body, when `doc` holds one in the shape this version reads.
fn parse(values: &super::Values) -> Option<UsageSync> {
    let map: serde_json::Map<String, Value> = values.clone().into_iter().collect();
    serde_json::from_value::<UsageSync>(Value::Object(map))
        .ok()
        .filter(|u| u.schema == SCHEMA)
}

/// Every machine's usage rows that open with `key` and sit under their own label, by machine
/// and month. A row moved under another machine's key does not open as that machine's.
pub fn read_all(key: &SyncKey, rows: &[Row]) -> Vec<UsageSync> {
    let mut out: Vec<UsageSync> = rows
        .iter()
        .filter(|r| r.kind == KIND_USAGE)
        .filter_map(|row| {
            let Opened::Doc(doc) = open(key, row) else {
                return None;
            };
            let body = parse(&doc.values)?;
            (row.key == key.row_key(&label(&body.machine_id, &body.month))).then_some(body)
        })
        .collect();
    out.sort_by(|a, b| (&a.month, &a.machine_id).cmp(&(&b.month, &b.machine_id)));
    out
}

/// Write this machine's rows for this month and the last one (a transcript read late still lands
/// in its own month), when they changed, and drop its rows older than [`KEEP_MONTHS`]. Returns
/// how many rows it wrote. Best effort past the first error: usage never fails a sync.
pub(crate) fn push(ctx: &Context<'_>, key: &SyncKey, rows: &[Row], tz: &TimeZone) -> usize {
    match push_inner(ctx, key, rows, tz) {
        Ok(n) => n,
        Err(err) => {
            debug!("usage sync skipped: {err}");
            0
        }
    }
}

fn push_inner(
    ctx: &Context<'_>,
    key: &SyncKey,
    rows: &[Row],
    tz: &TimeZone,
) -> Result<usize, SyncError> {
    let machine = crate::machine::machine_id(ctx.root);
    let mine: BTreeMap<String, (Row, Option<UsageSync>)> = rows
        .iter()
        .filter(|r| r.kind == KIND_USAGE)
        .filter_map(|row| match open(key, row) {
            Opened::Doc(doc) => {
                let body = parse(&doc.values)?;
                (body.machine_id == machine).then(|| (row.key.clone(), (row.clone(), Some(body))))
            }
            _ => None,
        })
        .collect();

    let emails = emails(ctx)?;
    let since = month_start(ctx.now, 1, tz);
    let history = report::rows(ctx.root, since);
    let readings = quota::read(&history_dir(ctx.root));
    let mut wrote = 0;
    for back in [1, 0] {
        let month = month_of(month_start(ctx.now, back, tz), tz);
        let Some(body) = summary(&history, &readings, &emails, &machine, &month, ctx.now, tz)
        else {
            continue;
        };
        let row_key = key.row_key(&label(&machine, &month));
        let current = mine.get(&row_key);
        let same = current
            .and_then(|(_, old)| old.as_ref())
            .is_some_and(|old| old.days == body.days && old.quota == body.quota);
        if same {
            continue;
        }
        // A row at this key that did not open as ours is replaced, as any other kind's.
        let version = current.map(|(row, _)| row.version).or_else(|| {
            rows.iter()
                .find(|r| r.kind == KIND_USAGE && r.key == row_key)
                .map(|r| r.version)
        });
        if write_row(
            ctx,
            key,
            KIND_USAGE,
            &row_key,
            to_values(&body)?,
            ctx.now,
            version,
        )?
        .is_some()
        {
            wrote += 1;
        }
    }

    let oldest = month_of(month_start(ctx.now, KEEP_MONTHS - 1, tz), tz);
    for (row_key, (_, body)) in &mine {
        if body.as_ref().is_some_and(|b| b.month < oldest) {
            ctx.remote.delete(KIND_USAGE, row_key)?;
        }
    }
    Ok(wrote)
}

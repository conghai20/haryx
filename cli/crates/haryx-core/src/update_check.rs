//! Passive "a newer version is available" notice. Port of `claude_swap/update_check.py` (the
//! check half) and `claude_swap/cache.py`.
//!
//! The lookup result is cached in `<store root>/cache/update_check.json` (`{"timestamp", "data"}`):
//! a successful lookup for a day ([`CACHE_TTL_S`]), a failed one (cached as `null`) for an hour
//! ([`FAILURE_TTL_S`]), so a dead network costs one attempt per hour rather than one per command.
//! `updates.check = false` in settings turns the check off entirely: no request at all.
//!
//! The lookup asks GitHub's latest-release API ([`RELEASES_URL`]) and caches the release
//! [`trim`]med to what [`crate::release::Release`] reads, so `haryx upgrade --check` and the apps
//! get notes and assets from the same cached answer. Older caches holding a bare version string
//! still read. A version is also accepted as `version` or PyPI's `info.version`.
//!
//! [`latest_release`] is the explicit check (`haryx upgrade --check`): it ignores
//! `updates.check`, since the user asked.

use std::cmp::Ordering;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{Value, json};

use crate::fsutil::{WriteOptions, atomic_write_text};
use crate::release::{Release, trim};
use crate::settings::load_update_settings;
use crate::usage_store::system_clock;
use crate::{PROG, VERSION};

/// Where the latest release is looked up. GitHub never answers a draft or a pre-release here.
pub const RELEASES_URL: Option<&str> =
    Some("https://api.github.com/repos/conghai20/haryx/releases/latest");
pub const CACHE_FILENAME: &str = "update_check.json";
/// A successful lookup is kept for a day.
pub const CACHE_TTL_S: f64 = 24.0 * 3600.0;
/// A failed one is retried after an hour.
pub const FAILURE_TTL_S: f64 = 3600.0;
/// Bound on the lookup request.
pub const FETCH_TIMEOUT: Duration = Duration::from_secs(2);

// ------------------------------------------------------------------------------------------ cache

/// `<store root>/cache/update_check.json`.
pub fn cache_path(store_root: &Path) -> PathBuf {
    store_root.join("cache").join(CACHE_FILENAME)
}

/// A cache read: nothing usable, or the stored `data` (which may itself be `null`).
#[derive(Clone, Debug, PartialEq)]
pub enum Cached {
    Missing,
    Hit(Value),
}

/// The stored `data` if the file exists, parses, and its `timestamp` is within `ttl` of `now`.
pub fn read_cache(path: &Path, ttl: f64, now: f64) -> Cached {
    let raw = std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok());
    let Some(Value::Object(mut raw)) = raw else {
        return Cached::Missing;
    };
    let Some(timestamp) = raw.get("timestamp").and_then(crate::oauth::py_num) else {
        return Cached::Missing;
    };
    match raw.remove("data") {
        Some(data) if now - timestamp < ttl => Cached::Hit(data),
        _ => Cached::Missing,
    }
}

/// Write `data` with a `timestamp` of `now`, creating parent directories.
pub fn write_cache(path: &Path, data: &Value, now: f64) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let text = json!({"timestamp": now, "data": data}).to_string();
    atomic_write_text(path, &text, WriteOptions::default())
}

// --------------------------------------------------------------------------------------- versions

/// A version as a sort key: the release first, then the pre-release rank and number, which puts
/// `0.27.0b1` below `0.27.0` and both below `0.28.0`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version {
    release: Vec<u64>,
    /// [`FINAL_RANK`] when this is not a pre-release.
    pre_rank: u8,
    pre_number: u64,
}

const FINAL_RANK: u8 = 3;
/// Pre-release spellings in regex alternation order (longer before their prefixes).
const PRE_TAGS: &[(&str, u8)] = &[
    ("alpha", 0),
    ("beta", 1),
    ("preview", 2),
    ("pre", 2),
    ("rc", 2),
    ("a", 0),
    ("b", 1),
    ("c", 2),
];

fn take_digits(s: &str) -> (&str, &str) {
    let end = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    s.split_at(end)
}

fn strip_sep(s: &str) -> &str {
    s.strip_prefix(['-', '_', '.']).unwrap_or(s)
}

/// Parse a release number with an optional PEP 440 pre-release suffix, matched at the start of
/// `v` like upstream's regex: `(\d+(?:\.\d+)*)(?:[-_.]?(alpha|beta|preview|pre|rc|a|b|c)[-_.]?(\d+)?)?`
/// (case-insensitive). Development and post releases collapse onto their release. `None` when `v`
/// does not start with a number.
pub fn parse_version(v: &str) -> Option<Version> {
    let (first, mut rest) = take_digits(v);
    if first.is_empty() {
        return None;
    }
    let mut release = vec![first.parse::<u64>().ok()?];
    while let Some((part, tail)) = rest
        .strip_prefix('.')
        .map(take_digits)
        .filter(|(part, _)| !part.is_empty())
    {
        release.push(part.parse().ok()?);
        rest = tail;
    }
    // 0.27 and 0.27.0 are the same release.
    while release.len() > 1 && release.last() == Some(&0) {
        release.pop();
    }
    let after_sep = strip_sep(rest).to_ascii_lowercase();
    let Some((tag, rank)) = PRE_TAGS.iter().find(|(tag, _)| after_sep.starts_with(tag)) else {
        return Some(Version {
            release,
            pre_rank: FINAL_RANK,
            pre_number: 0,
        });
    };
    let (digits, _) = take_digits(strip_sep(&after_sep[tag.len()..]));
    Some(Version {
        release,
        pre_rank: *rank,
        pre_number: if digits.is_empty() {
            0
        } else {
            digits.parse().ok()?
        },
    })
}

/// Whether `latest` is an upgrade worth telling the user about; `None` when either version is
/// unparseable. Nobody on a final release asked to be moved onto a pre-release; people already
/// running one still hear about later pre-releases.
pub fn is_newer(latest: &str, current: &str) -> Option<bool> {
    let latest = parse_version(latest)?;
    let current = parse_version(current)?;
    if latest.pre_rank != FINAL_RANK && current.pre_rank == FINAL_RANK {
        return Some(false);
    }
    Some(latest.cmp(&current) == Ordering::Greater)
}

// ------------------------------------------------------------------------------------------ check

/// The notice for a newer release, or `None`. Never fails: any problem means no notice.
///
/// Off when `updates.check` is false. A no-op while [`RELEASES_URL`] is `None`. The caller skips it
/// for `--json`, `purge` and `upgrade` runs, as upstream does.
pub fn check_for_update(store_root: &Path) -> Option<String> {
    if !load_update_settings(store_root).check {
        return None;
    }
    let url = RELEASES_URL?;
    check_with(&cache_path(store_root), VERSION, system_clock(), || {
        fetch_latest_release(url, FETCH_TIMEOUT)
            .map_err(|e| log::debug!("update lookup failed: {e}"))
            .ok()
    })
}

/// The latest release, from the cache unless `refresh`. `Err` names why a lookup failed; a
/// cached failure (within the hour) reads as `Ok(None)`. Not gated on `updates.check`.
pub fn latest_release(store_root: &Path, refresh: bool) -> Result<Option<Release>, String> {
    let url = RELEASES_URL.ok_or("this build has no release channel")?;
    let path = cache_path(store_root);
    let now = system_clock();
    let lookup = |refresh: bool| {
        let mut error = None;
        let data = cached_lookup(&path, now, refresh, || {
            fetch_latest_release(url, FETCH_TIMEOUT)
                .map_err(|e| error = Some(e))
                .ok()
        });
        (data, error)
    };
    let (mut data, mut error) = lookup(refresh);
    // An older cache holds only the version string: notes and assets need the release itself.
    if !refresh && data.as_ref().is_some_and(Value::is_string) {
        (data, error) = lookup(true);
    }
    match (data, error) {
        (_, Some(err)) => Err(err),
        (Some(data), None) => Ok(Release::from_json(&data)),
        (None, None) => Err(format!(
            "the last lookup failed less than an hour ago; try `{PROG} upgrade --check --refresh`"
        )),
    }
}

/// Whether a cached answer names a release: a bare version (older caches) or a release object.
fn usable(data: &Value) -> bool {
    data.is_string() || (data.is_object() && latest_version_of(data).is_some())
}

/// The cached lookup answer, looking up again when it is missing, stale or `refresh`. A failed
/// lookup is cached as `null` for [`FAILURE_TTL_S`], unless a good answer is already cached: that
/// one is kept (whatever its age) rather than replaced by a failure.
pub fn cached_lookup(
    cache_path: &Path,
    now: f64,
    refresh: bool,
    lookup: impl FnOnce() -> Option<Value>,
) -> Option<Value> {
    let mut cached = if refresh {
        Cached::Missing
    } else {
        read_cache(cache_path, CACHE_TTL_S, now)
    };
    if cached == Cached::Hit(Value::Null) {
        // A cached failure. Keep it for an hour, not a day.
        cached = read_cache(cache_path, FAILURE_TTL_S, now);
    }
    match cached {
        Cached::Hit(Value::Null) => return None,
        Cached::Hit(data) if usable(&data) => return Some(data),
        // Missing, stale, refreshed, or a cache in an unknown shape: look it up again.
        _ => {}
    }
    let latest = lookup();
    let keep_old = latest.is_none()
        && matches!(read_cache(cache_path, f64::INFINITY, now), Cached::Hit(ref old) if usable(old));
    if !keep_old {
        let data = latest.clone().unwrap_or(Value::Null);
        if let Err(err) = write_cache(cache_path, &data, now) {
            log::debug!("update check cache not written: {err}");
        }
    }
    latest
}

/// [`check_for_update`] with an explicit cache file, version, clock and lookup.
pub fn check_with(
    cache_path: &Path,
    current_version: &str,
    now: f64,
    lookup: impl FnOnce() -> Option<Value>,
) -> Option<String> {
    let data = cached_lookup(cache_path, now, false, lookup)?;
    let latest = match data {
        Value::String(v) => v,
        other => latest_version_of(&other)?,
    };
    if latest.is_empty() {
        return None;
    }
    is_newer(&latest, current_version)?.then(|| {
        format!(
            "A newer version of {PROG} is available ({latest}). You are using \
             {current_version}. Run `{PROG} upgrade` to update."
        )
    })
}

/// The latest release at `url`, [`trim`]med. `Err` says why not.
pub fn fetch_latest_release(url: &str, timeout: Duration) -> Result<Value, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .tls_config(
            ureq::tls::TlsConfig::builder()
                .root_certs(ureq::tls::RootCerts::PlatformVerifier)
                .build(),
        )
        .build()
        .into();
    let mut resp = agent
        .get(url)
        .header("User-Agent", &format!("{PROG}/{VERSION}"))
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| format!("could not reach {url}: {e}"))?;
    let data: Value = resp
        .body_mut()
        .read_json()
        .map_err(|e| format!("unexpected answer from {url}: {e}"))?;
    if latest_version_of(&data).is_none() {
        return Err(format!("{url} named no release"));
    }
    Ok(trim(&data))
}

/// The release a lookup response names: `tag_name` (a leading `v` stripped), `version`, or
/// `info.version`.
pub fn latest_version_of(data: &Value) -> Option<String> {
    if let Some(tag) = data.get("tag_name").and_then(Value::as_str) {
        return Some(tag.strip_prefix('v').unwrap_or(tag).to_owned());
    }
    data.get("version")
        .or_else(|| data.get("info").and_then(|i| i.get("version")))
        .and_then(Value::as_str)
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use tempfile::TempDir;

    const NOW: f64 = 1_000_000.0;

    fn cache(tmp: &TempDir) -> PathBuf {
        tmp.path().join("cache.json")
    }

    fn write_raw(path: &Path, data: Value, timestamp: f64) {
        write_cache(path, &data, timestamp).unwrap();
    }

    /// `check_with` with a lookup answering `answer`; returns (notice, lookup calls).
    fn run(path: &Path, current: &str, answer: Option<&str>) -> (Option<String>, u32) {
        let calls = Cell::new(0);
        let msg = check_with(path, current, NOW, || {
            calls.set(calls.get() + 1);
            answer.map(|v| json!(v))
        });
        (msg, calls.get())
    }

    // --- cache (test_cache.py) ---

    #[test]
    fn cache_round_trip_ttl_and_missing() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("sub").join("dir").join("test.json");
        assert_eq!(read_cache(&path, 60.0, NOW), Cached::Missing);
        let data = json!({"accounts": [1, 2, 3], "nested": {"a": true}});
        write_cache(&path, &data, NOW).unwrap();
        let raw: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(raw["data"], data);
        assert_eq!(raw["timestamp"], json!(NOW));
        assert_eq!(read_cache(&path, 60.0, NOW + 10.0), Cached::Hit(data));
        assert_eq!(read_cache(&path, 60.0, NOW + 100.0), Cached::Missing);
    }

    #[test]
    fn cache_corrupt_and_null() {
        let tmp = TempDir::new().unwrap();
        let path = cache(&tmp);
        std::fs::write(&path, "not valid json{{{").unwrap();
        assert_eq!(read_cache(&path, 60.0, NOW), Cached::Missing);
        write_raw(&path, Value::Null, NOW);
        assert_eq!(read_cache(&path, 60.0, NOW), Cached::Hit(Value::Null));
    }

    // --- check ---

    #[test]
    fn newer_version_available() {
        let tmp = TempDir::new().unwrap();
        let (msg, calls) = run(&cache(&tmp), "0.3.2", Some("0.4.0"));
        let msg = msg.unwrap();
        assert!(msg.contains("0.4.0") && msg.contains("0.3.2"));
        assert!(msg.contains("haryx upgrade` to update"));
        assert_eq!(calls, 1);
    }

    #[test]
    fn already_on_latest() {
        let tmp = TempDir::new().unwrap();
        assert_eq!(run(&cache(&tmp), "0.3.2", Some("0.3.2")).0, None);
    }

    #[test]
    fn failed_lookup_returns_none_and_is_cached() {
        let tmp = TempDir::new().unwrap();
        let path = cache(&tmp);
        assert_eq!(run(&path, "0.3.2", None), (None, 1));
        assert_eq!(read_cache(&path, 60.0, NOW), Cached::Hit(Value::Null));
        // A fresh cached failure skips the lookup.
        assert_eq!(run(&path, "0.3.2", Some("0.4.0")), (None, 0));
    }

    #[test]
    fn failed_lookup_is_retried_after_an_hour() {
        let tmp = TempDir::new().unwrap();
        let path = cache(&tmp);
        write_raw(&path, Value::Null, NOW - FAILURE_TTL_S - 1.0);
        let (msg, calls) = run(&path, "0.3.2", Some("0.4.0"));
        assert_eq!(calls, 1);
        assert!(msg.unwrap().contains("0.4.0"));
    }

    #[test]
    fn successful_lookup_outlives_the_failure_backoff() {
        let tmp = TempDir::new().unwrap();
        let path = cache(&tmp);
        write_raw(&path, json!("0.4.0"), NOW - FAILURE_TTL_S - 1.0);
        let (msg, calls) = run(&path, "0.3.2", None);
        assert_eq!(calls, 0);
        assert!(msg.unwrap().contains("0.4.0"));
    }

    #[test]
    fn stale_cache_is_looked_up_again() {
        let tmp = TempDir::new().unwrap();
        let path = cache(&tmp);
        write_raw(&path, json!("0.3.0"), NOW - CACHE_TTL_S - 1.0);
        let (msg, calls) = run(&path, "0.3.2", Some("0.4.0"));
        assert_eq!(calls, 1);
        assert!(msg.unwrap().contains("0.4.0"));
    }

    #[test]
    fn cache_in_an_unknown_shape_is_refetched() {
        let tmp = TempDir::new().unwrap();
        let path = cache(&tmp);
        write_raw(&path, json!({"latest": "9.9.9"}), NOW);
        assert_eq!(run(&path, "0.3.2", Some("0.3.2")), (None, 1));
    }

    #[test]
    fn prereleases() {
        let tmp = TempDir::new().unwrap();
        let p = |n: &str| tmp.path().join(n);
        let msg = run(&p("a"), "0.27.0b1", Some("0.28.0")).0.unwrap();
        assert!(msg.contains("0.28.0") && msg.contains("0.27.0b1"));
        assert!(run(&p("b"), "0.27.0b1", Some("0.27.0")).0.is_some());
        assert!(
            run(&p("c"), "0.27.0b2", Some("0.27.0rc1"))
                .0
                .unwrap()
                .contains("0.27.0rc1")
        );
        assert_eq!(run(&p("d"), "0.27.0", Some("0.28.0b1")).0, None);
        assert_eq!(run(&p("e"), "main", Some("0.28.0")).0, None);
    }

    #[test]
    fn a_cached_release_object_gives_the_notice() {
        let tmp = TempDir::new().unwrap();
        let path = cache(&tmp);
        write_raw(&path, json!({"tag_name": "v0.4.0", "assets": []}), NOW);
        let (msg, calls) = run(&path, "0.3.2", None);
        assert_eq!(calls, 0);
        assert!(msg.unwrap().contains("0.4.0"));
    }

    #[test]
    fn refresh_skips_a_fresh_cache() {
        let tmp = TempDir::new().unwrap();
        let path = cache(&tmp);
        write_raw(&path, json!({"tag_name": "v0.3.0"}), NOW);
        let calls = Cell::new(0);
        let data = cached_lookup(&path, NOW, true, || {
            calls.set(calls.get() + 1);
            Some(json!({"tag_name": "v0.4.0"}))
        });
        assert_eq!(calls.get(), 1);
        assert_eq!(data.unwrap()["tag_name"], "v0.4.0");
        assert_eq!(
            read_cache(&path, 60.0, NOW),
            Cached::Hit(json!({"tag_name": "v0.4.0"}))
        );
    }

    #[test]
    fn a_failed_refresh_keeps_a_good_cached_release() {
        let tmp = TempDir::new().unwrap();
        let path = cache(&tmp);
        write_raw(&path, json!({"tag_name": "v0.4.0"}), NOW - 10.0);
        assert_eq!(cached_lookup(&path, NOW, true, || None), None);
        assert_eq!(
            read_cache(&path, CACHE_TTL_S, NOW),
            Cached::Hit(json!({"tag_name": "v0.4.0"}))
        );
        // The good answer is still used by the next plain check.
        assert!(cached_lookup(&path, NOW, false, || panic!("no lookup")).is_some());
    }

    #[test]
    fn releases_url_is_the_github_latest_release() {
        assert_eq!(
            RELEASES_URL,
            Some("https://api.github.com/repos/conghai20/haryx/releases/latest")
        );
    }

    #[test]
    fn version_parsing() {
        let v = |s| parse_version(s).unwrap();
        assert_eq!(v("0.27"), v("0.27.0"));
        assert!(v("0.27.0b1") < v("0.27.0"));
        assert!(v("0.27.0") < v("0.28.0"));
        assert!(v("1.0.0a1") < v("1.0.0b1"));
        assert!(v("1.0.0b1") < v("1.0.0rc1"));
        assert_eq!(v("1.0-RC.2"), v("1.0rc2"));
        assert_eq!(v("1.0.0preview1"), v("1.0.0c1"));
        assert_eq!(v("1.0.0.dev3"), v("1.0.0"));
        assert_eq!(v("1.2.0b1"), v("1.2b1"));
        assert!(parse_version("main").is_none());
        assert!(parse_version("").is_none());
    }

    #[test]
    fn disabled_when_opted_out() {
        let tmp = TempDir::new().unwrap();
        crate::settings::set_setting(tmp.path(), "updates.check", "false").unwrap();
        assert!(!load_update_settings(tmp.path()).check);
        assert_eq!(check_for_update(tmp.path()), None);
        assert!(!cache_path(tmp.path()).exists(), "no lookup at all");
    }

    #[test]
    fn response_shapes() {
        let tag = json!({"tag_name": "v0.4.0"});
        assert_eq!(latest_version_of(&tag).as_deref(), Some("0.4.0"));
        assert_eq!(
            latest_version_of(&json!({"version": "1.0"})).as_deref(),
            Some("1.0")
        );
        let pypi = json!({"info": {"version": "2.0"}});
        assert_eq!(latest_version_of(&pypi).as_deref(), Some("2.0"));
        assert_eq!(latest_version_of(&json!([])), None);
    }

    fn serve_once(status: u16, body: &str) -> String {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let port = server.server_addr().to_ip().unwrap().port();
        let body = body.to_owned();
        std::thread::spawn(move || {
            if let Ok(req) = server.recv() {
                let resp = tiny_http::Response::from_string(body).with_status_code(status);
                let _ = req.respond(resp);
            }
        });
        format!("http://127.0.0.1:{port}/releases/latest")
    }

    #[test]
    fn fetch_against_a_local_stub() {
        let url = serve_once(
            200,
            r#"{"tag_name": "v9.0.0", "body": "notes", "node_id": "x", "assets": []}"#,
        );
        let data = fetch_latest_release(&url, FETCH_TIMEOUT).unwrap();
        assert_eq!(latest_version_of(&data).as_deref(), Some("9.0.0"));
        assert!(data.get("node_id").is_none(), "trimmed");
        let url = serve_once(500, "{}");
        assert!(fetch_latest_release(&url, FETCH_TIMEOUT).is_err());
        let url = serve_once(200, "not json");
        assert!(fetch_latest_release(&url, FETCH_TIMEOUT).is_err());
        let url = serve_once(200, r#"{"message": "Not Found"}"#);
        assert!(
            fetch_latest_release(&url, FETCH_TIMEOUT)
                .unwrap_err()
                .contains("named no release")
        );
    }

    #[test]
    fn end_to_end_through_the_stub() {
        let tmp = TempDir::new().unwrap();
        let url = serve_once(200, r#"{"tag_name": "v99.0.0"}"#);
        let msg = check_with(&cache(&tmp), VERSION, NOW, || {
            fetch_latest_release(&url, FETCH_TIMEOUT).ok()
        });
        assert!(msg.unwrap().contains("99.0.0"));
    }
}

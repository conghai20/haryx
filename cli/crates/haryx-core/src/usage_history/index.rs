//! Read new transcript lines into day files. Each run reads only what was appended since the last
//! (`index.json` keeps a byte offset per file); a file that shrank or was replaced is read again.

use std::collections::{BTreeMap, HashSet};
use std::io::{BufRead, BufReader, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{Map, Value, json};

use super::attribution::Attribution;
use super::transcript::parse_line;
use super::{DAYS, INDEX, Tokens, history_dir};
use crate::locking::FileLock;
use crate::settings::load_history_settings;

/// Request ids remembered per file, so a reply split across two runs is counted once.
const RECENT_IDS: usize = 64;
const LOCK_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_DEPTH: usize = 5;

/// A `projects/` folder and, for a `haryx run` profile with its own history, its account.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Source {
    pub projects: PathBuf,
    pub account: Option<i64>,
}

/// The default profile's `projects/`, then every session profile's that is not the same folder.
pub fn sources(store_root: &Path, default_profile: &Path) -> Vec<Source> {
    let mut out = vec![Source {
        projects: default_profile.join("projects"),
        account: None,
    }];
    let mut seen: HashSet<PathBuf> = out
        .iter()
        .filter_map(|s| std::fs::canonicalize(&s.projects).ok())
        .collect();
    let Ok(profiles) = std::fs::read_dir(store_root.join("sessions")) else {
        return out;
    };
    for profile in profiles.flatten() {
        let projects = profile.path().join("projects");
        let Ok(real) = std::fs::canonicalize(&projects) else {
            continue;
        };
        if !seen.insert(real) {
            continue; // shares the default history
        }
        let name = profile.file_name().to_string_lossy().into_owned();
        let account = name.split('-').next().and_then(|n| n.parse().ok());
        out.push(Source { projects, account });
    }
    out
}

/// What an index run did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    pub disabled: bool,
    /// `history.import` is still `ask`: nothing was read.
    pub needs_choice: bool,
    pub files: usize,
    pub replies: u64,
}

/// One aggregated row's key.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Key {
    day: String,
    hour: i64,
    account: Option<i64>,
    approximate: bool,
    project: String,
    session: String,
    model: String,
}

/// Read new lines from every transcript. `rebuild` forgets everything read so far first.
pub fn run(store_root: &Path, default_profile: &Path, rebuild: bool) -> Result<Outcome, String> {
    let settings = load_history_settings(store_root);
    if !settings.enabled {
        return Ok(Outcome {
            disabled: true,
            ..Outcome::default()
        });
    }
    let dir = history_dir(store_root);
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let _lock = FileLock::lock(dir.join(".index.lock"), LOCK_TIMEOUT).map_err(|e| e.to_string())?;
    if rebuild {
        let _ = std::fs::remove_dir_all(dir.join(DAYS));
        let _ = std::fs::remove_file(dir.join(INDEX));
    }
    let mut state = load_state(&dir);
    let first_run = !state
        .get("started")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let sources = sources(store_root, default_profile);
    if first_run {
        match settings.import.as_str() {
            "ask" => {
                return Ok(Outcome {
                    needs_choice: true,
                    ..Outcome::default()
                });
            }
            "none" => {
                skip_existing(&sources, &mut state);
                state.insert("started".into(), json!(true));
                save_state(&dir, &state)?;
                return Ok(Outcome::default());
            }
            _ => {}
        }
    }
    let attribution = Attribution::load(store_root);
    let mut rows: BTreeMap<Key, (u64, Tokens)> = BTreeMap::new();
    let mut outcome = Outcome::default();
    let files = state.entry("files").or_insert_with(|| json!({}));
    let files = files
        .as_object_mut()
        .ok_or("index.json is damaged; run with --rebuild")?;
    for source in &sources {
        for path in transcripts(&source.projects) {
            let read = read_file(&path, files, |reply| {
                let who = attribution.account(&reply.session, reply.at, source.account);
                let at = jiff::Timestamp::from_second(reply.at).unwrap_or_default();
                let key = Key {
                    day: at.strftime("%Y-%m-%d").to_string(),
                    hour: reply.at - reply.at.rem_euclid(3600),
                    account: who.account,
                    approximate: who.approximate,
                    project: reply.cwd.clone(),
                    session: reply.session.clone(),
                    model: reply.model.clone(),
                };
                let entry = rows.entry(key).or_default();
                entry.0 += 1;
                entry.1.add(&reply.tokens);
            });
            if read > 0 {
                outcome.files += 1;
                outcome.replies += read;
            }
        }
    }
    // Offsets first: a crash between the two loses this run's rows rather than counting them twice
    // on the next one.
    state.insert("started".into(), json!(true));
    save_state(&dir, &state)?;
    write_rows(&dir, &rows)?;
    if settings.keep_days > 0 {
        prune(&dir, settings.keep_days);
    }
    Ok(outcome)
}

fn load_state(dir: &Path) -> Map<String, Value> {
    std::fs::read_to_string(dir.join(INDEX))
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default()
}

fn save_state(dir: &Path, state: &Map<String, Value>) -> Result<(), String> {
    crate::settings::atomic_write_json(&dir.join(INDEX), &Value::Object(state.clone()))
        .map_err(|e| e.to_string())
}

/// "From now on": mark every transcript as read to its current end.
fn skip_existing(sources: &[Source], state: &mut Map<String, Value>) {
    let files = state.entry("files").or_insert_with(|| json!({}));
    let Some(files) = files.as_object_mut() else {
        return;
    };
    for source in sources {
        for path in transcripts(&source.projects) {
            if let Ok(meta) = std::fs::metadata(&path) {
                files.insert(
                    file_key(&path),
                    json!({"offset": meta.len(), "head": head_of(&path), "recent": []}),
                );
            }
        }
    }
}

/// Every `*.jsonl` under `projects/` (subagent transcripts included), a few levels deep.
pub fn transcripts(projects: &Path) -> Vec<PathBuf> {
    fn walk(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() && depth < MAX_DEPTH {
                walk(&path, depth + 1, out);
            } else if kind.is_file() && path.extension().is_some_and(|e| e == "jsonl") {
                out.push(path);
            }
        }
    }
    let mut out = Vec::new();
    walk(projects, 0, &mut out);
    out.sort();
    out
}

/// A file's key in `index.json`: rebuilt from its components so `a/b` and `a\\b` (both legal on
/// Windows) name the same file.
fn file_key(path: &Path) -> String {
    path.components().collect::<PathBuf>().display().to_string()
}

/// The first bytes of a file, to tell a replaced transcript from one that only grew.
const HEAD_BYTES: usize = 256;

fn head_of(path: &Path) -> String {
    use std::io::Read;
    let mut buf = vec![0u8; HEAD_BYTES];
    let n = std::fs::File::open(path)
        .and_then(|mut f| f.read(&mut buf))
        .unwrap_or(0);
    String::from_utf8_lossy(&buf[..n]).into_owned()
}

/// Mark `path` as read to its current end, so its lines are never counted (a conversation copied
/// into another profile by `ai sessions continue`; its replies were counted where they were made).
pub fn mark_read(store_root: &Path, path: &Path) -> Result<(), String> {
    let dir = history_dir(store_root);
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let _lock = FileLock::lock(dir.join(".index.lock"), LOCK_TIMEOUT).map_err(|e| e.to_string())?;
    let mut state = load_state(&dir);
    let len = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
    let files = state.entry("files").or_insert_with(|| json!({}));
    if let Some(files) = files.as_object_mut() {
        files.insert(
            file_key(path),
            json!({"offset": len, "head": head_of(path), "recent": []}),
        );
    }
    save_state(&dir, &state)
}

/// Read the complete new lines of one file, handing each new reply to `sink`; returns how many.
fn read_file(
    path: &Path,
    files: &mut Map<String, Value>,
    mut sink: impl FnMut(&super::transcript::Reply),
) -> u64 {
    let key = file_key(path);
    let Ok(meta) = std::fs::metadata(path) else {
        return 0;
    };
    let previous = files.get(&key);
    let mut offset = previous
        .and_then(|v| v.get("offset"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let mut recent: Vec<String> = previous
        .and_then(|v| v.get("recent"))
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|s| s.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    let head = head_of(path);
    let old_head = previous.and_then(|v| v.get("head")).and_then(Value::as_str);
    // Replaced or truncated: read it again. A head recorded from a shorter file is a prefix.
    let replaced = old_head.is_some_and(|h| !head.starts_with(h) && !h.starts_with(head.as_str()));
    if meta.len() < offset || replaced {
        offset = 0;
        recent.clear();
    }
    if meta.len() == offset {
        if old_head.is_none_or(|h| h.len() < head.len()) {
            files.insert(
                key,
                json!({"offset": offset, "head": head, "recent": recent}),
            );
        }
        return 0;
    }
    let Ok(mut file) = std::fs::File::open(path) else {
        return 0;
    };
    if file.seek(SeekFrom::Start(offset)).is_err() {
        return 0;
    }
    let mut seen: HashSet<String> = recent.iter().cloned().collect();
    let mut reader = BufReader::new(file);
    let mut bytes = Vec::new();
    let mut count = 0;
    loop {
        bytes.clear();
        let Ok(n) = reader.read_until(b'\n', &mut bytes) else {
            break;
        };
        if n == 0 || bytes.last() != Some(&b'\n') {
            break; // a line still being written waits for the next run
        }
        offset += n as u64;
        // A line that is not valid UTF-8 is skipped, not a reason to stall on this file.
        let line = String::from_utf8_lossy(&bytes);
        let Some(reply) = parse_line(line.trim_end()) else {
            continue;
        };
        if !seen.insert(reply.id.clone()) {
            continue;
        }
        recent.push(reply.id.clone());
        sink(&reply);
        count += 1;
    }
    let keep = recent.len().saturating_sub(RECENT_IDS);
    files.insert(
        key,
        json!({"offset": offset, "head": head, "recent": recent[keep..]}),
    );
    count
}

fn write_rows(dir: &Path, rows: &BTreeMap<Key, (u64, Tokens)>) -> Result<(), String> {
    if rows.is_empty() {
        return Ok(());
    }
    let days = dir.join(DAYS);
    std::fs::create_dir_all(&days).map_err(|e| format!("cannot create {}: {e}", days.display()))?;
    let mut by_day: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for (k, (replies, t)) in rows {
        let row = json!({
            "hour": k.hour,
            "account": k.account,
            "approximate": k.approximate,
            "project": k.project,
            "session": k.session,
            "model": k.model,
            "replies": replies,
            "input": t.input,
            "output": t.output,
            "cacheRead": t.cache_read,
            "cacheWrite5m": t.cache_write_5m,
            "cacheWrite1h": t.cache_write_1h,
        });
        by_day
            .entry(k.day.as_str())
            .or_default()
            .push(row.to_string());
    }
    for (day, lines) in by_day {
        let path = days.join(format!("{day}.jsonl"));
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
        for line in lines {
            writeln!(file, "{line}")
                .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
        }
    }
    Ok(())
}

/// Delete day files older than `keep_days`, and quota readings older than that.
fn prune(dir: &Path, keep_days: i64) {
    let cutoff = jiff::Timestamp::now().as_second() - keep_days * 86_400;
    let cutoff_day = jiff::Timestamp::from_second(cutoff)
        .unwrap_or_default()
        .strftime("%Y-%m-%d")
        .to_string();
    if let Ok(entries) = std::fs::read_dir(dir.join(DAYS)) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name
                .strip_suffix(".jsonl")
                .is_some_and(|d| d < cutoff_day.as_str())
            {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
    super::quota::prune(dir, cutoff);
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn reply(id: &str, at: &str, session: &str, output: u64) -> String {
        format!(
            r#"{{"type":"assistant","timestamp":"{at}","sessionId":"{session}","cwd":"/w/api","requestId":"{id}","message":{{"model":"claude-opus-5-5","usage":{{"input_tokens":10,"output_tokens":{output},"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}}}}}"#
        )
    }

    struct Env {
        tmp: TempDir,
    }

    impl Env {
        fn new(import: &str) -> Self {
            let tmp = TempDir::new().unwrap();
            std::fs::create_dir_all(tmp.path().join("store")).unwrap();
            crate::settings::set_setting(&tmp.path().join("store"), "history.import", import)
                .unwrap();
            std::fs::create_dir_all(tmp.path().join("claude/projects/-w-api")).unwrap();
            Env { tmp }
        }
        fn store(&self) -> PathBuf {
            self.tmp.path().join("store")
        }
        fn profile(&self) -> PathBuf {
            self.tmp.path().join("claude")
        }
        fn transcript(&self) -> PathBuf {
            self.profile().join("projects/-w-api/s1.jsonl")
        }
        fn append(&self, text: &str) {
            let mut f = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(self.transcript())
                .unwrap();
            f.write_all(text.as_bytes()).unwrap();
        }
        fn index(&self) -> Outcome {
            run(&self.store(), &self.profile(), false).unwrap()
        }
        fn rows(&self) -> Vec<Value> {
            let mut out = Vec::new();
            for path in transcripts(&history_dir(&self.store()).join(DAYS)) {
                let text = std::fs::read_to_string(path).unwrap();
                out.extend(
                    text.lines()
                        .map(|l| serde_json::from_str::<Value>(l).unwrap()),
                );
            }
            out
        }
    }

    fn now_iso() -> String {
        jiff::Timestamp::now()
            .strftime("%Y-%m-%dT%H:%M:%S.000Z")
            .to_string()
    }

    #[test]
    fn ask_reads_nothing_until_the_user_chooses() {
        let env = Env::new("ask");
        env.append(&format!("{}\n", reply("r1", &now_iso(), "s1", 5)));
        assert!(env.index().needs_choice);
        assert!(env.rows().is_empty());
    }

    #[test]
    fn none_starts_from_now() {
        let env = Env::new("none");
        env.append(&format!("{}\n", reply("old", &now_iso(), "s1", 5)));
        assert_eq!(env.index().replies, 0);
        env.append(&format!("{}\n", reply("new", &now_iso(), "s1", 7)));
        assert_eq!(env.index().replies, 1);
        let rows = env.rows();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["output"], 7);
    }

    #[test]
    fn all_reads_history_once_and_dedupes_split_replies() {
        let env = Env::new("all");
        let at = now_iso();
        // Claude Code writes one line per content block with the same usage.
        env.append(&format!(
            "{}\n{}\n",
            reply("r1", &at, "s1", 5),
            reply("r1", &at, "s1", 5)
        ));
        env.append(&format!("{}\n", reply("r2", &at, "s1", 3)));
        // A line still being written is left for the next run.
        env.append(&reply("r3", &at, "s1", 4));
        assert_eq!(env.index().replies, 2);
        assert_eq!(env.index().replies, 0, "nothing new");
        env.append(&format!("\n{}\n", reply("r1", &at, "s1", 5)));
        assert_eq!(
            env.index().replies,
            1,
            "r3 now complete; r1 again is a duplicate"
        );
        let output: u64 = env
            .rows()
            .iter()
            .map(|r| r["output"].as_u64().unwrap())
            .sum();
        assert_eq!(output, 5 + 3 + 4);
    }

    #[test]
    fn a_replaced_file_is_read_again_and_rebuild_starts_over() {
        let env = Env::new("all");
        let at = now_iso();
        env.append(&format!(
            "{}\n{}\n",
            reply("a", &at, "s1", 1),
            reply("b", &at, "s1", 1)
        ));
        assert_eq!(env.index().replies, 2);
        std::fs::write(env.transcript(), format!("{}\n", reply("c", &at, "s1", 1))).unwrap();
        assert_eq!(env.index().replies, 1);
        assert_eq!(run(&env.store(), &env.profile(), true).unwrap().replies, 1);
        assert_eq!(env.rows().len(), 1);
    }

    #[test]
    fn a_same_size_replacement_and_bad_bytes_are_handled() {
        let env = Env::new("all");
        let at = now_iso();
        env.append(&format!("{}\n", reply("a", &at, "s1", 1)));
        assert_eq!(env.index().replies, 1);
        // Replaced by a different file at least as long as the old offset.
        std::fs::write(
            env.transcript(),
            format!(
                "{}\n{}\n",
                reply("b", &at, "s2", 1),
                reply("c", &at, "s2", 1)
            ),
        )
        .unwrap();
        assert_eq!(env.index().replies, 2);
        // A line with invalid UTF-8 is skipped and reading goes on.
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(env.transcript())
            .unwrap();
        f.write_all(b"\xff\xfe not text\n").unwrap();
        env.append(&format!("{}\n", reply("d", &at, "s2", 1)));
        assert_eq!(env.index().replies, 1);
    }

    #[test]
    fn marked_files_are_not_counted() {
        let env = Env::new("all");
        let at = now_iso();
        env.append(&format!("{}\n", reply("copied", &at, "s1", 1)));
        mark_read(&env.store(), &env.transcript()).unwrap();
        assert_eq!(env.index().replies, 0);
        env.append(&format!("{}\n", reply("new", &at, "s1", 1)));
        assert_eq!(env.index().replies, 1);
    }

    #[test]
    fn disabled_reads_nothing() {
        let env = Env::new("all");
        crate::settings::set_setting(&env.store(), "history.enabled", "false").unwrap();
        assert!(env.index().disabled);
    }

    #[test]
    fn old_days_are_pruned() {
        let env = Env::new("all");
        env.append(&format!(
            "{}\n",
            reply("x", "2020-01-01T00:00:00.000Z", "s1", 1)
        ));
        env.append(&format!("{}\n", reply("y", &now_iso(), "s1", 1)));
        env.index();
        assert_eq!(env.rows().len(), 1, "2020 is past the 90-day default");
    }

    #[test]
    fn session_profiles_with_their_own_history_are_sources() {
        let tmp = TempDir::new().unwrap();
        let store = tmp.path().join("store");
        let default = tmp.path().join("claude");
        std::fs::create_dir_all(default.join("projects")).unwrap();
        std::fs::create_dir_all(store.join("sessions/3-a-x-com/projects")).unwrap();
        let found = sources(&store, &default);
        assert_eq!(found.len(), 2);
        assert_eq!(found[1].account, Some(3));
    }
}

#[cfg(all(test, windows))]
mod windows_tests {
    #[test]
    fn mixed_separators_name_the_same_file() {
        use std::path::Path;
        assert_eq!(
            super::file_key(Path::new(r"C:\u\claude\projects/-w-api/s1.jsonl")),
            super::file_key(Path::new(r"C:\u\claude\projects\-w-api\s1.jsonl"))
        );
    }
}

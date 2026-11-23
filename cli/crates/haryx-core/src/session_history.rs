//! Merging a session profile's own conversation history into the shared `~/.claude` history (upstream
//! `session._history_tree`, `_mkdir_private`, `_last_timestamp`, `_line_count`, `_profile_copy_wins`,
//! `_adds_nothing`, and `SessionManager._merge_history_into_source`, `_quarantine`,
//! `_shared_side_sessions`, `_shared_copy_in_use`).
//!
//! A merge never deletes a conversation: when both sides hold a file, the one that loses is moved into
//! `<store>/history-conflicts/<run>/{shared,profile}/...`.

use std::collections::HashSet;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::fsutil::is_link;
use crate::session;

/// Where history migration parks the losing copy of each collision.
pub const QUARANTINE_DIRNAME: &str = "history-conflicts";

/// What one merge did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct MergeCounts {
    pub moved: usize,
    pub quarantined: usize,
    pub run_dir: Option<PathBuf>,
}

/// Every path under `root`, deepest first; refuses links (a merge never follows one).
pub fn history_tree(root: &Path) -> io::Result<Vec<PathBuf>> {
    fn visit(dir: &Path, found: &mut Vec<PathBuf>) -> io::Result<()> {
        for entry in fs::read_dir(dir)? {
            let path = entry?.path();
            if is_link(&path) {
                return Err(io::Error::other(format!(
                    "{} is a link, which a history merge never follows; move it out of the profile to share the rest",
                    path.display()
                )));
            }
            let is_dir = fs::symlink_metadata(&path)?.is_dir();
            found.push(path.clone());
            if is_dir {
                visit(&path, found)?;
            }
        }
        Ok(())
    }
    let mut found = Vec::new();
    visit(root, &mut found)?;
    found.sort_by(|a, b| b.cmp(a));
    Ok(found)
}

/// Create `path` and any missing parents with mode 0700.
pub fn mkdir_private(path: &Path) -> io::Result<()> {
    let mut missing = Vec::new();
    let mut current = path.to_path_buf();
    while !current.exists() {
        missing.push(current.clone());
        match current.parent() {
            Some(parent) if parent != current => current = parent.to_path_buf(),
            _ => break,
        }
    }
    for dir in missing.iter().rev() {
        match fs::create_dir(dir) {
            Ok(()) => set_private(dir),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

#[cfg(unix)]
fn set_private(dir: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = fs::set_permissions(dir, fs::Permissions::from_mode(0o700));
}

#[cfg(not(unix))]
fn set_private(_dir: &Path) {}

/// The last `timestamp` field of a JSONL transcript.
fn last_timestamp(path: &Path) -> Option<String> {
    let bytes = fs::read(path).ok()?;
    let text = String::from_utf8_lossy(&bytes);
    text.lines()
        .rev()
        .filter(|l| !l.trim().is_empty())
        .find_map(|line| {
            let entry: Value = serde_json::from_str(line).ok()?;
            entry
                .get("timestamp")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
        })
}

fn line_count(path: &Path) -> usize {
    fs::read(path)
        .map(|b| {
            let newlines = b.iter().filter(|&&c| c == b'\n').count();
            newlines + usize::from(!b.is_empty() && !b.ends_with(b"\n"))
        })
        .unwrap_or(0)
}

/// Whether the profile's copy of a transcript is the newer one (later last timestamp, else longer).
fn profile_copy_wins(target: &Path, candidate: &Path) -> bool {
    let (t, c) = (last_timestamp(target), last_timestamp(candidate));
    if t != c {
        return match (t, c) {
            (None, _) => true,
            (_, None) => false,
            (Some(t), Some(c)) => c > t,
        };
    }
    line_count(candidate) > line_count(target)
}

/// Whether `candidate` is a byte prefix of `target` (it adds nothing).
fn adds_nothing(target: &Path, candidate: &Path) -> bool {
    let check = || -> io::Result<bool> {
        if fs::metadata(candidate)?.len() > fs::metadata(target)?.len() {
            return Ok(false);
        }
        let mut profile = fs::File::open(candidate)?;
        let mut shared = fs::File::open(target)?;
        let mut a = vec![0u8; 1 << 20];
        let mut b = vec![0u8; 1 << 20];
        loop {
            let n = profile.read(&mut a)?;
            if n == 0 {
                return Ok(true);
            }
            shared.read_exact(&mut b[..n])?;
            if a[..n] != b[..n] {
                return Ok(false);
            }
        }
    };
    check().unwrap_or(false)
}

/// Move a file (rename, or copy + remove across filesystems).
fn move_file(from: &Path, to: &Path) -> io::Result<()> {
    match fs::rename(from, to) {
        Ok(()) => Ok(()),
        Err(_) => {
            fs::copy(from, to)?;
            fs::remove_file(from)
        }
    }
}

/// Session ids live in the shared home and every session profile; `false` when any record could not be
/// read or carries no id (then the caller must assume a copy is in use).
fn shared_side_sessions(shared_root: &Path, sessions_dir: &Path) -> (HashSet<String>, bool) {
    let mut roots = vec![shared_root.to_path_buf()];
    match fs::read_dir(sessions_dir) {
        Ok(rd) => roots.extend(rd.flatten().map(|e| e.path()).filter(|p| p.is_dir())),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(_) => return (HashSet::new(), false),
    }
    let mut ids = HashSet::new();
    let mut complete = true;
    for root in roots {
        let (sessions, unreadable) = session::scan_live_sessions(&root);
        complete &= unreadable == 0;
        for live in sessions {
            if live.session_id.is_empty() {
                complete = false;
            } else {
                ids.insert(live.session_id);
            }
        }
    }
    (ids, complete)
}

fn shared_copy_in_use(shared_root: &Path, sessions_dir: &Path, rel: &Path) -> bool {
    let (live, known) = shared_side_sessions(shared_root, sessions_dir);
    if !known {
        return true;
    }
    let mut names: HashSet<String> = rel
        .parent()
        .map(|p| {
            p.components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    if let Some(name) = rel.file_name() {
        let name = name.to_string_lossy();
        names.insert(name.strip_suffix(".jsonl").unwrap_or(&name).to_owned());
    }
    !names.is_disjoint(&live)
}

fn quarantine(path: &Path, rel: &Path, run_dir: &Path, side: &str) -> io::Result<PathBuf> {
    let mut dest = run_dir.join(side).join(rel);
    if let Some(parent) = dest.parent() {
        mkdir_private(parent)?;
    }
    if dest.exists() {
        let stem = dest
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let suffix = dest
            .extension()
            .map(|e| format!(".{}", e.to_string_lossy()))
            .unwrap_or_default();
        let parent = dest.parent().map(Path::to_path_buf).unwrap_or_default();
        dest = (2..)
            .map(|n| parent.join(format!("{stem}.{n}{suffix}")))
            .find(|c| !c.exists())
            .unwrap_or(dest);
    }
    move_file(path, &dest)?;
    log::info!("Quarantined colliding history file to {}", dest.display());
    Ok(dest)
}

/// Merge the profile's own `dest` (a `projects/` tree or `history.jsonl`) into the shared `src`.
pub fn merge_history_into_source(
    src: &Path,
    dest: &Path,
    store_root: &Path,
    sessions_dir: &Path,
    counts: &mut MergeCounts,
) -> io::Result<()> {
    let same = src.exists()
        && crate::fsutil::canonicalize(src).ok() == crate::fsutil::canonicalize(dest).ok();
    if is_link(dest) || same {
        return Err(io::Error::other(format!(
            "{} is the shared history, not the profile's own",
            dest.display()
        )));
    }
    if !dest.is_dir() {
        return merge_jsonl(src, dest);
    }
    mkdir_private(src)?;
    let profile_name = dest
        .parent()
        .and_then(Path::file_name)
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let shared_root = src.parent().unwrap_or(src).to_path_buf();
    let quarantine_root = |counts: &mut MergeCounts| -> io::Result<PathBuf> {
        if let Some(dir) = &counts.run_dir {
            return Ok(dir.clone());
        }
        let stamp = jiff::Timestamp::now()
            .strftime("%Y-%m-%dT%H-%M-%S")
            .to_string();
        let dir = store_root
            .join(QUARANTINE_DIRNAME)
            .join(format!("{stamp}-{profile_name}"));
        mkdir_private(&dir)?;
        counts.run_dir = Some(dir.clone());
        Ok(dir)
    };
    for path in history_tree(dest)? {
        let rel = path
            .strip_prefix(dest)
            .map(Path::to_path_buf)
            .unwrap_or_default();
        let target = src.join(&rel);
        if fs::symlink_metadata(&path)?.is_dir() {
            fs::remove_dir(&path)?;
            continue;
        }
        if target.exists() {
            if adds_nothing(&target, &path) {
                fs::remove_file(&path)?;
                continue;
            }
            let jsonl = path.extension().is_some_and(|e| e == "jsonl");
            if jsonl
                && target.is_file()
                && profile_copy_wins(&target, &path)
                && !shared_copy_in_use(&shared_root, sessions_dir, &rel)
            {
                let root = quarantine_root(counts)?;
                quarantine(&target, &rel, &root, "shared")?;
                counts.quarantined += 1;
                if let Some(parent) = target.parent() {
                    mkdir_private(parent)?;
                }
                move_file(&path, &target)?;
                counts.moved += 1;
            } else {
                let root = quarantine_root(counts)?;
                quarantine(&path, &rel, &root, "profile")?;
                counts.quarantined += 1;
            }
            continue;
        }
        if let Some(parent) = target.parent() {
            mkdir_private(parent)?;
        }
        move_file(&path, &target)?;
        counts.moved += 1;
    }
    fs::remove_dir(dest)
}

/// `history.jsonl`: append the profile's lines the shared file lacks, then drop the profile's copy.
fn merge_jsonl(src: &Path, dest: &Path) -> io::Result<()> {
    let existing_text = if src.exists() {
        fs::read_to_string(src)?
    } else {
        String::new()
    };
    let existing: HashSet<&str> = existing_text.lines().collect();
    let profile = fs::read_to_string(dest)?;
    let lines: Vec<&str> = profile
        .lines()
        .filter(|l| !l.is_empty() && !existing.contains(l))
        .collect();
    if !lines.is_empty() {
        if let Some(parent) = src.parent() {
            fs::create_dir_all(parent)?;
        }
        let torn = !existing_text.is_empty() && !existing_text.ends_with('\n');
        let mut file = open_append_private(src)?;
        let terminator = if torn { "\n" } else { "" };
        writeln!(file, "{terminator}{}", lines.join("\n"))?;
    }
    fs::remove_file(dest)
}

#[cfg(unix)]
fn open_append_private(path: &Path) -> io::Result<fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    fs::OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(path)
}

#[cfg(not(unix))]
fn open_append_private(path: &Path) -> io::Result<fs::File> {
    fs::OpenOptions::new().create(true).append(true).open(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    #[test]
    fn jsonl_merge_appends_only_new_lines_and_repairs_a_torn_tail() {
        let tmp = tempfile::tempdir().unwrap();
        let (src, dest) = (
            tmp.path().join("shared/history.jsonl"),
            tmp.path().join("p/history.jsonl"),
        );
        write(&src, "a\nb");
        write(&dest, "b\nc\n");
        merge_history_into_source(
            &src,
            &dest,
            tmp.path(),
            &tmp.path().join("sessions"),
            &mut MergeCounts::default(),
        )
        .unwrap();
        assert_eq!(fs::read_to_string(&src).unwrap(), "a\nb\nc\n");
        assert!(!dest.exists());
    }

    #[test]
    fn tree_merge_moves_new_files_and_quarantines_conflicts() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("home/.claude/projects");
        let dest = tmp.path().join("store/sessions/1-a/projects");
        write(
            &src.join("p/same.jsonl"),
            "{\"timestamp\":\"2026-01-02\"}\n",
        );
        write(
            &dest.join("p/same.jsonl"),
            "{\"timestamp\":\"2026-01-01\"}\n",
        );
        write(
            &dest.join("p/new.jsonl"),
            "{\"timestamp\":\"2026-01-03\"}\n",
        );
        write(&dest.join("p/prefix.jsonl"), "");
        write(&src.join("p/prefix.jsonl"), "x\n");
        let mut counts = MergeCounts::default();
        let store = tmp.path().join("store");
        merge_history_into_source(&src, &dest, &store, &store.join("sessions"), &mut counts)
            .unwrap();
        assert!(src.join("p/new.jsonl").exists());
        assert_eq!(counts.moved, 1);
        assert_eq!(
            counts.quarantined, 1,
            "the older profile copy is kept aside"
        );
        let run = counts.run_dir.unwrap();
        assert!(run.join("profile/p/same.jsonl").exists());
        assert!(!dest.exists());
    }

    #[test]
    fn a_newer_profile_copy_replaces_the_shared_one() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("home/.claude/projects");
        let dest = tmp.path().join("store/sessions/1-a/projects");
        write(&src.join("p/c.jsonl"), "{\"timestamp\":\"2026-01-01\"}\n");
        write(&dest.join("p/c.jsonl"), "{\"timestamp\":\"2026-02-01\"}\n");
        let mut counts = MergeCounts::default();
        let store = tmp.path().join("store");
        merge_history_into_source(&src, &dest, &store, &store.join("sessions"), &mut counts)
            .unwrap();
        assert!(
            fs::read_to_string(src.join("p/c.jsonl"))
                .unwrap()
                .contains("2026-02-01")
        );
        assert!(counts.run_dir.unwrap().join("shared/p/c.jsonl").exists());
    }
}

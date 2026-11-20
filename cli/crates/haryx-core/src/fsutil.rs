//! Port of `claude_swap/fsutil.py`: filesystem primitives with no dependencies on the rest of the crate.
//!
//! Deliberately a leaf module: the atomic-write helpers here are needed by settings, credentials, mappings
//! and session. Functions return [`std::io::Result`], the counterpart of upstream raising `OSError`.

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Default attempt budget of [`read_text_with_retry`] and [`replace_with_retry`].
pub const DEFAULT_RETRY_ATTEMPTS: u32 = 10;
/// First back-off of the retry helpers; doubles up to [`MAX_RETRY_DELAY`].
pub const INITIAL_RETRY_DELAY: Duration = Duration::from_millis(2);
/// Cap of the retry back-off.
pub const MAX_RETRY_DELAY: Duration = Duration::from_millis(250);

/// NAME_MAX is 255 bytes on every filesystem haryx meets, and the temp name adds eight random characters
/// plus `.tmp` to the prefix. A prefix carrying the whole destination name would push a legal 243+ character
/// destination over the limit (ENAMETOOLONG at the temp, for a file that itself fits). Keep the association
/// readable for ordinary names and bounded for long ones.
const TEMP_PREFIX_CHARS: usize = 40;
const TEMP_RANDOM_CHARS: usize = 8;
const TEMP_CREATE_ATTEMPTS: u32 = 100;
/// Hops of a symlink chain before giving up with ELOOP (Linux's limit).
const MAX_SYMLINK_HOPS: u32 = 40;

/// Windows error codes that usually mean "someone else has the file open right now": ERROR_ACCESS_DENIED,
/// ERROR_SHARING_VIOLATION and ERROR_LOCK_VIOLATION. AV/indexer contention clears within milliseconds.
#[cfg(windows)]
const TRANSIENT_WIN_ERRORS: [i32; 3] = [5, 32, 33];

/// Whether `err` is a transient Windows sharing failure worth retrying. POSIX never is: there an `EACCES`
/// is a genuine, persistent permission problem that must surface at once.
fn is_transient(err: &io::Error) -> bool {
    #[cfg(windows)]
    {
        err.raw_os_error()
            .is_some_and(|code| TRANSIENT_WIN_ERRORS.contains(&code))
    }
    #[cfg(not(windows))]
    {
        let _ = err;
        false
    }
}

fn retry<T>(
    attempts: u32,
    initial_delay: Duration,
    mut op: impl FnMut() -> io::Result<T>,
) -> io::Result<T> {
    if attempts < 1 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "attempts must be >= 1",
        ));
    }
    let mut delay = initial_delay;
    let mut attempt = 0;
    loop {
        attempt += 1;
        match op() {
            Ok(value) => return Ok(value),
            Err(err) if !is_transient(&err) || attempt >= attempts => return Err(err),
            Err(_) => {
                std::thread::sleep(delay);
                delay = (delay * 2).min(MAX_RETRY_DELAY);
            }
        }
    }
}

/// `fs::read_to_string`, retried past transient Windows sharing failures.
///
/// A file published by rename is freshly modified exactly when AV or the indexer opens it opportunistically,
/// and a reader arriving in that window gets ERROR_SHARING_VIOLATION. POSIX is untouched: there is no such
/// window, and errors surface on the first attempt.
pub fn read_text_with_retry(
    path: &Path,
    attempts: u32,
    initial_delay: Duration,
) -> io::Result<String> {
    retry(attempts, initial_delay, || std::fs::read_to_string(path))
}

/// `fs::rename`, retried past transient Windows sharing failures.
///
/// POSIX `rename` is atomic and never fails this way. On Windows a replace onto a just-written target fails
/// for a few milliseconds while Defender or the indexer holds it. Only the contention codes are retried;
/// other errors (missing source, cross-device link) surface on the first attempt.
pub fn replace_with_retry(
    src: &Path,
    dst: &Path,
    attempts: u32,
    initial_delay: Duration,
) -> io::Result<()> {
    retry(attempts, initial_delay, || std::fs::rename(src, dst))
}

/// Filesystems that do not implement fsync answer one of these; the write goes ahead without the durability
/// it would otherwise buy (Claude Code's own writer tolerates the same set). Anything else, EIO above all,
/// raises.
fn fsync_unsupported(err: &io::Error) -> bool {
    #[cfg(unix)]
    {
        matches!(
            err.raw_os_error(),
            Some(libc::EINVAL | libc::ENOTSUP | libc::ENOSYS)
        ) || err.raw_os_error() == Some(libc::EOPNOTSUPP)
    }
    #[cfg(not(unix))]
    {
        err.kind() == io::ErrorKind::Unsupported
    }
}

fn eloop(path: &Path) -> io::Error {
    #[cfg(unix)]
    {
        let err = io::Error::from_raw_os_error(libc::ELOOP);
        io::Error::new(err.kind(), format!("{}: {}", path.display(), err))
    }
    #[cfg(not(unix))]
    {
        io::Error::other(format!(
            "{}: too many levels of symbolic links",
            path.display()
        ))
    }
}

/// Where a publish to `path` lands: `path` itself or, for a symlink, the end of its chain.
///
/// Written THROUGH a link rather than over it: a rename swaps a directory entry and never follows links, so
/// renaming onto a symlinked path detaches it -- the write succeeds and the link's target silently stops
/// receiving updates (upstream #192). Claude Code's own config writer writes through a symlinked
/// `~/.claude.json` for the same reason.
///
/// Resolution is strict with a missing final referent allowed (Python's `realpath(strict=ALLOW_MISSING)`):
/// the whole chain, a dangling link writes where it points, a loop or any other resolution error raises. Not
/// lexical: `loop/../victim.json` must not collapse into `victim.json`.
pub fn publish_target(path: &Path) -> io::Result<PathBuf> {
    if !is_symlink(path) {
        return Ok(path.to_path_buf());
    }
    let mut current = path.to_path_buf();
    for _ in 0..MAX_SYMLINK_HOPS {
        match std::fs::symlink_metadata(&current) {
            Ok(meta) if meta.file_type().is_symlink() => {
                let stored = std::fs::read_link(&current)?;
                current = match current.parent() {
                    Some(parent) if stored.is_relative() => parent.join(stored),
                    _ => stored,
                };
            }
            Ok(_) => return std::fs::canonicalize(&current),
            Err(err) if err.kind() == io::ErrorKind::NotFound => return resolve_missing(&current),
            Err(err) => return Err(err),
        }
    }
    Err(eloop(path))
}

/// A path whose final component does not exist: resolve its parent strictly and keep the name.
fn resolve_missing(path: &Path) -> io::Result<PathBuf> {
    match (path.parent(), path.file_name()) {
        (Some(parent), Some(name)) => {
            let parent = if parent.as_os_str().is_empty() {
                Path::new(".")
            } else {
                parent
            };
            Ok(std::fs::canonicalize(parent)?.join(name))
        }
        _ => std::fs::canonicalize(path),
    }
}

fn is_symlink(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink())
}

/// Options of [`atomic_write_bytes`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WriteOptions {
    /// Permission bits set on the file before it is published (Unix).
    pub mode: u32,
    /// Publish onto the end of a symlink chain so the link survives (see [`publish_target`]). `false` for a
    /// destination the user typed, where a planted link must not redirect the write.
    pub follow_symlinks: bool,
}

impl Default for WriteOptions {
    fn default() -> Self {
        WriteOptions {
            mode: 0o600,
            follow_symlinks: true,
        }
    }
}

/// Publish `data` at `path` by temp file + rename.
///
/// The temp is created beside the target (0600 from creation, same filesystem, so the rename is atomic);
/// the bytes are written, `mode` is set on the open descriptor, the content is fsynced, and the temp is
/// renamed over the target. A reader sees the old file or the new one, never a torn one: Claude Code watches
/// `~/.claude.json` and treats a torn read as a corrupted config (upstream #419).
///
/// A failure before the rename leaves the destination exactly as it was and removes the temp. The durability
/// of the rename itself is the directory's business and is not promised. Where the filesystem reports fsync
/// as unsupported, the write proceeds without that guarantee.
///
/// Not done on purpose: no creation of missing parents, and no in-place fallback when the rename is refused
/// (a bind-mounted target answers EBUSY, the write fails, the destination is untouched).
pub fn atomic_write_bytes(path: &Path, data: &[u8], opts: WriteOptions) -> io::Result<()> {
    let target = if opts.follow_symlinks {
        publish_target(path)?
    } else {
        path.to_path_buf()
    };
    let (file, tmp) = create_temp_beside(&target)?;
    let result = write_and_publish(file, &tmp, &target, data, opts.mode);
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

fn write_and_publish(
    mut file: File,
    tmp: &Path,
    target: &Path,
    data: &[u8],
    mode: u32,
) -> io::Result<()> {
    file.write_all(data)?;
    set_mode(&file, mode)?;
    if let Err(err) = file.sync_all() {
        if !fsync_unsupported(&err) {
            return Err(err);
        }
    }
    drop(file);
    replace_with_retry(tmp, target, DEFAULT_RETRY_ATTEMPTS, INITIAL_RETRY_DELAY)
}

#[cfg(unix)]
fn set_mode(file: &File, mode: u32) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    file.set_permissions(std::fs::Permissions::from_mode(mode))
}

#[cfg(not(unix))]
fn set_mode(_file: &File, _mode: u32) -> io::Result<()> {
    Ok(())
}

/// `mkstemp` beside `target`: `<name[:40]>.<8 random>.tmp`, created exclusively with mode 0600.
fn create_temp_beside(target: &Path) -> io::Result<(File, PathBuf)> {
    let dir = match target.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    let name = target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let prefix: String = name.chars().take(TEMP_PREFIX_CHARS).collect();
    for _ in 0..TEMP_CREATE_ATTEMPTS {
        let tmp = dir.join(format!("{prefix}.{}.tmp", random_suffix()));
        match open_exclusive(&tmp) {
            Ok(file) => return Ok((file, tmp)),
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(err),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "no usable temporary file name found",
    ))
}

fn random_suffix() -> String {
    use rand::Rng;
    const CHARS: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789_";
    let mut rng = rand::rng();
    (0..TEMP_RANDOM_CHARS)
        .map(|_| CHARS[rng.random_range(0..CHARS.len())] as char)
        .collect()
}

fn open_exclusive(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

/// [`atomic_write_bytes`] for UTF-8 text.
pub fn atomic_write_text(path: &Path, text: &str, opts: WriteOptions) -> io::Result<()> {
    atomic_write_bytes(path, text.as_bytes(), opts)
}

// ---------------------------------------------------------------------------------------------------------
// Links
// ---------------------------------------------------------------------------------------------------------
//
// A link, here, is a directory entry that leads somewhere else and holds no data of its own: a symlink (or,
// once Windows is supported, a directory junction). Code that must tell a haryx-made link from real user
// data goes through `is_link`.

/// Whether `path` is a symlink, dangling or not.
// TODO(windows): also count a directory junction (upstream `is_link`).
pub fn is_link(path: &Path) -> bool {
    is_symlink(path)
}

/// Whether the link at `link` stores `target` as its destination.
///
/// Compares the stored path, not where it leads: a link to an intermediate symlink is a different (and
/// worse) link than one to the resolved file even though both end at the same place. False for anything
/// that is not a link.
// TODO(windows): strip the `\\?\` / `\\?\UNC\` extended-length prefix from the stored path.
pub fn link_points_at(link: &Path, target: &Path) -> bool {
    std::fs::read_link(link).is_ok_and(|stored| stored == target)
}

/// Remove a symlink itself, never what it leads to. `rmdir` is the fallback for a platform that answers
/// EISDIR or EPERM to `unlink`.
pub fn remove_link(link: &Path) -> io::Result<()> {
    match std::fs::remove_file(link) {
        Err(err) if is_dir_refusal(&err) => std::fs::remove_dir(link),
        other => other,
    }
}

fn is_dir_refusal(err: &io::Error) -> bool {
    matches!(
        err.kind(),
        io::ErrorKind::IsADirectory | io::ErrorKind::PermissionDenied
    )
}

/// Put a directory link at `link` leading to `target` (a symlink on Unix).
#[cfg(unix)]
pub fn make_dir_link(target: &Path, link: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

/// Put a symlink at `link` leading to the file `target`.
#[cfg(unix)]
pub fn make_file_link(target: &Path, link: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

// TODO(windows): `make_dir_link` creates a junction, `make_file_link` a file symlink (needs Developer Mode).
#[cfg(not(unix))]
pub fn make_dir_link(_target: &Path, _link: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "links are not supported on this platform yet",
    ))
}

#[cfg(not(unix))]
pub fn make_file_link(_target: &Path, _link: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "links are not supported on this platform yet",
    ))
}

/// Whether this process may create a file symlink in `directory`. Always true off Windows, where trying is
/// the check.
// TODO(windows): probe once per directory (upstream `_probe_link`).
pub fn can_symlink(_directory: &Path) -> bool {
    cfg!(unix)
}

/// Whether a directory link from `directory` to `target` can be made. Always true off Windows (a symlink).
// TODO(windows): probe a junction once per directory and target (upstream `can_junction`).
pub fn can_junction(_directory: &Path, _target: Option<&Path>) -> bool {
    cfg!(unix)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};
    use tempfile::TempDir;

    fn write(path: &Path, text: &str) -> io::Result<()> {
        atomic_write_text(path, text, WriteOptions::default())
    }

    fn entries(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn retry_helpers_reject_nonpositive_attempts() {
        let tmp = TempDir::new().unwrap();
        let p = tmp.path().join("a");
        assert_eq!(
            read_text_with_retry(&p, 0, INITIAL_RETRY_DELAY)
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidInput
        );
        assert_eq!(
            replace_with_retry(&p, &p, 0, INITIAL_RETRY_DELAY)
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidInput
        );
    }

    #[test]
    fn posix_errors_surface_on_the_first_attempt() {
        let tmp = TempDir::new().unwrap();
        let missing = tmp.path().join("missing");
        let started = std::time::Instant::now();
        let err = read_text_with_retry(&missing, 10, Duration::from_millis(200)).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
        let err = replace_with_retry(
            &missing,
            &tmp.path().join("b"),
            10,
            Duration::from_millis(200),
        );
        assert!(err.is_err());
        assert!(
            started.elapsed() < Duration::from_millis(200),
            "no retry sleeps on POSIX"
        );
    }

    #[test]
    fn publishes_by_rename_and_leaves_no_temp() {
        let tmp = TempDir::new().unwrap();
        let p = tmp.path().join("f.json");
        std::fs::write(&p, "old").unwrap();
        write(&p, "new").unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "new");
        assert_eq!(entries(tmp.path()), vec!["f.json"]);
        assert_eq!(
            std::fs::metadata(&p).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[test]
    fn mode_is_applied() {
        let tmp = TempDir::new().unwrap();
        let p = tmp.path().join("f");
        atomic_write_bytes(
            &p,
            b"x",
            WriteOptions {
                mode: 0o644,
                follow_symlinks: true,
            },
        )
        .unwrap();
        assert_eq!(
            std::fs::metadata(&p).unwrap().permissions().mode() & 0o777,
            0o644
        );
    }

    #[test]
    fn failed_publish_leaves_destination_untouched_and_no_temp() {
        let tmp = TempDir::new().unwrap();
        // A non-empty directory at the destination: the rename is refused.
        let dest = tmp.path().join("dest");
        std::fs::create_dir(&dest).unwrap();
        std::fs::write(dest.join("keep"), "kept").unwrap();
        assert!(write(&dest, "new").is_err());
        assert_eq!(std::fs::read_to_string(dest.join("keep")).unwrap(), "kept");
        assert_eq!(entries(tmp.path()), vec!["dest"]);
    }

    #[test]
    fn missing_parent_is_not_created() {
        let tmp = TempDir::new().unwrap();
        let p = tmp.path().join("no").join("such").join("f");
        assert_eq!(write(&p, "x").unwrap_err().kind(), io::ErrorKind::NotFound);
        assert!(!tmp.path().join("no").exists());
    }

    #[test]
    fn bytes_round_trip() {
        let tmp = TempDir::new().unwrap();
        let p = tmp.path().join("b");
        let data: Vec<u8> = (0..=255u8).collect();
        atomic_write_bytes(&p, &data, WriteOptions::default()).unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), data);
    }

    #[test]
    fn writes_through_a_link_and_keeps_it() {
        let tmp = TempDir::new().unwrap();
        let real = tmp.path().join("real.json");
        let link = tmp.path().join("link.json");
        std::fs::write(&real, "old").unwrap();
        symlink(&real, &link).unwrap();
        write(&link, "new").unwrap();
        assert!(is_link(&link));
        assert_eq!(std::fs::read_to_string(&real).unwrap(), "new");
    }

    #[test]
    fn follows_a_relative_two_hop_chain() {
        let tmp = TempDir::new().unwrap();
        std::fs::create_dir(tmp.path().join("d")).unwrap();
        let real = tmp.path().join("d").join("real");
        std::fs::write(&real, "old").unwrap();
        symlink("d/real", tmp.path().join("hop")).unwrap();
        symlink("hop", tmp.path().join("entry")).unwrap();
        write(&tmp.path().join("entry"), "new").unwrap();
        assert_eq!(std::fs::read_to_string(&real).unwrap(), "new");
        assert!(is_link(&tmp.path().join("hop")));
        assert!(is_link(&tmp.path().join("entry")));
    }

    #[test]
    fn dangling_link_writes_where_it_points() {
        let tmp = TempDir::new().unwrap();
        let link = tmp.path().join("link");
        symlink("target", &link).unwrap();
        write(&link, "data").unwrap();
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("target")).unwrap(),
            "data"
        );
        assert!(is_link(&link));
    }

    #[test]
    fn follow_symlinks_false_replaces_the_link() {
        let tmp = TempDir::new().unwrap();
        let real = tmp.path().join("real");
        let link = tmp.path().join("link");
        std::fs::write(&real, "victim").unwrap();
        symlink(&real, &link).unwrap();
        atomic_write_text(
            &link,
            "mine",
            WriteOptions {
                follow_symlinks: false,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(!is_link(&link));
        assert_eq!(std::fs::read_to_string(&real).unwrap(), "victim");
        assert_eq!(std::fs::read_to_string(&link).unwrap(), "mine");
    }

    #[test]
    fn loop_raises_and_writes_nothing() {
        let tmp = TempDir::new().unwrap();
        symlink("b", tmp.path().join("a")).unwrap();
        symlink("a", tmp.path().join("b")).unwrap();
        assert!(write(&tmp.path().join("a"), "x").is_err());
        assert_eq!(entries(tmp.path()), vec!["a", "b"]);
    }

    #[test]
    fn loop_hidden_behind_dotdot_is_still_refused() {
        let tmp = TempDir::new().unwrap();
        symlink("loop", tmp.path().join("loop")).unwrap();
        std::fs::write(tmp.path().join("victim.json"), "safe").unwrap();
        symlink("loop/../victim.json", tmp.path().join("entry")).unwrap();
        assert!(write(&tmp.path().join("entry"), "x").is_err());
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("victim.json")).unwrap(),
            "safe"
        );
    }

    #[test]
    fn publish_target_of_a_plain_path_is_the_path() {
        let p = Path::new("/nonexistent/plain.json");
        assert_eq!(publish_target(p).unwrap(), p);
    }

    #[test]
    fn a_destination_name_near_name_max_still_publishes() {
        let tmp = TempDir::new().unwrap();
        let p = tmp.path().join("x".repeat(250));
        write(&p, "ok").unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "ok");
    }

    #[test]
    fn plain_entries_and_missing_paths_are_not_links() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("f"), "").unwrap();
        assert!(!is_link(&tmp.path().join("f")));
        assert!(!is_link(tmp.path()));
        assert!(!is_link(&tmp.path().join("missing")));
    }

    #[test]
    fn dir_and_file_links_are_links_even_when_dangling() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("dir");
        std::fs::create_dir(&dir).unwrap();
        let dlink = tmp.path().join("dlink");
        make_dir_link(&dir, &dlink).unwrap();
        assert!(is_link(&dlink));
        std::fs::remove_dir(&dir).unwrap();
        assert!(is_link(&dlink), "dangling");
        let flink = tmp.path().join("flink");
        make_file_link(&tmp.path().join("nofile"), &flink).unwrap();
        assert!(is_link(&flink));
    }

    #[test]
    fn link_points_at_compares_the_stored_path() {
        let tmp = TempDir::new().unwrap();
        let real = tmp.path().join("real");
        std::fs::create_dir(&real).unwrap();
        let hop = tmp.path().join("hop");
        symlink(&real, &hop).unwrap();
        let link = tmp.path().join("link");
        symlink(&hop, &link).unwrap();
        assert!(link_points_at(&link, &hop));
        assert!(
            !link_points_at(&link, &real),
            "stored path counts, not where it leads"
        );
        assert!(!link_points_at(&real, &real), "not a link");
        assert!(!link_points_at(&link, &tmp.path().join("other")));
    }

    #[test]
    fn remove_link_keeps_its_target() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("dir");
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(dir.join("data"), "x").unwrap();
        let dlink = tmp.path().join("dlink");
        make_dir_link(&dir, &dlink).unwrap();
        remove_link(&dlink).unwrap();
        assert!(!dlink.exists() && !is_link(&dlink));
        assert!(dir.join("data").exists());

        let file = tmp.path().join("file");
        std::fs::write(&file, "y").unwrap();
        let flink = tmp.path().join("flink");
        make_file_link(&file, &flink).unwrap();
        remove_link(&flink).unwrap();
        assert!(!is_link(&flink));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "y");
    }

    #[test]
    fn off_windows_link_probes_are_true() {
        let tmp = TempDir::new().unwrap();
        assert!(can_symlink(tmp.path()));
        assert!(can_junction(tmp.path(), None));
        assert_eq!(entries(tmp.path()), Vec::<String>::new());
    }
}

//! Port of `claude_swap/process_detection.py`: detect running Claude Code instances.
//!
//! Reads session PID files (`<config-home>/sessions/{pid}.json`) and IDE lockfiles
//! (`<config-home>/ide/{port}.lock`) to determine which Claude Code instances are running: the same
//! mechanism Claude Code itself uses internally. Liveness is `kill(pid, 0)`; a record's `procStart` stamp
//! guards against recycled pids, checked against `/proc/<pid>/stat` on Linux and `ps -o lstart` elsewhere
//! (macOS), exactly as upstream does.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{Map, Value};

use crate::spawn::{self, RunOptions};

/// A recycled pid belongs to a process that started after the recorded claude did. `ps` start times are
/// wall-clock and can move by small clock steps, so only a start this much later disqualifies.
pub const PID_REUSE_SLACK_S: i64 = 120;
/// Bound on each `ps` call.
const PS_TIMEOUT: Duration = Duration::from_secs(5);
const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// A running Claude Code session from `sessions/{pid}.json`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClaudeSession {
    pub pid: i32,
    pub session_id: String,
    pub cwd: String,
    /// Epoch milliseconds.
    pub started_at: i64,
    /// "interactive", "bg", "daemon", "daemon-worker".
    pub kind: String,
    /// "cli", "claude-vscode", "claude-desktop", "sdk-cli", "mcp".
    pub entrypoint: String,
    /// "busy", "idle", "waiting".
    pub status: Option<String>,
}

/// A running IDE instance from `ide/{port}.lock`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IdeInstance {
    /// From the file name.
    pub port: i64,
    pub pid: i32,
    /// "Visual Studio Code", "Cursor", "Windsurf".
    pub ide_name: String,
    pub workspace_folders: Vec<String>,
}

/// Whether a process with `pid` is running. `kill(pid, 0)`; EPERM means it exists but is not ours.
#[cfg(unix)]
pub fn is_pid_alive(pid: i64) -> bool {
    if pid <= 1 {
        return false;
    }
    let Ok(pid) = libc::pid_t::try_from(pid) else {
        return false;
    };
    // SAFETY: signal 0 performs only the existence and permission checks; nothing is delivered.
    let rc = unsafe { libc::kill(pid, 0) };
    rc == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// Whether a process with `pid` is running (upstream `_is_pid_alive_windows`): it opens for a limited
/// query and has not exited. Access denied means it exists but belongs to someone else.
#[cfg(windows)]
pub fn is_pid_alive(pid: i64) -> bool {
    use windows_sys::Win32::Foundation::{
        CloseHandle, ERROR_ACCESS_DENIED, GetLastError, STILL_ACTIVE,
    };
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    // 0 is the idle process and 4 the kernel; neither is ever a recorded claude.
    if pid <= 4 {
        return false;
    }
    let Ok(pid) = u32::try_from(pid) else {
        return false;
    };
    // SAFETY: OpenProcess takes plain values and returns a handle or null; the handle is closed below.
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if handle.is_null() {
        // SAFETY: reads this thread's last-error value; no preconditions.
        return unsafe { GetLastError() } == ERROR_ACCESS_DENIED;
    }
    let mut code = 0_u32;
    // SAFETY: `handle` is a live process handle opened with query rights; `code` outlives the call.
    let ok = unsafe { GetExitCodeProcess(handle, &mut code) };
    // SAFETY: `handle` came from OpenProcess and is closed exactly once.
    unsafe { CloseHandle(handle) };
    // A process that exits with code 259 (STILL_ACTIVE) reads as alive: the same ambiguity upstream has.
    ok != 0 && code == STILL_ACTIVE as u32
}

#[cfg(not(any(unix, windows)))]
pub fn is_pid_alive(_pid: i64) -> bool {
    false
}

/// Watches the process that started this one, so a helper started by a host app (the HaryX app's
/// `haryx auto`) can stop when the app is gone instead of running unattended.
#[derive(Debug)]
pub struct ParentWatch {
    #[cfg(unix)]
    original: i64,
    /// The parent's process handle, opened for `SYNCHRONIZE`, as an integer so the watch is `Send`.
    #[cfg(windows)]
    handle: Option<isize>,
}

impl ParentWatch {
    /// Start watching the current parent.
    pub fn start() -> Self {
        #[cfg(unix)]
        {
            ParentWatch {
                original: unix_parent_pid(),
            }
        }
        #[cfg(windows)]
        {
            ParentWatch {
                handle: windows_parent::open(),
            }
        }
        #[cfg(not(any(unix, windows)))]
        {
            ParentWatch {}
        }
    }

    /// Whether the parent has exited. Unix: we were re-parented, or adopted by PID 1, which also covers
    /// a parent that exited before we first looked. Windows (which never re-parents): the parent's handle
    /// is signalled, or the parent could not be opened at all.
    pub fn is_gone(&self) -> bool {
        #[cfg(unix)]
        {
            let parent = unix_parent_pid();
            parent == 1 || parent != self.original
        }
        #[cfg(windows)]
        {
            self.handle.is_none_or(windows_parent::has_exited)
        }
        #[cfg(not(any(unix, windows)))]
        {
            false
        }
    }
}

#[cfg(windows)]
impl Drop for ParentWatch {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            windows_parent::close(handle);
        }
    }
}

#[cfg(unix)]
fn unix_parent_pid() -> i64 {
    // SAFETY: getppid takes no arguments, cannot fail and touches no memory.
    i64::from(unsafe { libc::getppid() })
}

#[cfg(windows)]
mod windows_parent {
    use windows_sys::Win32::Foundation::{
        CloseHandle, HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0,
    };
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
        TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::System::Threading::{
        OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject,
    };

    /// This process's parent PID from a process snapshot (Windows keeps no live parent link).
    fn parent_pid() -> Option<u32> {
        let me = std::process::id();
        // SAFETY: a snapshot of all processes; the handle is checked and closed below.
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
        if snapshot == INVALID_HANDLE_VALUE {
            return None;
        }
        // SAFETY: an all-zero PROCESSENTRY32W is valid plain data; dwSize is set as the API requires.
        let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
        entry.dwSize = u32::try_from(std::mem::size_of::<PROCESSENTRY32W>()).unwrap_or(0);
        let mut found = None;
        // SAFETY: `snapshot` is live and `entry` is a properly sized, writable entry.
        let mut more = unsafe { Process32FirstW(snapshot, &mut entry) } != 0;
        while more {
            if entry.th32ProcessID == me {
                found = Some(entry.th32ParentProcessID);
                break;
            }
            // SAFETY: as above.
            more = unsafe { Process32NextW(snapshot, &mut entry) } != 0;
        }
        // SAFETY: `snapshot` came from CreateToolhelp32Snapshot and is closed once.
        unsafe { CloseHandle(snapshot) };
        found
    }

    pub fn open() -> Option<isize> {
        let pid = parent_pid()?;
        // SAFETY: plain values in; a null result means the parent is gone or not ours.
        let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
        (!handle.is_null()).then_some(handle as isize)
    }

    pub fn has_exited(handle: isize) -> bool {
        // SAFETY: `handle` is the live process handle `open` returned; a zero timeout only polls.
        unsafe { WaitForSingleObject(handle as HANDLE, 0) == WAIT_OBJECT_0 }
    }

    pub fn close(handle: isize) {
        // SAFETY: closes the handle `open` returned, exactly once (from Drop).
        unsafe { CloseHandle(handle as HANDLE) };
    }
}

/// `ps -o <columns> -p <pid>` under `LC_ALL=C TZ=UTC` like claude's own reading, or `None` when unknowable.
/// Every failure answers `None`: not knowing must never be read as "not the recorded process".
fn ps(pid: i32, columns: &[&str]) -> Option<String> {
    if !cfg!(unix) {
        return None;
    }
    let format: Vec<String> = columns.iter().map(|c| format!("{c}=")).collect();
    let pid = pid.to_string();
    let format = format.join(",");
    let opts = RunOptions {
        env: vec![("LC_ALL".into(), "C".into()), ("TZ".into(), "UTC".into())],
        input: None,
        timeout: Some(PS_TIMEOUT),
        ..Default::default()
    };
    let out = spawn::run(&["ps", "-o", format.as_str(), "-p", pid.as_str()], &opts).ok()?;
    let text = out.stdout.trim();
    (out.success() && !text.is_empty()).then(|| text.to_string())
}

/// Epoch seconds at which `pid` started, or `None` when unknowable. Read the way claude stamps `procStart`
/// into its record (`ps -o lstart=` under `LC_ALL=C TZ=UTC`), so the two agree to the second.
pub fn process_started_at(pid: i32) -> Option<i64> {
    lstart_seconds(&ps(pid, &["lstart"])?)
}

/// `Wed Sep  2 20:35:59 2026` (the `ps -o lstart` format under `LC_ALL=C TZ=UTC`) as epoch seconds. Parsed
/// by hand because month names must not depend on the locale.
pub fn lstart_seconds(text: &str) -> Option<i64> {
    let parts: Vec<&str> = text.split_whitespace().collect();
    let [_, month, day, clock, year] = parts.as_slice() else {
        return None;
    };
    let month = MONTHS.iter().position(|m| m == month)? + 1;
    let clock: Vec<i8> = clock
        .split(':')
        .map(|p| p.parse().ok())
        .collect::<Option<_>>()?;
    let [hours, minutes, seconds] = clock.as_slice() else {
        return None;
    };
    let date = jiff::civil::Date::new(
        year.parse().ok()?,
        i8::try_from(month).ok()?,
        day.parse().ok()?,
    )
    .ok()?;
    let time = jiff::civil::Time::new(*hours, *minutes, *seconds, 0).ok()?;
    let zoned = date
        .to_datetime(time)
        .to_zoned(jiff::tz::TimeZone::UTC)
        .ok()?;
    Some(zoned.timestamp().as_second())
}

/// `/proc/<pid>/stat`'s start time in clock ticks since boot, or `None` when unreadable. Linux by
/// construction: the file exists nowhere else.
pub fn process_start_ticks(pid: i32) -> Option<String> {
    let bytes = std::fs::read(format!("/proc/{pid}/stat")).ok()?;
    stat_start_ticks(&String::from_utf8_lossy(&bytes))
}

/// Field 22 of a `/proc/<pid>/stat` line, as the string claude stamps into `procStart`. The command name
/// sits in parentheses before the numeric fields and may itself contain spaces or parentheses, so the fields
/// are counted from the last closing one.
pub fn stat_start_ticks(text: &str) -> Option<String> {
    let rest = text.rfind(')').map_or("", |i| &text[i + 1..]);
    let field = rest.split_whitespace().nth(19)?;
    is_ascii_digits(field).then(|| field.to_string())
}

fn is_ascii_digits(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit())
}

/// Does the process at `pid` look like a claude, or `None` when unknowable. Judged from
/// `ps -o comm=,args=`: the native binary and its symlink are named `claude`, and an npm install runs
/// `cli.js` out of a `claude-code` package directory.
pub fn process_is_claude(pid: i32) -> Option<bool> {
    Some(
        ps(pid, &["comm", "args"])?
            .to_lowercase()
            .contains("claude"),
    )
}

/// Is the live process at `pid` the one that wrote a record stamped `proc_start`, claude's reading of its
/// own start?
///
/// On Linux the stamp is the `/proc/<pid>/stat` start time in clock ticks since boot, which a process keeps
/// for life and no two processes at one pid share, so equality is the whole test. Elsewhere it is
/// `ps -o lstart`, a wall-clock time: a recycled pid belongs to a process that started after the recorded
/// claude did, and only that direction disqualifies, and only a stranger (some `ps` builds move every start
/// time with each wall-clock step, so a live session can read as younger than its own record). Everything
/// unknowable passes, because "cannot tell" must never turn a live session into "nobody there".
pub fn pid_matches_record(pid: i32, proc_start: Option<&str>) -> bool {
    pid_matches_record_with(pid, proc_start, &SystemProbe)
}

/// The process facts [`pid_matches_record`] consults; a seam for tests.
trait ProcessProbe {
    fn start_ticks(&self, pid: i32) -> Option<String>;
    fn started_at(&self, pid: i32) -> Option<i64>;
    fn is_claude(&self, pid: i32) -> Option<bool>;
}

struct SystemProbe;

impl ProcessProbe for SystemProbe {
    fn start_ticks(&self, pid: i32) -> Option<String> {
        process_start_ticks(pid)
    }
    fn started_at(&self, pid: i32) -> Option<i64> {
        process_started_at(pid)
    }
    fn is_claude(&self, pid: i32) -> Option<bool> {
        process_is_claude(pid)
    }
}

fn pid_matches_record_with(pid: i32, proc_start: Option<&str>, probe: &dyn ProcessProbe) -> bool {
    let Some(proc_start) = proc_start.filter(|s| !s.is_empty()) else {
        return true;
    };
    if is_ascii_digits(proc_start) {
        return probe
            .start_ticks(pid)
            .is_none_or(|ticks| ticks == proc_start);
    }
    let Some(recorded) = lstart_seconds(proc_start) else {
        return true;
    };
    match probe.started_at(pid) {
        Some(started) if started > recorded + PID_REUSE_SLACK_S => {
            probe.is_claude(pid) != Some(false)
        }
        _ => true,
    }
}

/// Why a record could not be read (upstream's tuple of caught exceptions).
#[derive(Debug)]
struct Unreadable(String);

fn read_object(path: &Path) -> Result<Map<String, Value>, Unreadable> {
    let text = std::fs::read_to_string(path).map_err(|e| Unreadable(e.to_string()))?;
    match serde_json::from_str::<Value>(&text).map_err(|e| Unreadable(e.to_string()))? {
        Value::Object(map) => Ok(map),
        _ => Err(Unreadable("not a JSON object".into())),
    }
}

/// A record's `pid`: `Ok(None)` for a value upstream reads as "not alive" (a bool, `<= 1`), an error for a
/// wrong type or one too large for a pid.
fn record_pid(value: &Value) -> Result<Option<i32>, Unreadable> {
    let pid = match value {
        Value::Bool(_) => return Ok(None),
        Value::Number(n) => n
            .as_i64()
            .ok_or_else(|| Unreadable(format!("pid {n} is not an integer")))?,
        other => return Err(Unreadable(format!("pid {other} is not an integer"))),
    };
    if pid <= 1 {
        return Ok(None);
    }
    i32::try_from(pid)
        .map(Some)
        .map_err(|_| Unreadable(format!("pid {pid} is out of range")))
}

fn str_field(map: &Map<String, Value>, key: &str) -> String {
    map.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

/// One session record: `Ok(None)` when its process is gone or recycled.
fn read_session(
    path: &Path,
    probe: &dyn ProcessProbe,
    alive: &dyn Fn(i64) -> bool,
) -> Result<Option<ClaudeSession>, Unreadable> {
    let data = read_object(path)?;
    let pid = data
        .get("pid")
        .ok_or_else(|| Unreadable("missing pid".into()))?;
    let Some(pid) = record_pid(pid)? else {
        return Ok(None);
    };
    if !alive(i64::from(pid)) {
        return Ok(None);
    }
    let proc_start = match data.get("procStart") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => Some(s.as_str()),
        Some(other) => return Err(Unreadable(format!("procStart {other} is not a string"))),
    };
    if !pid_matches_record_with(pid, proc_start, probe) {
        log::debug!(
            "Skipping session file {}: pid {pid} was recycled",
            path.display()
        );
        return Ok(None);
    }
    Ok(Some(ClaudeSession {
        pid,
        session_id: str_field(&data, "sessionId"),
        cwd: str_field(&data, "cwd"),
        started_at: data.get("startedAt").and_then(Value::as_i64).unwrap_or(0),
        kind: str_field(&data, "kind"),
        entrypoint: str_field(&data, "entrypoint"),
        status: data.get("status").and_then(Value::as_str).map(String::from),
    }))
}

/// Live sessions, and how many records could NOT be read.
///
/// A record counts as live only when its pid is alive AND still belongs to the claude that wrote it (see
/// [`pid_matches_record`]).
///
/// Two kinds of caller need opposite things from an unparseable record: a SCAN (a listing) wants it
/// skipped, and a GUARD (before deleting a profile's Keychain entry, or removing an account) must know,
/// because "0 live" and "0 readable" are the same list and only the first is safe to act on. So the count is
/// returned rather than swallowed; [`list_sessions`] is the scan-shaped view that drops it.
///
/// A sessions directory that cannot be listed contributes exactly 1, which makes the count a lower bound;
/// callers only ask whether it is zero. A MISSING directory counts 0: a profile that has never been run holds
/// no records.
pub fn scan_sessions(claude_dir: &Path) -> (Vec<ClaudeSession>, usize) {
    scan_sessions_with(claude_dir, &SystemProbe, &is_pid_alive)
}

fn scan_sessions_with(
    claude_dir: &Path,
    probe: &dyn ProcessProbe,
    alive: &dyn Fn(i64) -> bool,
) -> (Vec<ClaudeSession>, usize) {
    let sessions_dir = claude_dir.join("sessions");
    let paths = match list_with_suffix(&sessions_dir, ".json") {
        Ok(paths) => paths,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return (Vec::new(), 0),
        Err(err) => {
            log::debug!(
                "Cannot list session directory {}: {err}",
                sessions_dir.display()
            );
            return (Vec::new(), 1);
        }
    };
    let mut sessions = Vec::new();
    let mut unreadable = 0;
    for path in paths {
        match read_session(&path, probe, alive) {
            Ok(Some(session)) => sessions.push(session),
            Ok(None) => {}
            Err(Unreadable(why)) => {
                unreadable += 1;
                log::debug!("Skipping session file {}: {why}", path.display());
            }
        }
    }
    (sessions, unreadable)
}

/// Entries of `dir` whose name ends with `suffix` (case-sensitive, like upstream's `match` on POSIX).
fn list_with_suffix(dir: &Path, suffix: &str) -> std::io::Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        if entry.file_name().to_string_lossy().ends_with(suffix) {
            paths.push(entry.path());
        }
    }
    Ok(paths)
}

/// Live sessions. A record that cannot be read is SKIPPED.
///
/// SCAN USE ONLY: the result cannot distinguish "no live sessions" from "no readable records", so anything
/// gating a destructive step must call [`scan_sessions`] and treat a non-zero count as live.
pub fn list_sessions(claude_dir: &Path) -> Vec<ClaudeSession> {
    scan_sessions(claude_dir).0
}

/// IDE lockfiles whose processes are alive.
pub fn list_ide_instances(claude_dir: &Path) -> Vec<IdeInstance> {
    list_ide_instances_with(claude_dir, &is_pid_alive)
}

fn list_ide_instances_with(claude_dir: &Path, alive: &dyn Fn(i64) -> bool) -> Vec<IdeInstance> {
    let ide_dir = claude_dir.join("ide");
    if !ide_dir.is_dir() {
        return Vec::new();
    }
    let Ok(paths) = list_with_suffix(&ide_dir, ".lock") else {
        return Vec::new();
    };
    paths
        .iter()
        .filter_map(|path| match read_ide_instance(path, alive) {
            Ok(instance) => instance,
            Err(Unreadable(why)) => {
                log::debug!("Skipping IDE lockfile {}: {why}", path.display());
                None
            }
        })
        .collect()
}

fn read_ide_instance(
    path: &Path,
    alive: &dyn Fn(i64) -> bool,
) -> Result<Option<IdeInstance>, Unreadable> {
    let data = read_object(path)?;
    let Some(pid) = data.get("pid").filter(|v| !v.is_null()) else {
        return Ok(None);
    };
    let Some(pid) = record_pid(pid)? else {
        return Ok(None);
    };
    if !alive(i64::from(pid)) {
        return Ok(None);
    }
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let port = stem
        .trim()
        .parse::<i64>()
        .map_err(|e| Unreadable(format!("port {stem:?}: {e}")))?;
    let workspace_folders = match data.get("workspaceFolders") {
        None => Vec::new(),
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(Value::as_str)
            .map(String::from)
            .collect(),
        Some(other) => {
            return Err(Unreadable(format!(
                "workspaceFolders {other} is not a list"
            )));
        }
    };
    Ok(Some(IdeInstance {
        port,
        pid,
        ide_name: data
            .get("ideName")
            .and_then(Value::as_str)
            .unwrap_or("Unknown IDE")
            .to_string(),
        workspace_folders,
    }))
}

/// All running Claude Code sessions and IDE instances under `claude_dir` (use
/// [`crate::paths::claude_config_home`] for the active profile).
pub fn get_running_instances(claude_dir: &Path) -> (Vec<ClaudeSession>, Vec<IdeInstance>) {
    (list_sessions(claude_dir), list_ide_instances(claude_dir))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    struct FakeProbe {
        ticks: Option<String>,
        started: Option<i64>,
        claude: Option<bool>,
    }

    impl ProcessProbe for FakeProbe {
        fn start_ticks(&self, _: i32) -> Option<String> {
            self.ticks.clone()
        }
        fn started_at(&self, _: i32) -> Option<i64> {
            self.started
        }
        fn is_claude(&self, _: i32) -> Option<bool> {
            self.claude
        }
    }

    const UNKNOWN: FakeProbe = FakeProbe {
        ticks: None,
        started: None,
        claude: None,
    };
    const LSTART: &str = "Wed Sep  2 20:35:59 2026";

    fn recorded() -> i64 {
        lstart_seconds(LSTART).unwrap()
    }

    fn alive_all(_: i64) -> bool {
        true
    }

    fn write_record(dir: &Path, name: &str, body: &str) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join(name), body).unwrap();
    }

    #[test]
    fn parent_watch_sees_a_live_parent() {
        // The test harness that started this process is still running.
        let watch = ParentWatch::start();
        assert!(!watch.is_gone());
        assert!(!watch.is_gone(), "polling twice does not change the answer");
    }

    #[test]
    fn pid_liveness() {
        assert!(is_pid_alive(i64::from(std::process::id())));
        assert!(!is_pid_alive(0));
        assert!(!is_pid_alive(1));
        assert!(!is_pid_alive(-5));
        assert!(!is_pid_alive(i64::from(i32::MAX)));
        assert!(!is_pid_alive(i64::MAX));
    }

    #[test]
    fn parses_ps_lstart_and_rejects_garbage() {
        assert_eq!(lstart_seconds(LSTART), Some(1_788_381_359));
        assert_eq!(lstart_seconds("Thu Jan  1 00:00:00 1970"), Some(0));
        for bad in [
            "",
            "garbage",
            "Wed Foo  2 20:35:59 2026",
            "Wed Sep  2 20:35 2026",
            "Wed Sep 2 x:1:2 2026",
        ] {
            assert_eq!(lstart_seconds(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn stat_start_ticks_reads_field_22_from_the_last_parenthesis() {
        let tail = "S 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 424242 20 21";
        assert_eq!(
            stat_start_ticks(&format!("123 (claude) {tail}")).as_deref(),
            Some("424242")
        );
        assert_eq!(
            stat_start_ticks(&format!("123 (a) (b c) {tail}")).as_deref(),
            Some("424242")
        );
        assert_eq!(stat_start_ticks("garbage"), None);
        assert_eq!(stat_start_ticks("1 (x) S 1 2"), None);
    }

    #[test]
    fn unreadable_proc_stat_is_unknowable() {
        assert_eq!(process_start_ticks(i32::MAX), None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn own_process_start_ticks_match_proc_self() {
        let own = std::fs::read_to_string("/proc/self/stat").unwrap();
        assert_eq!(
            process_start_ticks(std::process::id() as i32),
            stat_start_ticks(&own)
        );
    }

    #[cfg(unix)]
    #[test]
    fn own_process_started_in_the_past_and_unknown_pid_is_unknowable() {
        let now = jiff::Timestamp::now().as_second();
        let started = process_started_at(std::process::id() as i32).unwrap();
        assert!(started <= now && started > now - 86_400 * 365);
        assert_eq!(process_started_at(i32::MAX), None);
        assert_eq!(process_is_claude(i32::MAX), None);
    }

    #[test]
    fn unstamped_or_garbage_record_passes() {
        let stranger = FakeProbe {
            ticks: Some("1".into()),
            started: Some(i64::MAX / 2),
            claude: Some(false),
        };
        assert!(pid_matches_record_with(10, None, &stranger));
        assert!(pid_matches_record_with(10, Some(""), &stranger));
        assert!(pid_matches_record_with(10, Some("not a date"), &stranger));
    }

    #[test]
    fn start_ticks_decide_on_linux_stamps() {
        let probe = |t: &str| FakeProbe {
            ticks: Some(t.into()),
            ..UNKNOWN
        };
        assert!(pid_matches_record_with(10, Some("5000"), &probe("5000")));
        assert!(!pid_matches_record_with(10, Some("5000"), &probe("6000")));
        assert!(
            pid_matches_record_with(10, Some("5000"), &UNKNOWN),
            "unreadable ticks pass"
        );
    }

    #[test]
    fn lstart_stamps_disqualify_only_a_younger_stranger() {
        let at = |started: i64, claude: Option<bool>| FakeProbe {
            ticks: None,
            started: Some(started),
            claude,
        };
        assert!(
            pid_matches_record_with(10, Some(LSTART), &UNKNOWN),
            "unknowable start passes"
        );
        assert!(pid_matches_record_with(
            10,
            Some(LSTART),
            &at(recorded() - 50, Some(false))
        ));
        assert!(pid_matches_record_with(
            10,
            Some(LSTART),
            &at(recorded() + PID_REUSE_SLACK_S, Some(false))
        ));
        assert!(!pid_matches_record_with(
            10,
            Some(LSTART),
            &at(recorded() + 500, Some(false))
        ));
        assert!(
            pid_matches_record_with(10, Some(LSTART), &at(recorded() + 500, Some(true))),
            "a claude is kept"
        );
        assert!(
            pid_matches_record_with(10, Some(LSTART), &at(recorded() + 500, None)),
            "unknowable identity kept"
        );
    }

    fn session_json(pid: i64, extra: &str) -> String {
        format!(
            r#"{{"pid": {pid}, "sessionId": "s-{pid}", "cwd": "/work", "startedAt": 1700000000000,
                "kind": "interactive", "entrypoint": "cli"{extra}}}"#
        )
    }

    #[test]
    fn reads_valid_sessions_with_all_fields() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("sessions");
        write_record(
            &dir,
            "100.json",
            &session_json(100, r#", "status": "busy""#),
        );
        write_record(&dir, "101.json", &session_json(101, ""));
        let (mut sessions, unreadable) = scan_sessions_with(tmp.path(), &UNKNOWN, &alive_all);
        sessions.sort_by_key(|s| s.pid);
        assert_eq!(unreadable, 0);
        assert_eq!(
            sessions[0],
            ClaudeSession {
                pid: 100,
                session_id: "s-100".into(),
                cwd: "/work".into(),
                started_at: 1_700_000_000_000,
                kind: "interactive".into(),
                entrypoint: "cli".into(),
                status: Some("busy".into()),
            }
        );
        assert_eq!(sessions[1].status, None);
    }

    #[test]
    fn filters_dead_and_recycled_pids() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("sessions");
        write_record(&dir, "100.json", &session_json(100, ""));
        write_record(
            &dir,
            "200.json",
            &session_json(200, r#", "procStart": "5000""#),
        );
        let alive = |pid: i64| pid != 100;
        let recycled = FakeProbe {
            ticks: Some("6000".into()),
            ..UNKNOWN
        };
        assert_eq!(
            scan_sessions_with(tmp.path(), &recycled, &alive),
            (vec![], 0)
        );
        let same = FakeProbe {
            ticks: Some("5000".into()),
            ..UNKNOWN
        };
        let (sessions, _) = scan_sessions_with(tmp.path(), &same, &alive);
        assert_eq!(sessions.iter().map(|s| s.pid).collect::<Vec<_>>(), [200]);
    }

    #[test]
    fn real_scan_finds_this_process() {
        let tmp = TempDir::new().unwrap();
        let pid = i64::from(std::process::id());
        write_record(
            &tmp.path().join("sessions"),
            &format!("{pid}.json"),
            &session_json(pid, ""),
        );
        let (instances, ides) = get_running_instances(tmp.path());
        assert_eq!(instances.len(), 1);
        assert!(ides.is_empty());
    }

    #[test]
    fn corrupt_records_are_counted_not_listed() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("sessions");
        write_record(&dir, "1.json", "{not json");
        write_record(&dir, "2.json", r#"{"sessionId": "no pid"}"#);
        write_record(&dir, "3.json", "[1, 2]");
        write_record(&dir, "4.json", r#"{"pid": "123"}"#);
        write_record(&dir, "5.json", r#"{"pid": 99999999999999}"#);
        write_record(&dir, "6.json", r#"{"pid": 300, "procStart": 5}"#);
        write_record(
            &dir,
            "7.json",
            &format!("{}{}", "[".repeat(100_000), "]".repeat(100_000)),
        );
        std::fs::write(dir.join("8.json"), [0xff, 0xfe]).unwrap();
        write_record(&dir, "ignored.txt", "{not json");
        write_record(&dir, "9.json", r#"{"pid": 1}"#);
        let (sessions, unreadable) = scan_sessions_with(tmp.path(), &UNKNOWN, &alive_all);
        assert!(sessions.is_empty());
        assert_eq!(
            unreadable, 8,
            "pid 1 is merely not alive; .txt is not a record"
        );
        assert!(list_sessions(tmp.path()).is_empty());
    }

    #[test]
    fn a_missing_sessions_dir_is_provably_no_records() {
        let tmp = TempDir::new().unwrap();
        assert_eq!(scan_sessions(tmp.path()), (vec![], 0));
    }

    #[test]
    fn an_unlistable_sessions_dir_counts_as_unreadable() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("sessions"), "a file, not a dir").unwrap();
        assert_eq!(scan_sessions(tmp.path()), (vec![], 1));
    }

    #[test]
    fn record_name_casing_is_case_sensitive() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("sessions");
        write_record(&dir, "1.JSON", "{not json");
        assert_eq!(
            scan_sessions_with(tmp.path(), &UNKNOWN, &alive_all),
            (vec![], 0)
        );
    }

    #[test]
    fn reads_ide_lockfiles() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("ide");
        write_record(
            &dir,
            "12345.lock",
            r#"{"pid": 500, "ideName": "Cursor", "workspaceFolders": ["/a", "/b"]}"#,
        );
        write_record(&dir, "222.lock", r#"{"pid": 501}"#);
        write_record(&dir, "333.lock", r#"{"pid": 999}"#);
        write_record(&dir, "444.lock", r#"{"ideName": "no pid"}"#);
        write_record(&dir, "555.lock", "{corrupt");
        write_record(&dir, "notaport.lock", r#"{"pid": 502}"#);
        let alive = |pid: i64| pid != 999;
        let mut found = list_ide_instances_with(tmp.path(), &alive);
        found.sort_by_key(|i| i.port);
        assert_eq!(
            found,
            vec![
                IdeInstance {
                    port: 222,
                    pid: 501,
                    ide_name: "Unknown IDE".into(),
                    workspace_folders: vec![]
                },
                IdeInstance {
                    port: 12345,
                    pid: 500,
                    ide_name: "Cursor".into(),
                    workspace_folders: vec!["/a".into(), "/b".into()],
                },
            ]
        );
    }

    #[test]
    fn missing_ide_dir_is_empty() {
        let tmp = TempDir::new().unwrap();
        assert!(list_ide_instances(tmp.path()).is_empty());
        assert_eq!(get_running_instances(tmp.path()), (vec![], vec![]));
    }
}

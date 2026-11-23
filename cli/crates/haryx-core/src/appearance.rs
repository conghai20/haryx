//! Terminal appearance detection and theme resolution. Port of `claude_swap/appearance.py`.
//!
//! Determines whether the terminal has a light or dark background by querying it with OSC 11
//! (`ESC ] 11 ; ? BEL`) followed by DA1 (`ESC [ c`), reading through the ordered DA1 reply, and
//! classifying the preceding `rgb:…` reply by perceived luminance. Both the CLI printer and the
//! TUI resolve their theme through here.
//!
//! The query MUST happen while this process owns the terminal in cooked mode — before a TUI's
//! input driver starts — or the reply is reissued as keystrokes. Everything fails safe to `None`
//! (→ resolved dark): a terminal that doesn't answer, a pipe, or a parse failure never blocks
//! indefinitely and never errors.

use std::sync::OnceLock;
use std::time::{Duration, Instant};

pub use crate::printer::Theme;

#[cfg_attr(not(unix), allow(dead_code))]
const QUERY: &[u8] = b"\x1b]11;?\x07"; // OSC 11, BEL-terminated
#[cfg_attr(not(unix), allow(dead_code))]
const DA1_QUERY: &[u8] = b"\x1b[c"; // ordered response boundary
/// DA1 lets unsupported terminals return promptly; the cap mainly covers SSH latency or a
/// non-conforming terminal that answers neither query.
#[cfg_attr(not(unix), allow(dead_code))]
const TIMEOUT: Duration = Duration::from_secs(1);
const MAX_REPLY: usize = 256;
#[cfg_attr(not(unix), allow(dead_code))]
const READ_CHUNK: usize = 32;

// The full `ESC ]11;` opener is required so interleaved or echoed input (e.g. a shell echoing a
// pasted escape sequence) can't be misparsed as a background reply just because `]11;rgb:`
// appears somewhere without the ESC that actually starts an OSC sequence.
const RGB_OPENER: &[u8] = b"\x1b]11;rgb:";
const HEX_OPENER: &[u8] = b"\x1b]11;#";

/// Cache: the terminal background can't change within a process, so query once.
static CACHE: OnceLock<Option<Theme>> = OnceLock::new();

fn is_hex(b: u8) -> bool {
    b.is_ascii_hexdigit()
}

/// Length of the OSC string terminator (BEL or ST) at the start of `rest`, if any.
fn terminator_len(rest: &[u8]) -> Option<usize> {
    if rest.first() == Some(&0x07) {
        Some(1)
    } else if rest.starts_with(b"\x1b\\") {
        Some(2)
    } else {
        None
    }
}

/// Positions where `needle` starts in `hay`.
fn find_all<'a>(hay: &'a [u8], needle: &'a [u8]) -> impl Iterator<Item = usize> + 'a {
    (0..hay.len().saturating_sub(needle.len()) + 1)
        .filter(move |&i| hay.len() >= needle.len() && &hay[i..i + needle.len()] == needle)
}

/// One run of hex digits at the start of `s`: (digits, rest).
fn hex_run(s: &[u8]) -> Option<(&[u8], &[u8])> {
    let n = s.iter().take_while(|&&b| is_hex(b)).count();
    (n > 0).then(|| s.split_at(n))
}

/// A hex component normalised to 0..1 by its own width (`ff` → 1.0, `ffff` → 1.0).
fn normalise(digits: &[u8]) -> f64 {
    let value = digits.iter().fold(0.0_f64, |acc, &d| {
        acc * 16.0 + f64::from((d as char).to_digit(16).unwrap_or(0))
    });
    let max = 16_f64.powi(digits.len() as i32) - 1.0;
    value / max
}

fn parse_rgb_at(s: &[u8]) -> Option<(f64, f64, f64)> {
    let (r, rest) = hex_run(s)?;
    let rest = rest.strip_prefix(b"/")?;
    let (g, rest) = hex_run(rest)?;
    let rest = rest.strip_prefix(b"/")?;
    let (b, rest) = hex_run(rest)?;
    terminator_len(rest)?;
    Some((normalise(r), normalise(g), normalise(b)))
}

fn parse_hex_at(s: &[u8]) -> Option<(f64, f64, f64)> {
    if s.len() < 6 || !s[..6].iter().all(|&b| is_hex(b)) {
        return None;
    }
    terminator_len(&s[6..])?;
    let ch = |i: usize| normalise(&s[i..i + 2]);
    Some((ch(0), ch(2), ch(4)))
}

/// Parse an OSC 11 reply into (r, g, b), each normalised to 0..1.
pub fn parse_osc11(reply: &[u8]) -> Option<(f64, f64, f64)> {
    find_all(reply, RGB_OPENER)
        .find_map(|i| parse_rgb_at(&reply[i + RGB_OPENER.len()..]))
        .or_else(|| {
            find_all(reply, HEX_OPENER).find_map(|i| parse_hex_at(&reply[i + HEX_OPENER.len()..]))
        })
}

/// Light/dark from an OSC 11 reply, or `None` if unparseable.
pub fn classify(reply: &[u8]) -> Option<Theme> {
    let (r, g, b) = parse_osc11(reply)?;
    // BT.709-weighted brightness on gamma-encoded channels (approx.).
    let luminance = 0.2126 * r + 0.7152 * g + 0.0722 * b;
    Some(if luminance > 0.5 {
        Theme::Light
    } else {
        Theme::Dark
    })
}

/// Whether `buf` holds a primary device-attributes reply: `CSI ? Ps c` (CSI as `ESC [` or 0x9b).
/// Requiring `?` keeps an echoed DA1 query (`CSI c`) from looking complete; Ps may be empty.
pub fn has_da1_reply(buf: &[u8]) -> bool {
    (0..buf.len()).any(|i| {
        let rest = if buf[i..].starts_with(b"\x1b[") {
            &buf[i + 2..]
        } else if buf[i] == 0x9b {
            &buf[i + 1..]
        } else {
            return false;
        };
        let Some(rest) = rest.strip_prefix(b"?") else {
            return false;
        };
        let params = rest
            .iter()
            .take_while(|&&b| b.is_ascii_digit() || b == b';' || b == b':');
        rest.get(params.count()) == Some(&b'c')
    })
}

/// Read a reply until the DA1 boundary, the size cap, the deadline, or EOF.
///
/// `read_chunk(remaining)` waits at most `remaining` and returns `None` when nothing arrived (or on
/// error), `Some(empty)` at EOF. Isolated so tests can feed canned, fragmented replies. Terminals
/// answer in order, so the DA1 reply proves no colour-query bytes are still in flight — the loop
/// does not stop at the OSC reply.
pub fn read_reply(
    mut read_chunk: impl FnMut(Duration) -> Option<Vec<u8>>,
    timeout: Duration,
) -> Option<Vec<u8>> {
    let deadline = Instant::now() + timeout;
    let mut buf = Vec::new();
    while buf.len() < MAX_REPLY {
        let now = Instant::now();
        if now >= deadline {
            break;
        }
        let Some(chunk) = read_chunk(deadline - now) else {
            break;
        };
        if chunk.is_empty() {
            break;
        }
        buf.extend_from_slice(&chunk);
        if has_da1_reply(&buf) {
            break;
        }
    }
    (!buf.is_empty()).then_some(buf)
}

/// Whether the environment rules out the query: `TERM=dumb` and the Linux console don't support
/// it, and tmux/screen don't pass it through to the outer terminal by default.
pub fn query_unsupported(env: impl Fn(&str) -> Option<String>) -> bool {
    matches!(env("TERM").as_deref(), Some("dumb" | "linux"))
        || env("TMUX").is_some_and(|v| !v.is_empty())
        || env("STY").is_some_and(|v| !v.is_empty())
}

#[cfg_attr(not(unix), allow(dead_code))]
fn process_env(name: &str) -> Option<String> {
    std::env::var_os(name).map(|v| v.to_string_lossy().into_owned())
}

#[cfg(unix)]
mod tty {
    use super::*;
    use std::io::{IsTerminal, Write};

    /// Send OSC 11 + DA1 and read through the DA1 reply. `None` on any failure.
    pub(super) fn query_terminal_background() -> Option<Vec<u8>> {
        if query_unsupported(process_env) {
            return None;
        }
        if !(std::io::stdin().is_terminal() && std::io::stdout().is_terminal()) {
            return None;
        }
        let fd = libc::STDIN_FILENO;
        // SAFETY: termios is plain old data; tcgetattr fills it or fails.
        let mut old: libc::termios = unsafe { std::mem::zeroed() };
        if unsafe { libc::tcgetattr(fd, &mut old) } != 0 {
            return None;
        }
        let mut raw = old;
        // cbreak, as Python's tty.setcbreak: no echo, no line buffering, byte-at-a-time reads.
        raw.c_lflag &= !(libc::ECHO | libc::ICANON);
        raw.c_cc[libc::VMIN] = 1;
        raw.c_cc[libc::VTIME] = 0;
        // TCSANOW (not TCSADRAIN): draining first can block under terminal flow control, and
        // there's no pending output to drain anyway.
        // SAFETY: `raw` is a valid termios derived from the one the kernel returned.
        if unsafe { libc::tcsetattr(fd, libc::TCSANOW, &raw) } != 0 {
            return None;
        }
        let reply = write_and_read(fd);
        // SAFETY: restoring the attributes read above.
        unsafe { libc::tcsetattr(fd, libc::TCSANOW, &old) };
        reply
    }

    fn write_and_read(fd: libc::c_int) -> Option<Vec<u8>> {
        let mut out = std::io::stdout().lock();
        out.write_all(QUERY).ok()?;
        out.write_all(DA1_QUERY).ok()?;
        out.flush().ok()?;
        drop(out);
        read_reply(|remaining| read_ready(fd, remaining), TIMEOUT)
    }

    fn read_ready(fd: libc::c_int, remaining: Duration) -> Option<Vec<u8>> {
        let mut pfd = libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        };
        let ms = remaining.as_millis().min(i32::MAX as u128) as libc::c_int;
        // SAFETY: one valid pollfd.
        let ready = unsafe { libc::poll(&mut pfd, 1, ms) };
        if ready <= 0 {
            return None;
        }
        let mut chunk = [0_u8; READ_CHUNK];
        // SAFETY: reading into a stack buffer of the stated length.
        let n = unsafe { libc::read(fd, chunk.as_mut_ptr().cast(), chunk.len()) };
        if n < 0 {
            return None;
        }
        Some(chunk[..n as usize].to_vec())
    }

    /// Discard pending terminal input (e.g. a late OSC reply). Best-effort.
    pub(super) fn drain_stdin() {
        if !std::io::stdin().is_terminal() {
            return;
        }
        // SAFETY: tcflush on stdin has no memory-safety preconditions.
        unsafe { libc::tcflush(libc::STDIN_FILENO, libc::TCIFLUSH) };
    }
}

#[cfg(not(unix))]
mod tty {
    // TODO(windows): upstream returns None on Windows too; nothing to port.
    pub(super) fn query_terminal_background() -> Option<Vec<u8>> {
        None
    }
    pub(super) fn drain_stdin() {}
}

/// Light or dark from the terminal background, or `None` if undetectable.
///
/// Cached per process. MUST be first called in cooked mode (before a TUI starts).
pub fn detect_terminal_background() -> Option<Theme> {
    *CACHE.get_or_init(|| tty::query_terminal_background().and_then(|reply| classify(&reply)))
}

/// Resolve a `ui.theme` setting to a concrete theme.
///
/// `dark`/`light` pass through without probing; `auto` (or anything else) follows `detect()`,
/// falling back to dark when detection yields `None`.
pub fn resolve_theme(setting: &str, detect: impl FnOnce() -> Option<Theme>) -> Theme {
    Theme::from_name(setting).unwrap_or_else(|| detect().unwrap_or(Theme::Dark))
}

/// Whether the CLI should probe the terminal background before dispatch.
///
/// False when colours are off (nothing will render the theme anyway), when the first token is
/// `run` (execs a child that takes over the terminal), or when `--json` is present (the OSC query
/// must never precede machine-readable output on stdout).
pub fn cli_should_probe(argv: &[String], colors_enabled: bool) -> bool {
    if !colors_enabled {
        return false;
    }
    if argv.first().is_some_and(|a| a == "run") {
        return false;
    }
    !argv.iter().any(|a| a == "--json")
}

/// Resolve a theme for a plain-CLI invocation: probe only when colour will actually be emitted;
/// otherwise `auto` degrades to dark without a tty query.
pub fn cli_theme(setting: &str, detect: impl FnOnce() -> Option<Theme>, colors: bool) -> Theme {
    if setting == "auto" && !colors {
        return Theme::Dark;
    }
    resolve_theme(setting, detect)
}

/// Discard any pending terminal input (e.g. a late OSC reply) so it isn't reissued as keystrokes
/// once a TUI takes over. Best-effort; POSIX only.
///
/// A reply arriving after the detection deadline and after this drain can still reach the app as
/// stray keystrokes — inherent to any finite-timeout OSC probe.
pub fn drain_stdin() {
    tty::drain_stdin();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, VecDeque};

    fn approx(a: (f64, f64, f64), b: (f64, f64, f64)) -> bool {
        (a.0 - b.0).abs() < 1e-9 && (a.1 - b.1).abs() < 1e-9 && (a.2 - b.2).abs() < 1e-9
    }

    #[test]
    fn parses_16bit_rgb() {
        let rgb = parse_osc11(b"\x1b]11;rgb:ffff/ffff/ffff\x07").unwrap();
        assert!(approx(rgb, (1.0, 1.0, 1.0)));
    }

    #[test]
    fn parses_8bit_rgb() {
        let rgb = parse_osc11(b"\x1b]11;rgb:00/80/ff\x1b\\").unwrap();
        assert!(approx(rgb, (0.0, 128.0 / 255.0, 1.0)));
    }

    #[test]
    fn parses_hex() {
        let rgb = parse_osc11(b"\x1b]11;#ff0000\x07").unwrap();
        assert!(approx(rgb, (1.0, 0.0, 0.0)));
    }

    #[test]
    fn junk_and_unframed_replies_are_rejected() {
        assert_eq!(parse_osc11(b"garbage"), None);
        assert_eq!(parse_osc11(b"rgb:ffff/ffff/ffff"), None);
        assert_eq!(parse_osc11(b"]11;rgb:ffff/ffff/ffff\x07"), None);
        assert_eq!(parse_osc11(b"\x1b]11;rgb:ffff/ffff/ffff"), None);
        assert_eq!(parse_osc11(b"#ffffff"), None);
        assert_eq!(parse_osc11(b"\x1b]11;#ffffff"), None);
        assert!(parse_osc11(b"\x1b]11;#ffffff\x1b\\").is_some());
    }

    #[test]
    fn classifies() {
        assert_eq!(
            classify(b"\x1b]11;rgb:ffff/ffff/ffff\x07"),
            Some(Theme::Light)
        );
        assert_eq!(
            classify(b"\x1b]11;rgb:0000/0000/0000\x07"),
            Some(Theme::Dark)
        );
        // Pinned cutoff: luminance must be strictly above 0.5.
        assert_eq!(classify(b"\x1b]11;#808080\x07"), Some(Theme::Light));
        assert_eq!(classify(b"\x1b]11;#7f7f7f\x07"), Some(Theme::Dark));
        assert_eq!(classify(b"nope"), None);
    }

    fn reader(chunks: &[&[u8]]) -> impl FnMut(Duration) -> Option<Vec<u8>> {
        let mut queue: VecDeque<Vec<u8>> = chunks.iter().map(|c| c.to_vec()).collect();
        move |_| queue.pop_front()
    }

    #[test]
    fn waits_for_da1_after_complete_osc_reply() {
        let osc: &[u8] = b"\x1b]11;rgb:ffff/ffff/ffff\x07";
        let da1: &[u8] = b"\x1b[?62;22c";
        let reply = read_reply(reader(&[osc, da1, b"EXTRA"]), TIMEOUT).unwrap();
        assert_eq!(reply, [osc, da1].concat());
    }

    #[test]
    fn da1_first_means_osc11_unsupported() {
        let reply = read_reply(reader(&[b"\x1b[?1;2c"]), TIMEOUT).unwrap();
        assert_eq!(classify(&reply), None);
    }

    #[test]
    fn da1_reply_forms() {
        assert!(has_da1_reply(b"\x1b[?c"));
        assert!(has_da1_reply(b"\x9b?62c"));
        // An echoed DA1 query is not a reply.
        assert!(!has_da1_reply(b"\x1b[c"));
    }

    #[test]
    fn accepts_fragmented_da1_reply() {
        let chunks: &[&[u8]] = &[b"\x1b]11;rgb:0/0/0\x07\x1b[", b"?6", b"2c", b"tail"];
        let reply = read_reply(reader(chunks), TIMEOUT).unwrap();
        assert!(reply.ends_with(b"62c"));
        assert_eq!(classify(&reply), Some(Theme::Dark));
    }

    #[test]
    fn nothing_read_is_none() {
        assert_eq!(read_reply(reader(&[]), TIMEOUT), None);
    }

    #[test]
    fn resolve_theme_rules() {
        let never = || -> Option<Theme> { panic!("must not detect") };
        assert_eq!(resolve_theme("dark", never), Theme::Dark);
        assert_eq!(resolve_theme("light", never), Theme::Light);
        assert_eq!(resolve_theme("auto", || Some(Theme::Light)), Theme::Light);
        assert_eq!(resolve_theme("auto", || None), Theme::Dark);
    }

    #[test]
    fn cli_theme_skips_detection_without_colors() {
        let never = || -> Option<Theme> { panic!("must not detect") };
        assert_eq!(cli_theme("auto", never, false), Theme::Dark);
        assert_eq!(cli_theme("auto", || Some(Theme::Light), true), Theme::Light);
        assert_eq!(cli_theme("light", never, true), Theme::Light);
    }

    #[test]
    fn cli_should_probe_rules() {
        let argv = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(!cli_should_probe(&argv(&["run", "work"]), true));
        assert!(!cli_should_probe(&argv(&["list", "--json"]), true));
        assert!(!cli_should_probe(&argv(&["list"]), false));
        assert!(cli_should_probe(&argv(&["list"]), true));
    }

    #[test]
    fn query_short_circuits_on_unsupported_environments() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            let map: HashMap<&str, &str> = pairs.iter().copied().collect();
            move |n: &str| map.get(n).map(|v| v.to_string())
        };
        assert!(query_unsupported(env(&[(
            "TMUX",
            "/tmp/tmux-1/default,1,0"
        )])));
        assert!(query_unsupported(env(&[("STY", "1.pts")])));
        assert!(query_unsupported(env(&[("TERM", "dumb")])));
        assert!(query_unsupported(env(&[("TERM", "linux")])));
        assert!(!query_unsupported(env(&[("TERM", "xterm-256color")])));
    }
}

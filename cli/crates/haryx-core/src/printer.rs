//! Console output formatting. Port of `claude_swap/printer.py`.
//!
//! Subtle terminal styling with a single warm accent colour, dim secondary text, and bold for
//! structure. Falls back to plain text when colours aren't supported (`NO_COLOR`, a pipe,
//! `TERM=dumb`); `FORCE_COLOR` turns them on regardless.
//!
//! Upstream's `force_utf8_output` has no counterpart: Rust writes UTF-8 bytes unconditionally.

use std::io::{IsTerminal, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU8, Ordering};

// ANSI escape codes -- structural, theme-independent.
const RESET: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";

/// The colours of one palette.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Palette {
    accent: &'static str,
    muted: &'static str,
    red: &'static str,
    yellow: &'static str,
}

const DARK: Palette = Palette {
    accent: "\x1b[38;5;173m", // warm salmon/terracotta (xterm 173)
    muted: "\x1b[38;5;250m",  // soft gray -- readable, but quieter than normal
    red: "\x1b[31m",
    yellow: "\x1b[33m",
};

const LIGHT: Palette = Palette {
    accent: "\x1b[38;2;149;76;42m", // #954c2a burnt sienna
    muted: "\x1b[38;2;99;93;85m",   // #635d55
    red: "\x1b[38;2;173;49;40m",    // #ad3128
    yellow: "\x1b[38;2;121;89;17m", // #795911 deep ochre
};

/// A concrete CLI colour palette.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Theme {
    #[default]
    Dark,
    Light,
}

impl Theme {
    /// `"dark"` / `"light"`; anything else is `None`.
    pub fn from_name(name: &str) -> Option<Theme> {
        match name {
            "dark" => Some(Theme::Dark),
            "light" => Some(Theme::Light),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Theme::Dark => "dark",
            Theme::Light => "light",
        }
    }

    fn palette(self) -> Palette {
        match self {
            Theme::Dark => DARK,
            Theme::Light => LIGHT,
        }
    }
}

const THEME_DARK: u8 = 0;
const THEME_LIGHT: u8 = 1;
static THEME: AtomicU8 = AtomicU8::new(THEME_DARK);

const COLORS_UNSET: u8 = 0;
const COLORS_OFF: u8 = 1;
const COLORS_ON: u8 = 2;
static COLORS: AtomicU8 = AtomicU8::new(COLORS_UNSET);

/// Select the CLI colour palette by name. Unknown names fall back to dark.
pub fn set_theme(name: &str) {
    set_theme_to(Theme::from_name(name).unwrap_or(Theme::Dark));
}

/// Select the CLI colour palette.
pub fn set_theme_to(theme: Theme) {
    let raw = match theme {
        Theme::Dark => THEME_DARK,
        Theme::Light => THEME_LIGHT,
    };
    THEME.store(raw, Ordering::Relaxed);
}

/// The palette currently in use.
pub fn theme() -> Theme {
    if THEME.load(Ordering::Relaxed) == THEME_LIGHT {
        Theme::Light
    } else {
        Theme::Dark
    }
}

fn pal() -> Palette {
    theme().palette()
}

/// Colour detection over an explicit environment, so it can be tested without touching the
/// process environment. `env(name)` returns the variable's value when it is set (even empty).
pub fn detect_color_support_with(
    env: impl Fn(&str) -> Option<String>,
    stdout_is_tty: bool,
) -> bool {
    // Respect the NO_COLOR convention (https://no-color.org/): presence alone disables.
    if env("NO_COLOR").is_some() {
        return false;
    }
    // Respect FORCE_COLOR for CI/testing.
    if env("FORCE_COLOR").is_some() {
        return true;
    }
    // Not a TTY (piped output) -> no colour.
    if !stdout_is_tty {
        return false;
    }
    // TODO(windows): upstream `_enable_windows_vt` enables VT processing on the console here.
    env("TERM").as_deref() != Some("dumb")
}

fn detect_color_support() -> bool {
    detect_color_support_with(
        |name| std::env::var_os(name).map(|v| v.to_string_lossy().into_owned()),
        std::io::stdout().is_terminal(),
    )
}

/// Whether colour output is active. Detected on the first call and cached for the process.
pub fn colors_enabled() -> bool {
    match COLORS.load(Ordering::Relaxed) {
        COLORS_ON => true,
        COLORS_OFF => false,
        _ => {
            let on = detect_color_support();
            COLORS.store(if on { COLORS_ON } else { COLORS_OFF }, Ordering::Relaxed);
            on
        }
    }
}

/// Override the cached colour decision (`None` forgets it so the next call re-detects).
pub fn set_colors_enabled(value: Option<bool>) {
    let raw = match value {
        None => COLORS_UNSET,
        Some(true) => COLORS_ON,
        Some(false) => COLORS_OFF,
    };
    COLORS.store(raw, Ordering::Relaxed);
}

/// Guard returned by [`force_color`]; restores the prior colour cache when dropped.
#[must_use = "colour is forced only while the guard lives"]
pub struct ForceColorGuard {
    saved: u8,
}

impl Drop for ForceColorGuard {
    fn drop(&mut self) {
        COLORS.store(self.saved, Ordering::Relaxed);
    }
}

/// Temporarily force coloured output on, restoring the prior cache when the guard drops.
///
/// Used by the TUI when capturing CLI output into a buffer: the capture is not a TTY, which
/// would otherwise disable colour, but the TUI re-renders the ANSI itself and wants the codes.
pub fn force_color() -> ForceColorGuard {
    let saved = COLORS.swap(COLORS_ON, Ordering::Relaxed);
    ForceColorGuard { saved }
}

fn style(text: &str, codes: &[&str]) -> String {
    if !colors_enabled() {
        return text.to_string();
    }
    format!("{}{text}{RESET}", codes.concat())
}

// --- Inline stylers (return styled strings for composing lines) ---

/// Warm accent colour for important elements.
pub fn accent(text: &str) -> String {
    style(text, &[pal().accent])
}

/// Slightly dimmer than normal -- for usage stats, org tags.
pub fn muted(text: &str) -> String {
    style(text, &[pal().muted])
}

/// Dim for tertiary info -- tree connectors, hints.
pub fn dimmed(text: &str) -> String {
    style(text, &[DIM])
}

/// Bold (no colour) for structure.
pub fn bolded(text: &str) -> String {
    style(text, &[BOLD])
}

/// Bold + accent for key markers like `(active)`.
pub fn bold_accent(text: &str) -> String {
    style(text, &[BOLD, pal().accent])
}

/// Yellow for warning-toned text (string form; [`warning`] prints).
pub fn yellowed(text: &str) -> String {
    style(text, &[pal().yellow])
}

/// Red, the error colour (string form; [`error`] prints).
pub fn reddened(text: &str) -> String {
    style(text, &[pal().red])
}

// --- Line printers ---

/// Print an error message (red) to stderr.
pub fn error(msg: &str) {
    write_line(&mut std::io::stderr().lock(), &reddened(msg));
}

/// Print a warning message (yellow) to stdout.
pub fn warning(msg: &str) {
    write_line(&mut std::io::stdout().lock(), &yellowed(msg));
}

/// Print a warning message (yellow) to stderr: for anything a long-running process emits (stderr
/// stays line-buffered when redirected to a file) and for anything printed while `--json` owns
/// stdout.
pub fn warning_stderr(msg: &str) {
    write_line(&mut std::io::stderr().lock(), &yellowed(msg));
}

/// Write one line; a closed stream (EPIPE) must never panic the CLI, so write errors are dropped.
fn write_line(out: &mut impl Write, line: &str) {
    let _ = writeln!(out, "{line}");
}

// --- Display helpers for process detection ---

/// A human-readable label for a Claude Code entrypoint; unknown ones pass through.
pub fn entrypoint_label(entrypoint: &str) -> &str {
    match entrypoint {
        "cli" => "CLI",
        "claude-vscode" => "VS Code",
        "claude-desktop" => "Desktop",
        "sdk-cli" | "sdk-ts" | "sdk-py" => "SDK",
        "mcp" => "MCP",
        "local-agent" => "Agent",
        "remote" => "Remote",
        other => other,
    }
}

/// A short display name for an IDE; unknown ones pass through.
pub fn ide_short_name(ide_name: &str) -> &str {
    match ide_name {
        "Visual Studio Code" => "VS Code",
        other => other,
    }
}

/// Replace the user's home directory prefix with `~`.
pub fn abbreviate_path(path: &str) -> String {
    match dirs::home_dir() {
        Some(home) => abbreviate_path_with(path, &home),
        None => path.to_string(),
    }
}

/// [`abbreviate_path`] against an explicit home directory. Like upstream, a plain string-prefix
/// test (so `/home/al` also abbreviates `/home/alice`).
pub fn abbreviate_path_with(path: &str, home: &Path) -> String {
    let home = home.to_string_lossy();
    match path.strip_prefix(home.as_ref()) {
        Some(rest) => format!("~{rest}"),
        None => path.to_string(),
    }
}

/// Format a millisecond epoch timestamp as a human-readable age.
pub fn format_age(started_at_ms: i64) -> String {
    format_age_at(started_at_ms, jiff::Timestamp::now().as_second())
}

/// [`format_age`] against an explicit "now" (epoch seconds).
pub fn format_age_at(started_at_ms: i64, now_s: i64) -> String {
    let elapsed = now_s - started_at_ms.div_euclid(1000);
    if elapsed < 60 {
        "just now".to_string()
    } else if elapsed < 3600 {
        format!("{}m ago", elapsed / 60)
    } else if elapsed < 86400 {
        format!("{}h ago", elapsed / 3600)
    } else {
        format!("{}d ago", elapsed / 86400)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    /// The colour cache and theme are process globals; tests touching them run one at a time.
    static GLOBALS: Mutex<()> = Mutex::new(());

    fn env_of(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |name| map.get(name).cloned()
    }

    #[test]
    fn no_color_env_disables() {
        assert!(!detect_color_support_with(
            env_of(&[("NO_COLOR", "1")]),
            true
        ));
    }

    #[test]
    fn no_color_empty_value_disables() {
        assert!(!detect_color_support_with(
            env_of(&[("NO_COLOR", "")]),
            true
        ));
    }

    #[test]
    fn no_color_beats_force_color() {
        let env = env_of(&[("NO_COLOR", "1"), ("FORCE_COLOR", "1")]);
        assert!(!detect_color_support_with(env, true));
    }

    #[test]
    fn force_color_enables_even_when_piped() {
        assert!(detect_color_support_with(
            env_of(&[("FORCE_COLOR", "1")]),
            false
        ));
    }

    #[test]
    fn non_tty_disables() {
        assert!(!detect_color_support_with(env_of(&[]), false));
    }

    #[test]
    fn dumb_term_disables() {
        assert!(!detect_color_support_with(
            env_of(&[("TERM", "dumb")]),
            true
        ));
        assert!(detect_color_support_with(
            env_of(&[("TERM", "xterm-256color")]),
            true
        ));
    }

    #[test]
    fn styling_with_colors_disabled_is_plain() {
        let _g = GLOBALS.lock().unwrap_or_else(|e| e.into_inner());
        set_colors_enabled(Some(false));
        assert_eq!(accent("x"), "x");
        assert_eq!(muted("x"), "x");
        assert_eq!(bolded("x"), "x");
        set_colors_enabled(None);
    }

    #[test]
    fn styling_with_colors_enabled() {
        let _g = GLOBALS.lock().unwrap_or_else(|e| e.into_inner());
        set_colors_enabled(Some(true));
        set_theme("dark");
        assert_eq!(accent("hi"), "\x1b[38;5;173mhi\x1b[0m");
        assert_eq!(muted("hi"), "\x1b[38;5;250mhi\x1b[0m");
        assert_eq!(dimmed("hi"), "\x1b[2mhi\x1b[0m");
        assert_eq!(bolded("hi"), "\x1b[1mhi\x1b[0m");
        assert_eq!(bold_accent("hi"), "\x1b[1m\x1b[38;5;173mhi\x1b[0m");
        assert_eq!(yellowed("hi"), "\x1b[33mhi\x1b[0m");
        assert_eq!(reddened("hi"), "\x1b[31mhi\x1b[0m");
        set_colors_enabled(None);
    }

    #[test]
    fn light_theme_changes_palette_and_unknown_falls_back_to_dark() {
        let _g = GLOBALS.lock().unwrap_or_else(|e| e.into_inner());
        set_colors_enabled(Some(true));
        set_theme("light");
        assert_eq!(accent("a"), "\x1b[38;2;149;76;42ma\x1b[0m");
        assert_eq!(reddened("a"), "\x1b[38;2;173;49;40ma\x1b[0m");
        set_theme("solarized");
        assert_eq!(theme(), Theme::Dark);
        assert_eq!(accent("a"), "\x1b[38;5;173ma\x1b[0m");
        set_colors_enabled(None);
    }

    #[test]
    fn force_color_overrides_and_restores() {
        let _g = GLOBALS.lock().unwrap_or_else(|e| e.into_inner());
        set_colors_enabled(Some(false));
        {
            let _forced = force_color();
            assert!(colors_enabled());
            assert_ne!(bolded("x"), "x");
        }
        assert!(!colors_enabled());
        set_colors_enabled(None);
    }

    #[test]
    fn write_line_appends_newline() {
        let mut buf = Vec::new();
        write_line(&mut buf, "boom");
        assert_eq!(buf, b"boom\n");
    }

    #[test]
    fn display_labels() {
        assert_eq!(entrypoint_label("claude-vscode"), "VS Code");
        assert_eq!(entrypoint_label("sdk-py"), "SDK");
        assert_eq!(entrypoint_label("weird"), "weird");
        assert_eq!(ide_short_name("Visual Studio Code"), "VS Code");
        assert_eq!(ide_short_name("Zed"), "Zed");
    }

    #[test]
    fn abbreviates_home_prefix() {
        let home = Path::new("/home/al");
        assert_eq!(abbreviate_path_with("/home/al/proj", home), "~/proj");
        assert_eq!(abbreviate_path_with("/srv/x", home), "/srv/x");
    }

    #[test]
    fn ages() {
        let now = 1_000_000;
        assert_eq!(format_age_at(now * 1000 - 30_000, now), "just now");
        assert_eq!(format_age_at((now - 120) * 1000, now), "2m ago");
        assert_eq!(format_age_at((now - 7200) * 1000, now), "2h ago");
        assert_eq!(format_age_at((now - 3 * 86400) * 1000, now), "3d ago");
    }
}

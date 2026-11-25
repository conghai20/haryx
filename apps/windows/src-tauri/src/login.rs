//! In-app sign-in: `haryx ai login [slot]` runs on a pseudo-terminal the app owns (ConPTY on Windows)
//! and its own window shows the sign-in link, the live status and a field for the code the sign-in
//! page shows. The macOS app does the same (`LoginSession.swift`, `LoginTranscript.swift`).
//!
//! Claude Code's sign-in reads the pasted code in raw mode, so pipes do not work: it needs a terminal.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent};

use crate::state::AppState;

/// Wide enough that the sign-in link is never wrapped across lines.
const COLUMNS: u16 = 1000;
const ROWS: u16 = 50;
/// Time for the last output to drain after the child exits, before judging it.
const DRAIN: Duration = Duration::from_millis(300);
/// Raw output kept per run. Sign-in prints a few lines; this only bounds a run left spinning.
const MAX_RAW: usize = 256 * 1024;

// ----------------------------------------------------------------------------------- transcript

/// What a run printed, cleaned for display, plus what the sign-in window needs from it.
///
/// Output arrives in arbitrary chunks (an escape sequence or a UTF-8 character can be split across
/// two), so the raw bytes are kept and the whole transcript is cleaned again on every append.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Transcript {
    raw: Vec<u8>,
    text: String,
}

impl Transcript {
    pub fn append(&mut self, chunk: &[u8]) {
        self.raw.extend_from_slice(chunk);
        if self.raw.len() > MAX_RAW {
            self.raw.drain(..self.raw.len() - MAX_RAW / 2);
        }
        self.text = clean(&String::from_utf8_lossy(&self.raw));
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    /// The last sign-in link printed (`claude auth login` prints one when the browser may not open).
    pub fn url(&self) -> Option<&str> {
        let start = self.text.rfind("https://")?;
        let rest = &self.text[start..];
        let end = rest
            .find(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '"' | '\''))
            .unwrap_or(rest.len());
        (end > "https://".len()).then(|| &rest[..end])
    }

    /// Claude Code asks for the code the sign-in page shows when it cannot receive it by itself.
    pub fn awaiting_code(&self) -> bool {
        self.last_line().to_lowercase().contains("paste code")
    }

    /// A yes/no question (haryx asks before replacing an occupied slot).
    pub fn awaiting_answer(&self) -> bool {
        self.last_line().contains("[y/N]")
    }

    /// The last non-empty line, trimmed.
    pub fn last_line(&self) -> &str {
        self.text
            .split('\n')
            .rfind(|l| !l.is_empty())
            .map_or("", |l| l.trim())
    }

    fn lines(&self) -> impl DoubleEndedIterator<Item = &str> {
        self.text
            .split('\n')
            .map(str::trim)
            .filter(|l| !l.is_empty())
    }

    /// The line to show when the run failed: haryx's `Error:` line, else the last line printed.
    pub fn failure_message(&self) -> String {
        if let Some(error) = self.lines().rev().find(|l| l.starts_with("Error:")) {
            return error["Error:".len()..].trim().to_string();
        }
        self.lines()
            .next_back()
            .unwrap_or("Sign-in did not finish.")
            .to_string()
    }

    /// The line haryx printed on success ("Signed in Account 3: …" / "Added Account 4: …").
    pub fn success_message(&self) -> Option<&str> {
        self.lines().rev().find(|l| {
            l.starts_with("Signed in")
                || l.starts_with("Added")
                || l.starts_with("Updated credentials")
        })
    }
}

/// Terminal output → plain text: escape sequences dropped (OSC hyperlinks keep their visible text),
/// CRLF → LF, a bare CR overwrites its line, other control characters dropped.
///
/// ConPTY repaints rather than replays, so two of its habits are kept as text: a cursor jump to
/// another row (`CSI row;col H`) ends the line, and a cursor-forward (`CSI n C`) stands for spaces.
pub fn clean(raw: &str) -> String {
    let chars: Vec<char> = raw.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        i += 1;
        match c {
            '\u{1b}' => i = skip_escape(&chars, i, &mut out),
            '\r' => {
                // A terminal sends "\r\r\n" (output post-processing on "\r\n"): runs of CR are one.
                while chars.get(i) == Some(&'\r') {
                    i += 1;
                }
                match chars.get(i) {
                    None => {}
                    Some('\n') => {
                        out.push('\n');
                        i += 1;
                    }
                    // A bare CR returns to the line start: what follows overwrites the line.
                    Some(_) => out.truncate(out.rfind('\n').map_or(0, |n| n + 1)),
                }
            }
            '\n' | '\t' => out.push(c),
            c if c >= ' ' && c != '\u{7f}' => out.push(c),
            _ => {}
        }
    }
    out
}

/// Skip one escape sequence that starts at `chars[i]` (just after ESC): CSI (`ESC [ … final`), OSC
/// (`ESC ] … BEL|ESC \`), or a two-character escape. Returns the index after it.
fn skip_escape(chars: &[char], mut i: usize, out: &mut String) -> usize {
    let Some(&kind) = chars.get(i) else {
        return i;
    };
    i += 1;
    match kind {
        '[' => {
            let start = i;
            while let Some(&c) = chars.get(i) {
                i += 1;
                if ('\u{40}'..='\u{7e}').contains(&c) {
                    let params: String = chars[start..i - 1].iter().collect();
                    csi_as_text(c, &params, out);
                    break;
                }
            }
        }
        ']' => {
            while let Some(&c) = chars.get(i) {
                i += 1;
                if c == '\u{07}' {
                    break;
                }
                if c == '\u{1b}' {
                    i += 1;
                    break;
                }
            }
        }
        _ => {}
    }
    i
}

/// The few CSI sequences that stand for text when ConPTY writes them.
fn csi_as_text(final_char: char, params: &str, out: &mut String) {
    match final_char {
        'C' => {
            let n = params
                .parse::<usize>()
                .unwrap_or(1)
                .clamp(1, usize::from(COLUMNS));
            out.extend(std::iter::repeat_n(' ', n));
        }
        'H' | 'f' if !out.is_empty() && !out.ends_with('\n') => out.push('\n'),
        _ => {}
    }
}

// ------------------------------------------------------------------------------------- session

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

/// One run's state, driven by its output and its exit. The outcome's message is read from the
/// transcript each time, so output that drains after the exit still reaches it.
#[derive(Debug, Clone)]
pub struct Session {
    pub title: String,
    pub slot: Option<u32>,
    pub transcript: Transcript,
    pub phase: Phase,
}

/// What the sign-in window draws (`LoginView` in `src/lib/types.ts`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct View {
    pub id: u64,
    pub title: String,
    /// `running`, `succeeded`, `failed` or `cancelled`.
    pub state: &'static str,
    pub message: Option<String>,
    pub text: String,
    pub url: Option<String>,
    pub awaiting_code: bool,
    pub awaiting_answer: bool,
}

impl Session {
    pub fn new(title: String, slot: Option<u32>) -> Self {
        Session {
            title,
            slot,
            transcript: Transcript::default(),
            phase: Phase::Running,
        }
    }

    pub fn output(&mut self, chunk: &[u8]) {
        self.transcript.append(chunk);
    }

    /// The child exited. A run already cancelled stays cancelled.
    pub fn exited(&mut self, success: bool) {
        if self.phase == Phase::Running {
            self.phase = if success {
                Phase::Succeeded
            } else {
                Phase::Failed
            };
        }
    }

    /// Returns whether the run was still going (and so needs killing).
    pub fn cancel(&mut self) -> bool {
        let running = self.phase == Phase::Running;
        if running {
            self.phase = Phase::Cancelled;
        }
        running
    }

    /// Writable only while running; the line gets the Return a terminal would send.
    pub fn input_line(&self, line: &str) -> Option<String> {
        let line = line.trim_matches(['\r', '\n']);
        (self.phase == Phase::Running && !line.trim().is_empty()).then(|| format!("{line}\r"))
    }

    pub fn view(&self, id: u64) -> View {
        let t = &self.transcript;
        let (state, message) = match self.phase {
            Phase::Running => ("running", None),
            Phase::Succeeded => (
                "succeeded",
                Some(
                    t.success_message()
                        .unwrap_or("Nothing was changed.")
                        .to_string(),
                ),
            ),
            Phase::Failed => ("failed", Some(t.failure_message())),
            Phase::Cancelled => (
                "cancelled",
                Some("Sign-in cancelled. Nothing was changed.".to_string()),
            ),
        };
        View {
            id,
            title: self.title.clone(),
            state,
            message,
            text: t.text().to_string(),
            url: t.url().map(str::to_string),
            awaiting_code: self.phase == Phase::Running && t.awaiting_code(),
            awaiting_answer: self.phase == Phase::Running && t.awaiting_answer(),
        }
    }
}

/// `haryx` arguments for a sign-in: again into `slot`, or a new account.
pub fn login_args(slot: Option<u32>) -> Vec<String> {
    let mut args = vec!["ai".to_string(), "login".to_string()];
    args.extend(slot.map(|n| n.to_string()));
    args
}

// -------------------------------------------------------------------------------------- runtime

/// A live run: its state plus the pty ends. Dropping it closes the terminal.
struct Run {
    session: Session,
    writer: Box<dyn Write + Send>,
    killer: Box<dyn ChildKiller + Send + Sync>,
    /// Closed after the child exits: on Windows the reader sees EOF only once the pseudoconsole closes.
    master: Option<Box<dyn MasterPty + Send>>,
}

/// Every sign-in run with an open window, by id.
#[derive(Default)]
pub struct Logins {
    runs: Mutex<HashMap<u64, Run>>,
    next: Mutex<u64>,
}

impl Logins {
    fn runs(&self) -> std::sync::MutexGuard<'_, HashMap<u64, Run>> {
        self.runs.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn view(&self, id: u64) -> Option<View> {
        self.runs().get(&id).map(|r| r.session.view(id))
    }

    pub fn send(&self, id: u64, line: &str) -> Result<(), String> {
        let mut runs = self.runs();
        let run = runs.get_mut(&id).ok_or("this sign-in has ended")?;
        let bytes = run
            .session
            .input_line(line)
            .ok_or("this sign-in is not waiting for input")?;
        run.writer
            .write_all(bytes.as_bytes())
            .and_then(|()| run.writer.flush())
            .map_err(|e| format!("could not send the code: {e}"))
    }

    pub fn cancel(&self, id: u64) {
        if let Some(run) = self.runs().get_mut(&id)
            && run.session.cancel()
        {
            let _ = run.killer.kill();
        }
    }

    /// The sign-in link the run printed, if it is one to open.
    pub fn url(&self, id: u64) -> Option<String> {
        let runs = self.runs();
        let url = runs.get(&id)?.session.transcript.url()?;
        url.starts_with("https://").then(|| url.to_string())
    }
}

fn window_label(id: u64) -> String {
    format!("login-{id}")
}

/// Start `haryx ai login [slot]` on a pseudo-terminal and open its window. A sign-in still running
/// for the same slot is brought forward instead.
pub fn start(app: &AppHandle, slot: Option<u32>, title: String) -> Result<u64, String> {
    let logins = app.state::<Logins>();
    let existing = logins
        .runs()
        .iter()
        .find(|(_, r)| r.session.slot == slot && r.session.phase == Phase::Running)
        .map(|(id, _)| *id);
    if let Some(id) = existing {
        if let Some(w) = app.get_webview_window(&window_label(id)) {
            let _ = w.unminimize();
            let _ = w.set_focus();
        }
        return Ok(id);
    }

    let exe = app.state::<Arc<AppState>>().cli()?.exe;
    let pair = native_pty_system()
        .openpty(PtySize {
            rows: ROWS,
            cols: COLUMNS,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| format!("Could not open a terminal for the sign-in: {e}"))?;
    let mut command = CommandBuilder::new(&exe);
    command.args(login_args(slot));
    command.env("TERM", "xterm-256color");
    let mut child = pair
        .slave
        .spawn_command(command)
        .map_err(|e| format!("Could not start haryx for the sign-in: {e}"))?;
    drop(pair.slave); // the child holds its own copy; EOF on the master then means it exited
    let io = pair
        .master
        .try_clone_reader()
        .and_then(|r| Ok((r, pair.master.take_writer()?)));
    let (reader, writer) = match io {
        Ok(io) => io,
        Err(e) => {
            let _ = child.kill();
            return Err(format!("Could not read the sign-in terminal: {e}"));
        }
    };

    let id = {
        let mut next = logins.next.lock().unwrap_or_else(|e| e.into_inner());
        *next += 1;
        *next
    };
    logins.runs().insert(
        id,
        Run {
            session: Session::new(title.clone(), slot),
            writer,
            killer: child.clone_killer(),
            master: Some(pair.master),
        },
    );
    if let Err(e) = open_window(app, id, &title) {
        logins.cancel(id);
        let run = logins.runs().remove(&id);
        std::thread::spawn(move || drop(run));
        return Err(format!("Could not open the sign-in window: {e}"));
    }

    let handle = app.clone();
    std::thread::spawn(move || read_output(&handle, id, reader));
    let handle = app.clone();
    std::thread::spawn(move || {
        let success = child.wait().map(|s| s.success()).unwrap_or(false);
        std::thread::sleep(DRAIN);
        let logins = handle.state::<Logins>();
        let master = logins.runs().get_mut(&id).and_then(|run| {
            run.session.exited(success);
            run.master.take()
        });
        drop(master); // outside the lock: closing a pseudoconsole waits for its output to drain
        emit(&handle, id);
        handle.state::<Arc<AppState>>().refresh_soon();
        if success {
            // A new or re-signed account: push its metadata, pull what sync holds for it.
            crate::sync::request(&handle, Duration::ZERO);
        }
    });
    Ok(id)
}

fn read_output(app: &AppHandle, id: u64, mut reader: Box<dyn Read + Send>) {
    let mut buffer = [0_u8; 4096];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) | Err(_) => return, // EOF/EIO: the terminal closed
            Ok(n) => {
                match app.state::<Logins>().runs().get_mut(&id) {
                    Some(run) => run.session.output(&buffer[..n]),
                    None => return,
                }
                emit(app, id);
            }
        }
    }
}

fn emit(app: &AppHandle, id: u64) {
    if let Some(view) = app.state::<Logins>().view(id) {
        let _ = app.emit_to(window_label(id), "login", view);
    }
}

fn open_window(app: &AppHandle, id: u64, title: &str) -> tauri::Result<()> {
    crate::tray::hide_flyout(app);
    let window = WebviewWindowBuilder::new(
        app,
        window_label(id),
        WebviewUrl::App(format!("index.html?view=login&id={id}").into()),
    )
    .title(title)
    .inner_size(520.0, 460.0)
    .min_inner_size(420.0, 380.0)
    .theme(app.state::<Arc<AppState>>().prefs().theme.window_theme())
    .build()?;
    let handle = app.clone();
    window.on_window_event(move |event| {
        // Closing the window abandons a sign-in still waiting.
        if let WindowEvent::Destroyed = event {
            let logins = handle.state::<Logins>();
            logins.cancel(id);
            let run = logins.runs().remove(&id);
            std::thread::spawn(move || drop(run));
        }
    });
    let _ = window.set_focus();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Captured from `claude auth login` on a pseudo-terminal (ESC/BEL written out).
    const CLAUDE_LOGIN_OUTPUT: &str = concat!(
        "\u{04}\u{08}\u{08}Opening browser to sign in…\r\n",
        "If the browser didn't open, visit:\r\n",
        "\u{1b}]8;;https://claude.ai/oauth/authorize?code=true&login_hint=a%40x.com\u{07}",
        "\u{1b}[94mhttps://claude.ai/oauth/authorize?code=true&login_hint=a%40x.com\u{1b}[39m\u{1b}]8;;\u{07}\r\n",
        "Paste code here if prompted > ",
    );

    fn transcript(chunks: &[&str]) -> Transcript {
        let mut t = Transcript::default();
        for c in chunks {
            t.append(c.as_bytes());
        }
        t
    }

    #[test]
    fn cleans_claude_login_output_and_finds_link() {
        let t = transcript(&[CLAUDE_LOGIN_OUTPUT]);
        assert_eq!(
            t.text(),
            "Opening browser to sign in…\nIf the browser didn't open, visit:\n\
             https://claude.ai/oauth/authorize?code=true&login_hint=a%40x.com\n\
             Paste code here if prompted > "
        );
        assert_eq!(
            t.url(),
            Some("https://claude.ai/oauth/authorize?code=true&login_hint=a%40x.com")
        );
        assert!(t.awaiting_code());
    }

    #[test]
    fn escape_sequence_split_across_chunks() {
        assert_eq!(transcript(&["a\u{1b}[3", "9mb\r\n"]).text(), "ab\n");
    }

    #[test]
    fn utf8_character_split_across_chunks() {
        let bytes = "sign in…\r\n".as_bytes();
        let mut t = Transcript::default();
        t.append(&bytes[..9]); // inside the three bytes of "…"
        t.append(&bytes[9..]);
        assert_eq!(t.text(), "sign in…\n");
    }

    #[test]
    fn pty_double_carriage_return_keeps_the_line() {
        assert_eq!(clean("visit:\r\r\nhttps://x\r\r\n"), "visit:\nhttps://x\n");
    }

    #[test]
    fn bare_carriage_return_overwrites_the_line() {
        assert_eq!(clean("one\nwait 1\rwait 2\r\n"), "one\nwait 2\n");
    }

    #[test]
    fn reports_haryx_outcome() {
        let ok = transcript(&["Signed in Account 3: a@x.com [Team]\r\n"]);
        assert_eq!(
            ok.success_message(),
            Some("Signed in Account 3: a@x.com [Team]")
        );
        let failed = transcript(&[
            "Paste code here if prompted > \r\nError: You signed in as b@x.com, but Account-3 is a@x.com.\r\n",
        ]);
        assert_eq!(
            failed.failure_message(),
            "You signed in as b@x.com, but Account-3 is a@x.com."
        );
        assert!(!failed.awaiting_code());
        assert_eq!(
            transcript(&["\r\n"]).failure_message(),
            "Sign-in did not finish."
        );
    }

    #[test]
    fn detects_overwrite_question() {
        assert!(
            transcript(&["Slot 2 already occupied\r\nOverwrite slot 2? [y/N] "]).awaiting_answer()
        );
    }

    #[test]
    fn strips_conpty_frame_and_keeps_its_text() {
        // ConPTY's first frame (hide cursor, clear, home, window title), then a line drawn with a
        // cursor-forward for a space and a jump to the next row instead of a newline.
        let raw = "\u{1b}[?25l\u{1b}[2J\u{1b}[m\u{1b}[H\u{1b}]0;C:\\haryx.exe\u{07}\u{1b}[?25h\
                   Opening\u{1b}[1Cbrowser\u{1b}[2;1HPaste code here if prompted >\u{1b}[K";
        assert_eq!(clean(raw), "Opening browser\nPaste code here if prompted >");
        assert!(transcript(&[raw]).awaiting_code());
    }

    #[test]
    fn cursor_jump_after_the_prompt_keeps_it_the_last_line() {
        let t = transcript(&["Paste code here if prompted > \u{1b}[3;31H"]);
        assert!(t.text().ends_with('\n'));
        assert!(t.awaiting_code());
    }

    #[test]
    fn no_link_without_a_host() {
        assert_eq!(transcript(&["visit https:// "]).url(), None);
        assert_eq!(transcript(&["nothing"]).url(), None);
    }

    #[test]
    fn session_succeeds_with_the_line_haryx_printed_even_after_exit() {
        let mut s = Session::new("Sign In Again: a@x.com".into(), Some(3));
        s.output(CLAUDE_LOGIN_OUTPUT.as_bytes());
        let running = s.view(7);
        assert_eq!(running.state, "running");
        assert!(running.awaiting_code);
        assert_eq!(running.message, None);
        s.exited(true);
        assert_eq!(s.view(7).message.as_deref(), Some("Nothing was changed."));
        s.output(b"\r\nSigned in Account 3: a@x.com\r\n"); // drained late
        let done = s.view(7);
        assert_eq!(done.state, "succeeded");
        assert_eq!(
            done.message.as_deref(),
            Some("Signed in Account 3: a@x.com")
        );
        assert!(!done.awaiting_code, "a finished run waits for nothing");
    }

    #[test]
    fn session_fails_with_the_error_line() {
        let mut s = Session::new("Sign In New Account".into(), None);
        s.output(b"Error: Sign-in did not finish (Claude Code exited with an error).\r\n");
        s.exited(false);
        let v = s.view(1);
        assert_eq!(v.state, "failed");
        assert_eq!(
            v.message.as_deref(),
            Some("Sign-in did not finish (Claude Code exited with an error).")
        );
    }

    #[test]
    fn cancel_wins_over_the_exit_it_causes_and_stops_input() {
        let mut s = Session::new("t".into(), None);
        assert_eq!(s.input_line("abc#def\n").as_deref(), Some("abc#def\r"));
        assert_eq!(s.input_line("  \r\n"), None);
        assert!(s.cancel());
        assert!(!s.cancel(), "only the first cancel kills");
        s.exited(false);
        assert_eq!(s.view(1).state, "cancelled");
        assert_eq!(s.input_line("abc"), None);
    }

    #[test]
    fn transcript_is_bounded() {
        let mut t = Transcript::default();
        for _ in 0..100 {
            t.append(&[b'x'; 4096]);
        }
        assert!(t.raw.len() <= MAX_RAW);
    }

    #[test]
    fn login_arguments() {
        assert_eq!(login_args(Some(4)), ["ai", "login", "4"]);
        assert_eq!(login_args(None), ["ai", "login"]);
    }
}

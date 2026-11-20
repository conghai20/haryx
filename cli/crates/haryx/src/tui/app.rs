//! TUI state and key handling. No I/O here: actions become [`Job`]s for the worker.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use haryx_core::autoswitch::{Event, pct_label};
use haryx_core::models::AccountsSnapshot;
use haryx_core::settings::AutoSwitchSettings;

use super::view::Palette;
use super::worker::{Job, Msg};

/// How often a screen re-reads the store (the store paces the network, not this).
pub const REFRESH_EVERY: Duration = Duration::from_secs(5);
/// Auto view's event log length.
const EVENT_LOG_CAP: usize = 500;
/// Threshold bounds (`autoswitch.threshold`).
const THRESHOLD_MIN: f64 = 50.0;
const THRESHOLD_MAX: f64 = 99.9;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screen {
    Dashboard,
    Switch,
    Watch,
    Auto,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Pending {
    SwitchTo(String),
    GoLive,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Modal {
    Confirm {
        text: String,
        on_yes: Pending,
    },
    Output {
        title: String,
        body: String,
        ok: bool,
    },
}

pub struct AutoState {
    pub settings: AutoSwitchSettings,
    pub live: bool,
    pub adjusting: bool,
    pub events: VecDeque<(String, String)>,
    pub next_tick: Option<Instant>,
    pub in_flight: bool,
}

pub struct App {
    stack: Vec<Screen>,
    pub snapshot: Option<AccountsSnapshot>,
    pub cursor: usize,
    pub cursor_visible: bool,
    pub modal: Option<Modal>,
    pub status: String,
    pub busy: bool,
    pub light: bool,
    pub quit: bool,
    pub auto: AutoState,
    pub next_refresh: Instant,
    snapshot_in_flight: bool,
}

impl App {
    pub fn new(settings: AutoSwitchSettings, start_watch: bool, light: bool) -> Self {
        let mut stack = vec![Screen::Dashboard];
        if start_watch {
            stack.push(Screen::Watch);
        }
        App {
            stack,
            snapshot: None,
            cursor: 0,
            cursor_visible: false,
            modal: None,
            status: String::new(),
            busy: false,
            light,
            quit: false,
            auto: AutoState {
                settings,
                live: false,
                adjusting: false,
                events: VecDeque::new(),
                next_tick: None,
                in_flight: false,
            },
            next_refresh: Instant::now(),
            snapshot_in_flight: false,
        }
    }

    pub fn screen(&self) -> Screen {
        *self.stack.last().unwrap_or(&Screen::Dashboard)
    }

    pub fn palette(&self) -> Palette {
        if self.light {
            Palette::LIGHT
        } else {
            Palette::DARK
        }
    }

    pub fn thresholds(&self) -> (f64, Option<f64>) {
        (
            self.auto.settings.threshold,
            self.auto.settings.weekly_threshold,
        )
    }

    fn account_count(&self) -> usize {
        self.snapshot.as_ref().map_or(0, |s| s.accounts.len())
    }

    fn highlighted(&self) -> Option<String> {
        self.snapshot
            .as_ref()?
            .accounts
            .get(self.cursor)
            .map(|a| a.number.clone())
    }

    fn push(&mut self, screen: Screen) -> Vec<Job> {
        self.stack.push(screen);
        if screen == Screen::Auto {
            self.auto.next_tick = Some(Instant::now());
        }
        Vec::new()
    }

    fn pop(&mut self) -> Vec<Job> {
        let left = self.stack.pop();
        if self.stack.is_empty() {
            self.stack.push(Screen::Dashboard);
        }
        if left == Some(Screen::Auto) {
            self.auto.next_tick = None;
            self.auto.adjusting = false;
            return vec![Job::AutoStop];
        }
        Vec::new()
    }

    /// Periodic work: due snapshot refreshes and auto ticks.
    pub fn due_jobs(&mut self, now: Instant) -> Vec<Job> {
        let mut jobs = Vec::new();
        if now >= self.next_refresh && !self.snapshot_in_flight {
            self.snapshot_in_flight = true;
            self.next_refresh = now + REFRESH_EVERY;
            // While the auto view drives collection, read the store only.
            jobs.push(Job::Snapshot {
                store_only: self.screen() == Screen::Auto,
            });
        }
        if let Some(at) = self.auto.next_tick
            && now >= at
            && !self.auto.in_flight
            && !self.auto.adjusting
        {
            self.auto.in_flight = true;
            self.auto.next_tick = None;
            jobs.push(Job::AutoTick {
                settings: self.auto.settings.clone(),
                dry_run: !self.auto.live,
            });
        }
        jobs
    }

    pub fn on_msg(&mut self, msg: Msg, now: Instant) -> Vec<Job> {
        match msg {
            Msg::Snapshot(snapshot) => {
                self.snapshot_in_flight = false;
                self.cursor = self.cursor.min(snapshot.accounts.len().saturating_sub(1));
                self.snapshot = Some(*snapshot);
            }
            Msg::Action { title, ok, body } => {
                self.busy = false;
                self.modal = Some(Modal::Output { title, body, ok });
                return self.refresh_now(now);
            }
            Msg::AutoEvent(event) => self.log_event(&event),
            Msg::AutoTicked => {
                self.auto.in_flight = false;
                if self.screen() == Screen::Auto {
                    let every =
                        Duration::from_secs_f64(self.auto.settings.interval_seconds.max(15.0));
                    self.auto.next_tick = Some(now + every);
                }
                return self.refresh_now(now);
            }
        }
        Vec::new()
    }

    fn log_event(&mut self, event: &Event) {
        let stamp = jiff::Zoned::now().strftime("%H:%M:%S").to_string();
        self.auto.events.push_back((
            event.kind().to_owned(),
            format!("{stamp}  {}", event.human()),
        ));
        while self.auto.events.len() > EVENT_LOG_CAP {
            self.auto.events.pop_front();
        }
    }

    fn refresh_now(&mut self, now: Instant) -> Vec<Job> {
        self.next_refresh = now;
        Vec::new()
    }

    /// Handle one key; returns jobs for the worker.
    pub fn on_key(&mut self, key: KeyEvent) -> Vec<Job> {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && key.code == KeyCode::Char('c') {
            self.quit = true;
            return Vec::new();
        }
        if ctrl && key.code == KeyCode::Char('t') {
            self.light = !self.light;
            return Vec::new();
        }
        if let Some(modal) = self.modal.take() {
            return self.on_modal_key(modal, key.code);
        }
        match self.screen() {
            Screen::Dashboard => self.on_dashboard_key(key.code),
            Screen::Switch => self.on_switch_key(key.code),
            Screen::Watch => self.on_watch_key(key.code),
            Screen::Auto => self.on_auto_key(key.code),
        }
    }

    fn on_modal_key(&mut self, modal: Modal, code: KeyCode) -> Vec<Job> {
        match (&modal, code) {
            (Modal::Confirm { on_yes, .. }, KeyCode::Char('y') | KeyCode::Enter) => {
                self.run_pending(on_yes.clone())
            }
            (Modal::Confirm { .. }, KeyCode::Char('n') | KeyCode::Esc) => Vec::new(),
            (Modal::Output { .. }, KeyCode::Enter | KeyCode::Esc | KeyCode::Char('q')) => {
                Vec::new()
            }
            _ => {
                self.modal = Some(modal);
                Vec::new()
            }
        }
    }

    fn run_pending(&mut self, pending: Pending) -> Vec<Job> {
        match pending {
            Pending::SwitchTo(num) => {
                self.busy = true;
                vec![Job::SwitchTo(num)]
            }
            Pending::GoLive => {
                self.auto.live = true;
                self.auto.next_tick = Some(Instant::now());
                Vec::new()
            }
        }
    }

    fn move_cursor(&mut self, code: KeyCode) -> bool {
        let n = self.account_count();
        match code {
            KeyCode::Down | KeyCode::Char('j') if n > 0 => {
                self.cursor = (self.cursor + 1).min(n - 1)
            }
            KeyCode::Up | KeyCode::Char('k') => self.cursor = self.cursor.saturating_sub(1),
            _ => return false,
        }
        self.cursor_visible = true;
        true
    }

    fn on_dashboard_key(&mut self, code: KeyCode) -> Vec<Job> {
        if self.move_cursor(code) {
            return Vec::new();
        }
        match code {
            KeyCode::Char('q') => {
                self.quit = true;
                Vec::new()
            }
            KeyCode::Char('s') => self.push(Screen::Switch),
            KeyCode::Char('w') => self.push(Screen::Watch),
            KeyCode::Char('g') => self.push(Screen::Auto),
            KeyCode::Char('f') => self.refresh_now(Instant::now()),
            KeyCode::Esc | KeyCode::Left => {
                self.cursor_visible = false;
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    fn on_switch_key(&mut self, code: KeyCode) -> Vec<Job> {
        if self.move_cursor(code) {
            return Vec::new();
        }
        match code {
            KeyCode::Enter => match self.highlighted() {
                Some(num) => {
                    self.busy = true;
                    vec![Job::SwitchTo(num)]
                }
                None => Vec::new(),
            },
            KeyCode::Char('b') => {
                self.busy = true;
                vec![Job::SwitchBest]
            }
            KeyCode::Esc | KeyCode::Char('q' | 's') => self.pop(),
            _ => Vec::new(),
        }
    }

    fn on_watch_key(&mut self, code: KeyCode) -> Vec<Job> {
        if self.move_cursor(code) {
            return Vec::new();
        }
        match code {
            KeyCode::Enter | KeyCode::Char('s') => {
                if let Some(num) = self.highlighted() {
                    self.modal = Some(Modal::Confirm {
                        text: format!("Switch to Account-{num}?"),
                        on_yes: Pending::SwitchTo(num),
                    });
                }
                Vec::new()
            }
            KeyCode::Char('f') => self.refresh_now(Instant::now()),
            KeyCode::Esc | KeyCode::Char('q') => self.pop(),
            _ => Vec::new(),
        }
    }

    fn on_auto_key(&mut self, code: KeyCode) -> Vec<Job> {
        if self.auto.adjusting {
            match code {
                KeyCode::Left => self.step_threshold(-1.0),
                KeyCode::Right => self.step_threshold(1.0),
                KeyCode::Enter | KeyCode::Char('t') | KeyCode::Esc => {
                    self.auto.adjusting = false;
                    self.auto.next_tick = Some(Instant::now());
                }
                _ => {}
            }
            return Vec::new();
        }
        match code {
            KeyCode::Char('l') => {
                if self.auto.live {
                    self.auto.live = false;
                } else {
                    self.modal = Some(Modal::Confirm {
                        text: format!(
                            "Go live? Auto-switch will switch accounts for real at {}%.",
                            pct_label(self.auto.settings.threshold)
                        ),
                        on_yes: Pending::GoLive,
                    });
                }
                Vec::new()
            }
            KeyCode::Char('t') => {
                self.auto.adjusting = true;
                Vec::new()
            }
            KeyCode::Esc | KeyCode::Char('q') => self.pop(),
            _ => Vec::new(),
        }
    }

    fn step_threshold(&mut self, delta: f64) {
        let t = (self.auto.settings.threshold + delta).round();
        self.auto.settings.threshold = t.clamp(THRESHOLD_MIN, THRESHOLD_MAX);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyEventKind;
    use haryx_core::models::AccountSnapshot;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        }
    }

    fn app_with_accounts(n: usize) -> App {
        let mut app = App::new(AutoSwitchSettings::default(), false, false);
        app.snapshot = Some(AccountsSnapshot {
            active_number: Some("1".into()),
            accounts: (1..=n)
                .map(|i| AccountSnapshot {
                    number: i.to_string(),
                    ..Default::default()
                })
                .collect(),
            taken_at: 0.0,
        });
        app
    }

    #[test]
    fn switch_screen_switches_the_highlighted_account() {
        let mut app = app_with_accounts(3);
        app.on_key(key(KeyCode::Char('s')));
        assert_eq!(app.screen(), Screen::Switch);
        app.on_key(key(KeyCode::Down));
        let jobs = app.on_key(key(KeyCode::Enter));
        assert!(matches!(jobs.as_slice(), [Job::SwitchTo(n)] if n == "2"));
        assert!(app.busy);
        app.on_key(key(KeyCode::Esc));
        assert_eq!(app.screen(), Screen::Dashboard);
    }

    #[test]
    fn watch_confirms_before_switching() {
        let mut app = app_with_accounts(2);
        app.on_key(key(KeyCode::Char('w')));
        assert!(app.on_key(key(KeyCode::Enter)).is_empty());
        assert!(matches!(app.modal, Some(Modal::Confirm { .. })));
        assert!(app.on_key(key(KeyCode::Char('n'))).is_empty());
        assert!(app.modal.is_none());
        app.on_key(key(KeyCode::Enter));
        let jobs = app.on_key(key(KeyCode::Char('y')));
        assert!(matches!(jobs.as_slice(), [Job::SwitchTo(n)] if n == "1"));
    }

    #[test]
    fn auto_view_starts_dry_and_going_live_needs_confirmation() {
        let mut app = app_with_accounts(1);
        app.on_key(key(KeyCode::Char('g')));
        let jobs = app.due_jobs(Instant::now());
        assert!(
            jobs.iter()
                .any(|j| matches!(j, Job::AutoTick { dry_run: true, .. }))
        );
        app.on_key(key(KeyCode::Char('l')));
        assert!(!app.auto.live);
        app.on_key(key(KeyCode::Char('y')));
        assert!(app.auto.live);
        let jobs = app.on_key(key(KeyCode::Esc));
        assert!(matches!(jobs.as_slice(), [Job::AutoStop]));
    }

    #[test]
    fn threshold_steps_and_clamps() {
        let mut app = app_with_accounts(1);
        app.on_key(key(KeyCode::Char('g')));
        app.on_key(key(KeyCode::Char('t')));
        for _ in 0..20 {
            app.on_key(key(KeyCode::Right));
        }
        assert_eq!(app.auto.settings.threshold, THRESHOLD_MAX);
        app.on_key(key(KeyCode::Left));
        assert_eq!(app.auto.settings.threshold, 99.0);
        app.on_key(key(KeyCode::Enter));
        assert!(!app.auto.adjusting);
    }

    #[test]
    fn start_watch_opens_over_the_dashboard() {
        let mut app = App::new(AutoSwitchSettings::default(), true, false);
        assert_eq!(app.screen(), Screen::Watch);
        app.on_key(key(KeyCode::Esc));
        assert_eq!(app.screen(), Screen::Dashboard);
    }
}

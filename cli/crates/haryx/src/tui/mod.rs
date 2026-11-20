//! `haryx tui` / `haryx watch` (and bare `haryx` in a terminal): the interactive dashboard.
//!
//! Port of upstream's Textual app onto ratatui. The UI thread only renders and handles keys;
//! one worker thread owns every blocking call (snapshots, switches, the auto engine).

mod app;
mod view;
mod worker;

use std::sync::mpsc;
use std::time::{Duration, Instant};

use crossterm::event::{self, Event as TermEvent, KeyEventKind};
use haryx_core::appearance::{Theme, detect_terminal_background};
use haryx_core::settings::load_settings;
use haryx_core::switcher::Switcher;

use app::App;
use worker::{Job, Msg};

/// UI poll period: key latency and the clock in the header.
const FRAME: Duration = Duration::from_millis(200);

/// Run the TUI until the user quits. `start_watch` opens on the watch page.
pub fn run(switcher: &Switcher, start_watch: bool) -> std::io::Result<()> {
    let light = detect_terminal_background() == Some(Theme::Light);
    let settings = load_settings(switcher.backup_dir());
    let mut app = App::new(settings, start_watch, light);
    let (job_tx, job_rx) = mpsc::channel::<Job>();
    let (msg_tx, msg_rx) = mpsc::channel::<Msg>();
    std::thread::scope(|scope| {
        scope.spawn(move || worker::serve(switcher, job_rx, msg_tx));
        let mut terminal = ratatui::init();
        let result = (|| -> std::io::Result<()> {
            while !app.quit {
                let now = Instant::now();
                while let Ok(msg) = msg_rx.try_recv() {
                    send(&job_tx, app.on_msg(msg, now));
                }
                send(&job_tx, app.due_jobs(now));
                terminal.draw(|frame| view::draw(frame, &app))?;
                if event::poll(FRAME)?
                    && let TermEvent::Key(key) = event::read()?
                    && key.kind == KeyEventKind::Press
                {
                    send(&job_tx, app.on_key(key));
                }
            }
            Ok(())
        })();
        ratatui::restore();
        // Hanging up the job channel ends the worker once its current job finishes.
        drop(job_tx);
        result
    })
}

fn send(tx: &mpsc::Sender<Job>, jobs: Vec<Job>) {
    for job in jobs {
        let _ = tx.send(job);
    }
}

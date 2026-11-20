//! Background jobs for the TUI. Everything here blocks (file locks, keychain subprocesses,
//! network), so it runs on one worker thread and reports back over a channel; the UI thread only
//! renders.

use std::sync::mpsc::{Receiver, Sender};

use haryx_core::autoswitch::{AutoSwitchEngine, Event};
use haryx_core::models::AccountsSnapshot;
use haryx_core::settings::{self, AutoSwitchSettings};
use haryx_core::snapshot::SnapshotSource;
use haryx_core::switcher::Switcher;
use serde_json::Value;

/// A request from the UI.
pub enum Job {
    Snapshot {
        store_only: bool,
    },
    SwitchTo(String),
    SwitchBest,
    /// One auto-switch tick; a changed `settings`/`dry_run` rebuilds the engine.
    AutoTick {
        settings: AutoSwitchSettings,
        dry_run: bool,
    },
    /// Leave the auto view: drop the engine.
    AutoStop,
}

/// A result for the UI.
pub enum Msg {
    Snapshot(Box<AccountsSnapshot>),
    Action {
        title: String,
        ok: bool,
        body: String,
    },
    AutoEvent(Box<Event>),
    AutoTicked,
}

/// Run jobs until the UI hangs up.
pub fn serve(switcher: &Switcher, jobs: Receiver<Job>, out: Sender<Msg>) {
    let source = SnapshotSource::new(switcher);
    let mut engine: Option<(AutoSwitchEngine<'_>, AutoSwitchSettings, bool)> = None;
    for job in jobs {
        let msg = match job {
            Job::Snapshot { store_only } => Msg::Snapshot(Box::new(source.take(store_only))),
            Job::SwitchTo(num) => action("Switch", switcher.switch_to(&num, true, false)),
            Job::SwitchBest => {
                let models = settings::parse_model_names(
                    settings::load_settings(switcher.backup_dir())
                        .model
                        .as_deref(),
                );
                let source = (!models.is_empty()).then_some("autoswitch.model");
                action(
                    "Best pick",
                    switcher.switch(Some("best"), true, &models, source),
                )
            }
            Job::AutoTick { settings, dry_run } => {
                let stale = engine
                    .as_ref()
                    .is_none_or(|(_, s, d)| *s != settings || *d != dry_run);
                if stale {
                    let sink = out.clone();
                    let emit = move |e: &Event| {
                        let _ = sink.send(Msg::AutoEvent(Box::new(e.clone())));
                    };
                    let built = AutoSwitchEngine::new(switcher, settings.clone(), emit, dry_run);
                    engine = Some((built, settings, dry_run));
                }
                match engine.as_mut() {
                    Some((e, _, _)) => {
                        e.tick();
                        Msg::AutoTicked
                    }
                    None => continue,
                }
            }
            Job::AutoStop => {
                engine = None;
                continue;
            }
        };
        if out.send(msg).is_err() {
            return;
        }
    }
}

fn action(title: &str, result: haryx_core::error::Result<Option<Value>>) -> Msg {
    match result {
        Ok(Some(payload)) => {
            let message = payload
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("Done")
                .to_owned();
            let warnings: Vec<String> = payload
                .get("warnings")
                .and_then(Value::as_array)
                .map(|w| {
                    w.iter()
                        .filter_map(Value::as_str)
                        .map(|w| format!("⚠ {w}"))
                        .collect()
                })
                .unwrap_or_default();
            let body = std::iter::once(message)
                .chain(warnings)
                .collect::<Vec<_>>()
                .join("\n");
            Msg::Action {
                title: title.into(),
                ok: true,
                body,
            }
        }
        Ok(None) => Msg::Action {
            title: title.into(),
            ok: true,
            body: "Done".into(),
        },
        Err(err) => Msg::Action {
            title: format!("{title} — failed"),
            ok: false,
            body: format!("Error: {err}"),
        },
    }
}

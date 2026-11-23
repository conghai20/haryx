//! `haryx auto`: the foreground auto-switch loop (`_auto_command`).

use std::io::Write;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use clap::ValueEnum;
use haryx_core::autoswitch::{AutoSwitchEngine, Event, pct_label};
use haryx_core::json_output::error_envelope;
use haryx_core::models::pyfmt::json_dumps;
use haryx_core::printer;
use haryx_core::settings::{AutoSwitchOverrides, load_settings, merged_with_cli};
use haryx_core::switcher::Switcher;

use super::{CommandError, guard};
use crate::cli::AutoOptions;

/// Exit code after Ctrl-C (`KeyboardInterrupt`).
const INTERRUPTED_EXIT: u8 = 130;

static INTERRUPTED: AtomicBool = AtomicBool::new(false);
static TERMINATED: AtomicBool = AtomicBool::new(false);

extern "C" fn on_signal(signal: libc::c_int) {
    // Only async-signal-safe work here: flip a flag the loop polls.
    if signal == libc::SIGINT {
        INTERRUPTED.store(true, Ordering::SeqCst);
    } else {
        TERMINATED.store(true, Ordering::SeqCst);
    }
}

fn install_signal_handlers() {
    let handler = on_signal as extern "C" fn(libc::c_int) as libc::sighandler_t;
    // SAFETY: `on_signal` only stores to atomics, which is async-signal-safe; installing a handler
    // for SIGINT/SIGTERM has no other preconditions.
    unsafe {
        libc::signal(libc::SIGINT, handler);
        libc::signal(libc::SIGTERM, handler);
    }
}

fn overrides(opts: &AutoOptions) -> AutoSwitchOverrides {
    AutoSwitchOverrides {
        threshold: opts.threshold,
        weekly_threshold: opts.weekly_threshold,
        interval: opts.interval,
        cooldown: opts.cooldown,
        include_api_key_accounts: opts.include_api_key_accounts,
        model: opts.model.clone(),
        strategy: opts
            .strategy
            .and_then(|s| s.to_possible_value())
            .map(|v| v.get_name().to_owned()),
    }
}

fn jsonl_emit(event: &Event) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{}", json_dumps(&event.to_json()));
    let _ = out.flush();
}

fn human_emit(event: &Event) {
    let stamp = jiff::Zoned::now().strftime("%H:%M:%S").to_string();
    let line = event.human();
    let line = match event.kind() {
        "switch" => printer::accent(&line),
        "error" | "account-quarantined" => printer::yellowed(&line),
        "poll" | "no-switch" | "sleep" => printer::dimmed(&line),
        _ => line,
    };
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{stamp}  {line}");
    let _ = out.flush();
}

pub fn run(opts: &AutoOptions, debug: bool) -> Result<(), CommandError> {
    guard::refuse_root()?;
    let switcher = match Switcher::from_process(debug) {
        Ok(s) => s,
        Err(err) => {
            if opts.json {
                println!("{}", json_dumps(&error_envelope(&err)));
            } else {
                printer::error(&format!("Error: {err}"));
            }
            return Err(CommandError::silent(1));
        }
    };
    let settings = merged_with_cli(&load_settings(switcher.backup_dir()), &overrides(opts));
    let emit: fn(&Event) = if opts.json { jsonl_emit } else { human_emit };
    let mut engine = AutoSwitchEngine::new(&switcher, settings, emit, opts.dry_run);

    if opts.once {
        return match engine.tick().code() {
            0 => Ok(()),
            code => Err(CommandError::silent(u8::try_from(code).unwrap_or(1))),
        };
    }

    install_signal_handlers();
    let stop = engine.stop_handle();
    let watcher = spawn_signal_watcher(Arc::clone(&stop));
    if !opts.json {
        let s = engine.settings();
        let weekly = s
            .weekly_threshold
            .map(|w| format!(", weekly {}%", pct_label(w)))
            .unwrap_or_default();
        let dry = if opts.dry_run { " (dry-run)" } else { "" };
        println!(
            "{}",
            printer::dimmed(&format!(
                "Auto-switch running: threshold {}%{weekly}, every {:.0}s{dry} — Ctrl-C to stop",
                pct_label(s.threshold),
                s.interval_seconds
            ))
        );
    }
    let code = engine.run_loop();
    stop.store(true, Ordering::SeqCst);
    let _ = watcher.join();
    if INTERRUPTED.load(Ordering::SeqCst) {
        let message = format!("\n{}", printer::dimmed("Auto-switch stopped"));
        if opts.json {
            eprintln!("{message}");
        } else {
            println!("{message}");
        }
        return Err(CommandError::silent(INTERRUPTED_EXIT));
    }
    match code {
        0 => Ok(()),
        code => Err(CommandError::silent(u8::try_from(code).unwrap_or(1))),
    }
}

/// Set by a host app (HaryX for macOS) so the loop ends when the app is gone instead of
/// auto-switching unattended forever; macOS has no parent-death signal to do this for us.
const EXIT_WITH_PARENT_ENV: &str = "HARYX_EXIT_WITH_PARENT";

/// Whether the process that started us has exited: we were re-parented, or adopted by launchd
/// (PID 1), which also covers a parent that exited before we first looked.
fn parent_gone(original_parent: libc::pid_t) -> bool {
    // SAFETY: getppid takes no arguments, cannot fail and touches no memory.
    let parent = unsafe { libc::getppid() };
    parent == 1 || parent != original_parent
}

/// Relay SIGINT/SIGTERM flags, and the host app's exit when asked to, to the engine's stop handle.
fn spawn_signal_watcher(stop: Arc<AtomicBool>) -> std::thread::JoinHandle<()> {
    let watch_parent = std::env::var_os(EXIT_WITH_PARENT_ENV).is_some_and(|v| v == "1");
    // SAFETY: as in `parent_gone`.
    let original_parent = unsafe { libc::getppid() };
    std::thread::spawn(move || {
        while !stop.load(Ordering::SeqCst) {
            if INTERRUPTED.load(Ordering::SeqCst) || TERMINATED.load(Ordering::SeqCst) {
                stop.store(true, Ordering::SeqCst);
                return;
            }
            if watch_parent && parent_gone(original_parent) {
                log::info!("auto: parent process exited; stopping ({EXIT_WITH_PARENT_ENV})");
                stop.store(true, Ordering::SeqCst);
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    })
}

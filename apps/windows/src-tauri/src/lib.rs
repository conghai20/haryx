//! HaryX for Windows: a tray app over the `haryx` CLI (plan: `docs/windows-app-plan.md`). The CLI owns
//! accounts and credentials; this app polls it, shows the result, and runs the commands the UI asks for.

mod account;
mod autostart;
mod autoswitch;
mod cli;
mod commands;
mod login;
mod mascot;
mod models;
mod notify;
mod prefsync;
mod rules;
mod settings;
mod state;
mod sync;
mod terminals;
mod tray;
mod updater;
mod weekly;

use std::sync::Arc;

use tauri::Manager;

use crate::state::AppState;

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            // A second launch (Start menu, autostart) opens the running app's dashboard.
            tray::show_dashboard(app, None);
        }))
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--background"]),
        ))
        .setup(|app| {
            let handle = app.handle().clone();
            let config_dir = handle.path().app_config_dir()?;
            let state = Arc::new(AppState::new(config_dir, commands::bundled_bin(&handle)));
            let prefs = state.prefs();
            app.manage(Arc::clone(&state));
            app.manage(login::Logins::default());
            app.manage(account::AccountLogin::default());
            app.manage(sync::Syncer::default());
            tray::build(&handle)?;
            state::start_loops(&handle, tray::update);
            tray::start_animation(&handle);
            autostart::apply(&handle, &prefs);
            updater::start_loop(&handle);
            weekly::start_loop(&handle);
            sync::start_loop(&handle);
            if prefs.auto_switch_on_launch {
                let _ = commands::auto_start(handle.clone(), handle.state());
            }
            // Started by hand (not at login): show the dashboard once so the app is found.
            if !std::env::args().any(|a| a == "--background") {
                tray::show_dashboard(&handle, None);
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::haryx,
            commands::haryx_json,
            commands::snapshot,
            commands::refresh,
            commands::set_prefs,
            commands::auto_start,
            commands::auto_stop,
            commands::open_terminal,
            commands::open_dashboard,
            commands::mappings,
            commands::reveal,
            commands::hide_flyout,
            commands::quit,
            commands::update_check,
            commands::update_download,
            commands::update_restart,
            commands::update_set,
            commands::open_link,
            commands::login_start,
            commands::login_get,
            commands::login_send,
            commands::login_cancel,
            commands::login_open,
            commands::account_login_start,
            commands::account_login_cancel,
            commands::account_login_open,
            commands::sync_now,
            commands::sync_after_change,
            commands::sync_info,
        ])
        .build(tauri::generate_context!())
        .expect("error while building HaryX")
        .run(|_app, event| {
            // Closing the dashboard keeps HaryX in the tray; only Quit ends it.
            if let tauri::RunEvent::ExitRequested { api, code, .. } = event
                && code.is_none()
            {
                api.prevent_exit();
            }
        });
}

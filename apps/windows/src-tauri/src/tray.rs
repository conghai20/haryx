//! The tray icon, its flyout and the dashboard window.
//!
//! Left click toggles the flyout, a frameless window anchored above the tray that hides when it loses
//! focus. Right click opens a short menu. The dashboard is an ordinary window that hides on close, so
//! the app keeps running in the tray.

use std::sync::Arc;

use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, Rect, WebviewUrl,
    WebviewWindowBuilder, WindowEvent,
};

use crate::rules;
use crate::state::AppState;

pub const TRAY_ID: &str = "haryx";
pub const FLYOUT: &str = "flyout";
pub const DASHBOARD: &str = "main";
/// Flyout size in logical pixels: the macOS panel's 380pt width.
const FLYOUT_W: f64 = 380.0;
const FLYOUT_H: f64 = 560.0;
/// Space kept between the flyout and the taskbar.
const FLYOUT_GAP: f64 = 8.0;

const ICON: &[u8] = include_bytes!("../icons/tray@2x.png");
const ICON_RUNNING: &[u8] = include_bytes!("../icons/tray-running@2x.png");

pub fn build(app: &AppHandle) -> tauri::Result<TrayIcon> {
    let use_next = MenuItem::with_id(app, "use-next", "Use next account", true, None::<&str>)?;
    let dashboard = MenuItem::with_id(app, "dashboard", "Open dashboard", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit HaryX", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&use_next, &dashboard, &separator, &quit])?;
    TrayIconBuilder::with_id(TRAY_ID)
        .icon(Image::from_bytes(ICON)?)
        .tooltip("HaryX")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "use-next" => use_next_account(app),
            "dashboard" => show_dashboard(app, None),
            "quit" => {
                let state = app.state::<Arc<AppState>>();
                state.runner.stop();
                crate::updater::install_on_quit(&state);
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                rect,
                ..
            } = event
            {
                toggle_flyout(tray.app_handle(), rect);
            }
        })
        .build(app)
}

fn use_next_account(app: &AppHandle) {
    let state = Arc::clone(&app.state::<Arc<AppState>>());
    let app = app.clone();
    std::thread::spawn(move || {
        let result = state.cli().and_then(|cli| cli.ok(&["switch"]));
        state.refresh_soon();
        if let Err(message) = result {
            crate::notify::show(&app, "Could not switch", &message);
        }
    });
}

/// Tray frames per second while the active account's mascot walks.
const WALK_FPS: u64 = 8;

/// Icon and tooltip for the current state: the mark (with a dot while the active account runs
/// Claude), or the active account's mascot when that preference is on.
pub fn update(app: &AppHandle) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return;
    };
    let state = app.state::<Arc<AppState>>();
    let running = state.active_is_running();
    let image = mascot_frame(&state, running.then_some(0))
        .or_else(|| Image::from_bytes(if running { ICON_RUNNING } else { ICON }).ok());
    if let Some(image) = image {
        let _ = tray.set_icon(Some(image));
    }
    let tooltip = match state.active_account() {
        Some(a) => format!("HaryX: {} · {}", a.display_name(), rules::usage_line(&a)),
        None => "HaryX".to_string(),
    };
    let _ = tray.set_tooltip(Some(tooltip));
}

/// The active account's mascot as a tray image, when the preference is on: the same species and
/// colour as its card. `tick` is the walk frame while it runs Claude; `None` draws it standing with
/// its mood's mark, as the macOS menu bar does.
fn mascot_frame(state: &AppState, tick: Option<u64>) -> Option<Image<'static>> {
    use crate::mascot;
    let prefs = state.prefs();
    if !prefs.tray_mascot {
        return None;
    }
    let accounts = state
        .accounts
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    let keys: Vec<String> = accounts.iter().map(|a| a.identity_key()).collect();
    let active = accounts.iter().find(|a| a.active)?;
    let style = *mascot::resolve(&prefs.mascot_styles, &keys).get(&active.identity_key())?;
    let pose = match tick {
        Some(tick) => mascot::Pose::running(tick),
        None => mascot::Pose::still(mascot::mood(active, state.active_is_running())),
    };
    let (rgba, side) = mascot::rgba(&mascot::pixels(style, pose), 2);
    Some(Image::new_owned(rgba, side, side))
}

/// Walk the tray mascot while the active account runs Claude. Idle otherwise (no wakeups beyond a
/// once-a-second check), and still when Windows animations are off.
pub fn start_animation(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        let mut tick = 0_u64;
        loop {
            let state = app.state::<Arc<AppState>>();
            let walking =
                state.prefs().tray_mascot && state.active_is_running() && animations_enabled();
            if !walking {
                std::thread::sleep(std::time::Duration::from_secs(1));
                continue;
            }
            tick = tick.wrapping_add(1);
            if let (Some(tray), Some(image)) =
                (app.tray_by_id(TRAY_ID), mascot_frame(&state, Some(tick)))
            {
                let _ = tray.set_icon(Some(image));
            }
            std::thread::sleep(std::time::Duration::from_millis(1000 / WALK_FPS));
        }
    });
}

/// Windows' "Animation effects" setting (Settings › Accessibility › Visual effects).
#[cfg(windows)]
fn animations_enabled() -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SPI_GETCLIENTAREAANIMATION, SystemParametersInfoW,
    };
    let mut on: i32 = 1;
    // SAFETY: the call writes one BOOL into `on`, which outlives it.
    let ok =
        unsafe { SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, (&raw mut on).cast(), 0) };
    ok == 0 || on != 0
}

#[cfg(not(windows))]
fn animations_enabled() -> bool {
    true
}

/// Where the flyout goes for a tray icon at `tray` (physical pixels): centred over the icon, above it
/// when the taskbar is at the bottom and below it when the icon sits in the top half of the screen,
/// kept inside the monitor.
pub fn flyout_position(
    tray: (f64, f64, f64, f64),
    size: (f64, f64),
    monitor: (f64, f64, f64, f64),
    gap: f64,
) -> (f64, f64) {
    let (tx, ty, tw, th) = tray;
    let (w, h) = size;
    let (mx, my, mw, mh) = monitor;
    let x = (tx + tw / 2.0 - w / 2.0).clamp(mx + gap, (mx + mw - w - gap).max(mx + gap));
    let y = if ty + th / 2.0 > my + mh / 2.0 {
        ty - h - gap
    } else {
        ty + th + gap
    };
    (x, y.clamp(my + gap, (my + mh - h - gap).max(my + gap)))
}

fn toggle_flyout(app: &AppHandle, rect: Rect) {
    let window = match app.get_webview_window(FLYOUT) {
        Some(w) => w,
        None => match create_flyout(app) {
            Ok(w) => w,
            Err(e) => {
                log::error!("flyout: {e}");
                return;
            }
        },
    };
    if window.is_visible().unwrap_or(false) {
        let _ = window.hide();
        return;
    }
    let scale = window.scale_factor().unwrap_or(1.0);
    let pos = rect.position.to_physical::<f64>(scale);
    let size = rect.size.to_physical::<f64>(scale);
    let monitor = window
        .current_monitor()
        .ok()
        .flatten()
        .map(|m| {
            let area = m.work_area();
            (
                f64::from(area.position.x),
                f64::from(area.position.y),
                f64::from(area.size.width),
                f64::from(area.size.height),
            )
        })
        .unwrap_or((0.0, 0.0, 1920.0, 1080.0));
    let (x, y) = flyout_position(
        (pos.x, pos.y, size.width, size.height),
        (FLYOUT_W * scale, FLYOUT_H * scale),
        monitor,
        FLYOUT_GAP * scale,
    );
    let _ = window.set_size(PhysicalSize::new(FLYOUT_W * scale, FLYOUT_H * scale));
    let _ = window.set_position(PhysicalPosition::new(x, y));
    let _ = window.show();
    let _ = window.set_focus();
    let _ = app.emit_to(FLYOUT, "flyout-shown", ());
    app.state::<Arc<AppState>>().refresh_soon();
}

fn create_flyout(app: &AppHandle) -> tauri::Result<tauri::WebviewWindow> {
    let window = WebviewWindowBuilder::new(
        app,
        FLYOUT,
        WebviewUrl::App("index.html?view=flyout".into()),
    )
    .title("HaryX")
    .inner_size(FLYOUT_W, FLYOUT_H)
    .decorations(false)
    .resizable(false)
    .skip_taskbar(true)
    .always_on_top(true)
    .visible(false)
    .shadow(true)
    .theme(app.state::<Arc<AppState>>().prefs().theme.window_theme())
    .build()?;
    let handle = window.clone();
    window.on_window_event(move |event| {
        if let WindowEvent::Focused(false) = event {
            let _ = handle.hide();
        }
    });
    Ok(window)
}

/// Give every open window the Theme preference (title bar and the page's light/dark scheme).
pub fn apply_theme(app: &AppHandle) {
    let theme = app.state::<Arc<AppState>>().prefs().theme.window_theme();
    for window in app.webview_windows().values() {
        let _ = window.set_theme(theme);
    }
}

pub fn hide_flyout(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(FLYOUT) {
        let _ = w.hide();
    }
}

/// Show the dashboard (creating it the first time), optionally on `pane`.
pub fn show_dashboard(app: &AppHandle, pane: Option<&str>) {
    hide_flyout(app);
    let window = match app.get_webview_window(DASHBOARD) {
        Some(w) => w,
        None => {
            let built =
                WebviewWindowBuilder::new(app, DASHBOARD, WebviewUrl::App("index.html".into()))
                    .title("HaryX")
                    .inner_size(1080.0, 720.0)
                    .min_inner_size(760.0, 520.0)
                    .theme(app.state::<Arc<AppState>>().prefs().theme.window_theme())
                    .visible(false)
                    .build();
            match built {
                Ok(w) => {
                    let handle = w.clone();
                    w.on_window_event(move |event| {
                        if let WindowEvent::CloseRequested { api, .. } = event {
                            api.prevent_close();
                            let _ = handle.hide();
                        }
                    });
                    w
                }
                Err(e) => {
                    log::error!("dashboard: {e}");
                    return;
                }
            }
        }
    };
    let _ = window.show();
    let _ = window.unminimize();
    let _ = window.set_focus();
    if let Some(pane) = pane {
        let _ = app.emit_to(DASHBOARD, "open-pane", pane);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MONITOR: (f64, f64, f64, f64) = (0.0, 0.0, 1920.0, 1040.0);

    #[test]
    fn sits_above_a_bottom_taskbar_icon() {
        let (x, y) = flyout_position((1700.0, 1045.0, 24.0, 24.0), (380.0, 560.0), MONITOR, 8.0);
        assert_eq!(x, 1712.0 - 190.0);
        // The work area ends above the taskbar, so the flyout keeps the gap above that edge.
        assert_eq!(y, 1040.0 - 560.0 - 8.0);
    }

    #[test]
    fn stays_inside_the_screen_at_the_right_edge() {
        let (x, _) = flyout_position((1900.0, 1045.0, 24.0, 24.0), (380.0, 560.0), MONITOR, 8.0);
        assert_eq!(x, 1920.0 - 380.0 - 8.0);
    }

    #[test]
    fn drops_below_a_top_taskbar_icon() {
        let (_, y) = flyout_position((1700.0, 0.0, 24.0, 24.0), (380.0, 560.0), MONITOR, 8.0);
        assert_eq!(y, 32.0);
    }
}

//! Windows toast notifications. A failure to show one is logged, never fatal.

use tauri::AppHandle;
use tauri_plugin_notification::NotificationExt;

pub fn show(app: &AppHandle, title: &str, body: &str) {
    if let Err(e) = app.notification().builder().title(title).body(body).show() {
        log::warn!("notification failed: {e}");
    }
}

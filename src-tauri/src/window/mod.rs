//! Pet window behaviour: visibility, chat window routing, and (below) the
//! per-pixel click-through hit test and auto-walk engine.

pub mod hit_test;
pub mod pet_window;
pub mod walk;

use tauri::{AppHandle, Emitter, Manager};

/// Show and focus the chat window.
pub fn show_chat(app: &AppHandle) {
    tracing::debug!("showing chat window");
    if let Some(win) = app.get_webview_window("chat") {
        let _ = win.show();
        let _ = win.unminimize();
        let _ = win.set_focus();
    }
}

/// Show the chat window and ask the frontend to switch to the settings tab.
pub fn show_settings(app: &AppHandle) {
    show_chat(app);
    let _ = app.emit("ui://open-settings", ());
}

/// Toggle pet overlay visibility.
pub fn toggle_pet(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("pet") {
        match win.is_visible() {
            Ok(true) => {
                let _ = win.hide();
            }
            _ => {
                let _ = win.show();
            }
        }
    }
}

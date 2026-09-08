//! System tray: the only way to reach the app when the pet is hidden or
//! click-through is fully enabled.

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager};

pub fn build(app: &AppHandle) -> tauri::Result<()> {
    let chat = MenuItem::with_id(app, "open_chat", "打开聊天", true, None::<&str>)?;
    let settings = MenuItem::with_id(app, "open_settings", "设置", true, None::<&str>)?;
    let toggle = MenuItem::with_id(app, "toggle_pet", "显示 / 隐藏宠物", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&chat, &settings, &toggle, &separator, &quit])?;

    let mut builder = TrayIconBuilder::with_id("main")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .tooltip("桌宠")
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open_chat" => crate::window::show_chat(app),
            "open_settings" => crate::window::show_settings(app),
            "toggle_pet" => crate::window::toggle_pet(app),
            "quit" => {
                if let Some(state) = app.try_state::<crate::state::AppState>() {
                    state.chat.cancel_all();
                }
                app.exit(0)
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                crate::window::show_chat(tray.app_handle());
            }
        });

    if let Some(icon) = app.default_window_icon().cloned() {
        builder = builder.icon(icon);
    } else {
        tracing::warn!("no default window icon; tray icon will be blank");
    }

    builder.build(app)?;
    Ok(())
}

//! Desktop pet application shell: windows, tray, IPC command surface.

mod agent_bridge;
pub mod chat;
pub mod commands;
pub mod events;
pub mod state;
mod tray;
mod tts;
mod window;

use tauri::{Manager, WindowEvent};
use tracing_subscriber::EnvFilter;

/// Entry point used by `main.rs`.
pub fn run() {
    init_tracing();

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            window::show_chat(app);
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .setup(|app| {
            let state = state::AppState::initialize(app.handle())?;
            app.manage(state);
            tray::build(app.handle())?;
            window::pet_window::activate_active_pet(app.handle())?;
            // Wry-specific background loops: hit testing needs the global cursor
            // position, which the mock runtime does not provide.
            window::hit_test::start(app.handle().clone());
            window::walk::start(app.handle().clone());
            agent_bridge::start(app.handle().clone());

            // First run with no pet installed: open the chat window so the user
            // can see what to do instead of staring at nothing.
            let has_pet = app
                .try_state::<state::AppState>()
                .map(|s| !s.library.list().is_empty())
                .unwrap_or(false);
            if !has_pet {
                window::show_chat(app.handle());
            }

            #[cfg(target_os = "macos")]
            {
                // No Dock icon: this is a background companion, not a document app.
                let _ = app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "chat" {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_bootstrap_state,
            commands::list_pets,
            commands::validate_pet,
            commands::import_pet,
            commands::export_pet,
            commands::remove_pet,
            commands::use_pet,
            commands::list_personas,
            commands::persona_templates,
            commands::save_persona,
            commands::delete_persona,
            commands::duplicate_persona,
            commands::import_persona,
            commands::export_persona,
            commands::use_persona,
            commands::list_providers,
            commands::save_provider,
            commands::delete_provider,
            commands::test_provider,
            commands::set_api_key,
            commands::has_api_key,
            commands::list_conversations,
            commands::create_conversation,
            commands::get_messages,
            commands::delete_conversation,
            commands::send_message,
            commands::cancel_message,
            commands::list_facts,
            commands::delete_fact,
            commands::clear_memory,
            commands::get_settings,
            commands::save_settings,
            commands::set_pet_state,
            commands::speak,
            commands::stop_speaking,
            commands::hooks_status,
            commands::install_hooks,
            commands::uninstall_hooks,
            commands::open_chat,
            commands::hide_chat,
            commands::open_data_dir,
            commands::export_logs,
            commands::pet_spritesheet_data_url,
            commands::pet_ready,
            commands::pet_sprite_index,
            commands::pet_animation_finished,
            commands::pet_drag_started,
            commands::pet_drag_ended,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

fn init_tracing() {
    let filter = EnvFilter::try_from_env("BYTEPET_LOG")
        .unwrap_or_else(|_| EnvFilter::new("info,bytepet_app=debug,bytepet_core=debug"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .try_init();
}

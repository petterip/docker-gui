mod commands;
mod config;
mod error;
mod registry;

use config::AppState;
use registry::StacksRegistry;
use tauri::Manager;

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let app_state = tauri::async_runtime::block_on(AppState::new());
            app.manage(app_state);

            let registry = StacksRegistry::load(app.handle())
                .unwrap_or_else(|e| {
                    eprintln!("Warning: failed to load stacks registry: {e}. Starting with empty registry.");
                    StacksRegistry::empty()
                });
            app.manage(registry);

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::containers::list_containers,
            commands::containers::start_container,
            commands::containers::stop_container,
            commands::containers::restart_container,
            commands::containers::remove_container,
            commands::containers::get_container_logs,
            commands::containers::inspect_container,
            commands::images::list_images,
            commands::images::remove_image,
            commands::images::pull_image,
            commands::images::inspect_image,
            commands::volumes::list_volumes,
            commands::volumes::create_volume,
            commands::volumes::remove_volume,
            commands::volumes::inspect_volume,
            commands::compose::list_stacks,
            commands::compose::register_stack,
            commands::compose::remove_stack,
            commands::compose::stack_up,
            commands::compose::stack_down,
            commands::compose::stack_restart,
            commands::compose::stack_logs,
            commands::system::get_docker_info,
            commands::system::check_connection,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

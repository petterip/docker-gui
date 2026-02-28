mod commands;
mod config;
mod engine;
mod error;
mod registry;

use config::AppState;
use engine::EngineRegistry;
use registry::StacksRegistry;
use tauri::Manager;

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let app_state = tauri::async_runtime::block_on(AppState::new());
            let engine_registry = EngineRegistry::load(app.handle()).unwrap_or_else(|e| {
                eprintln!(
                    "Warning: failed to load engine provider registry: {e}. Starting with empty config."
                );
                EngineRegistry::empty()
            });
            let active_provider = tauri::async_runtime::block_on(engine_registry.get()).active_provider;
            if let Some(provider) = active_provider {
                let endpoint = provider.endpoint();
                tauri::async_runtime::block_on(
                    app_state.set_preferred_endpoint(Some(endpoint.clone())),
                );
                let _ = tauri::async_runtime::block_on(
                    app_state.reconnect_with_endpoint(&endpoint),
                );
            }

            app.manage(app_state);
            app.manage(engine_registry);

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
            commands::engine::get_engine_status,
            commands::engine::install_engine_provider,
            commands::engine::switch_active_engine,
            commands::engine::repair_active_engine,
            commands::engine::get_connection_guidance,
            commands::engine::start_engine_provisioning,
            commands::engine::retry_engine_provisioning,
            commands::engine::resume_engine_provisioning_if_needed,
            commands::engine::get_privileged_action_contract,
            commands::engine::set_custom_host_endpoint,
            commands::engine::list_wsl_engine_distros,
            commands::engine::set_wsl_engine_distro,
            commands::engine::get_engine_diagnostics,
            commands::engine::export_engine_diagnostics,
            commands::engine::remove_managed_engine,
            commands::system::get_docker_info,
            commands::system::check_connection,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

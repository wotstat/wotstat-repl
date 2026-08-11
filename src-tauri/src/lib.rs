mod commands;
mod install;
mod jedi;
mod mcp;
mod process;
mod protocol;
mod session;
mod transport;

use session::AppState;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::default())
        .setup(|app| {
            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }
            let state = app.state::<AppState>();
            let mcp = state.mcp.clone();
            let client = state.client.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(error) = mcp.start(client).await {
                    log::error!("cannot initialize MCP server: {error}");
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::ping,
            commands::mcp_connection_info,
            commands::mcp_set_enabled,
            commands::mcp_cli_status,
            commands::mcp_add_to_codex,
            commands::mcp_add_to_claude,
            commands::mcp_remove_from_codex,
            commands::mcp_remove_from_claude,
            commands::default_buffer_dir,
            commands::stubs_dir,
            commands::write_stubs,
            commands::detect_games,
            commands::inspect_game_dir,
            commands::install_agent,
            commands::launch_game,
            commands::list_clients,
            commands::start_client,
            commands::close_client,
            commands::kill_client,
            commands::connect,
            commands::disconnect,
            commands::exec_code,
            commands::complete,
            commands::inspect,
            commands::lint_code,
            commands::dump_object,
            commands::jedi_start,
            commands::jedi_complete,
            commands::jedi_lint,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

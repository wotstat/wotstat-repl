//! Tauri command surface: the only desktop<->backend boundary.

use std::sync::Arc;
use std::time::Duration;

use tauri::ipc::Channel;
use tauri::State;

use crate::install::{self, GameInfo};
use crate::mcp::{ConnectionInfo, McpActivitySnapshot, McpIntegrationStatus};
use crate::protocol::{InFrame, OutFrame, ServerEvent};
use crate::session::{AppState, ClientStatus, CloseResult};
use crate::transport::{AgentConnectionInfo, EventSink};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

async fn request_outframe(state: &State<'_, AppState>, frame: InFrame) -> Result<OutFrame, String> {
    state
        .client
        .request_with_timeout(frame, REQUEST_TIMEOUT)
        .await
}

#[tauri::command]
pub fn ping() -> &'static str {
    "pong"
}

#[tauri::command]
pub fn mcp_connection_info(state: State<'_, AppState>) -> Result<ConnectionInfo, String> {
    state.mcp.connection_info()
}

#[tauri::command]
pub fn mcp_activity(
    state: State<'_, AppState>,
    since_revision: Option<u64>,
) -> McpActivitySnapshot {
    state.mcp.activity(since_revision)
}

#[tauri::command]
pub async fn mcp_set_enabled(
    state: State<'_, AppState>,
    enabled: bool,
) -> Result<ConnectionInfo, String> {
    let mcp = state.mcp.clone();
    let client = state.client.clone();
    mcp.set_enabled(enabled, client).await
}

#[tauri::command]
pub async fn mcp_integration_status(
    state: State<'_, AppState>,
) -> Result<McpIntegrationStatus, String> {
    let mcp = state.mcp.clone();
    tauri::async_runtime::spawn_blocking(move || mcp.integration_status())
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn mcp_add_to_chatgpt_codex(state: State<'_, AppState>) -> Result<String, String> {
    let mcp = state.mcp.clone();
    tauri::async_runtime::spawn_blocking(move || mcp.add_to_chatgpt_codex())
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn mcp_add_to_claude(state: State<'_, AppState>) -> Result<String, String> {
    let mcp = state.mcp.clone();
    tauri::async_runtime::spawn_blocking(move || mcp.add_to_claude())
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn mcp_remove_from_chatgpt_codex(state: State<'_, AppState>) -> Result<String, String> {
    let mcp = state.mcp.clone();
    tauri::async_runtime::spawn_blocking(move || mcp.remove_from_chatgpt_codex())
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn mcp_remove_from_claude(state: State<'_, AppState>) -> Result<String, String> {
    let mcp = state.mcp.clone();
    tauri::async_runtime::spawn_blocking(move || mcp.remove_from_claude())
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub fn agent_connection_info(state: State<'_, AppState>) -> Result<AgentConnectionInfo, String> {
    state.client.connection_info()
}

// --- Automated setup (PJOrion-style) ------------------------------------------

#[tauri::command]
pub fn detect_games() -> Vec<GameInfo> {
    install::detect_games()
}

/// Validate a manually-picked folder; `None` if it isn't a WoT/Tanki install.
#[tauri::command]
pub fn inspect_game_dir(dir: String) -> Option<GameInfo> {
    install::inspect_dir(std::path::Path::new(&dir))
}

#[tauri::command]
pub fn install_agent(
    state: State<'_, AppState>,
    game_dir: String,
    mods_version: String,
) -> Result<(), String> {
    state.client.install_agent(&game_dir, &mods_version)
}

#[tauri::command]
pub fn launch_game(
    state: State<'_, AppState>,
    game_dir: String,
    exe: String,
    replay: Option<String>,
) -> Result<(), String> {
    state
        .client
        .launch(&game_dir, &exe, replay.as_deref())
        .map(|_| ())
}

#[tauri::command]
pub fn list_clients(state: State<'_, AppState>) -> Result<Vec<ClientStatus>, String> {
    state.client.list()
}

#[tauri::command]
pub fn start_client(
    state: State<'_, AppState>,
    game_dir: String,
    replay: Option<String>,
    on_event: Channel<ServerEvent>,
) -> Result<ClientStatus, String> {
    let sink: EventSink = Arc::new(move |event| {
        let _ = on_event.send(event);
    });
    state.client.start(&game_dir, replay.as_deref(), sink)
}

#[tauri::command]
pub async fn close_client(
    state: State<'_, AppState>,
    timeout_ms: Option<u64>,
) -> Result<CloseResult, String> {
    let client = state.client.clone();
    tauri::async_runtime::spawn_blocking(move || {
        client.close(Duration::from_millis(
            timeout_ms.unwrap_or(10_000).min(60_000),
        ))
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub fn kill_client(state: State<'_, AppState>) -> Result<ClientStatus, String> {
    state.client.kill()
}

// --- Session ------------------------------------------------------------------

#[tauri::command]
pub fn connect(
    state: State<'_, AppState>,
    lan_enabled: bool,
    secure_enabled: bool,
    on_event: Channel<ServerEvent>,
) -> Result<(), String> {
    let sink: EventSink = Arc::new(move |event| {
        let _ = on_event.send(event);
    });
    state.client.connect(lan_enabled, secure_enabled, sink)
}

#[tauri::command]
pub fn disconnect(state: State<'_, AppState>) -> Result<(), String> {
    state.client.disconnect()
}

#[tauri::command]
pub async fn exec_code(state: State<'_, AppState>, code: String) -> Result<OutFrame, String> {
    request_outframe(&state, InFrame::Exec { id: new_id(), code }).await
}

#[tauri::command]
pub async fn complete(
    state: State<'_, AppState>,
    prefix: String,
    budget: u32,
) -> Result<OutFrame, String> {
    request_outframe(
        &state,
        InFrame::Complete {
            id: new_id(),
            prefix,
            budget,
        },
    )
    .await
}

#[tauri::command]
pub async fn inspect(state: State<'_, AppState>, expr: String) -> Result<OutFrame, String> {
    request_outframe(&state, InFrame::Inspect { id: new_id(), expr }).await
}

#[tauri::command]
pub async fn lint_code(state: State<'_, AppState>, code: String) -> Result<OutFrame, String> {
    request_outframe(&state, InFrame::Lint { id: new_id(), code }).await
}

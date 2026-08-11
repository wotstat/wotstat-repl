//! Tauri command surface: the only desktop<->backend boundary.

use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use tauri::ipc::Channel;
use tauri::State;

use crate::install::{self, GameInfo};
use crate::jedi::JediWorker;
use crate::mcp::{self, ConnectionInfo, McpCliStatus};
use crate::protocol::{InFrame, OutFrame, ServerEvent};
use crate::session::{AppState, ClientStatus, CloseResult};
use crate::transport::EventSink;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

async fn await_value(rx: Receiver<Value>, timeout: Duration) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || rx.recv_timeout(timeout))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|_| "jedi worker did not respond in time".to_string())
}

fn jedi(state: &State<'_, AppState>) -> Result<Arc<JediWorker>, String> {
    state
        .jedi
        .lock()
        .map_err(|_| "state lock poisoned".to_string())?
        .clone()
        .ok_or_else(|| "jedi worker not started".to_string())
}

async fn request_outframe(state: &State<'_, AppState>, frame: InFrame) -> Result<OutFrame, String> {
    state
        .client
        .request_with_timeout(frame, REQUEST_TIMEOUT)
        .await
}

async fn jedi_request(
    worker: &JediWorker,
    payload: serde_json::Value,
    timeout: Duration,
) -> Result<serde_json::Value, String> {
    let rx = worker.request(payload).map_err(|e| e.to_string())?;
    await_value(rx, timeout).await
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
pub async fn mcp_set_enabled(
    state: State<'_, AppState>,
    enabled: bool,
) -> Result<ConnectionInfo, String> {
    let mcp = state.mcp.clone();
    let client = state.client.clone();
    mcp.set_enabled(enabled, client).await
}

#[tauri::command]
pub async fn mcp_cli_status() -> Result<McpCliStatus, String> {
    tauri::async_runtime::spawn_blocking(mcp::cli_status)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn mcp_add_to_codex(state: State<'_, AppState>) -> Result<String, String> {
    let mcp = state.mcp.clone();
    tauri::async_runtime::spawn_blocking(move || mcp.add_to_codex())
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
pub async fn mcp_remove_from_codex(state: State<'_, AppState>) -> Result<String, String> {
    let mcp = state.mcp.clone();
    tauri::async_runtime::spawn_blocking(move || mcp.remove_from_codex())
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
pub fn default_buffer_dir() -> String {
    install::default_buffer_dir_path()
        .to_string_lossy()
        .into_owned()
}

#[tauri::command]
pub fn stubs_dir() -> String {
    install::stubs_dir_path().to_string_lossy().into_owned()
}

/// Persist runtime-generated `.pyi` stubs to the canonical jedi sys_path root.
#[tauri::command]
pub fn write_stubs(stubs: std::collections::HashMap<String, String>) -> Result<String, String> {
    install::write_stubs(&stubs)
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
pub fn install_agent(game_dir: String, mods_version: String) -> Result<String, String> {
    install::install_agent(&game_dir, &mods_version)
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
    buffer_dir: String,
    on_event: Channel<ServerEvent>,
) -> Result<(), String> {
    let dir = PathBuf::from(&buffer_dir);
    let sink: EventSink = Arc::new(move |event| {
        let _ = on_event.send(event);
    });
    state.client.connect(dir, sink)
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

/// Deep runtime introspection of a live expression (e.g. "BigWorld.player()").
/// Persists any runtime-informed `.pyi` class stubs it returns to the stubs dir.
#[tauri::command]
pub async fn dump_object(
    state: State<'_, AppState>,
    expr: String,
    depth: u32,
) -> Result<OutFrame, String> {
    let frame = request_outframe(
        &state,
        InFrame::Dump {
            id: new_id(),
            expr,
            depth,
        },
    )
    .await?;
    if let OutFrame::Dump { stubs, .. } = &frame {
        if !stubs.is_empty() {
            let _ = install::write_stubs(stubs);
        }
    }
    Ok(frame)
}

// --- jedi static worker -------------------------------------------------------

#[tauri::command]
pub async fn jedi_start(
    state: State<'_, AppState>,
    python: String,
    script: String,
    root: String,
    sys_path: Vec<String>,
) -> Result<Value, String> {
    let worker = JediWorker::spawn(&python, &script).map_err(|e| e.to_string())?;
    let result = jedi_request(
        &worker,
        json!({ "op": "configure", "root": root, "sys_path": sys_path }),
        Duration::from_secs(20),
    )
    .await?;
    state
        .jedi
        .lock()
        .map_err(|_| "state lock poisoned".to_string())?
        .replace(Arc::clone(&worker));
    Ok(result)
}

#[tauri::command]
pub async fn jedi_complete(
    state: State<'_, AppState>,
    code: String,
    line: u32,
    column: u32,
) -> Result<Value, String> {
    let worker = jedi(&state)?;
    jedi_request(
        &worker,
        json!({ "op": "complete", "code": code, "line": line, "column": column }),
        REQUEST_TIMEOUT,
    )
    .await
}

#[tauri::command]
pub async fn jedi_lint(state: State<'_, AppState>, code: String) -> Result<Value, String> {
    let worker = jedi(&state)?;
    jedi_request(
        &worker,
        json!({ "op": "lint", "code": code }),
        REQUEST_TIMEOUT,
    )
    .await
}

//! Persistent settings and lifecycle for the embedded MCP server.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::Router;
use rmcp::handler::server::{router::tool::ToolRouter, wrapper::Parameters};
use rmcp::model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerInfo};
use rmcp::transport::streamable_http_server::{
    session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
};
use rmcp::{schemars, ServerHandler};
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use crate::install;
use crate::protocol::{InFrame, OutFrame};
use crate::session::ClientManager;

pub(crate) const MCP_BIND_IPV4: Ipv4Addr = Ipv4Addr::UNSPECIFIED;
pub(crate) const MCP_PORT: u16 = 8765;
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionInfo {
    pub enabled: bool,
    pub url: String,
    pub network_url: String,
    pub status: McpStatus,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct McpCliState {
    pub installed: bool,
    pub configured: bool,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct McpCliStatus {
    pub codex: McpCliState,
    pub claude: McpCliState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum McpStatus {
    Disabled,
    Starting,
    Listening,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct McpConfig {
    token: String,
    #[serde(default = "enabled_by_default")]
    enabled: bool,
}

pub struct McpConfigStore {
    path: PathBuf,
    lock: Mutex<()>,
}

#[derive(Clone)]
pub struct McpRuntime {
    store: Arc<McpConfigStore>,
    server: Arc<Mutex<ServerState>>,
    bind_address: SocketAddr,
}

struct ServerState {
    status: McpStatus,
    error: Option<String>,
    generation: u64,
    shutdown: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<()>>,
    bound_address: Option<SocketAddr>,
}

impl Default for McpRuntime {
    fn default() -> Self {
        Self::new(
            install::app_data_root().join("mcp.json"),
            SocketAddr::from((MCP_BIND_IPV4, MCP_PORT)),
        )
    }
}

impl McpRuntime {
    fn new(path: PathBuf, bind_address: SocketAddr) -> Self {
        Self {
            store: Arc::new(McpConfigStore::new(path)),
            server: Arc::new(Mutex::new(ServerState {
                status: McpStatus::Disabled,
                error: None,
                generation: 0,
                shutdown: None,
                task: None,
                bound_address: None,
            })),
            bind_address,
        }
    }

    pub fn connection_info(&self) -> Result<ConnectionInfo, String> {
        let config = self.store.load_or_create_locked()?;
        let state = self
            .server
            .lock()
            .map_err(|_| "MCP server lock poisoned".to_string())?;
        Ok(connection_info(config, state.status, state.error.clone()))
    }

    pub fn add_to_codex(&self) -> Result<String, String> {
        let url = local_url(&self.store.load_or_create_locked()?.token);
        run_cli("codex", &codex_add_args(&url))?;
        Ok("Added wot_repl to Codex.".to_string())
    }

    pub fn add_to_claude(&self) -> Result<String, String> {
        let url = local_url(&self.store.load_or_create_locked()?.token);
        run_cli("claude", &claude_add_args(&url))?;
        Ok("Added wot_repl to Claude Code.".to_string())
    }

    pub fn remove_from_codex(&self) -> Result<String, String> {
        run_cli("codex", &MCP_REMOVE_ARGS)?;
        Ok("Removed wot_repl from Codex.".to_string())
    }

    pub fn remove_from_claude(&self) -> Result<String, String> {
        run_cli("claude", &MCP_REMOVE_ARGS)?;
        Ok("Removed wot_repl from Claude Code.".to_string())
    }

    pub async fn start(&self, client: ClientManager) -> Result<ConnectionInfo, String> {
        let config = self.store.load_or_create_locked()?;
        if !config.enabled {
            self.stop().await;
            return self.connection_info();
        }

        let generation = {
            let mut state = self
                .server
                .lock()
                .map_err(|_| "MCP server lock poisoned".to_string())?;
            if matches!(state.status, McpStatus::Starting | McpStatus::Listening) {
                None
            } else {
                state.generation = state.generation.wrapping_add(1);
                state.status = McpStatus::Starting;
                state.error = None;
                Some(state.generation)
            }
        };
        let Some(generation) = generation else {
            return self.connection_info();
        };

        let listener = match tokio::net::TcpListener::bind(self.bind_address).await {
            Ok(listener) => listener,
            Err(error) => {
                self.record_start_error(generation, error.to_string());
                return self.connection_info();
            }
        };
        let bound_address = match listener.local_addr() {
            Ok(address) => address,
            Err(error) => {
                self.record_start_error(generation, error.to_string());
                return self.connection_info();
            }
        };
        let (shutdown, shutdown_rx) = oneshot::channel();
        let server_state = Arc::clone(&self.server);
        let server_config = StreamableHttpServerConfig::default()
            .disable_allowed_hosts()
            .with_json_response(true);
        let cancellation = server_config.cancellation_token.clone();
        let router = mcp_router(config.token, client, server_config);

        let mut state = self
            .server
            .lock()
            .map_err(|_| "MCP server lock poisoned".to_string())?;
        if state.generation != generation {
            drop(state);
            return self.connection_info();
        }
        let task = tokio::spawn(async move {
            let result = axum::serve(listener, router)
                .with_graceful_shutdown(async move {
                    let _ = shutdown_rx.await;
                    cancellation.cancel();
                })
                .await;
            let Ok(mut state) = server_state.lock() else {
                return;
            };
            if state.generation == generation {
                let error = result
                    .err()
                    .map(|error| error.to_string())
                    .unwrap_or_else(|| "MCP listener stopped unexpectedly".to_string());
                log::error!("{error}");
                state.status = McpStatus::Error;
                state.error = Some(error);
                state.shutdown = None;
                state.bound_address = None;
            }
        });
        state.status = McpStatus::Listening;
        state.error = None;
        state.shutdown = Some(shutdown);
        state.task = Some(task);
        state.bound_address = Some(bound_address);
        drop(state);
        self.connection_info()
    }

    pub async fn set_enabled(
        &self,
        enabled: bool,
        client: ClientManager,
    ) -> Result<ConnectionInfo, String> {
        self.store.set_enabled_locked(enabled)?;
        if enabled {
            self.start(client).await
        } else {
            self.stop().await;
            self.connection_info()
        }
    }

    pub async fn stop(&self) {
        let (shutdown, task) = match self.server.lock() {
            Ok(mut state) => {
                state.generation = state.generation.wrapping_add(1);
                state.status = McpStatus::Disabled;
                state.error = None;
                state.bound_address = None;
                (state.shutdown.take(), state.task.take())
            }
            Err(_) => return,
        };
        if let Some(shutdown) = shutdown {
            let _ = shutdown.send(());
        }
        if let Some(task) = task {
            let _ = task.await;
        }
    }

    fn record_start_error(&self, generation: u64, error: String) {
        let Ok(mut state) = self.server.lock() else {
            return;
        };
        if state.generation == generation {
            log::error!("cannot start MCP listener: {error}");
            state.status = McpStatus::Error;
            state.error = Some(error);
        }
    }
}

impl McpConfigStore {
    fn new(path: PathBuf) -> Self {
        Self {
            path,
            lock: Mutex::new(()),
        }
    }

    fn load_or_create_locked(&self) -> Result<McpConfig, String> {
        let _guard = self
            .lock
            .lock()
            .map_err(|_| "MCP config lock poisoned".to_string())?;
        self.load_or_create()
    }

    fn set_enabled_locked(&self, enabled: bool) -> Result<(), String> {
        let _guard = self
            .lock
            .lock()
            .map_err(|_| "MCP config lock poisoned".to_string())?;
        let mut config = self.load_or_create()?;
        config.enabled = enabled;
        write_config(&self.path, &config)
    }

    fn load_or_create(&self) -> Result<McpConfig, String> {
        if self.path.exists() {
            return read_config(&self.path);
        }
        let config = McpConfig {
            token: uuid::Uuid::new_v4().to_string(),
            enabled: true,
        };
        let parent = self
            .path
            .parent()
            .ok_or_else(|| "MCP config path has no parent".to_string())?;
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "cannot create MCP config directory {}: {error}",
                parent.display()
            )
        })?;
        let body = serialize_config(&config)?;
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&self.path)
        {
            Ok(mut file) => file.write_all(&body).map_err(|error| {
                format!("cannot write MCP config {}: {error}", self.path.display())
            })?,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                return read_config(&self.path)
            }
            Err(error) => {
                return Err(format!(
                    "cannot create MCP config {}: {error}",
                    self.path.display()
                ))
            }
        }
        Ok(config)
    }
}

fn enabled_by_default() -> bool {
    true
}

fn read_config(path: &Path) -> Result<McpConfig, String> {
    let body = fs::read_to_string(path)
        .map_err(|error| format!("cannot read MCP config {}: {error}", path.display()))?;
    let config: McpConfig = serde_json::from_str(&body)
        .map_err(|error| format!("invalid MCP config {}: {error}", path.display()))?;
    uuid::Uuid::parse_str(&config.token).map_err(|error| {
        format!(
            "invalid MCP config {}: invalid token: {error}",
            path.display()
        )
    })?;
    Ok(config)
}

fn write_config(path: &Path, config: &McpConfig) -> Result<(), String> {
    fs::write(path, serialize_config(config)?)
        .map_err(|error| format!("cannot write MCP config {}: {error}", path.display()))
}

fn serialize_config(config: &McpConfig) -> Result<Vec<u8>, String> {
    serde_json::to_vec_pretty(config).map_err(|error| error.to_string())
}

fn connection_info(config: McpConfig, status: McpStatus, error: Option<String>) -> ConnectionInfo {
    let url = local_url(&config.token);
    let network_url = format!(
        "http://{}:{MCP_PORT}/mcp?token={}",
        advertised_ipv4(),
        config.token
    );
    ConnectionInfo {
        enabled: config.enabled,
        url,
        network_url,
        status,
        error,
    }
}

fn local_url(token: &str) -> String {
    format!(
        "http://{}:{MCP_PORT}/mcp?token={token}",
        Ipv4Addr::LOCALHOST
    )
}

pub fn cli_status() -> McpCliStatus {
    McpCliStatus {
        codex: cli_state("codex"),
        claude: cli_state("claude"),
    }
}

fn cli_state(program: &str) -> McpCliState {
    let installed = cli_succeeds(program, &["--version"]);
    McpCliState {
        installed,
        configured: installed && cli_succeeds(program, &MCP_GET_ARGS),
    }
}

fn cli_succeeds(program: &str, args: &[&str]) -> bool {
    cli_output(program, args).is_ok_and(|output| output.status.success())
}

const MCP_GET_ARGS: [&str; 3] = ["mcp", "get", "wot_repl"];
const MCP_REMOVE_ARGS: [&str; 3] = ["mcp", "remove", "wot_repl"];

fn codex_add_args<'a>(url: &'a str) -> Vec<&'a str> {
    vec!["mcp", "add", "wot_repl", "--url", url]
}

fn claude_add_args<'a>(url: &'a str) -> Vec<&'a str> {
    vec![
        "mcp",
        "add",
        "--transport",
        "http",
        "--scope",
        "user",
        "wot_repl",
        url,
    ]
}

fn run_cli(program: &str, args: &[&str]) -> Result<(), String> {
    let output =
        cli_output(program, args).map_err(|error| format!("cannot run {program}: {error}"))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let detail = [stderr.trim(), stdout.trim()]
        .into_iter()
        .find(|text| !text.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| output.status.to_string());
    Err(format!("{program} failed: {detail}"))
}

#[cfg(windows)]
fn cli_output(program: &str, args: &[&str]) -> std::io::Result<Output> {
    Command::new("cmd")
        .creation_flags(CREATE_NO_WINDOW)
        .args(["/C", program])
        .args(args)
        .output()
}

#[cfg(not(windows))]
fn cli_output(program: &str, args: &[&str]) -> std::io::Result<Output> {
    Command::new(program).args(args).output()
}

fn advertised_ipv4() -> Ipv4Addr {
    UdpSocket::bind((MCP_BIND_IPV4, 0))
        .and_then(|socket| {
            socket.connect((Ipv4Addr::new(8, 8, 8, 8), 80))?;
            socket.local_addr()
        })
        .ok()
        .and_then(|address| match address.ip() {
            IpAddr::V4(ip) if !ip.is_loopback() && !ip.is_unspecified() => Some(ip),
            _ => None,
        })
        .unwrap_or(Ipv4Addr::LOCALHOST)
}

#[derive(Clone)]
struct WotMcpServer {
    client: ClientManager,
    tool_router: ToolRouter<Self>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct StartClientInput {
    /// Absolute path to a supported World of Tanks installation.
    game_dir: String,
    /// Optional absolute path to a replay opened when the client starts.
    replay_path: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct CloseClientInput {
    /// How long to wait for graceful shutdown, in milliseconds (default 10000, clamped to 0..60000).
    timeout_ms: Option<i64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ExecInput {
    /// Python source to execute.
    code: String,
    /// Response timeout in milliseconds (default 30000, clamped to 1..30000).
    timeout_ms: Option<i64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ReadLogInput {
    /// Return entries whose sequence is greater than this cursor; omit for the latest entries.
    cursor: Option<u64>,
    /// Maximum entries to return (default 200, clamped to 1..1000).
    limit: Option<i64>,
    /// Wait for a new entry when none is available (default 0, clamped to 0..5000 ms).
    wait_ms: Option<i64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ExecOutput {
    ok: bool,
    repr: Option<String>,
    stdout: String,
    stderr: String,
    exception: Option<String>,
    duration_ms: u64,
}

impl WotMcpServer {
    fn new(client: ClientManager) -> Self {
        Self {
            client,
            tool_router: Self::tool_router(),
        }
    }
}

#[rmcp::tool_router(router = tool_router)]
impl WotMcpServer {
    #[rmcp::tool(
        name = "wot_list_clients",
        description = "List all detected World of Tanks installations and their process and agent statuses.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn list_clients(&self) -> CallToolResult {
        match self.client.list() {
            Ok(clients) => {
                let text = format!("Found {} World of Tanks client(s).", clients.len());
                tool_success(text, serde_json::json!({ "clients": clients }))
            }
            Err(error) => tool_error(error),
        }
    }

    #[rmcp::tool(
        name = "wot_read_log",
        description = "Read recent stdout, stderr, and game log entries from the active client, optionally waiting briefly for new output.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn read_log(
        &self,
        Parameters(ReadLogInput {
            cursor,
            limit,
            wait_ms,
        }): Parameters<ReadLogInput>,
    ) -> CallToolResult {
        match self.client.read_log(cursor, limit, wait_ms).await {
            Ok(result) => {
                let text = format!("Returned {} log entries.", result.entries.len());
                tool_success(text, result)
            }
            Err(error) => tool_error(error),
        }
    }

    #[rmcp::tool(
        name = "wot_start_client",
        description = "Install and connect the agent if needed, then start the selected World of Tanks client. Starting the already active client is a no-op.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn start_client(
        &self,
        Parameters(StartClientInput {
            game_dir,
            replay_path,
        }): Parameters<StartClientInput>,
    ) -> CallToolResult {
        match self
            .client
            .start(&game_dir, replay_path.as_deref(), Arc::new(|_| {}))
        {
            Ok(client) => tool_success("Client started or already active.", client),
            Err(error) => tool_error(error),
        }
    }

    #[rmcp::tool(
        name = "wot_close_client",
        description = "Request a graceful close of the active World of Tanks client and wait for it to stop. This never force-kills the process.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn close_client(
        &self,
        Parameters(CloseClientInput { timeout_ms }): Parameters<CloseClientInput>,
    ) -> CallToolResult {
        let client = self.client.clone();
        let timeout = Duration::from_millis(timeout_ms.unwrap_or(10_000).clamp(0, 60_000) as u64);
        match tokio::task::spawn_blocking(move || client.close(timeout)).await {
            Ok(Ok(result)) => {
                let text = if result.still_running {
                    "Graceful close requested; client is still running."
                } else {
                    "Client closed."
                };
                tool_success(text, result)
            }
            Ok(Err(error)) => tool_error(error),
            Err(error) => tool_error(format!("close task failed: {error}")),
        }
    }

    #[rmcp::tool(
        name = "wot_exec",
        description = "Execute arbitrary Python code in the active World of Tanks client on its main thread. The code can mutate game state and interact with external systems.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn exec(
        &self,
        Parameters(ExecInput { code, timeout_ms }): Parameters<ExecInput>,
    ) -> CallToolResult {
        if code.trim().is_empty() {
            return tool_error("code must not be blank");
        }
        if code.len() > 262_144 {
            return tool_error("code exceeds the 262144-byte UTF-8 limit");
        }

        let started = Instant::now();
        let timeout = Duration::from_millis(timeout_ms.unwrap_or(30_000).clamp(1, 30_000) as u64);
        let frame = self
            .client
            .request_with_timeout(
                InFrame::Exec {
                    id: uuid::Uuid::new_v4().to_string(),
                    code,
                },
                timeout,
            )
            .await;
        match frame {
            Ok(OutFrame::Result {
                ok,
                repr,
                exc,
                stdout,
                stderr,
                ..
            }) => tool_success(
                if ok {
                    "Python execution succeeded."
                } else {
                    "Python execution raised an exception."
                },
                ExecOutput {
                    ok,
                    repr,
                    stdout,
                    stderr,
                    exception: exc,
                    duration_ms: started.elapsed().as_millis() as u64,
                },
            ),
            Ok(_) => tool_error("agent returned an unexpected response to exec"),
            Err(error) => tool_error(error),
        }
    }

    #[rmcp::tool(
        name = "wot_kill_client",
        description = "Forcefully terminate the active World of Tanks client after verifying the saved process identity.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn kill_client(&self) -> CallToolResult {
        match self.client.kill() {
            Ok(client) => tool_success("Client kill requested.", client),
            Err(error) => tool_error(error),
        }
    }
}

fn tool_success(text: impl Into<String>, value: impl Serialize) -> CallToolResult {
    match serde_json::to_value(value) {
        Ok(value) => {
            let mut result = CallToolResult::structured(value);
            result.content = vec![ContentBlock::text(text)];
            result
        }
        Err(error) => tool_error(format!("cannot serialize tool result: {error}")),
    }
}

fn tool_error(error: impl Into<String>) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(error)])
}

#[rmcp::tool_handler(router = self.tool_router)]
impl ServerHandler for WotMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build()).with_server_info(
            Implementation::new("fuflo-wot-repl", env!("CARGO_PKG_VERSION"))
                .with_title("Fuflo World of Tanks REPL")
                .with_description(
                    "A development MCP server for controlling a local World of Tanks client and executing Python 2.7 code inside the running game.",
                ),
        )
        .with_instructions(
            "WoT means World of Tanks, not Web of Things. Start with wot_list_clients. Only one client can be active. Use wot_close_client for graceful shutdown; use wot_kill_client only if the client is unresponsive. wot_exec runs arbitrary Python 2.7 inside the game process.",
        )
    }
}

fn mcp_router(token: String, client: ClientManager, config: StreamableHttpServerConfig) -> Router {
    let service: StreamableHttpService<WotMcpServer, LocalSessionManager> =
        StreamableHttpService::new(
            move || Ok(WotMcpServer::new(client.clone())),
            Default::default(),
            config,
        );
    Router::new()
        .route_service("/mcp", service)
        .route_layer(middleware::from_fn_with_state(token, require_token))
}

async fn require_token(State(token): State<String>, request: Request, next: Next) -> Response {
    if request.uri().query() == Some(format!("token={token}").as_str()) {
        next.run(request).await
    } else {
        StatusCode::UNAUTHORIZED.into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn temp_path() -> PathBuf {
        std::env::temp_dir()
            .join(format!("wms_mcp_config_{}", uuid::Uuid::new_v4()))
            .join("mcp.json")
    }

    #[test]
    fn creates_reloads_and_toggles_without_changing_token() {
        let path = temp_path();
        let first = McpConfigStore::new(path.clone());
        let created = first.load_or_create().unwrap();
        assert!(created.enabled);
        let info = connection_info(created.clone(), McpStatus::Disabled, None);
        assert!(info.enabled);
        assert_eq!(
            info.url,
            format!(
                "http://{}:{MCP_PORT}/mcp?token={}",
                Ipv4Addr::LOCALHOST,
                created.token
            )
        );
        assert_eq!(
            info.network_url,
            format!(
                "http://{}:{MCP_PORT}/mcp?token={}",
                advertised_ipv4(),
                created.token
            )
        );
        assert_eq!(
            serde_json::to_value(&info).unwrap(),
            serde_json::json!({
                "enabled": true,
                "url": info.url,
                "networkUrl": info.network_url,
                "status": "disabled",
                "error": null,
            })
        );

        let second = McpConfigStore::new(path.clone());
        let reloaded = second.load_or_create().unwrap();
        assert_eq!(reloaded.token, created.token);
        second.set_enabled_locked(false).unwrap();

        let toggled = McpConfigStore::new(path.clone()).load_or_create().unwrap();
        assert!(!toggled.enabled);
        assert_eq!(toggled.token, created.token);
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn cli_arguments_are_exact() {
        let url = "http://127.0.0.1:8765/mcp?token=test";
        assert_eq!(
            codex_add_args(url),
            ["mcp", "add", "wot_repl", "--url", url]
        );
        assert_eq!(
            claude_add_args(url),
            [
                "mcp",
                "add",
                "--transport",
                "http",
                "--scope",
                "user",
                "wot_repl",
                url,
            ]
        );
        assert_eq!(MCP_GET_ARGS, ["mcp", "get", "wot_repl"]);
        assert_eq!(MCP_REMOVE_ARGS, ["mcp", "remove", "wot_repl"]);
    }

    #[test]
    fn invalid_token_is_reported_without_overwrite() {
        let path = temp_path();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let body = serde_json::json!({
            "token": "not-a-uuid\nurl = \"http://evil\"",
            "enabled": true,
        })
        .to_string();
        fs::write(&path, &body).unwrap();

        let error = McpConfigStore::new(path.clone())
            .load_or_create_locked()
            .unwrap_err();
        assert!(error.contains("invalid MCP config"));
        assert_eq!(fs::read_to_string(&path).unwrap(), body);
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn exec_output_has_no_internal_correlation_id() {
        let value = serde_json::to_value(ExecOutput {
            ok: true,
            repr: Some("42".into()),
            stdout: "out".into(),
            stderr: "err".into(),
            exception: None,
            duration_ms: 12,
        })
        .unwrap();
        assert_eq!(
            value,
            serde_json::json!({
                "ok": true,
                "repr": "42",
                "stdout": "out",
                "stderr": "err",
                "exception": null,
                "durationMs": 12,
            })
        );
    }

    #[tokio::test]
    async fn listener_checks_token_and_releases_ephemeral_port() {
        let path = temp_path();
        let runtime = McpRuntime::new(path.clone(), "127.0.0.1:0".parse().unwrap());
        let client = ClientManager::default();

        let started = runtime.start(client.clone()).await.unwrap();
        assert_eq!(started.status, McpStatus::Listening);
        let address = runtime.server.lock().unwrap().bound_address.unwrap();
        let token = runtime.store.load_or_create_locked().unwrap().token;
        assert_eq!(http_status(address, "/mcp").await, 401);
        assert_eq!(http_status(address, "/mcp?token=wrong").await, 401);
        assert_eq!(
            http_status(address, &format!("/mcp?token={token}&extra=1")).await,
            401
        );

        const PROTOCOL_VERSION: &str = "2025-11-25";
        let endpoint = format!("/mcp?token={token}");
        let initialized = http_post(
            address,
            &endpoint,
            None,
            &format!(
                r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{"protocolVersion":"{PROTOCOL_VERSION}","capabilities":{{}},"clientInfo":{{"name":"raw-smoke","version":"1"}}}}}}"#
            ),
        )
        .await;
        assert_eq!(initialized.status, 200, "{}", initialized.body);
        let initialized_body = json_body(&initialized);
        assert_eq!(
            initialized_body["result"]["serverInfo"]["name"],
            "fuflo-wot-repl"
        );
        assert_eq!(
            initialized_body["result"]["serverInfo"]["title"],
            "Fuflo World of Tanks REPL"
        );
        let description = initialized_body["result"]["serverInfo"]["description"]
            .as_str()
            .unwrap();
        assert!(description.contains("World of Tanks"));
        assert!(description.contains("Python 2.7"));
        let instructions = initialized_body["result"]["instructions"].as_str().unwrap();
        assert!(instructions.contains("not Web of Things"));
        assert!(instructions.contains("wot_list_clients"));
        assert!(instructions.contains("Only one client"));
        assert!(instructions.contains("wot_close_client for graceful shutdown"));
        assert!(instructions.contains("wot_kill_client only if the client is unresponsive"));
        let session_id = initialized.header("mcp-session-id").unwrap();

        let notification = http_post(
            address,
            &endpoint,
            Some(session_id),
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
        )
        .await;
        assert_eq!(notification.status, 202, "{}", notification.body);

        let tools = http_post(
            address,
            &endpoint,
            Some(session_id),
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}"#,
        )
        .await;
        assert_eq!(tools.status, 200, "{}", tools.body);
        let tools_body = json_body(&tools);
        let tool_names: Vec<_> = tools_body["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| tool["name"].as_str().unwrap())
            .collect();
        assert_eq!(
            tool_names,
            [
                "wot_close_client",
                "wot_exec",
                "wot_kill_client",
                "wot_list_clients",
                "wot_read_log",
                "wot_start_client",
            ]
        );

        let close = http_post(
            address,
            &endpoint,
            Some(session_id),
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"wot_close_client","arguments":{"timeout_ms":-1}}}"#,
        )
        .await;
        assert_eq!(close.status, 200, "{}", close.body);
        let close_body = json_body(&close);
        assert_eq!(close_body["result"]["isError"], true);
        assert!(close_body["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("NO_ACTIVE_CLIENT"));

        let exec = http_post(
            address,
            &endpoint,
            Some(session_id),
            r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"wot_exec","arguments":{"code":"1 + 1"}}}"#,
        )
        .await;
        assert_eq!(exec.status, 200, "{}", exec.body);
        let exec_body = json_body(&exec);
        assert_eq!(exec_body["result"]["isError"], true);
        assert!(exec_body["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("not connected"));

        let list = http_post(
            address,
            &endpoint,
            Some(session_id),
            r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"wot_list_clients","arguments":{}}}"#,
        )
        .await;
        assert_eq!(list.status, 200, "{}", list.body);
        let list_body = json_body(&list);
        assert_eq!(list_body["result"]["isError"], false);
        assert!(list_body["result"]["structuredContent"].is_object());
        assert!(list_body["result"]["structuredContent"]["clients"].is_array());
        assert!(list_body["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .starts_with("Found "));

        let log = http_post(
            address,
            &endpoint,
            Some(session_id),
            r#"{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"wot_read_log","arguments":{}}}"#,
        )
        .await;
        assert_eq!(log.status, 200, "{}", log.body);
        let log_body = json_body(&log);
        assert_eq!(log_body["result"]["isError"], false);
        assert_eq!(
            log_body["result"]["structuredContent"],
            serde_json::json!({"entries": [], "nextCursor": 0, "truncated": false})
        );

        let repeated = runtime.start(client).await.unwrap();
        assert_eq!(repeated.status, McpStatus::Listening);
        assert_eq!(runtime.server.lock().unwrap().bound_address, Some(address));

        let stopped = runtime
            .set_enabled(false, ClientManager::default())
            .await
            .unwrap();
        assert_eq!(stopped.status, McpStatus::Disabled);
        let stopped_again = runtime
            .set_enabled(false, ClientManager::default())
            .await
            .unwrap();
        assert_eq!(stopped_again.status, McpStatus::Disabled);
        let rebound = tokio::net::TcpListener::bind(address).await.unwrap();
        drop(rebound);

        let restarted = runtime
            .set_enabled(true, ClientManager::default())
            .await
            .unwrap();
        assert_eq!(restarted.status, McpStatus::Listening);
        runtime
            .set_enabled(false, ClientManager::default())
            .await
            .unwrap();
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[tokio::test]
    async fn occupied_port_becomes_error_status() {
        let path = temp_path();
        let occupied = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = occupied.local_addr().unwrap();
        let runtime = McpRuntime::new(path.clone(), address);

        let info = runtime.start(ClientManager::default()).await.unwrap();
        assert_eq!(info.status, McpStatus::Error);
        assert!(info.error.is_some());
        drop(occupied);
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    async fn http_status(address: SocketAddr, path: &str) -> u16 {
        http_request(address, path, "", None, false).await.status
    }

    async fn http_post(
        address: SocketAddr,
        path: &str,
        session_id: Option<&str>,
        body: &str,
    ) -> HttpResponse {
        http_request(address, path, body, session_id, true).await
    }

    async fn http_request(
        address: SocketAddr,
        path: &str,
        body: &str,
        session_id: Option<&str>,
        mcp_headers: bool,
    ) -> HttpResponse {
        let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
        let session_header = session_id
            .map(|id| format!("Mcp-Session-Id: {id}\r\n"))
            .unwrap_or_default();
        let mcp_headers = mcp_headers
            .then_some(
                "Content-Type: application/json\r\nAccept: application/json, text/event-stream\r\nMCP-Protocol-Version: 2025-11-25\r\n",
            )
            .unwrap_or_default();
        stream
            .write_all(
                format!(
                    "POST {path} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n{mcp_headers}{session_header}Content-Length: {}\r\n\r\n{body}",
                    body.len()
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).await.unwrap();
        let (headers, body) = response.split_once("\r\n\r\n").unwrap();
        let mut lines = headers.lines();
        let status = lines
            .next()
            .unwrap()
            .split_whitespace()
            .nth(1)
            .unwrap()
            .parse()
            .unwrap();
        let headers: std::collections::HashMap<String, String> = lines
            .filter_map(|line| line.split_once(':'))
            .map(|(name, value)| (name.to_ascii_lowercase(), value.trim().to_string()))
            .collect();
        let body = if headers
            .get("transfer-encoding")
            .is_some_and(|value| value.eq_ignore_ascii_case("chunked"))
        {
            decode_chunked(body)
        } else {
            body.to_string()
        };
        HttpResponse {
            status,
            headers,
            body,
        }
    }

    fn decode_chunked(mut input: &str) -> String {
        let mut body = String::new();
        loop {
            let (size, rest) = input.split_once("\r\n").unwrap();
            let size = usize::from_str_radix(size.split(';').next().unwrap(), 16).unwrap();
            if size == 0 {
                return body;
            }
            let (chunk, rest) = rest.split_at(size);
            body.push_str(chunk);
            input = rest.strip_prefix("\r\n").unwrap();
        }
    }

    fn json_body(response: &HttpResponse) -> serde_json::Value {
        let body = response
            .body
            .lines()
            .find_map(|line| {
                line.strip_prefix("data: ")
                    .filter(|data| data.starts_with('{'))
            })
            .unwrap_or(&response.body);
        serde_json::from_str(body).unwrap_or_else(|error| panic!("{error}: {:?}", response.body))
    }

    struct HttpResponse {
        status: u16,
        headers: std::collections::HashMap<String, String>,
        body: String,
    }

    impl HttpResponse {
        fn header(&self, name: &str) -> Option<&str> {
            self.headers
                .get(&name.to_ascii_lowercase())
                .map(String::as_str)
        }
    }
}

//! Persistent settings and lifecycle for the embedded MCP server.

use std::collections::VecDeque;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use atomic_write_file::AtomicWriteFile;
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::Router;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use rmcp::handler::server::{router::tool::ToolRouter, wrapper::Parameters};
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation,
    ProgressNotificationParam, ServerCapabilities, ServerInfo,
};
use rmcp::service::RequestContext;
use rmcp::transport::streamable_http_server::{
    session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
};
use rmcp::{schemars, RoleServer, ServerHandler};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use toml_edit::{value, DocumentMut, Item, Table};

use crate::install;
use crate::protocol::{InFrame, OutFrame};
use crate::screenshot::PendingScreenshot;
use crate::session::{
    AgentStatus, ClientCapabilities, ClientKind, ClientManager, ClientStatus, CloseResult,
    ProcessStatus,
};

pub(crate) const MCP_BIND_IPV4: Ipv4Addr = Ipv4Addr::UNSPECIFIED;
pub(crate) const MCP_PORT: u16 = 8765;
const MCP_ACTIVITY_LIMIT: usize = 500;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionInfo {
    pub enabled: bool,
    pub url: String,
    pub network_url: String,
    pub mode: McpMode,
    pub status: McpStatus,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum McpMode {
    Full,
    RemoteRepl,
}

impl McpMode {
    const fn for_current_platform() -> Self {
        if cfg!(windows) {
            Self::Full
        } else {
            Self::RemoteRepl
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpIntegrationState {
    pub available: bool,
    pub configured: bool,
    pub config_path: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpIntegrationStatus {
    pub chatgpt_codex: McpIntegrationState,
    pub claude_code: McpIntegrationState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum McpStatus {
    Disabled,
    Starting,
    Listening,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum McpActivityStatus {
    Pending,
    Success,
    Error,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpActivityEntry {
    pub id: u64,
    pub command: String,
    pub status: McpActivityStatus,
    pub started_at_ms: u64,
    pub finished_at_ms: Option<u64>,
    pub duration_ms: Option<u64>,
    pub request: Value,
    pub response: Option<Value>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpActivitySnapshot {
    pub revision: u64,
    pub entries: Option<Vec<McpActivityEntry>>,
}

#[derive(Clone, Default)]
struct McpActivityLog {
    store: Arc<Mutex<McpActivityStore>>,
}

#[derive(Default)]
struct McpActivityStore {
    entries: VecDeque<McpActivityEntry>,
    next_id: u64,
    revision: u64,
}

struct McpActivityGuard {
    log: McpActivityLog,
    id: u64,
    started: Instant,
    finished: bool,
}

impl McpActivityLog {
    fn start(&self, command: String, request: Value) -> McpActivityGuard {
        let mut id = 0;
        if let Ok(mut store) = self.store.lock() {
            store.next_id = store.next_id.wrapping_add(1);
            id = store.next_id;
            if store.entries.len() == MCP_ACTIVITY_LIMIT {
                store.entries.pop_front();
            }
            store.entries.push_back(McpActivityEntry {
                id,
                command,
                status: McpActivityStatus::Pending,
                started_at_ms: unix_time_ms(),
                finished_at_ms: None,
                duration_ms: None,
                request,
                response: None,
            });
            store.revision = store.revision.wrapping_add(1);
        }
        McpActivityGuard {
            log: self.clone(),
            id,
            started: Instant::now(),
            finished: false,
        }
    }

    fn snapshot(&self, since_revision: Option<u64>) -> McpActivitySnapshot {
        let Ok(store) = self.store.lock() else {
            return McpActivitySnapshot {
                revision: 0,
                entries: Some(Vec::new()),
            };
        };
        McpActivitySnapshot {
            revision: store.revision,
            entries: (since_revision != Some(store.revision))
                .then(|| store.entries.iter().rev().cloned().collect()),
        }
    }

    fn finish(&self, id: u64, status: McpActivityStatus, duration: Duration, response: Value) {
        let Ok(mut store) = self.store.lock() else {
            return;
        };
        let Some(entry) = store.entries.iter_mut().find(|entry| entry.id == id) else {
            return;
        };
        entry.status = status;
        entry.finished_at_ms = Some(unix_time_ms());
        entry.duration_ms = Some(duration.as_millis().min(u64::MAX as u128) as u64);
        entry.response = Some(response);
        store.revision = store.revision.wrapping_add(1);
    }
}

impl McpActivityGuard {
    fn complete(mut self, result: &Result<CallToolResponse, rmcp::ErrorData>) {
        let (status, response) = activity_response(result);
        self.log
            .finish(self.id, status, self.started.elapsed(), response);
        self.finished = true;
    }
}

impl Drop for McpActivityGuard {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        self.log.finish(
            self.id,
            McpActivityStatus::Error,
            self.started.elapsed(),
            serde_json::json!({
                "error": {
                    "message": "MCP command was cancelled before a response was produced"
                }
            }),
        );
    }
}

fn unix_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}

fn activity_response(
    result: &Result<CallToolResponse, rmcp::ErrorData>,
) -> (McpActivityStatus, Value) {
    match result {
        Ok(CallToolResponse::Complete(response)) => {
            let structured_failure = response
                .structured_content
                .as_ref()
                .and_then(|value| value.get("ok"))
                .and_then(Value::as_bool)
                == Some(false);
            let status = if response.is_error == Some(true) || structured_failure {
                McpActivityStatus::Error
            } else {
                McpActivityStatus::Success
            };
            (
                status,
                serde_json::json!({
                    "result": serde_json::to_value(response).unwrap_or(Value::Null)
                }),
            )
        }
        Ok(CallToolResponse::InputRequired(response)) => (
            McpActivityStatus::Success,
            serde_json::json!({
                "result": serde_json::to_value(response).unwrap_or(Value::Null)
            }),
        ),
        Ok(CallToolResponse::Task(response)) => (
            McpActivityStatus::Success,
            serde_json::json!({
                "result": serde_json::to_value(response).unwrap_or(Value::Null)
            }),
        ),
        Ok(response) => (
            McpActivityStatus::Success,
            serde_json::json!({ "result": format!("Unsupported response: {response:?}") }),
        ),
        Err(error) => (
            McpActivityStatus::Error,
            serde_json::json!({
                "error": serde_json::to_value(error).unwrap_or(Value::Null)
            }),
        ),
    }
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

struct CodexConfigStore {
    path: Result<PathBuf, String>,
    lock: Mutex<()>,
}

struct ClaudeConfigStore {
    path: Result<PathBuf, String>,
    lock: Mutex<()>,
}

#[derive(Clone)]
pub struct McpRuntime {
    store: Arc<McpConfigStore>,
    codex_config: Arc<CodexConfigStore>,
    claude_config: Arc<ClaudeConfigStore>,
    server: Arc<Mutex<ServerState>>,
    activity: McpActivityLog,
    bind_address: SocketAddr,
    mode: McpMode,
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
        Self::with_mode(path, bind_address, McpMode::for_current_platform())
    }

    fn with_mode(path: PathBuf, bind_address: SocketAddr, mode: McpMode) -> Self {
        Self {
            store: Arc::new(McpConfigStore::new(path)),
            codex_config: Arc::new(CodexConfigStore::default()),
            claude_config: Arc::new(ClaudeConfigStore::default()),
            server: Arc::new(Mutex::new(ServerState {
                status: McpStatus::Disabled,
                error: None,
                generation: 0,
                shutdown: None,
                task: None,
                bound_address: None,
            })),
            activity: McpActivityLog::default(),
            bind_address,
            mode,
        }
    }

    pub fn connection_info(&self) -> Result<ConnectionInfo, String> {
        let config = self.store.load_or_create_locked()?;
        let state = self
            .server
            .lock()
            .map_err(|_| "MCP server lock poisoned".to_string())?;
        Ok(connection_info(
            config,
            self.mode,
            state.status,
            state.error.clone(),
        ))
    }

    pub fn integration_status(&self) -> Result<McpIntegrationStatus, String> {
        let url = local_url(&self.store.load_or_create_locked()?.token);
        Ok(McpIntegrationStatus {
            chatgpt_codex: self.codex_config.status(&url),
            claude_code: self.claude_config.status(&url),
        })
    }

    pub fn activity(&self, since_revision: Option<u64>) -> McpActivitySnapshot {
        self.activity.snapshot(since_revision)
    }

    pub fn add_to_chatgpt_codex(&self) -> Result<String, String> {
        let url = local_url(&self.store.load_or_create_locked()?.token);
        self.codex_config.add(&url)?;
        Ok(
            "Added wot_repl to ChatGPT Desktop and Codex. Restart ChatGPT/Codex to load it."
                .to_string(),
        )
    }

    pub fn add_to_claude(&self) -> Result<String, String> {
        let url = local_url(&self.store.load_or_create_locked()?.token);
        self.claude_config.add(&url)?;
        Ok("Added wot_repl to Claude Code. Restart Claude Code to load it.".to_string())
    }

    pub fn remove_from_chatgpt_codex(&self) -> Result<String, String> {
        self.codex_config.remove()?;
        Ok(
            "Removed wot_repl from ChatGPT Desktop and Codex. Restart ChatGPT/Codex to apply the change."
                .to_string(),
        )
    }

    pub fn remove_from_claude(&self) -> Result<String, String> {
        self.claude_config.remove()?;
        Ok(
            "Removed wot_repl from Claude Code. Restart Claude Code to apply the change."
                .to_string(),
        )
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
        let router = mcp_router(
            config.token,
            client,
            self.activity.clone(),
            self.mode,
            server_config,
        );

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

impl Default for CodexConfigStore {
    fn default() -> Self {
        Self {
            path: codex_config_path(),
            lock: Mutex::new(()),
        }
    }
}

impl CodexConfigStore {
    #[cfg(test)]
    fn new(path: PathBuf) -> Self {
        Self {
            path: Ok(path),
            lock: Mutex::new(()),
        }
    }

    fn path(&self) -> Result<&Path, String> {
        self.path
            .as_ref()
            .map(PathBuf::as_path)
            .map_err(Clone::clone)
    }

    fn status(&self, expected_url: &str) -> McpIntegrationState {
        let path = match self.path() {
            Ok(path) => path,
            Err(error) => {
                return McpIntegrationState {
                    available: false,
                    configured: false,
                    config_path: None,
                    error: Some(error),
                };
            }
        };
        let config_path = Some(path.display().to_string());
        let _guard = match self.lock.lock() {
            Ok(guard) => guard,
            Err(_) => {
                return McpIntegrationState {
                    available: false,
                    configured: false,
                    config_path,
                    error: Some("ChatGPT/Codex config lock poisoned".to_string()),
                };
            }
        };
        match read_codex_document(path).and_then(|document| {
            codex_server_url(&document, path)
                .map(|configured_url| configured_url == Some(expected_url))
        }) {
            Ok(configured) => McpIntegrationState {
                available: true,
                configured,
                config_path,
                error: None,
            },
            Err(error) => McpIntegrationState {
                available: false,
                configured: false,
                config_path,
                error: Some(error),
            },
        }
    }

    fn add(&self, url: &str) -> Result<(), String> {
        let path = self.path()?;
        let _guard = self
            .lock
            .lock()
            .map_err(|_| "ChatGPT/Codex config lock poisoned".to_string())?;
        let mut document = read_codex_document(path)?;

        if !document.contains_key("mcp_servers") {
            document["mcp_servers"] = Item::Table(Table::new());
        }
        let servers = document["mcp_servers"].as_table_like_mut().ok_or_else(|| {
            format!(
                "invalid ChatGPT/Codex config {}: mcp_servers must be a table",
                path.display()
            )
        })?;
        let mut server = Table::new();
        server["url"] = value(url);
        servers.insert("wot_repl", Item::Table(server));

        write_codex_document(path, &document)
    }

    fn remove(&self) -> Result<(), String> {
        let path = self.path()?;
        let _guard = self
            .lock
            .lock()
            .map_err(|_| "ChatGPT/Codex config lock poisoned".to_string())?;
        if !path.exists() {
            return Ok(());
        }
        let mut document = read_codex_document(path)?;
        let (removed, remove_parent) = {
            let Some(servers) = document.get_mut("mcp_servers") else {
                return Ok(());
            };
            let servers = servers.as_table_like_mut().ok_or_else(|| {
                format!(
                    "invalid ChatGPT/Codex config {}: mcp_servers must be a table",
                    path.display()
                )
            })?;
            let removed = servers.remove("wot_repl").is_some();
            (removed, servers.is_empty())
        };
        if !removed {
            return Ok(());
        }
        if remove_parent {
            document.remove("mcp_servers");
        }
        write_codex_document(path, &document)
    }
}

impl Default for ClaudeConfigStore {
    fn default() -> Self {
        Self {
            path: claude_config_path(),
            lock: Mutex::new(()),
        }
    }
}

impl ClaudeConfigStore {
    #[cfg(test)]
    fn new(path: PathBuf) -> Self {
        Self {
            path: Ok(path),
            lock: Mutex::new(()),
        }
    }

    fn path(&self) -> Result<&Path, String> {
        self.path
            .as_ref()
            .map(PathBuf::as_path)
            .map_err(Clone::clone)
    }

    fn status(&self, expected_url: &str) -> McpIntegrationState {
        let path = match self.path() {
            Ok(path) => path,
            Err(error) => {
                return McpIntegrationState {
                    available: false,
                    configured: false,
                    config_path: None,
                    error: Some(error),
                };
            }
        };
        let config_path = Some(path.display().to_string());
        let _guard = match self.lock.lock() {
            Ok(guard) => guard,
            Err(_) => {
                return McpIntegrationState {
                    available: false,
                    configured: false,
                    config_path,
                    error: Some("Claude Code config lock poisoned".to_string()),
                };
            }
        };
        match read_claude_document(path).and_then(|(document, _)| {
            claude_server_url(&document, path)
                .map(|configured_url| configured_url == Some(expected_url))
        }) {
            Ok(configured) => McpIntegrationState {
                available: true,
                configured,
                config_path,
                error: None,
            },
            Err(error) => McpIntegrationState {
                available: false,
                configured: false,
                config_path,
                error: Some(error),
            },
        }
    }

    fn add(&self, url: &str) -> Result<(), String> {
        let path = self.path()?;
        let _guard = self
            .lock
            .lock()
            .map_err(|_| "Claude Code config lock poisoned".to_string())?;
        let (mut document, original) = read_claude_document(path)?;
        let root = document.as_object_mut().ok_or_else(|| {
            format!(
                "invalid Claude Code config {}: root must be a JSON object",
                path.display()
            )
        })?;
        if !root.contains_key("mcpServers") {
            root.insert("mcpServers".to_string(), Value::Object(Map::new()));
        }
        let servers = root
            .get_mut("mcpServers")
            .and_then(Value::as_object_mut)
            .ok_or_else(|| {
                format!(
                    "invalid Claude Code config {}: mcpServers must be a JSON object",
                    path.display()
                )
            })?;
        servers.insert(
            "wot_repl".to_string(),
            serde_json::json!({
                "type": "http",
                "url": url,
            }),
        );
        write_claude_document(path, &document, original.as_deref())
    }

    fn remove(&self) -> Result<(), String> {
        let path = self.path()?;
        let _guard = self
            .lock
            .lock()
            .map_err(|_| "Claude Code config lock poisoned".to_string())?;
        if !path.exists() {
            return Ok(());
        }
        let (mut document, original) = read_claude_document(path)?;
        let root = document.as_object_mut().ok_or_else(|| {
            format!(
                "invalid Claude Code config {}: root must be a JSON object",
                path.display()
            )
        })?;
        let (removed, remove_parent) = {
            let Some(servers) = root.get_mut("mcpServers") else {
                return Ok(());
            };
            let servers = servers.as_object_mut().ok_or_else(|| {
                format!(
                    "invalid Claude Code config {}: mcpServers must be a JSON object",
                    path.display()
                )
            })?;
            let removed = servers.remove("wot_repl").is_some();
            (removed, servers.is_empty())
        };
        if !removed {
            return Ok(());
        }
        if remove_parent {
            root.remove("mcpServers");
        }
        write_claude_document(path, &document, original.as_deref())
    }
}

fn codex_config_path() -> Result<PathBuf, String> {
    if let Some(codex_home) = std::env::var_os("CODEX_HOME") {
        if codex_home.is_empty() {
            return Err("CODEX_HOME is empty".to_string());
        }
        let root = PathBuf::from(codex_home);
        if !root.is_dir() {
            return Err(format!(
                "CODEX_HOME does not point to an existing directory: {}",
                root.display()
            ));
        }
        return Ok(root.join("config.toml"));
    }

    #[cfg(windows)]
    let home = std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .ok_or_else(|| "cannot locate ChatGPT/Codex config: USERPROFILE is not set".to_string())?;
    #[cfg(not(windows))]
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| "cannot locate ChatGPT/Codex config: HOME is not set".to_string())?;

    Ok(home.join(".codex").join("config.toml"))
}

fn claude_config_path() -> Result<PathBuf, String> {
    if let Some(config_dir) = std::env::var_os("CLAUDE_CONFIG_DIR") {
        if config_dir.is_empty() {
            return Err("CLAUDE_CONFIG_DIR is empty".to_string());
        }
        return Ok(PathBuf::from(config_dir).join(".claude.json"));
    }

    #[cfg(windows)]
    let home = std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .ok_or_else(|| "cannot locate Claude Code config: USERPROFILE is not set".to_string())?;
    #[cfg(not(windows))]
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| "cannot locate Claude Code config: HOME is not set".to_string())?;

    Ok(home.join(".claude.json"))
}

fn read_codex_document(path: &Path) -> Result<DocumentMut, String> {
    if !path.exists() {
        return Ok(DocumentMut::new());
    }
    let body = fs::read_to_string(path).map_err(|error| {
        format!(
            "cannot read ChatGPT/Codex config {}: {error}",
            path.display()
        )
    })?;
    body.parse::<DocumentMut>()
        .map_err(|error| format!("invalid ChatGPT/Codex config {}: {error}", path.display()))
}

fn codex_server_url<'a>(document: &'a DocumentMut, path: &Path) -> Result<Option<&'a str>, String> {
    let Some(servers) = document.get("mcp_servers") else {
        return Ok(None);
    };
    let servers = servers.as_table_like().ok_or_else(|| {
        format!(
            "invalid ChatGPT/Codex config {}: mcp_servers must be a table",
            path.display()
        )
    })?;
    Ok(servers
        .get("wot_repl")
        .and_then(Item::as_table_like)
        .and_then(|server| server.get("url"))
        .and_then(Item::as_str))
}

fn write_codex_document(path: &Path, document: &DocumentMut) -> Result<(), String> {
    write_atomic_config(path, document.to_string().as_bytes(), "ChatGPT/Codex")
}

fn read_claude_document(path: &Path) -> Result<(Value, Option<Vec<u8>>), String> {
    if !path.exists() {
        return Ok((Value::Object(Map::new()), None));
    }
    let body = fs::read(path)
        .map_err(|error| format!("cannot read Claude Code config {}: {error}", path.display()))?;
    let document = serde_json::from_slice(&body)
        .map_err(|error| format!("invalid Claude Code config {}: {error}", path.display()))?;
    Ok((document, Some(body)))
}

fn claude_server_url<'a>(document: &'a Value, path: &Path) -> Result<Option<&'a str>, String> {
    let root = document.as_object().ok_or_else(|| {
        format!(
            "invalid Claude Code config {}: root must be a JSON object",
            path.display()
        )
    })?;
    let Some(servers) = root.get("mcpServers") else {
        return Ok(None);
    };
    let servers = servers.as_object().ok_or_else(|| {
        format!(
            "invalid Claude Code config {}: mcpServers must be a JSON object",
            path.display()
        )
    })?;
    let Some(server) = servers.get("wot_repl").and_then(Value::as_object) else {
        return Ok(None);
    };
    let transport_supported = matches!(
        server.get("type").and_then(Value::as_str),
        Some("http" | "streamable-http")
    );
    Ok(transport_supported
        .then(|| server.get("url").and_then(Value::as_str))
        .flatten())
}

fn write_claude_document(
    path: &Path,
    document: &Value,
    original: Option<&[u8]>,
) -> Result<(), String> {
    let body = serde_json::to_vec_pretty(document)
        .map_err(|error| format!("cannot serialize Claude Code config: {error}"))?;
    ensure_claude_config_unchanged(path, original)?;
    write_atomic_config(path, &body, "Claude Code")
}

fn ensure_claude_config_unchanged(path: &Path, original: Option<&[u8]>) -> Result<(), String> {
    let unchanged = match original {
        Some(original) => fs::read(path).is_ok_and(|current| current == original),
        None => matches!(
            fs::symlink_metadata(path),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound
        ),
    };
    if unchanged {
        Ok(())
    } else {
        Err(format!(
            "Claude Code config {} changed while the MCP update was being prepared; retry the operation",
            path.display()
        ))
    }
}

fn write_atomic_config(path: &Path, body: &[u8], name: &str) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("{name} config path has no parent"))?;
    fs::create_dir_all(parent).map_err(|error| {
        format!(
            "cannot create {name} config directory {}: {error}",
            parent.display()
        )
    })?;
    let write_path = match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            path.canonicalize().map_err(|error| {
                format!(
                    "cannot resolve {name} config symlink {}: {error}",
                    path.display()
                )
            })?
        }
        Ok(_) => path.to_path_buf(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => path.to_path_buf(),
        Err(error) => {
            return Err(format!(
                "cannot inspect {name} config {}: {error}",
                path.display()
            ));
        }
    };
    let mut file = AtomicWriteFile::open(&write_path).map_err(|error| {
        format!(
            "cannot prepare atomic {name} config update {}: {error}",
            path.display()
        )
    })?;
    file.write_all(body)
        .map_err(|error| format!("cannot write {name} config {}: {error}", path.display()))?;
    file.commit()
        .map_err(|error| format!("cannot commit {name} config {}: {error}", path.display()))
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

fn connection_info(
    config: McpConfig,
    mode: McpMode,
    status: McpStatus,
    error: Option<String>,
) -> ConnectionInfo {
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
        mode,
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
    activity: McpActivityLog,
    tool_router: ToolRouter<Self>,
}

#[derive(Clone)]
struct RemoteReplMcpServer {
    client: ClientManager,
    activity: McpActivityLog,
    tool_router: ToolRouter<Self>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct McpClientStatus {
    path: String,
    version: String,
    exe: String,
    process_status: ProcessStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    agent_status: Option<AgentStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pid: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    agent_version: Option<String>,
    capabilities: ClientCapabilities,
}

impl TryFrom<ClientStatus> for McpClientStatus {
    type Error = String;

    fn try_from(status: ClientStatus) -> Result<Self, Self::Error> {
        let ClientStatus {
            game,
            kind,
            process_status,
            agent_status,
            pid,
            agent_version,
            agent_pid: _,
            capabilities,
        } = status;
        if kind != ClientKind::Local {
            return Err("MCP_REQUIRES_LOCAL_CLIENT: remote clients are not exposed to MCP".into());
        }
        let game = game.ok_or_else(|| "local client has no installation path".to_string())?;
        let agent_status = (process_status != ProcessStatus::Stopped).then_some(agent_status);
        Ok(Self {
            path: game.path,
            version: game.version,
            exe: game.exe,
            process_status,
            agent_status,
            pid,
            agent_version,
            capabilities,
        })
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct McpCloseOutput {
    still_running: bool,
    client: McpClientStatus,
}

impl TryFrom<CloseResult> for McpCloseOutput {
    type Error = String;

    fn try_from(result: CloseResult) -> Result<Self, Self::Error> {
        Ok(Self {
            still_running: result.still_running,
            client: result.client.try_into()?,
        })
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct StartClientInput {
    /// Absolute path to a supported World of Tanks installation.
    game_dir: String,
    /// Optional absolute path to a replay opened when the client starts.
    replay_path: Option<String>,
    /// Wait until the agent has executed a probe on the game main thread (default true).
    wait_until_ready: Option<bool>,
    /// Optional safety timeout in milliseconds (clamped to 30000..1800000). By default readiness waits as long as the game process is running.
    ready_timeout_ms: Option<i64>,
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

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ScreenshotInput {
    /// Image format: "jpg" (default) or "png".
    format: Option<String>,
    /// Total capture and local file-read timeout in milliseconds (default 15000, clamped to 1000..30000).
    timeout_ms: Option<i64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct MouseInput {
    /// Mouse operation: move, down, up, click, or wheel.
    action: String,
    /// Virtual cursor X coordinate in game-window pixels. Supply together with y.
    x: Option<f64>,
    /// Virtual cursor Y coordinate in game-window pixels. Supply together with x.
    y: Option<f64>,
    /// Button for down/up/click: left, right, or middle.
    button: Option<String>,
    /// Non-zero BigWorld wheel delta for a wheel operation.
    wheel_delta: Option<i32>,
    /// Held modifiers: Shift, Control/Ctrl, and/or Alt.
    #[serde(default)]
    modifiers: Vec<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct KeyboardInput {
    /// Keyboard operation: down, up, or press.
    action: String,
    /// Key name such as A, Enter, Escape, ArrowLeft, F1, or an exact KEY_* name.
    key: String,
    /// Optional single character delivered with the key event.
    character: Option<String>,
    /// Held modifiers: Shift, Control/Ctrl, and/or Alt.
    #[serde(default)]
    modifiers: Vec<String>,
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

#[derive(Clone, Copy)]
enum ExecTarget {
    Local,
    Remote,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ScreenshotOutput {
    mime_type: String,
    size: u64,
    sha256: String,
    width: Option<u32>,
    height: Option<u32>,
    duration_ms: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct InputOutput {
    delivered: bool,
    x: Option<f64>,
    y: Option<f64>,
    width: Option<f64>,
    height: Option<f64>,
    key: Option<String>,
}

impl WotMcpServer {
    fn new(client: ClientManager, activity: McpActivityLog) -> Self {
        Self {
            client,
            activity,
            tool_router: Self::tool_router(),
        }
    }

    async fn wait_for_main_thread(
        &self,
        timeout: Option<Duration>,
        context: &RequestContext<RoleServer>,
    ) -> Result<(), String> {
        const PROCESS_HANDOFF_GRACE: Duration = Duration::from_secs(30);

        let started = Instant::now();
        let deadline = timeout.map(|timeout| tokio::time::Instant::now() + timeout);
        let mut stopped_since = None;
        let mut last_progress = Instant::now()
            .checked_sub(Duration::from_secs(2))
            .unwrap_or_else(Instant::now);
        loop {
            if context.ct.is_cancelled() {
                return Err("client start request was cancelled".to_string());
            }
            if deadline.is_some_and(|deadline| tokio::time::Instant::now() >= deadline) {
                return Err(
                    "client started, but its game agent did not connect before the requested readiness timeout"
                        .to_string(),
                );
            }
            let status = self.client.active_status()?;
            if status.process_status == ProcessStatus::Stopped {
                let since = stopped_since.get_or_insert_with(Instant::now);
                if since.elapsed() >= PROCESS_HANDOFF_GRACE {
                    return Err("game process exited before its agent connected".to_string());
                }
            } else {
                stopped_since = None;
            }
            match self
                .client
                .require_local_agent_capability("main_thread_probe")
            {
                Ok(_) => break,
                Err(error) if error.starts_with("AGENT_CAPABILITY_UNAVAILABLE:") => {
                    return Err(error);
                }
                Err(_) => {
                    if last_progress.elapsed() >= Duration::from_secs(1) {
                        notify_progress(
                            context,
                            (5.0 + started.elapsed().as_secs_f64() / 10.0).min(35.0),
                            "Waiting for the running game to load its agent",
                        )
                        .await;
                        last_progress = Instant::now();
                    }
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            }
        }

        let remaining = deadline
            .map(|deadline| deadline.saturating_duration_since(tokio::time::Instant::now()));
        if remaining.is_some_and(|remaining| remaining.is_zero()) {
            return Err(
                "client started, but its main thread did not become responsive before the requested readiness timeout"
                    .to_string(),
            );
        }
        let readiness = self.client.request_while_active_process_running(
            InFrame::Ready {
                id: uuid::Uuid::new_v4().to_string(),
            },
            remaining,
        );
        tokio::pin!(readiness);
        let mut progress = tokio::time::interval(Duration::from_secs(1));
        let frame = loop {
            tokio::select! {
                response = &mut readiness => break response?,
                _ = context.ct.cancelled() => {
                    return Err("client start request was cancelled".to_string());
                }
                _ = progress.tick() => {
                    notify_progress(
                        context,
                        40.0,
                        "Game process is running; waiting for its main thread",
                    ).await;
                }
            }
        };
        match frame {
            OutFrame::Ready { ok: true, .. } => {
                notify_progress(context, 100.0, "Game main thread is ready").await;
                Ok(())
            }
            OutFrame::Ready { error, .. } => {
                Err(error.unwrap_or_else(|| "game main-thread readiness probe failed".to_string()))
            }
            _ => Err("agent returned an unexpected readiness response".to_string()),
        }
    }
}

impl RemoteReplMcpServer {
    fn new(client: ClientManager, activity: McpActivityLog) -> Self {
        Self {
            client,
            activity,
            tool_router: Self::tool_router(),
        }
    }
}

async fn execute_python(
    client: &ClientManager,
    ExecInput { code, timeout_ms }: ExecInput,
    target: ExecTarget,
) -> CallToolResult {
    if code.trim().is_empty() {
        return tool_error("code must not be blank");
    }
    if code.len() > 262_144 {
        return tool_error("code exceeds the 262144-byte UTF-8 limit");
    }
    let capability = match target {
        ExecTarget::Local => client.require_local_agent_capability("repl").map(|_| ()),
        ExecTarget::Remote => client.require_remote_agent_capability("repl"),
    };
    if let Err(error) = capability {
        return tool_error(error);
    }

    let started = Instant::now();
    let timeout = Duration::from_millis(timeout_ms.unwrap_or(30_000).clamp(1, 30_000) as u64);
    let frame = client
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

fn remaining_timeout(deadline: tokio::time::Instant) -> Result<Duration, String> {
    let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
    if remaining.is_zero() {
        Err("screenshot did not complete before its timeout".to_string())
    } else {
        Ok(remaining)
    }
}

async fn notify_progress(context: &RequestContext<RoleServer>, progress: f64, message: &str) {
    let Some(token) = context.meta.get_progress_token() else {
        return;
    };
    let _ = context
        .peer
        .notify_progress(
            ProgressNotificationParam::new(token, progress)
                .with_total(100.0)
                .with_message(message),
        )
        .await;
}

#[rmcp::tool_router(router = tool_router)]
impl WotMcpServer {
    #[rmcp::tool(
        name = "wot_list_clients",
        description = "List local World of Tanks installations available on the desktop app's computer. Every listed client can be passed to wot_start_client; the agent mod is installed or updated automatically during startup.",
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
                if clients
                    .iter()
                    .any(|client| client.kind == ClientKind::Remote)
                {
                    return tool_error(
                        "MCP_REQUIRES_LOCAL_CLIENT: disconnect the remote game; the desktop app and game must run on the same computer",
                    );
                }
                let clients: Result<Vec<McpClientStatus>, String> = clients
                    .into_iter()
                    .filter(|client| client.kind == ClientKind::Local)
                    .map(McpClientStatus::try_from)
                    .collect();
                let clients = match clients {
                    Ok(clients) => clients,
                    Err(error) => return tool_error(error),
                };
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
        if let Err(error) = self.client.require_local_client() {
            return tool_error(error);
        }
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
        description = "Install and connect the agent if needed, start a World of Tanks client on the desktop app's computer, and by default wait until a probe executes on the game main thread. Readiness has no fixed timeout while the game process is running; ready_timeout_ms can set an optional safety limit. Set wait_until_ready=false for fire-and-forget startup.",
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
            wait_until_ready,
            ready_timeout_ms,
        }): Parameters<StartClientInput>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let started = match self
            .client
            .start(&game_dir, replay_path.as_deref(), Arc::new(|_| {}))
        {
            Ok(client) => client,
            Err(error) => return tool_error(error),
        };
        if !wait_until_ready.unwrap_or(true) {
            return match McpClientStatus::try_from(started) {
                Ok(client) => tool_success("Client started or already active.", client),
                Err(error) => tool_error(error),
            };
        }
        let timeout = ready_timeout_ms
            .map(|timeout| Duration::from_millis(timeout.clamp(30_000, 1_800_000) as u64));
        match self.wait_for_main_thread(timeout, &context).await {
            Ok(()) => match self.client.active_status() {
                Ok(client) => match McpClientStatus::try_from(client) {
                    Ok(client) => {
                        tool_success("Client started and its main thread is ready.", client)
                    }
                    Err(error) => tool_error(error),
                },
                Err(error) => tool_error(error),
            },
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
                match McpCloseOutput::try_from(result) {
                    Ok(result) => tool_success(text, result),
                    Err(error) => tool_error(error),
                }
            }
            Ok(Err(error)) => tool_error(error),
            Err(error) => tool_error(format!("close task failed: {error}")),
        }
    }

    #[rmcp::tool(
        name = "wot_exec",
        description = "Execute arbitrary Python code in the connected local World of Tanks client on its main thread. The code can mutate game state and interact with external systems.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn exec(&self, Parameters(input): Parameters<ExecInput>) -> CallToolResult {
        execute_python(&self.client, input, ExecTarget::Local).await
    }

    #[rmcp::tool(
        name = "wot_screenshot",
        description = "Capture the connected local World of Tanks window and return it as standard MCP image content. BigWorld creates a temporary image which the desktop reads and removes through the shared local filesystem; no screenshot bytes pass through the in-game agent transport. Capture does not focus the OS window or take over the user's desktop.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn screenshot(
        &self,
        Parameters(ScreenshotInput { format, timeout_ms }): Parameters<ScreenshotInput>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let game = match self.client.require_local_agent_capability("screenshot") {
            Ok(game) => game,
            Err(error) => return tool_error(error),
        };
        let format = format.unwrap_or_else(|| "jpg".to_string()).to_lowercase();
        let format = if format == "jpeg" {
            "jpg".to_string()
        } else {
            format
        };
        if !matches!(format.as_str(), "jpg" | "png") {
            return tool_error("format must be jpg or png");
        }
        let timeout =
            Duration::from_millis(timeout_ms.unwrap_or(15_000).clamp(1_000, 30_000) as u64);
        let deadline = tokio::time::Instant::now() + timeout;
        let started = Instant::now();
        let capture_id = uuid::Uuid::new_v4().simple().to_string();
        let pending = match PendingScreenshot::new(Path::new(&game.path), &capture_id, &format) {
            Ok(pending) => pending,
            Err(error) => return tool_error(error),
        };
        notify_progress(&context, 0.0, "Requesting screenshot from the game").await;
        let capture_timeout = match remaining_timeout(deadline) {
            Ok(timeout) => timeout,
            Err(error) => return tool_error(error),
        };
        let frame = self
            .client
            .request_with_timeout(
                InFrame::Screenshot {
                    id: uuid::Uuid::new_v4().to_string(),
                    format,
                    capture_id,
                },
                capture_timeout,
            )
            .await;
        let (width, height) = match frame {
            Ok(OutFrame::ScreenshotStarted {
                ok: true,
                width,
                height,
                ..
            }) => (width, height),
            Ok(OutFrame::ScreenshotStarted { error, .. }) => {
                return tool_error(
                    error.unwrap_or_else(|| "screenshot capture failed".to_string()),
                );
            }
            Ok(_) => return tool_error("agent returned an unexpected screenshot response"),
            Err(error) => return tool_error(error),
        };
        notify_progress(&context, 20.0, "Waiting for the local screenshot file").await;
        let read = pending.read(deadline);
        tokio::pin!(read);
        let mut progress = tokio::time::interval(Duration::from_millis(500));
        let collected = loop {
            tokio::select! {
                result = &mut read => break result,
                _ = context.ct.cancelled() => {
                    break Err("screenshot request was cancelled".to_string());
                }
                _ = progress.tick() => {
                    notify_progress(
                        &context,
                        (20.0 + started.elapsed().as_secs_f64() * 2.0).min(90.0),
                        "Waiting for the game to finish writing the screenshot",
                    ).await;
                }
            }
        };
        match collected {
            Ok(capture) => {
                let size = capture.bytes.len() as u64;
                let sha256 = hex::encode(Sha256::digest(&capture.bytes));
                let image = BASE64.encode(capture.bytes);
                let mime_type = capture.mime_type.to_string();
                let metadata = ScreenshotOutput {
                    mime_type: mime_type.clone(),
                    size,
                    sha256,
                    width,
                    height,
                    duration_ms: started.elapsed().as_millis() as u64,
                };
                notify_progress(&context, 100.0, "Screenshot ready").await;
                match serde_json::to_value(metadata) {
                    Ok(value) => {
                        let mut result = CallToolResult::structured(value);
                        result.content = vec![
                            ContentBlock::text("World of Tanks screenshot captured."),
                            ContentBlock::image(image, mime_type),
                        ];
                        result
                    }
                    Err(error) => {
                        tool_error(format!("cannot serialize screenshot result: {error}"))
                    }
                }
            }
            Err(error) => tool_error(error),
        }
    }

    #[rmcp::tool(
        name = "wot_mouse",
        description = "Send a virtual mouse move, button, click, or wheel event directly through the connected game's input pipeline. Coordinates are game-window pixels. This never moves the host OS cursor or steals desktop focus.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn mouse(
        &self,
        Parameters(MouseInput {
            action,
            x,
            y,
            button,
            wheel_delta,
            modifiers,
        }): Parameters<MouseInput>,
    ) -> CallToolResult {
        if let Err(error) = self.client.require_local_agent_capability("virtual_input") {
            return tool_error(error);
        }
        let frame = self
            .client
            .request_with_timeout(
                InFrame::Mouse {
                    id: uuid::Uuid::new_v4().to_string(),
                    action,
                    x,
                    y,
                    button,
                    wheel_delta,
                    modifiers,
                },
                Duration::from_secs(5),
            )
            .await;
        input_tool_result(frame, "mouse")
    }

    #[rmcp::tool(
        name = "wot_keyboard",
        description = "Send a virtual key down, key up, or complete key press directly through the connected game's input pipeline. This never focuses the OS window or captures the user's physical keyboard.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn keyboard(
        &self,
        Parameters(KeyboardInput {
            action,
            key,
            character,
            modifiers,
        }): Parameters<KeyboardInput>,
    ) -> CallToolResult {
        if let Err(error) = self.client.require_local_agent_capability("virtual_input") {
            return tool_error(error);
        }
        let frame = self
            .client
            .request_with_timeout(
                InFrame::Keyboard {
                    id: uuid::Uuid::new_v4().to_string(),
                    action,
                    key,
                    character,
                    modifiers,
                },
                Duration::from_secs(5),
            )
            .await;
        input_tool_result(frame, "keyboard")
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
            Ok(client) => match McpClientStatus::try_from(client) {
                Ok(client) => tool_success("Client kill requested.", client),
                Err(error) => tool_error(error),
            },
            Err(error) => tool_error(error),
        }
    }
}

#[rmcp::tool_router(router = tool_router)]
impl RemoteReplMcpServer {
    #[rmcp::tool(
        name = "wot_exec",
        description = "Execute arbitrary Python 2.7 code on the main thread of the remote World of Tanks client currently connected to this desktop app. No local process, filesystem, screenshot, mouse, keyboard, or log access is available in this MCP mode.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn exec(&self, Parameters(input): Parameters<ExecInput>) -> CallToolResult {
        execute_python(&self.client, input, ExecTarget::Remote).await
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

fn input_tool_result(frame: Result<OutFrame, String>, input_kind: &str) -> CallToolResult {
    match frame {
        Ok(OutFrame::Input {
            ok: true,
            x,
            y,
            width,
            height,
            key,
            ..
        }) => tool_success(
            format!("Virtual {input_kind} event delivered to World of Tanks."),
            InputOutput {
                delivered: true,
                x,
                y,
                width,
                height,
                key,
            },
        ),
        Ok(OutFrame::Input { error, .. }) => {
            tool_error(error.unwrap_or_else(|| format!("virtual {input_kind} event failed")))
        }
        Ok(_) => tool_error(format!(
            "agent returned an unexpected response to virtual {input_kind} input"
        )),
        Err(error) => tool_error(error),
    }
}

#[rmcp::tool_handler(router = self.tool_router)]
impl ServerHandler for WotMcpServer {
    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, rmcp::ErrorData> {
        let command = request.name.to_string();
        let request_value = serde_json::json!({
            "method": "tools/call",
            "params": serde_json::to_value(&request).unwrap_or(Value::Null),
        });
        let activity = self.activity.start(command, request_value);
        let tool_context =
            rmcp::handler::server::tool::ToolCallContext::new(self, request, context);
        let result = self.tool_router.call(tool_context).await;
        activity.complete(&result);
        result
    }

    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build()).with_server_info(
            Implementation::new("wotstat-repl", crate::APP_VERSION)
                .with_title("WotStat World of Tanks REPL")
                .with_description(
                    "A development MCP server for controlling World of Tanks clients on the desktop app's computer and executing Python 2.7 code inside the game.",
                ),
        )
        .with_instructions(
            "WoT means World of Tanks, not Web of Things. The desktop app and game must run on the same computer; the MCP client may connect to the desktop app over the network. Start with wot_list_clients and inspect capabilities. Every listed client is launchable with wot_start_client; the agent mod is installed or updated automatically during startup and is intentionally not reported by wot_list_clients. Only one agent can be active. wot_screenshot reads the game-created image through the desktop's local filesystem and returns standard MCP image content. wot_mouse and wot_keyboard inject only into the game and never take over host OS input. Use wot_close_client for graceful shutdown and wot_kill_client only if it is unresponsive. wot_exec runs arbitrary Python 2.7 inside the game process.",
        )
    }
}

#[rmcp::tool_handler(router = self.tool_router)]
impl ServerHandler for RemoteReplMcpServer {
    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, rmcp::ErrorData> {
        let command = request.name.to_string();
        let request_value = serde_json::json!({
            "method": "tools/call",
            "params": serde_json::to_value(&request).unwrap_or(Value::Null),
        });
        let activity = self.activity.start(command, request_value);
        let tool_context =
            rmcp::handler::server::tool::ToolCallContext::new(self, request, context);
        let result = self.tool_router.call(tool_context).await;
        activity.complete(&result);
        result
    }

    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build()).with_server_info(
            Implementation::new("wotstat-repl", crate::APP_VERSION)
                .with_title("WotStat Remote World of Tanks REPL")
                .with_description(
                    "A reduced MCP server for executing Python 2.7 inside a remotely connected World of Tanks client.",
                ),
        )
        .with_instructions(
            "WoT means World of Tanks, not Web of Things. This is the remote REPL-only MCP mode. It exposes exactly one tool: wot_exec. The tool runs arbitrary Python 2.7 on the game main thread through the connected remote agent. Client discovery, start, close, kill, logs, screenshots, mouse, keyboard, and host filesystem or process access are intentionally unavailable. For the full MCP toolset, run the Windows desktop application on the same computer as World of Tanks.",
        )
    }
}

fn mcp_router(
    token: String,
    client: ClientManager,
    activity: McpActivityLog,
    mode: McpMode,
    config: StreamableHttpServerConfig,
) -> Router {
    let router = match mode {
        McpMode::Full => {
            let service: StreamableHttpService<WotMcpServer, LocalSessionManager> =
                StreamableHttpService::new(
                    move || Ok(WotMcpServer::new(client.clone(), activity.clone())),
                    Default::default(),
                    config,
                );
            Router::new().route_service("/mcp", service)
        }
        McpMode::RemoteRepl => {
            let service: StreamableHttpService<RemoteReplMcpServer, LocalSessionManager> =
                StreamableHttpService::new(
                    move || Ok(RemoteReplMcpServer::new(client.clone(), activity.clone())),
                    Default::default(),
                    config,
                );
            Router::new().route_service("/mcp", service)
        }
    };
    router.route_layer(middleware::from_fn_with_state(token, require_token))
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

    fn temp_codex_path() -> PathBuf {
        std::env::temp_dir()
            .join(format!("wms_codex_config_{}", uuid::Uuid::new_v4()))
            .join("config.toml")
    }

    fn temp_claude_path() -> PathBuf {
        std::env::temp_dir()
            .join(format!("wms_claude_config_{}", uuid::Uuid::new_v4()))
            .join(".claude.json")
    }

    #[test]
    fn creates_reloads_and_toggles_without_changing_token() {
        let path = temp_path();
        let first = McpConfigStore::new(path.clone());
        let created = first.load_or_create().unwrap();
        assert!(created.enabled);
        let info = connection_info(created.clone(), McpMode::Full, McpStatus::Disabled, None);
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
                "mode": "full",
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
    fn codex_config_add_update_and_remove_preserve_unrelated_content() {
        let path = temp_codex_path();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let original = r#"# keep this comment
model = "gpt-5"

[mcp_servers.other]
url = "http://other.example/mcp"
"#;
        fs::write(&path, original).unwrap();
        let store = CodexConfigStore::new(path.clone());
        let first_url = "http://127.0.0.1:8765/mcp?token=first";
        let second_url = "http://127.0.0.1:8765/mcp?token=second";

        let initial = store.status(first_url);
        assert!(initial.available);
        assert!(!initial.configured);
        assert_eq!(initial.config_path, Some(path.display().to_string()));

        store.add(first_url).unwrap();
        assert!(store.status(first_url).configured);
        let added = fs::read_to_string(&path).unwrap();
        assert!(added.contains("# keep this comment"));
        assert!(added.contains("model = \"gpt-5\""));
        assert!(added.contains("[mcp_servers.other]"));
        assert!(added.contains("[mcp_servers.wot_repl]"));
        assert!(added.contains(first_url));

        store.add(second_url).unwrap();
        assert!(!store.status(first_url).configured);
        assert!(store.status(second_url).configured);
        let updated = fs::read_to_string(&path).unwrap();
        assert!(!updated.contains(first_url));
        assert!(updated.contains(second_url));

        store.remove().unwrap();
        let removed = fs::read_to_string(&path).unwrap();
        assert!(removed.contains("# keep this comment"));
        assert!(removed.contains("model = \"gpt-5\""));
        assert!(removed.contains("[mcp_servers.other]"));
        assert!(!removed.contains("wot_repl"));
        assert!(!store.status(second_url).configured);
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn codex_config_is_created_and_empty_parent_is_removed() {
        let path = temp_codex_path();
        let store = CodexConfigStore::new(path.clone());
        let url = "http://127.0.0.1:8765/mcp?token=test";

        store.add(url).unwrap();
        assert!(store.status(url).configured);
        store.remove().unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "");
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn invalid_codex_config_is_reported_without_overwrite() {
        let path = temp_codex_path();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let body = "[mcp_servers.wot_repl\nurl = \"broken\"\n";
        fs::write(&path, body).unwrap();
        let store = CodexConfigStore::new(path.clone());

        let status = store.status("http://127.0.0.1:8765/mcp?token=test");
        assert!(!status.available);
        assert!(status
            .error
            .unwrap()
            .contains("invalid ChatGPT/Codex config"));
        assert!(store
            .add("http://127.0.0.1:8765/mcp?token=test")
            .unwrap_err()
            .contains("invalid ChatGPT/Codex config"));
        assert_eq!(fs::read_to_string(&path).unwrap(), body);
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn codex_config_update_preserves_a_dotfiles_symlink() {
        let path = temp_codex_path();
        let root = path.parent().unwrap();
        let target_dir = root.join("dotfiles");
        let target = target_dir.join("config.toml");
        fs::create_dir_all(&target_dir).unwrap();
        fs::write(&target, "model = \"gpt-5\"\n").unwrap();
        std::os::unix::fs::symlink(&target, &path).unwrap();
        let store = CodexConfigStore::new(path.clone());
        let url = "http://127.0.0.1:8765/mcp?token=symlink";

        store.add(url).unwrap();

        assert!(fs::symlink_metadata(&path)
            .unwrap()
            .file_type()
            .is_symlink());
        let body = fs::read_to_string(&target).unwrap();
        assert!(body.contains("model = \"gpt-5\""));
        assert!(body.contains(url));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn claude_config_add_update_and_remove_preserve_unrelated_content() {
        let path = temp_claude_path();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let original = serde_json::json!({
            "firstStartTime": "2026-08-30T00:00:00.000Z",
            "projects": {
                "C:\\work\\mod": {
                    "allowedTools": ["Read", "Edit"]
                }
            },
            "mcpServers": {
                "other": {
                    "type": "http",
                    "url": "http://other.example/mcp"
                }
            }
        });
        fs::write(&path, serde_json::to_vec_pretty(&original).unwrap()).unwrap();
        let store = ClaudeConfigStore::new(path.clone());
        let first_url = "http://127.0.0.1:8765/mcp?token=first";
        let second_url = "http://127.0.0.1:8765/mcp?token=second";

        let initial = store.status(first_url);
        assert!(initial.available);
        assert!(!initial.configured);
        assert_eq!(initial.config_path, Some(path.display().to_string()));

        store.add(first_url).unwrap();
        assert!(store.status(first_url).configured);
        let added: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(added["firstStartTime"], original["firstStartTime"]);
        assert_eq!(added["projects"], original["projects"]);
        assert_eq!(
            added["mcpServers"]["other"],
            original["mcpServers"]["other"]
        );
        assert_eq!(added["mcpServers"]["wot_repl"]["type"], "http");
        assert_eq!(added["mcpServers"]["wot_repl"]["url"], first_url);

        store.add(second_url).unwrap();
        assert!(!store.status(first_url).configured);
        assert!(store.status(second_url).configured);

        store.remove().unwrap();
        let removed: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(removed["firstStartTime"], original["firstStartTime"]);
        assert_eq!(removed["projects"], original["projects"]);
        assert_eq!(
            removed["mcpServers"]["other"],
            original["mcpServers"]["other"]
        );
        assert!(removed["mcpServers"].get("wot_repl").is_none());
        assert!(!store.status(second_url).configured);
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn claude_config_is_created_and_empty_mcp_servers_is_removed() {
        let path = temp_claude_path();
        let store = ClaudeConfigStore::new(path.clone());
        let url = "http://127.0.0.1:8765/mcp?token=test";

        store.add(url).unwrap();
        assert!(store.status(url).configured);
        store.remove().unwrap();
        let removed: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(removed, serde_json::json!({}));
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn invalid_claude_config_is_reported_without_overwrite() {
        let path = temp_claude_path();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let body = b"{\"mcpServers\": {\"wot_repl\": }";
        fs::write(&path, body).unwrap();
        let store = ClaudeConfigStore::new(path.clone());

        let status = store.status("http://127.0.0.1:8765/mcp?token=test");
        assert!(!status.available);
        assert!(status.error.unwrap().contains("invalid Claude Code config"));
        assert!(store
            .add("http://127.0.0.1:8765/mcp?token=test")
            .unwrap_err()
            .contains("invalid Claude Code config"));
        assert_eq!(fs::read(&path).unwrap(), body);
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn claude_config_refuses_to_overwrite_a_concurrent_change() {
        let path = temp_claude_path();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"{\"version\": 1}").unwrap();
        let (mut document, original) = read_claude_document(&path).unwrap();
        document["mcpServers"] = serde_json::json!({
            "wot_repl": {
                "type": "http",
                "url": "http://127.0.0.1:8765/mcp?token=test"
            }
        });
        fs::write(&path, b"{\"version\": 2}").unwrap();

        let error = write_claude_document(&path, &document, original.as_deref()).unwrap_err();

        assert!(error.contains("changed while the MCP update was being prepared"));
        assert_eq!(fs::read(&path).unwrap(), b"{\"version\": 2}");
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn claude_status_accepts_the_streamable_http_alias() {
        let path = temp_claude_path();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let url = "http://127.0.0.1:8765/mcp?token=test";
        fs::write(
            &path,
            serde_json::to_vec_pretty(&serde_json::json!({
                "mcpServers": {
                    "wot_repl": {
                        "type": "streamable-http",
                        "url": url
                    }
                }
            }))
            .unwrap(),
        )
        .unwrap();

        assert!(ClaudeConfigStore::new(path.clone()).status(url).configured);
        let _ = fs::remove_dir_all(path.parent().unwrap());
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

    #[test]
    fn activity_log_tracks_lifecycle_and_keeps_the_newest_500_commands() {
        let activity = McpActivityLog::default();
        let pending = activity.start("pending-command".into(), serde_json::json!({ "value": 1 }));
        let pending_snapshot = activity.snapshot(None);
        assert_eq!(pending_snapshot.entries.as_ref().unwrap().len(), 1);
        assert_eq!(
            pending_snapshot.entries.as_ref().unwrap()[0].status,
            McpActivityStatus::Pending
        );

        let failed: Result<CallToolResponse, rmcp::ErrorData> =
            Ok(CallToolResult::error(vec![ContentBlock::text("failed")]).into());
        pending.complete(&failed);
        let failed_snapshot = activity.snapshot(None);
        assert_eq!(
            failed_snapshot.entries.as_ref().unwrap()[0].status,
            McpActivityStatus::Error
        );
        assert!(failed_snapshot.entries.as_ref().unwrap()[0]
            .response
            .is_some());

        let structured_failure: Result<CallToolResponse, rmcp::ErrorData> =
            Ok(CallToolResult::structured(serde_json::json!({ "ok": false })).into());
        activity
            .start("failed-exec".into(), serde_json::json!({}))
            .complete(&structured_failure);
        assert_eq!(
            activity.snapshot(None).entries.as_ref().unwrap()[0].status,
            McpActivityStatus::Error,
            "a command that completed with structured ok=false should be shown as failed"
        );

        let succeeded: Result<CallToolResponse, rmcp::ErrorData> =
            Ok(CallToolResult::success(Vec::new()).into());
        for index in 0..MCP_ACTIVITY_LIMIT + 2 {
            activity
                .start(
                    format!("command-{index}"),
                    serde_json::json!({ "index": index }),
                )
                .complete(&succeeded);
        }

        let snapshot = activity.snapshot(None);
        let entries = snapshot.entries.as_ref().unwrap();
        assert_eq!(entries.len(), MCP_ACTIVITY_LIMIT);
        assert_eq!(entries.first().unwrap().command, "command-501");
        assert_eq!(entries.last().unwrap().command, "command-2");
        assert_eq!(entries.first().unwrap().status, McpActivityStatus::Success);
        assert!(
            activity.snapshot(Some(snapshot.revision)).entries.is_none(),
            "an unchanged activity snapshot should not resend every payload"
        );
    }

    #[test]
    fn successful_virtual_input_reports_delivery_not_handler_consumption() {
        let frame = serde_json::from_value(serde_json::json!({
            "type": "input",
            "id": "mouse-1",
            "ok": true,
            "handled": false,
            "x": 94.0,
            "y": 466.0,
            "width": 2763.0,
            "height": 1622.0,
            "key": null,
            "error": null,
        }))
        .unwrap();

        let result = input_tool_result(Ok(frame), "mouse");
        let value = serde_json::to_value(result).unwrap();
        let output = &value["structuredContent"];

        assert_eq!(output["delivered"], true);
        assert!(output.get("handled").is_none());
    }

    #[test]
    fn stopped_mcp_client_is_startable_without_agent_installation_details() {
        let value = serde_json::to_value(
            McpClientStatus::try_from(ClientStatus {
                game: Some(install::GameInfo {
                    path: "C:/Games/WoT".into(),
                    version: "1.2.3.4".into(),
                    mods_version: "1.2.3.4".into(),
                    exe: "WorldOfTanks.exe".into(),
                    installed: false,
                }),
                kind: ClientKind::Local,
                process_status: ProcessStatus::Stopped,
                agent_status: AgentStatus::Unavailable,
                pid: None,
                agent_version: None,
                agent_pid: None,
                capabilities: ClientCapabilities {
                    repl: false,
                    input: false,
                    screenshot: false,
                    start: true,
                    close: false,
                    kill: false,
                },
            })
            .unwrap(),
        )
        .unwrap();

        assert_eq!(
            value,
            serde_json::json!({
                "path": "C:/Games/WoT",
                "version": "1.2.3.4",
                "exe": "WorldOfTanks.exe",
                "processStatus": "stopped",
                "capabilities": {
                    "repl": false,
                    "input": false,
                    "screenshot": false,
                    "start": true,
                    "close": false,
                    "kill": false,
                },
            })
        );
    }

    #[tokio::test]
    async fn listener_checks_token_and_releases_ephemeral_port() {
        let path = temp_path();
        let runtime =
            McpRuntime::with_mode(path.clone(), "127.0.0.1:0".parse().unwrap(), McpMode::Full);
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
            "wotstat-repl"
        );
        assert_eq!(
            initialized_body["result"]["serverInfo"]["title"],
            "WotStat World of Tanks REPL"
        );
        let description = initialized_body["result"]["serverInfo"]["description"]
            .as_str()
            .unwrap();
        assert!(description.contains("World of Tanks"));
        assert!(description.contains("Python 2.7"));
        let instructions = initialized_body["result"]["instructions"].as_str().unwrap();
        assert!(instructions.contains("not Web of Things"));
        assert!(instructions.contains("wot_list_clients"));
        assert!(instructions.contains("agent mod is installed or updated automatically"));
        assert!(instructions.contains("Only one agent"));
        assert!(instructions.contains("desktop app and game must run on the same computer"));
        assert!(instructions.contains("MCP client may connect"));
        assert!(instructions.contains("local filesystem"));
        assert!(instructions.contains("wot_close_client for graceful shutdown"));
        assert!(instructions.contains("wot_kill_client only if it is unresponsive"));
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
                "wot_keyboard",
                "wot_kill_client",
                "wot_list_clients",
                "wot_mouse",
                "wot_read_log",
                "wot_screenshot",
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
            .contains("NO_ACTIVE_CLIENT"));

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
        assert_eq!(log_body["result"]["isError"], true);
        assert!(log_body["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("NO_ACTIVE_CLIENT"));

        let activity = runtime.activity(None);
        let entries = activity.entries.as_ref().unwrap();
        assert_eq!(entries.len(), 4);
        assert_eq!(entries[0].command, "wot_read_log");
        assert_eq!(entries[0].status, McpActivityStatus::Error);
        assert_eq!(entries[1].command, "wot_list_clients");
        assert_eq!(entries[1].status, McpActivityStatus::Success);
        assert_eq!(entries[3].command, "wot_close_client");
        assert_eq!(entries[3].request["params"]["arguments"]["timeout_ms"], -1);
        assert!(entries.iter().all(|entry| entry.duration_ms.is_some()));
        assert!(entries.iter().all(|entry| entry.response.is_some()));

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
        let release_deadline = Instant::now() + Duration::from_secs(1);
        let rebound = loop {
            match tokio::net::TcpListener::bind(address).await {
                Ok(listener) => break listener,
                Err(_) if Instant::now() < release_deadline => {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
                Err(error) => panic!("MCP listener was not released: {error}"),
            }
        };
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
    async fn remote_mcp_exposes_only_exec() {
        let path = temp_path();
        let runtime = McpRuntime::with_mode(
            path.clone(),
            "127.0.0.1:0".parse().unwrap(),
            McpMode::RemoteRepl,
        );

        let started = runtime.start(ClientManager::default()).await.unwrap();
        assert_eq!(started.status, McpStatus::Listening);
        assert_eq!(started.mode, McpMode::RemoteRepl);
        let address = runtime.server.lock().unwrap().bound_address.unwrap();
        let token = runtime.store.load_or_create_locked().unwrap().token;
        let endpoint = format!("/mcp?token={token}");
        let initialized = http_post(
            address,
            &endpoint,
            None,
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"raw-smoke","version":"1"}}}"#,
        )
        .await;
        assert_eq!(initialized.status, 200, "{}", initialized.body);
        let initialized_body = json_body(&initialized);
        assert_eq!(
            initialized_body["result"]["serverInfo"]["title"],
            "WotStat Remote World of Tanks REPL"
        );
        let instructions = initialized_body["result"]["instructions"].as_str().unwrap();
        assert!(instructions.contains("remote REPL-only MCP mode"));
        assert!(instructions.contains("exactly one tool: wot_exec"));
        assert!(instructions.contains("screenshots"));
        assert!(instructions.contains("Windows desktop application"));
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
        assert_eq!(tool_names, ["wot_exec"]);

        let exec = http_post(
            address,
            &endpoint,
            Some(session_id),
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"wot_exec","arguments":{"code":"1 + 1"}}}"#,
        )
        .await;
        assert_eq!(exec.status, 200, "{}", exec.body);
        let exec_body = json_body(&exec);
        assert_eq!(exec_body["result"]["isError"], true);
        assert!(exec_body["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("NO_REMOTE_AGENT"));

        runtime.stop().await;
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
        let mcp_headers = if mcp_headers {
            "Content-Type: application/json\r\nAccept: application/json, text/event-stream\r\nMCP-Protocol-Version: 2025-11-25\r\n"
        } else {
            ""
        };
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

//! Backend state shared by the desktop commands and other adapters.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use tokio::sync::Notify;

use crate::install::{self, GameInfo};
use crate::mcp::McpRuntime;
use crate::process;
use crate::protocol::{InFrame, LogLine, OutFrame, ServerEvent};
use crate::transport::{AgentConfigStore, AgentConnectionInfo, EventSink, NetworkTransport};

const LOG_MAX_ENTRIES: usize = 10_000;
const LOG_MAX_TEXT_BYTES: usize = 4 * 1024 * 1024;

#[derive(Default)]
pub struct AppState {
    pub client: ClientManager,
    pub mcp: McpRuntime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessStatus {
    Stopped,
    Starting,
    Running,
    Stopping,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentStatus {
    Unavailable,
    Connecting,
    Ready,
    Unresponsive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ClientKind {
    Local,
    Remote,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ClientCapabilities {
    pub repl: bool,
    pub start: bool,
    pub close: bool,
    pub kill: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientStatus {
    #[serde(flatten)]
    pub game: Option<GameInfo>,
    pub kind: ClientKind,
    pub process_status: ProcessStatus,
    pub agent_status: AgentStatus,
    pub pid: Option<u32>,
    pub agent_version: Option<String>,
    pub agent_pid: Option<i64>,
    pub capabilities: ClientCapabilities,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CloseResult {
    pub still_running: bool,
    pub client: ClientStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LogEntry {
    pub sequence: u64,
    pub stream: String,
    pub level: Option<String>,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogRead {
    pub entries: Vec<LogEntry>,
    pub next_cursor: u64,
    pub truncated: bool,
}

#[derive(Debug)]
struct PendingRequest {
    receiver: Receiver<OutFrame>,
    generation: u64,
}

#[derive(Clone)]
pub struct ClientManager {
    inner: Arc<Mutex<ClientState>>,
    log_notify: Arc<Notify>,
    agent_config: Arc<AgentConfigStore>,
}

#[derive(Default)]
struct ClientState {
    transport: Option<Arc<NetworkTransport>>,
    active: Option<ActiveClient>,
    remote: Option<RemoteClient>,
    connection_generation: u64,
    logs: LogBuffer,
}

#[derive(Default)]
struct LogBuffer {
    entries: VecDeque<LogEntry>,
    text_bytes: usize,
    last_sequence: u64,
    discarded_through: u64,
}

#[derive(Clone)]
struct ActiveClient {
    game: GameInfo,
    expected_exe: PathBuf,
    pid: Option<u32>,
    process_status: ProcessStatus,
    agent_status: AgentStatus,
    agent_version: Option<String>,
}

#[derive(Clone)]
struct RemoteClient {
    agent_version: Option<String>,
    agent_pid: Option<i64>,
    agent_status: AgentStatus,
}

#[derive(Clone)]
struct AttachedLocalProcess {
    game: GameInfo,
    executable: PathBuf,
}

#[derive(Debug)]
enum StartDecision {
    Reserved,
    Already(ClientStatus),
}

impl Default for ClientManager {
    fn default() -> Self {
        Self::with_config_path(install::app_data_root().join("agent-network.json"))
    }
}

impl ClientManager {
    fn with_config_path(path: PathBuf) -> Self {
        Self {
            inner: Arc::new(Mutex::new(ClientState::default())),
            log_notify: Arc::new(Notify::new()),
            agent_config: Arc::new(AgentConfigStore::new(path)),
        }
    }

    pub fn connection_info(&self) -> Result<AgentConnectionInfo, String> {
        self.agent_config.connection_info()
    }

    pub fn install_agent(&self, game_dir: &str, mods_version: &str) -> Result<(), String> {
        let config = self.agent_config.load_or_create()?;
        let body = serde_json::to_vec_pretty(&config).map_err(|error| error.to_string())?;
        install::install_agent(game_dir, mods_version, &body)
    }

    pub fn connect(
        &self,
        lan_enabled: bool,
        secure_enabled: bool,
        sink: EventSink,
    ) -> Result<(), String> {
        let (generation, old) = {
            let mut state = self
                .inner
                .lock()
                .map_err(|_| "state lock poisoned".to_string())?;
            state.connection_generation = state.connection_generation.wrapping_add(1);
            state.remote = None;
            if let Some(active) = &mut state.active {
                if active.process_status != ProcessStatus::Stopped {
                    active.agent_status = AgentStatus::Connecting;
                }
            }
            (state.connection_generation, state.transport.take())
        };
        if let Some(old) = old {
            old.stop();
        }
        let state = Arc::clone(&self.inner);
        let log_notify = Arc::clone(&self.log_notify);
        let tracked_sink: EventSink = Arc::new(move |event| {
            let attached = match &event {
                ServerEvent::Hello {
                    pid: Some(pid),
                    remote: false,
                    ..
                } => u32::try_from(*pid).ok().and_then(game_for_process),
                _ => None,
            };
            let (current, log_changed) = if let Ok(mut state) = state.lock() {
                if state.connection_generation == generation {
                    let log_changed = match &event {
                        ServerEvent::Hello {
                            version,
                            pid,
                            remote,
                        } => state.agent_connected(*pid, version.clone(), *remote, attached),
                        ServerEvent::Disconnected => {
                            state.agent_disconnected();
                            false
                        }
                        ServerEvent::Log { lines } => state.logs.push_lines(lines),
                    };
                    (true, log_changed)
                } else {
                    (false, false)
                }
            } else {
                (false, false)
            };
            if log_changed {
                log_notify.notify_waiters();
            }
            if current {
                sink(event);
            }
        });
        let config = self.agent_config.load_or_create()?;
        let transport = NetworkTransport::start(config, lan_enabled, secure_enabled, tracked_sink)?;
        let mut state = self
            .inner
            .lock()
            .map_err(|_| "state lock poisoned".to_string())?;
        if state.connection_generation != generation {
            transport.stop();
        } else {
            state.transport = Some(transport);
        }
        Ok(())
    }

    pub fn disconnect(&self) -> Result<(), String> {
        let transport = {
            let mut state = self
                .inner
                .lock()
                .map_err(|_| "state lock poisoned".to_string())?;
            state.connection_generation = state.connection_generation.wrapping_add(1);
            state.agent_disconnected();
            state.transport.take()
        };
        if let Some(transport) = transport {
            transport.stop();
        }
        Ok(())
    }

    fn request(&self, frame: InFrame) -> Result<PendingRequest, String> {
        let (transport, generation) = {
            let state = self
                .inner
                .lock()
                .map_err(|_| "state lock poisoned".to_string())?;
            let transport = state
                .transport
                .clone()
                .ok_or_else(|| "not connected".to_string())?;
            (transport, state.connection_generation)
        };
        Ok(PendingRequest {
            receiver: transport.request(frame),
            generation,
        })
    }

    pub async fn request_with_timeout(
        &self,
        frame: InFrame,
        timeout: Duration,
    ) -> Result<OutFrame, String> {
        let PendingRequest {
            receiver,
            generation,
        } = self.request(frame)?;
        let received = tokio::task::spawn_blocking(move || receiver.recv_timeout(timeout))
            .await
            .map_err(|error| format!("agent response wait failed: {error}"))?;
        match received {
            Ok(frame) => {
                self.record_agent_response(generation);
                Ok(frame)
            }
            Err(RecvTimeoutError::Timeout) => {
                self.record_agent_timeout(generation);
                Err("agent did not respond in time".to_string())
            }
            Err(RecvTimeoutError::Disconnected) => {
                Err("agent response channel disconnected".to_string())
            }
        }
    }

    fn record_agent_response(&self, generation: u64) {
        if let Ok(mut state) = self.inner.lock() {
            state.agent_responded(generation);
        }
    }

    fn record_agent_timeout(&self, generation: u64) {
        if let Ok(mut state) = self.inner.lock() {
            state.agent_timed_out(generation);
        }
    }

    pub fn list(&self) -> Result<Vec<ClientStatus>, String> {
        let games = install::detect_games();
        let mut state = self
            .inner
            .lock()
            .map_err(|_| "state lock poisoned".to_string())?;
        state.refresh_process();
        Ok(state.statuses(games))
    }

    pub async fn read_log(
        &self,
        cursor: Option<u64>,
        limit: Option<i64>,
        wait_ms: Option<i64>,
    ) -> Result<LogRead, String> {
        let limit = limit.unwrap_or(200).clamp(1, 1000) as usize;
        let wait = Duration::from_millis(wait_ms.unwrap_or(0).clamp(0, 5000) as u64);
        let deadline = tokio::time::Instant::now() + wait;
        loop {
            let notified = self.log_notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();

            let result = self.read_log_now(cursor, limit)?;
            if !result.entries.is_empty() || result.truncated || wait.is_zero() {
                return Ok(result);
            }
            if tokio::time::Instant::now() >= deadline {
                return Ok(result);
            }
            if tokio::time::timeout_at(deadline, notified).await.is_err() {
                return self.read_log_now(cursor, limit);
            }
        }
    }

    fn read_log_now(&self, cursor: Option<u64>, limit: usize) -> Result<LogRead, String> {
        self.inner
            .lock()
            .map_err(|_| "state lock poisoned".to_string())
            .map(|state| state.logs.read(cursor, limit))
    }

    pub fn start(
        &self,
        game_dir: &str,
        replay: Option<&str>,
        sink: EventSink,
    ) -> Result<ClientStatus, String> {
        let game = install::inspect_dir(Path::new(game_dir))
            .ok_or_else(|| "not a supported WoT install".to_string())?;
        match self.reserve_start(game.clone())? {
            StartDecision::Already(status) => return Ok(status),
            StartDecision::Reserved => {}
        }

        if let Err(error) = self.install_agent(&game.path, &game.mods_version) {
            self.fail_start(&game);
            return Err(error);
        }
        if let Ok(mut state) = self.inner.lock() {
            if let Some(active) = &mut state.active {
                if same_game(&active.game, &game) {
                    active.game.installed = true;
                }
            }
        }
        if let Err(error) = self.connect(false, true, sink) {
            self.fail_start(&game);
            return Err(error);
        }
        if let Err(error) = self.spawn_reserved(&game, replay) {
            let _ = self.disconnect();
            self.fail_start(&game);
            return Err(error);
        }
        self.active_status()
    }

    pub fn launch(
        &self,
        game_dir: &str,
        exe: &str,
        replay: Option<&str>,
    ) -> Result<ClientStatus, String> {
        let game = install::inspect_dir(Path::new(game_dir))
            .ok_or_else(|| "not a supported WoT install".to_string())?;
        if !game.exe.eq_ignore_ascii_case(exe) {
            return Err(format!("unexpected client executable: {exe}"));
        }
        match self.reserve_start(game.clone())? {
            StartDecision::Already(status) => Ok(status),
            StartDecision::Reserved => {
                if let Err(error) = self.spawn_reserved(&game, replay) {
                    self.fail_start(&game);
                    return Err(error);
                }
                self.active_status()
            }
        }
    }

    pub fn close(&self, timeout: Duration) -> Result<CloseResult, String> {
        let (pid, expected_exe) = self.active_target()?;
        process::request_close(pid, &expected_exe)?;
        {
            let mut state = self
                .inner
                .lock()
                .map_err(|_| "state lock poisoned".to_string())?;
            if let Some(active) = &mut state.active {
                active.process_status = ProcessStatus::Stopping;
            }
        }

        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline && process::is_expected_process(pid, &expected_exe) {
            std::thread::sleep(Duration::from_millis(50));
        }
        let client = self.active_status()?;
        Ok(CloseResult {
            still_running: client.process_status != ProcessStatus::Stopped,
            client,
        })
    }

    pub fn kill(&self) -> Result<ClientStatus, String> {
        let (pid, expected_exe) = self.active_target()?;
        process::kill(pid, &expected_exe)?;
        if let Ok(mut state) = self.inner.lock() {
            if let Some(active) = &mut state.active {
                active.process_status = ProcessStatus::Stopping;
            }
        }
        self.active_status()
    }

    fn reserve_start(&self, game: GameInfo) -> Result<StartDecision, String> {
        let mut state = self
            .inner
            .lock()
            .map_err(|_| "state lock poisoned".to_string())?;
        state.refresh_process();
        let decision = state.reserve_start(game)?;
        let cleared = matches!(&decision, StartDecision::Reserved) && state.logs.clear();
        drop(state);
        if cleared {
            self.log_notify.notify_waiters();
        }
        Ok(decision)
    }

    fn spawn_reserved(&self, game: &GameInfo, replay: Option<&str>) -> Result<(), String> {
        let pid = install::launch_game(&game.path, &game.exe, replay)?;
        let mut state = self
            .inner
            .lock()
            .map_err(|_| "state lock poisoned".to_string())?;
        let agent_connecting = state.transport.is_some();
        let active = state
            .active
            .as_mut()
            .filter(|active| same_game(&active.game, game))
            .ok_or_else(|| "active client changed while starting".to_string())?;
        active.pid = Some(pid);
        active.process_status = ProcessStatus::Running;
        if agent_connecting {
            active.agent_status = AgentStatus::Connecting;
        }
        Ok(())
    }

    fn fail_start(&self, game: &GameInfo) {
        if let Ok(mut state) = self.inner.lock() {
            if let Some(active) = &mut state.active {
                if same_game(&active.game, game) && active.process_status == ProcessStatus::Starting
                {
                    active.process_status = ProcessStatus::Stopped;
                    active.agent_status = AgentStatus::Unavailable;
                }
            }
        }
    }

    fn active_target(&self) -> Result<(u32, PathBuf), String> {
        let mut state = self
            .inner
            .lock()
            .map_err(|_| "state lock poisoned".to_string())?;
        state.refresh_process();
        if state.remote.is_some() {
            return Err(
                "REMOTE_CLIENT_UNMANAGED: the connected game is remote; use REPL and log tools, not process-control tools"
                    .to_string(),
            );
        }
        let active = state
            .active
            .as_ref()
            .filter(|active| active.process_status != ProcessStatus::Stopped)
            .ok_or_else(|| "NO_ACTIVE_CLIENT: no running client".to_string())?;
        let pid = active
            .pid
            .ok_or_else(|| "CLIENT_NOT_RUNNING: client process has not started".to_string())?;
        Ok((pid, active.expected_exe.clone()))
    }

    fn active_status(&self) -> Result<ClientStatus, String> {
        let mut state = self
            .inner
            .lock()
            .map_err(|_| "state lock poisoned".to_string())?;
        state.refresh_process();
        state
            .active
            .clone()
            .map(ClientStatus::from)
            .ok_or_else(|| "NO_ACTIVE_CLIENT: no client selected".to_string())
    }
}

impl ClientState {
    fn reserve_start(&mut self, game: GameInfo) -> Result<StartDecision, String> {
        if self.remote.is_some() {
            return Err(
                "REMOTE_CLIENT_ACTIVE: a remote game is connected; it supports REPL and logs but cannot be started from this UI"
                    .to_string(),
            );
        }
        if let Some(active) = &self.active {
            if active.process_status != ProcessStatus::Stopped {
                if same_game(&active.game, &game) {
                    return Ok(StartDecision::Already(active.clone().into()));
                }
                return Err(format!(
                    "ACTIVE_CLIENT_EXISTS: {} is already {:?}",
                    active.game.path, active.process_status
                ));
            }
        }
        let agent_status = if self.transport.is_some() {
            AgentStatus::Connecting
        } else {
            AgentStatus::Unavailable
        };
        let expected_exe = PathBuf::from(&game.path).join(&game.exe);
        self.active = Some(ActiveClient {
            game,
            expected_exe,
            pid: None,
            process_status: ProcessStatus::Starting,
            agent_status,
            agent_version: None,
        });
        Ok(StartDecision::Reserved)
    }

    fn status_for(&self, game: GameInfo) -> ClientStatus {
        self.active
            .as_ref()
            .filter(|active| same_game(&active.game, &game))
            .cloned()
            .map(ClientStatus::from)
            .unwrap_or(ClientStatus {
                game: Some(game),
                kind: ClientKind::Local,
                process_status: ProcessStatus::Stopped,
                agent_status: AgentStatus::Unavailable,
                pid: None,
                agent_version: None,
                agent_pid: None,
                capabilities: ClientCapabilities {
                    repl: false,
                    start: true,
                    close: false,
                    kill: false,
                },
            })
    }

    fn statuses(&self, mut games: Vec<GameInfo>) -> Vec<ClientStatus> {
        if let Some(active) = &self.active {
            if !games.iter().any(|game| same_game(game, &active.game)) {
                games.push(active.game.clone());
            }
        }
        let mut statuses: Vec<_> = games
            .into_iter()
            .map(|game| self.status_for(game))
            .collect();
        if let Some(remote) = &self.remote {
            statuses.push(remote.clone().into());
        }
        statuses
    }

    fn refresh_process(&mut self) {
        let Some(active) = &mut self.active else {
            return;
        };
        let Some(pid) = active.pid else {
            return;
        };
        if !process::is_expected_process(pid, &active.expected_exe) {
            active.pid = None;
            active.process_status = ProcessStatus::Stopped;
            active.agent_status = AgentStatus::Unavailable;
        }
    }

    fn agent_connected(
        &mut self,
        reported_pid: Option<i64>,
        agent_version: Option<String>,
        remote_connection: bool,
        attached_process: Option<AttachedLocalProcess>,
    ) -> bool {
        if remote_connection {
            let cleared = self.logs.clear();
            self.remote = Some(RemoteClient {
                agent_version,
                agent_pid: reported_pid,
                agent_status: AgentStatus::Ready,
            });
            return cleared;
        }
        let local_pid = reported_pid.and_then(|pid| u32::try_from(pid).ok());
        if let (Some(pid), Some(attached)) = (local_pid, attached_process) {
            let can_bind = self
                .active
                .as_ref()
                .map(|active| {
                    active.process_status == ProcessStatus::Stopped
                        || same_game(&active.game, &attached.game)
                })
                .unwrap_or(true);
            if can_bind {
                let cleared = self
                    .active
                    .as_ref()
                    .is_none_or(|active| active.process_status == ProcessStatus::Stopped)
                    && self.logs.clear();
                self.active = Some(ActiveClient {
                    game: attached.game,
                    expected_exe: attached.executable,
                    pid: Some(pid),
                    process_status: ProcessStatus::Running,
                    agent_status: AgentStatus::Ready,
                    agent_version,
                });
                self.remote = None;
                return cleared;
            }
        }
        let can_attach = self
            .active
            .as_ref()
            .map(|active| active.process_status == ProcessStatus::Stopped)
            .unwrap_or(true);
        if can_attach {
            let cleared = self.logs.clear();
            self.remote = Some(RemoteClient {
                agent_version,
                agent_pid: reported_pid,
                agent_status: AgentStatus::Ready,
            });
            return cleared;
        }
        let active = self.active.as_mut().unwrap();
        if active.pid.is_none() || (local_pid.is_some() && active.pid != local_pid) {
            return false;
        }
        active.agent_status = AgentStatus::Ready;
        active.agent_version = agent_version;
        self.remote = None;
        false
    }

    fn agent_disconnected(&mut self) {
        self.remote = None;
        if let Some(active) = &mut self.active {
            active.agent_status = AgentStatus::Unavailable;
        }
    }

    fn agent_responded(&mut self, generation: u64) {
        if self.connection_generation != generation {
            return;
        }
        if let Some(remote) = &mut self.remote {
            remote.agent_status = AgentStatus::Ready;
            return;
        }
        if let Some(active) = &mut self.active {
            if active.process_status != ProcessStatus::Stopped {
                active.agent_status = AgentStatus::Ready;
            }
        }
    }

    fn agent_timed_out(&mut self, generation: u64) {
        if self.connection_generation != generation {
            return;
        }
        if let Some(remote) = &mut self.remote {
            if remote.agent_status != AgentStatus::Unavailable {
                remote.agent_status = AgentStatus::Unresponsive;
            }
            return;
        }
        if let Some(active) = &mut self.active {
            if active.process_status != ProcessStatus::Stopped
                && active.agent_status != AgentStatus::Unavailable
            {
                active.agent_status = AgentStatus::Unresponsive;
            }
        }
    }
}

impl LogBuffer {
    fn push_lines(&mut self, lines: &[LogLine]) -> bool {
        for line in lines {
            self.last_sequence = self
                .last_sequence
                .checked_add(1)
                .expect("log sequence exhausted");
            let text = truncate_utf8(&line.text, LOG_MAX_TEXT_BYTES);
            self.text_bytes += text.len();
            self.entries.push_back(LogEntry {
                sequence: self.last_sequence,
                stream: line.stream.clone(),
                level: line.level.clone(),
                text,
            });
            while self.entries.len() > LOG_MAX_ENTRIES || self.text_bytes > LOG_MAX_TEXT_BYTES {
                let removed = self.entries.pop_front().unwrap();
                self.text_bytes -= removed.text.len();
                self.discarded_through = removed.sequence;
            }
        }
        !lines.is_empty()
    }

    fn clear(&mut self) -> bool {
        let changed = !self.entries.is_empty() || self.discarded_through < self.last_sequence;
        self.entries.clear();
        self.text_bytes = 0;
        self.discarded_through = self.last_sequence;
        changed
    }

    fn read(&self, cursor: Option<u64>, limit: usize) -> LogRead {
        let entries: Vec<LogEntry> = match cursor {
            Some(cursor) => self
                .entries
                .iter()
                .filter(|entry| entry.sequence > cursor)
                .take(limit)
                .cloned()
                .collect(),
            None => self
                .entries
                .iter()
                .skip(self.entries.len().saturating_sub(limit))
                .cloned()
                .collect(),
        };
        let next_cursor = entries
            .last()
            .map(|entry| entry.sequence)
            .unwrap_or_else(|| {
                cursor
                    .map(|cursor| cursor.max(self.last_sequence))
                    .unwrap_or(self.last_sequence)
            });
        LogRead {
            entries,
            next_cursor,
            truncated: cursor.is_some_and(|cursor| cursor < self.discarded_through),
        }
    }
}

fn truncate_utf8(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.to_string();
    }
    let mut end = max_bytes;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_string()
}

impl From<ActiveClient> for ClientStatus {
    fn from(active: ActiveClient) -> Self {
        let can_control_process =
            active.pid.is_some() && active.process_status != ProcessStatus::Stopped;
        Self {
            game: Some(active.game),
            kind: ClientKind::Local,
            process_status: active.process_status,
            agent_status: active.agent_status,
            pid: active.pid,
            agent_version: active.agent_version,
            agent_pid: None,
            capabilities: ClientCapabilities {
                repl: active.agent_status == AgentStatus::Ready,
                start: active.process_status == ProcessStatus::Stopped,
                close: can_control_process,
                kill: can_control_process,
            },
        }
    }
}

impl From<RemoteClient> for ClientStatus {
    fn from(remote: RemoteClient) -> Self {
        Self {
            game: None,
            kind: ClientKind::Remote,
            process_status: ProcessStatus::Running,
            agent_status: remote.agent_status,
            pid: None,
            agent_version: remote.agent_version,
            agent_pid: remote.agent_pid,
            capabilities: ClientCapabilities {
                repl: remote.agent_status == AgentStatus::Ready,
                start: false,
                close: false,
                kill: false,
            },
        }
    }
}

fn same_game(left: &GameInfo, right: &GameInfo) -> bool {
    left.path.eq_ignore_ascii_case(&right.path) && left.exe.eq_ignore_ascii_case(&right.exe)
}

fn game_for_process(pid: u32) -> Option<AttachedLocalProcess> {
    let executable = process::executable_path(pid).ok()?;
    let game = install::inspect_game_executable(&executable)?;
    Some(AttachedLocalProcess { game, executable })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn game(path: &str) -> GameInfo {
        GameInfo {
            path: path.into(),
            version: "1.0".into(),
            mods_version: "1.0".into(),
            exe: "Tanki.exe".into(),
            installed: false,
        }
    }

    fn attached(game: GameInfo) -> AttachedLocalProcess {
        let executable = PathBuf::from(&game.path).join(&game.exe);
        AttachedLocalProcess { game, executable }
    }

    fn log_line(text: impl Into<String>) -> LogLine {
        LogLine {
            stream: "stdout".into(),
            level: None,
            timestamp: None,
            source: None,
            text: text.into(),
        }
    }

    #[test]
    fn log_buffer_evicts_and_preserves_cursor_across_clear() {
        let mut logs = LogBuffer::default();
        let line = log_line("x");
        for _ in 0..=LOG_MAX_ENTRIES {
            logs.push_lines(std::slice::from_ref(&line));
        }

        assert_eq!(logs.entries.len(), LOG_MAX_ENTRIES);
        let after_eviction = logs.read(Some(0), 3);
        assert!(after_eviction.truncated);
        assert_eq!(after_eviction.entries[0].sequence, 2);
        assert_eq!(after_eviction.next_cursor, 4);
        let latest = logs.read(None, 2);
        assert_eq!(latest.entries[0].sequence, LOG_MAX_ENTRIES as u64);
        assert_eq!(latest.next_cursor, LOG_MAX_ENTRIES as u64 + 1);

        let last_sequence = logs.last_sequence;
        assert!(logs.clear());
        let after_clear = logs.read(Some(0), 3);
        assert!(after_clear.truncated);
        assert_eq!(after_clear.next_cursor, last_sequence);
        logs.push_lines(&[log_line("next")]);
        assert_eq!(logs.entries[0].sequence, last_sequence + 1);

        let mut oversized = LogBuffer::default();
        oversized.push_lines(&[log_line("é".repeat(LOG_MAX_TEXT_BYTES / 2 + 1))]);
        assert_eq!(oversized.text_bytes, LOG_MAX_TEXT_BYTES);
        assert_eq!(oversized.entries[0].text.len(), LOG_MAX_TEXT_BYTES);
    }

    #[tokio::test]
    async fn log_wait_wakes_for_a_new_entry() {
        let manager = ClientManager::default();
        let waiting_manager = manager.clone();
        let waiting = tokio::spawn(async move {
            waiting_manager
                .read_log(Some(0), Some(10), Some(5000))
                .await
                .unwrap()
        });
        tokio::task::yield_now().await;

        manager
            .inner
            .lock()
            .unwrap()
            .logs
            .push_lines(&[log_line("awake")]);
        manager.log_notify.notify_waiters();

        let result = tokio::time::timeout(Duration::from_secs(1), waiting)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(result.entries.len(), 1);
        assert_eq!(result.entries[0].text, "awake");
        assert_eq!(result.next_cursor, 1);
        assert!(!result.truncated);
    }

    #[test]
    fn a_new_session_clears_logs_without_resetting_the_cursor() {
        let manager = ClientManager::default();
        let same_game = game("C:/Games/one");
        {
            let mut state = manager.inner.lock().unwrap();
            state.reserve_start(same_game.clone()).unwrap();
            state.active.as_mut().unwrap().process_status = ProcessStatus::Stopped;
            state.logs.push_lines(&[log_line("old")]);
        }

        assert!(matches!(
            manager.reserve_start(same_game.clone()).unwrap(),
            StartDecision::Reserved
        ));
        let after_reserve = manager.read_log_now(Some(0), 10).unwrap();
        assert!(after_reserve.entries.is_empty());
        assert!(after_reserve.truncated);
        assert_eq!(after_reserve.next_cursor, 1);

        {
            let mut state = manager.inner.lock().unwrap();
            state.active.as_mut().unwrap().process_status = ProcessStatus::Stopped;
            state.logs.push_lines(&[log_line("old attach")]);
            assert!(state.agent_connected(
                Some(42),
                Some("test".into()),
                false,
                Some(attached(same_game))
            ));
        }
        let after_attach = manager.read_log_now(Some(1), 10).unwrap();
        assert!(after_attach.entries.is_empty());
        assert!(after_attach.truncated);
        assert_eq!(after_attach.next_cursor, 2);
    }

    #[test]
    fn only_one_client_can_be_reserved() {
        let mut state = ClientState::default();
        assert!(matches!(
            state.reserve_start(game("C:/Games/one")),
            Ok(StartDecision::Reserved)
        ));
        assert!(matches!(
            state.reserve_start(game("c:/games/ONE")),
            Ok(StartDecision::Already(_))
        ));
        assert!(state
            .reserve_start(game("C:/Games/two"))
            .unwrap_err()
            .starts_with("ACTIVE_CLIENT_EXISTS:"));
    }

    #[test]
    fn agent_status_follows_observed_signals() {
        let mut state = ClientState::default();
        state.reserve_start(game("C:/Games/one")).unwrap();
        let active = state.active.as_mut().unwrap();
        active.pid = Some(42);
        active.process_status = ProcessStatus::Running;
        state.agent_connected(Some(42), Some("test".into()), false, None);
        let active = state.active.as_ref().unwrap();
        assert_eq!(active.process_status, ProcessStatus::Running);
        assert_eq!(active.agent_status, AgentStatus::Ready);

        state.connection_generation = 1;
        state.agent_timed_out(0);
        assert_eq!(
            state.active.as_ref().unwrap().agent_status,
            AgentStatus::Ready,
            "a stale timeout must not update the new connection"
        );
        state.agent_timed_out(1);
        assert_eq!(
            state.active.as_ref().unwrap().agent_status,
            AgentStatus::Unresponsive
        );
        state.agent_responded(1);
        assert_eq!(
            state.active.as_ref().unwrap().agent_status,
            AgentStatus::Ready
        );
        state.agent_disconnected();
        assert_eq!(
            state.active.as_ref().unwrap().agent_status,
            AgentStatus::Unavailable
        );

        let active = state.active.as_mut().unwrap();
        active.pid = Some(u32::MAX);
        state.refresh_process();
        assert_eq!(
            state.active.as_ref().unwrap().process_status,
            ProcessStatus::Stopped
        );
    }

    #[test]
    fn hello_attaches_a_verified_game_to_the_empty_state() {
        let mut state = ClientState::default();
        let attached_game = game("C:/Games/one");
        state.agent_connected(
            Some(42),
            Some("test".into()),
            false,
            Some(attached(attached_game)),
        );

        let status = state.statuses(Vec::new()).pop().unwrap();
        assert_eq!(status.kind, ClientKind::Local);
        assert_eq!(status.pid, Some(42));
        assert_eq!(status.process_status, ProcessStatus::Running);
        assert_eq!(status.agent_status, AgentStatus::Ready);
        assert!(status.capabilities.repl);
        assert!(status.capabilities.close);
        assert!(status.capabilities.kill);
    }

    #[test]
    fn verified_hello_rebinds_launcher_pid_to_architecture_process() {
        let mut state = ClientState::default();
        let launched = game("C:/Games/one");
        state.reserve_start(launched.clone()).unwrap();
        let active = state.active.as_mut().unwrap();
        active.pid = Some(41);
        active.process_status = ProcessStatus::Running;

        let executable = PathBuf::from(&launched.path)
            .join("win64")
            .join(&launched.exe);
        state.agent_connected(
            Some(42),
            Some("test".into()),
            false,
            Some(AttachedLocalProcess {
                game: launched,
                executable: executable.clone(),
            }),
        );

        let active = state.active.as_ref().unwrap();
        assert_eq!(active.pid, Some(42));
        assert_eq!(active.game.path, "C:/Games/one");
        assert_eq!(active.expected_exe, executable);
        assert_eq!(active.process_status, ProcessStatus::Running);
        assert_eq!(active.agent_status, AgentStatus::Ready);

        let statuses = state.statuses(vec![game("C:/Games/one")]);
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].pid, Some(42));
        assert_eq!(statuses[0].process_status, ProcessStatus::Running);
        assert!(statuses[0].capabilities.repl);
        assert!(statuses[0].capabilities.close);
        assert!(statuses[0].capabilities.kill);
    }

    #[test]
    fn remote_agent_is_listed_without_local_process_controls() {
        let mut state = ClientState::default();
        state.agent_connected(
            Some(4242),
            Some("0.4.1".into()),
            true,
            Some(attached(game("C:/Games/coincidental-pid"))),
        );

        let statuses = state.statuses(vec![game("C:/Games/local")]);
        assert_eq!(statuses.len(), 2);
        let remote = statuses
            .iter()
            .find(|status| status.kind == ClientKind::Remote)
            .unwrap();
        assert!(remote.game.is_none());
        assert_eq!(remote.pid, None);
        assert_eq!(remote.agent_pid, Some(4242));
        assert_eq!(remote.agent_version.as_deref(), Some("0.4.1"));
        assert!(remote.capabilities.repl);
        assert!(!remote.capabilities.start);
        assert!(!remote.capabilities.close);
        assert!(!remote.capabilities.kill);

        let json = serde_json::to_value(remote).unwrap();
        assert_eq!(json["kind"], "remote");
        assert!(json.get("path").is_none());
        assert!(json.get("exe").is_none());
        assert_eq!(json["capabilities"]["repl"], true);
        assert_eq!(json["capabilities"]["start"], false);

        assert!(state
            .reserve_start(game("C:/Games/another"))
            .unwrap_err()
            .starts_with("REMOTE_CLIENT_ACTIVE:"));

        state.agent_disconnected();
        assert!(state
            .statuses(Vec::new())
            .iter()
            .all(|status| status.kind != ClientKind::Remote));
        assert!(matches!(
            state.reserve_start(game("C:/Games/another")),
            Ok(StartDecision::Reserved)
        ));
    }

    #[test]
    fn remote_agent_cannot_be_closed_or_killed_as_a_local_process() {
        let manager = ClientManager::default();
        manager.inner.lock().unwrap().remote = Some(RemoteClient {
            agent_version: Some("0.4.1".into()),
            agent_pid: Some(4242),
            agent_status: AgentStatus::Ready,
        });

        assert!(manager
            .close(Duration::ZERO)
            .unwrap_err()
            .starts_with("REMOTE_CLIENT_UNMANAGED:"));
        assert!(manager
            .kill()
            .unwrap_err()
            .starts_with("REMOTE_CLIENT_UNMANAGED:"));
    }

    #[test]
    fn disconnect_clears_the_transport() {
        let dir = std::env::temp_dir().join(format!("wms_client_manager_{}", uuid::Uuid::new_v4()));
        let manager = ClientManager::with_config_path(dir.join("agent-network.json"));

        manager.connect(false, true, Arc::new(|_| {})).unwrap();
        manager.disconnect().unwrap();
        assert_eq!(
            manager
                .request(InFrame::Exec {
                    id: "test".into(),
                    code: "1".into(),
                })
                .unwrap_err(),
            "not connected"
        );

        let _ = std::fs::remove_dir_all(dir);
    }
}

//! Backend state shared by the desktop commands and other adapters.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use tokio::sync::Notify;

use crate::install::{self, GameInfo};
use crate::jedi::JediWorker;
use crate::mcp::McpRuntime;
use crate::process;
use crate::protocol::{InFrame, LogLine, OutFrame, ServerEvent};
use crate::transport::{EventSink, FileBufferTransport};

const LOG_MAX_ENTRIES: usize = 10_000;
const LOG_MAX_TEXT_BYTES: usize = 4 * 1024 * 1024;

#[derive(Default)]
pub struct AppState {
    pub client: ClientManager,
    pub jedi: Mutex<Option<Arc<JediWorker>>>,
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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientStatus {
    #[serde(flatten)]
    pub game: GameInfo,
    pub process_status: ProcessStatus,
    pub agent_status: AgentStatus,
    pub pid: Option<u32>,
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

#[derive(Clone, Default)]
pub struct ClientManager {
    inner: Arc<Mutex<ClientState>>,
    log_notify: Arc<Notify>,
}

#[derive(Default)]
struct ClientState {
    transport: Option<Arc<FileBufferTransport>>,
    active: Option<ActiveClient>,
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
}

#[derive(Debug)]
enum StartDecision {
    Reserved,
    Already(ClientStatus),
}

impl ClientManager {
    pub fn connect(&self, dir: PathBuf, sink: EventSink) -> Result<(), String> {
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let (generation, old) = {
            let mut state = self
                .inner
                .lock()
                .map_err(|_| "state lock poisoned".to_string())?;
            state.connection_generation = state.connection_generation.wrapping_add(1);
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
                ServerEvent::Hello { pid: Some(pid), .. } => {
                    u32::try_from(*pid).ok().and_then(game_for_process)
                }
                _ => None,
            };
            let (current, log_changed) = if let Ok(mut state) = state.lock() {
                if state.connection_generation == generation {
                    let log_changed = match &event {
                        ServerEvent::Hello { pid, .. } => state.agent_connected(*pid, attached),
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
        let transport = FileBufferTransport::start(dir, tracked_sink);
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
        if frame.id().is_none() {
            return Err("control frames cannot be requested".to_string());
        }
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

        if let Err(error) = install::install_agent(&game.path, &game.mods_version) {
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
        if let Err(error) = self.connect(install::default_buffer_dir_path(), sink) {
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
                game,
                process_status: ProcessStatus::Stopped,
                agent_status: AgentStatus::Unavailable,
                pid: None,
            })
    }

    fn statuses(&self, mut games: Vec<GameInfo>) -> Vec<ClientStatus> {
        if let Some(active) = &self.active {
            if !games.iter().any(|game| same_game(game, &active.game)) {
                games.push(active.game.clone());
            }
        }
        games
            .into_iter()
            .map(|game| self.status_for(game))
            .collect()
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
        attached_game: Option<GameInfo>,
    ) -> bool {
        let reported_pid = reported_pid.and_then(|pid| u32::try_from(pid).ok());
        let can_attach = self
            .active
            .as_ref()
            .map(|active| active.process_status == ProcessStatus::Stopped)
            .unwrap_or(true);
        if can_attach {
            let (Some(pid), Some(game)) = (reported_pid, attached_game) else {
                return false;
            };
            let cleared = self.logs.clear();
            let expected_exe = PathBuf::from(&game.path).join(&game.exe);
            self.active = Some(ActiveClient {
                game,
                expected_exe,
                pid: Some(pid),
                process_status: ProcessStatus::Running,
                agent_status: AgentStatus::Ready,
            });
            return cleared;
        }
        let active = self.active.as_mut().unwrap();
        if active.pid.is_none() || (reported_pid.is_some() && active.pid != reported_pid) {
            return false;
        }
        active.agent_status = AgentStatus::Ready;
        false
    }

    fn agent_disconnected(&mut self) {
        if let Some(active) = &mut self.active {
            active.agent_status = AgentStatus::Unavailable;
        }
    }

    fn agent_responded(&mut self, generation: u64) {
        if self.connection_generation != generation {
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
        Self {
            game: active.game,
            process_status: active.process_status,
            agent_status: active.agent_status,
            pid: active.pid,
        }
    }
}

fn same_game(left: &GameInfo, right: &GameInfo) -> bool {
    left.path.eq_ignore_ascii_case(&right.path) && left.exe.eq_ignore_ascii_case(&right.exe)
}

fn game_for_process(pid: u32) -> Option<GameInfo> {
    let executable = process::executable_path(pid).ok()?;
    let game = install::inspect_dir(executable.parent()?)?;
    let actual_exe = executable.file_name()?.to_string_lossy();
    actual_exe.eq_ignore_ascii_case(&game.exe).then_some(game)
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

    fn log_line(text: impl Into<String>) -> LogLine {
        LogLine {
            stream: "stdout".into(),
            level: None,
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
            assert!(state.agent_connected(Some(42), Some(same_game)));
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
        state.agent_connected(Some(42), None);
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
        let attached = game("C:/Games/one");
        state.agent_connected(Some(42), Some(attached));

        let status = state.statuses(Vec::new()).pop().unwrap();
        assert_eq!(status.pid, Some(42));
        assert_eq!(status.process_status, ProcessStatus::Running);
        assert_eq!(status.agent_status, AgentStatus::Ready);
    }

    #[test]
    fn disconnect_clears_the_transport() {
        let manager = ClientManager::default();
        let dir = std::env::temp_dir().join(format!("wms_client_manager_{}", uuid::Uuid::new_v4()));

        manager.connect(dir.clone(), Arc::new(|_| {})).unwrap();
        assert_eq!(
            manager.request(InFrame::Hello).unwrap_err(),
            "control frames cannot be requested"
        );
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

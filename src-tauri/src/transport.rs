//! TCP transport between the desktop and the in-game agent.
//!
//! The game agent connects out to this listener. For LAN sessions it discovers
//! the listener with a UDP broadcast; localhost sessions skip discovery. Token
//! authentication is required by default and can be relaxed explicitly for
//! config-free agents. Application frames remain newline-delimited JSON and
//! carry a session/sequence pair so reconnects can replay unacknowledged frames
//! safely.

use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::net::{Ipv4Addr, Shutdown, SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

use crate::protocol::{InFrame, LogLine, OutFrame, ServerEvent};

pub const AGENT_PROTOCOL_VERSION: u32 = 1;
pub const AGENT_TCP_PORT: u16 = 8766;
pub const AGENT_DISCOVERY_PORT: u16 = 8767;

const MAX_FRAME_BYTES: usize = 2 * 1024 * 1024;
const ACCEPT_POLL: Duration = Duration::from_millis(25);
const READ_POLL: Duration = Duration::from_millis(100);
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);
const HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(20);
const HELLO_TIMEOUT: Duration = Duration::from_secs(5);

type HmacSha256 = Hmac<Sha256>;
pub type EventSink = Arc<dyn Fn(ServerEvent) + Send + Sync>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentNetworkConfig {
    pub token: String,
    #[serde(default = "default_agent_host")]
    pub host: String,
    #[serde(default = "default_tcp_port")]
    pub tcp_port: u16,
    #[serde(default = "default_discovery_port")]
    pub discovery_port: u16,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentConnectionInfo {
    pub local_address: String,
    pub network_address: String,
    pub config_path: String,
    pub client_config: String,
}

pub struct AgentConfigStore {
    path: PathBuf,
    lock: Mutex<()>,
}

impl AgentConfigStore {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            lock: Mutex::new(()),
        }
    }

    pub fn load_or_create(&self) -> Result<AgentNetworkConfig, String> {
        let _guard = self
            .lock
            .lock()
            .map_err(|_| "agent network config lock poisoned".to_string())?;
        if self.path.exists() {
            return read_config(&self.path);
        }
        let config = AgentNetworkConfig {
            token: uuid::Uuid::new_v4().to_string(),
            host: default_agent_host(),
            tcp_port: AGENT_TCP_PORT,
            discovery_port: AGENT_DISCOVERY_PORT,
        };
        let parent = self
            .path
            .parent()
            .ok_or_else(|| "agent network config path has no parent".to_string())?;
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "cannot create agent config directory {}: {error}",
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
                format!("cannot write agent config {}: {error}", self.path.display())
            })?,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                return read_config(&self.path)
            }
            Err(error) => {
                return Err(format!(
                    "cannot create agent config {}: {error}",
                    self.path.display()
                ))
            }
        }
        Ok(config)
    }

    pub fn connection_info(&self) -> Result<AgentConnectionInfo, String> {
        let config = self.load_or_create()?;
        Ok(AgentConnectionInfo {
            local_address: format!("{}:{}", Ipv4Addr::LOCALHOST, config.tcp_port),
            network_address: format!("{}:{}", advertised_ipv4(), config.tcp_port),
            config_path: self.path.to_string_lossy().into_owned(),
            client_config: String::from_utf8(serialize_config(&config)?)
                .map_err(|error| error.to_string())?,
        })
    }
}

fn default_agent_host() -> String {
    "auto".to_string()
}

fn default_tcp_port() -> u16 {
    AGENT_TCP_PORT
}

fn default_discovery_port() -> u16 {
    AGENT_DISCOVERY_PORT
}

fn read_config(path: &Path) -> Result<AgentNetworkConfig, String> {
    let body = fs::read_to_string(path)
        .map_err(|error| format!("cannot read agent config {}: {error}", path.display()))?;
    let config: AgentNetworkConfig = serde_json::from_str(&body)
        .map_err(|error| format!("invalid agent config {}: {error}", path.display()))?;
    validate_config(path, &config)?;
    Ok(config)
}

fn validate_config(path: &Path, config: &AgentNetworkConfig) -> Result<(), String> {
    uuid::Uuid::parse_str(&config.token).map_err(|error| {
        format!(
            "invalid agent config {}: invalid token: {error}",
            path.display()
        )
    })?;
    if config.host.trim().is_empty() {
        return Err(format!(
            "invalid agent config {}: host is empty",
            path.display()
        ));
    }
    if config.tcp_port == 0 || config.discovery_port == 0 {
        return Err(format!(
            "invalid agent config {}: ports must be non-zero",
            path.display()
        ));
    }
    Ok(())
}

fn serialize_config(config: &AgentNetworkConfig) -> Result<Vec<u8>, String> {
    serde_json::to_vec_pretty(config).map_err(|error| error.to_string())
}

fn advertised_ipv4() -> Ipv4Addr {
    UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))
        .and_then(|socket| {
            socket.connect((Ipv4Addr::new(8, 8, 8, 8), 80))?;
            socket.local_addr()
        })
        .ok()
        .and_then(|address| match address.ip() {
            std::net::IpAddr::V4(ip) => Some(ip),
            std::net::IpAddr::V6(_) => None,
        })
        .unwrap_or(Ipv4Addr::LOCALHOST)
}

#[derive(Deserialize)]
struct AgentHello {
    #[serde(rename = "type")]
    kind: String,
    protocol: u32,
    agent_id: String,
    session: String,
    nonce: String,
    #[serde(default)]
    proof: String,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    pid: Option<i64>,
    #[serde(default)]
    acked_seq: u64,
}

#[derive(Serialize)]
struct Welcome<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    protocol: u32,
    agent_id: &'a str,
    session: &'a str,
    nonce: &'a str,
    server_id: &'a str,
    secure: bool,
    proof: String,
}

#[derive(Deserialize)]
struct WireOutFrame {
    session: String,
    seq: u64,
    #[serde(flatten)]
    frame: OutFrame,
}

#[derive(Serialize)]
struct Ack<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    session: &'a str,
    seq: u64,
}

#[derive(Serialize)]
struct Ping {
    #[serde(rename = "type")]
    kind: &'static str,
}

#[derive(Deserialize)]
struct DiscoveryRequest {
    #[serde(rename = "type")]
    kind: String,
    protocol: u32,
    agent_id: String,
    nonce: String,
    #[serde(default)]
    proof: String,
}

#[derive(Serialize)]
struct DiscoveryOffer<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    protocol: u32,
    agent_id: &'a str,
    nonce: &'a str,
    tcp_port: u16,
    server_id: &'a str,
    secure: bool,
    proof: String,
}

struct ActiveWriter {
    connection_id: u64,
    stream: TcpStream,
}

#[derive(Default)]
struct DeliveryCursor {
    session: String,
    seq: u64,
}

pub struct NetworkTransport {
    pending: Mutex<HashMap<String, Sender<OutFrame>>>,
    running: Arc<AtomicBool>,
    sink: EventSink,
    token: String,
    secure_enabled: bool,
    server_id: String,
    tcp_port: u16,
    writer: Mutex<Option<ActiveWriter>>,
    next_connection_id: AtomicU64,
    delivery: Mutex<DeliveryCursor>,
    accept_thread: Mutex<Option<thread::JoinHandle<()>>>,
    discovery_thread: Mutex<Option<thread::JoinHandle<()>>>,
}

impl NetworkTransport {
    pub fn start(
        config: AgentNetworkConfig,
        lan_enabled: bool,
        secure_enabled: bool,
        sink: EventSink,
    ) -> Result<Arc<Self>, String> {
        let bind_ip = if lan_enabled {
            Ipv4Addr::UNSPECIFIED
        } else {
            Ipv4Addr::LOCALHOST
        };
        let tcp_address = SocketAddr::from((bind_ip, config.tcp_port));
        let udp_address = lan_enabled.then_some(SocketAddr::from((
            Ipv4Addr::UNSPECIFIED,
            config.discovery_port,
        )));
        Self::start_with_addresses(config.token, secure_enabled, tcp_address, udp_address, sink)
    }

    fn start_with_addresses(
        token: String,
        secure_enabled: bool,
        tcp_address: SocketAddr,
        udp_address: Option<SocketAddr>,
        sink: EventSink,
    ) -> Result<Arc<Self>, String> {
        let listener = TcpListener::bind(tcp_address)
            .map_err(|error| format!("cannot bind agent TCP listener on {tcp_address}: {error}"))?;
        listener
            .set_nonblocking(true)
            .map_err(|error| error.to_string())?;
        let tcp_port = listener
            .local_addr()
            .map_err(|error| error.to_string())?
            .port();
        let discovery = match udp_address {
            Some(address) => {
                let socket = UdpSocket::bind(address).map_err(|error| {
                    format!("cannot bind agent discovery listener on {address}: {error}")
                })?;
                socket
                    .set_read_timeout(Some(Duration::from_millis(200)))
                    .map_err(|error| error.to_string())?;
                Some(socket)
            }
            None => None,
        };

        let transport = Arc::new(Self {
            pending: Mutex::new(HashMap::new()),
            running: Arc::new(AtomicBool::new(true)),
            sink,
            token,
            secure_enabled,
            server_id: uuid::Uuid::new_v4().to_string(),
            tcp_port,
            writer: Mutex::new(None),
            next_connection_id: AtomicU64::new(1),
            delivery: Mutex::new(DeliveryCursor::default()),
            accept_thread: Mutex::new(None),
            discovery_thread: Mutex::new(None),
        });

        let acceptor = Arc::clone(&transport);
        let accept_thread = thread::spawn(move || acceptor.accept_loop(listener));
        *transport.accept_thread.lock().unwrap() = Some(accept_thread);
        if let Some(socket) = discovery {
            let responder = Arc::clone(&transport);
            let discovery_thread = thread::spawn(move || responder.discovery_loop(socket));
            *transport.discovery_thread.lock().unwrap() = Some(discovery_thread);
        }
        Ok(transport)
    }

    pub fn stop(&self) {
        if !self.running.swap(false, Ordering::Relaxed) {
            return;
        }
        if let Ok(mut writer) = self.writer.lock() {
            if let Some(active) = writer.take() {
                let _ = active.stream.shutdown(Shutdown::Both);
            }
        }
        if let Ok(mut pending) = self.pending.lock() {
            pending.clear();
        }
        // Wake the nonblocking accept loop and wait until both bound sockets are
        // dropped, so an immediate reconnect can bind the same ports reliably.
        let _ = TcpStream::connect((Ipv4Addr::LOCALHOST, self.tcp_port));
        if let Ok(mut thread) = self.accept_thread.lock() {
            if let Some(thread) = thread.take() {
                let _ = thread.join();
            }
        }
        if let Ok(mut thread) = self.discovery_thread.lock() {
            if let Some(thread) = thread.take() {
                let _ = thread.join();
            }
        }
    }

    pub fn request(&self, frame: InFrame) -> Receiver<OutFrame> {
        let (tx, rx) = mpsc::channel();
        let id = frame.id();
        self.pending.lock().unwrap().insert(id.to_string(), tx);
        if self.send_json(&frame).is_err() {
            self.pending.lock().unwrap().remove(id);
        }
        rx
    }

    fn accept_loop(self: Arc<Self>, listener: TcpListener) {
        while self.running.load(Ordering::Relaxed) {
            match listener.accept() {
                Ok((stream, peer)) => {
                    if !self.running.load(Ordering::Relaxed) {
                        break;
                    }
                    let transport = Arc::clone(&self);
                    thread::spawn(move || {
                        transport.connection_loop(stream, !peer.ip().is_loopback())
                    });
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    thread::sleep(ACCEPT_POLL);
                }
                Err(error) => {
                    log::error!("agent TCP accept failed: {error}");
                    thread::sleep(ACCEPT_POLL);
                }
            }
        }
    }

    fn connection_loop(&self, mut stream: TcpStream, remote: bool) {
        let _ = stream.set_read_timeout(Some(READ_POLL));
        let mut received = Vec::new();
        let hello_deadline = Instant::now() + HELLO_TIMEOUT;
        let hello = loop {
            if !self.running.load(Ordering::Relaxed) || Instant::now() >= hello_deadline {
                return;
            }
            match read_available_frames(&mut stream, &mut received) {
                Ok((frames, closed)) => {
                    if let Some(line) = frames.into_iter().next() {
                        match serde_json::from_str::<AgentHello>(&line) {
                            Ok(hello) if self.valid_hello(&hello) => break hello,
                            _ => return,
                        }
                    }
                    if closed {
                        return;
                    }
                }
                Err(_) => return,
            }
        };

        let connection_id = self.next_connection_id.fetch_add(1, Ordering::Relaxed);
        let authenticated = self.authenticated_hello(&hello);
        let proof = proof(
            &self.token,
            &[
                "welcome",
                &hello.protocol.to_string(),
                &hello.agent_id,
                &hello.session,
                &hello.nonce,
                &self.server_id,
            ],
        );
        let welcome = Welcome {
            kind: "welcome",
            protocol: AGENT_PROTOCOL_VERSION,
            agent_id: &hello.agent_id,
            session: &hello.session,
            nonce: &hello.nonce,
            server_id: &self.server_id,
            secure: authenticated,
            proof,
        };
        let Ok(writer_stream) = stream.try_clone() else {
            return;
        };
        let Ok(mut writer) = self.writer.lock() else {
            return;
        };
        if writer.is_some() {
            let _ = stream.shutdown(Shutdown::Both);
            return;
        }
        if write_json_line(&mut stream, &welcome).is_err() {
            return;
        }
        *writer = Some(ActiveWriter {
            connection_id,
            stream: writer_stream,
        });
        drop(writer);

        {
            let Ok(mut delivery) = self.delivery.lock() else {
                return;
            };
            if delivery.session != hello.session {
                delivery.session.clone_from(&hello.session);
                delivery.seq = hello.acked_seq;
            }
        }
        (self.sink)(ServerEvent::Hello {
            version: hello.version,
            pid: hello.pid,
            remote,
        });

        let mut last_seen = Instant::now();
        let mut last_ping = Instant::now();
        let mut terminate = false;
        let mut disconnect_notified = false;
        while self.running.load(Ordering::Relaxed) {
            if !self.is_active_connection(connection_id) {
                break;
            }
            match read_available_frames(&mut stream, &mut received) {
                Ok((frames, closed)) => {
                    if closed {
                        break;
                    }
                    for line in frames {
                        last_seen = Instant::now();
                        let kind = serde_json::from_str::<serde_json::Value>(&line)
                            .ok()
                            .and_then(|value| value.get("type")?.as_str().map(str::to_owned));
                        if kind.as_deref() == Some("pong") {
                            continue;
                        }
                        let Ok(wire) = serde_json::from_str::<WireOutFrame>(&line) else {
                            terminate = true;
                            break;
                        };
                        if wire.session != hello.session {
                            terminate = true;
                            break;
                        }
                        let disconnected = matches!(&wire.frame, OutFrame::Disconnected);
                        if self.deliver(wire).is_err() {
                            terminate = true;
                            break;
                        }
                        if disconnected {
                            disconnect_notified = true;
                            terminate = true;
                            break;
                        }
                    }
                    if terminate {
                        break;
                    }
                }
                Err(error) => {
                    log::debug!("agent connection read failed: {error}");
                    break;
                }
            }
            if last_seen.elapsed() >= HEARTBEAT_TIMEOUT {
                break;
            }
            if last_ping.elapsed() >= HEARTBEAT_INTERVAL {
                if self
                    .send_json_for(connection_id, &Ping { kind: "ping" })
                    .is_err()
                {
                    break;
                }
                last_ping = Instant::now();
            }
        }

        let active = if let Ok(mut writer) = self.writer.lock() {
            if writer
                .as_ref()
                .is_some_and(|active| active.connection_id == connection_id)
            {
                writer.take();
                true
            } else {
                false
            }
        } else {
            false
        };
        let _ = stream.shutdown(Shutdown::Both);
        if active && !disconnect_notified {
            (self.sink)(ServerEvent::Disconnected);
        }
    }

    fn is_active_connection(&self, connection_id: u64) -> bool {
        self.writer
            .lock()
            .ok()
            .and_then(|writer| {
                writer
                    .as_ref()
                    .map(|active| active.connection_id == connection_id)
            })
            .unwrap_or(false)
    }

    fn valid_hello(&self, hello: &AgentHello) -> bool {
        hello.kind == "hello"
            && hello.protocol == AGENT_PROTOCOL_VERSION
            && !hello.agent_id.is_empty()
            && !hello.session.is_empty()
            && (!self.secure_enabled || self.authenticated_hello(hello))
    }

    fn authenticated_hello(&self, hello: &AgentHello) -> bool {
        verify_proof(
            &self.token,
            &[
                "hello",
                &hello.protocol.to_string(),
                &hello.agent_id,
                &hello.session,
                &hello.nonce,
            ],
            &hello.proof,
        )
    }

    fn deliver(&self, wire: WireOutFrame) -> Result<(), String> {
        let (duplicate, gap) = {
            let mut delivery = self
                .delivery
                .lock()
                .map_err(|_| "delivery cursor lock poisoned".to_string())?;
            if delivery.session != wire.session {
                delivery.session.clone_from(&wire.session);
                delivery.seq = 0;
            }
            if wire.seq <= delivery.seq {
                (true, None)
            } else {
                let gap = (wire.seq > delivery.seq + 1)
                    .then_some((delivery.seq.saturating_add(1), wire.seq - 1));
                delivery.seq = wire.seq;
                (false, gap)
            }
        };
        if !duplicate {
            if let Some((from, to)) = gap {
                (self.sink)(ServerEvent::Log {
                    lines: vec![LogLine {
                        stream: "system".to_string(),
                        level: Some("WARN".to_string()),
                        text: format!("agent dropped buffered frames {from}..{to}\n"),
                    }],
                });
            }
            match wire.frame {
                OutFrame::Disconnected => (self.sink)(ServerEvent::Disconnected),
                OutFrame::Stdout {
                    stream,
                    level,
                    text,
                } => (self.sink)(ServerEvent::Log {
                    lines: vec![LogLine {
                        stream,
                        level,
                        text,
                    }],
                }),
                frame => {
                    if let Some(id) = frame.correlation_id() {
                        let waiter = self.pending.lock().unwrap().remove(id);
                        if let Some(tx) = waiter {
                            let _ = tx.send(frame);
                        }
                    }
                }
            }
        }
        self.send_json(&Ack {
            kind: "ack",
            session: &wire.session,
            seq: wire.seq,
        })
    }

    fn discovery_loop(&self, socket: UdpSocket) {
        let mut buffer = [0_u8; 4096];
        while self.running.load(Ordering::Relaxed) {
            match socket.recv_from(&mut buffer) {
                Ok((size, peer)) => {
                    let Ok(request) = serde_json::from_slice::<DiscoveryRequest>(&buffer[..size])
                    else {
                        continue;
                    };
                    if request.kind != "discover"
                        || request.protocol != AGENT_PROTOCOL_VERSION
                        || request.agent_id.is_empty()
                        || request.nonce.is_empty()
                    {
                        continue;
                    }
                    let authenticated = verify_proof(
                        &self.token,
                        &["discover", &request.agent_id, &request.nonce],
                        &request.proof,
                    );
                    if self.secure_enabled && !authenticated {
                        continue;
                    }
                    let port = self.tcp_port.to_string();
                    let offer = DiscoveryOffer {
                        kind: "offer",
                        protocol: AGENT_PROTOCOL_VERSION,
                        agent_id: &request.agent_id,
                        nonce: &request.nonce,
                        tcp_port: self.tcp_port,
                        server_id: &self.server_id,
                        secure: authenticated,
                        proof: proof(
                            &self.token,
                            &[
                                "offer",
                                &request.agent_id,
                                &request.nonce,
                                &port,
                                &self.server_id,
                            ],
                        ),
                    };
                    if let Ok(body) = serde_json::to_vec(&offer) {
                        let _ = socket.send_to(&body, peer);
                    }
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) => {}
                Err(error) => log::debug!("agent discovery receive failed: {error}"),
            }
        }
    }

    fn send_json<T: Serialize>(&self, frame: &T) -> Result<(), String> {
        let mut writer = self
            .writer
            .lock()
            .map_err(|_| "agent writer lock poisoned".to_string())?;
        let active = writer
            .as_mut()
            .ok_or_else(|| "agent is not connected".to_string())?;
        write_json_line(&mut active.stream, frame).map_err(|error| error.to_string())
    }

    fn send_json_for<T: Serialize>(&self, connection_id: u64, frame: &T) -> Result<(), String> {
        let mut writer = self
            .writer
            .lock()
            .map_err(|_| "agent writer lock poisoned".to_string())?;
        let active = writer
            .as_mut()
            .filter(|active| active.connection_id == connection_id)
            .ok_or_else(|| "agent connection was replaced".to_string())?;
        write_json_line(&mut active.stream, frame).map_err(|error| error.to_string())
    }
}

fn write_json_line<T: Serialize>(stream: &mut TcpStream, value: &T) -> io::Result<()> {
    let mut body = serde_json::to_vec(value).map_err(io::Error::other)?;
    if body.len() > MAX_FRAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "agent frame exceeds size limit",
        ));
    }
    body.push(b'\n');
    stream.write_all(&body)
}

fn read_available_frames(
    stream: &mut TcpStream,
    received: &mut Vec<u8>,
) -> io::Result<(Vec<String>, bool)> {
    let mut chunk = [0_u8; 16 * 1024];
    let mut closed = false;
    loop {
        match stream.read(&mut chunk) {
            Ok(0) => {
                closed = true;
                break;
            }
            Ok(size) => {
                received.extend_from_slice(&chunk[..size]);
                if received.len() > MAX_FRAME_BYTES && !received.contains(&b'\n') {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "agent frame exceeds size limit",
                    ));
                }
                if size < chunk.len() {
                    break;
                }
            }
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) =>
            {
                break
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }

    let mut frames = Vec::new();
    while let Some(newline) = received.iter().position(|byte| *byte == b'\n') {
        if newline > MAX_FRAME_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "agent frame exceeds size limit",
            ));
        }
        let line: Vec<u8> = received.drain(..=newline).collect();
        let text = std::str::from_utf8(&line[..line.len() - 1])
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?
            .trim();
        if !text.is_empty() {
            frames.push(text.to_string());
        }
    }
    Ok((frames, closed))
}

fn proof(token: &str, parts: &[&str]) -> String {
    let mut mac = HmacSha256::new_from_slice(token.as_bytes()).expect("HMAC accepts any key size");
    mac.update(parts.join("|").as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

fn verify_proof(token: &str, parts: &[&str], candidate: &str) -> bool {
    let Ok(candidate) = hex::decode(candidate) else {
        return false;
    };
    let mut mac = HmacSha256::new_from_slice(token.as_bytes()).expect("HMAC accepts any key size");
    mac.update(parts.join("|").as_bytes());
    mac.verify_slice(&candidate).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::InFrame;
    use std::process::Command;
    use std::sync::mpsc::RecvTimeoutError;

    const TOKEN: &str = "00000000-0000-4000-8000-000000000001";

    fn start_transport(events: Arc<Mutex<Vec<ServerEvent>>>) -> Arc<NetworkTransport> {
        start_transport_with_security(events, true)
    }

    fn start_transport_with_security(
        events: Arc<Mutex<Vec<ServerEvent>>>,
        secure_enabled: bool,
    ) -> Arc<NetworkTransport> {
        NetworkTransport::start_with_addresses(
            TOKEN.to_string(),
            secure_enabled,
            "127.0.0.1:0".parse().unwrap(),
            None,
            Arc::new(move |event| events.lock().unwrap().push(event)),
        )
        .unwrap()
    }

    fn open_agent(transport: &NetworkTransport, session: &str, token: &str) -> TcpStream {
        let address = SocketAddr::from((Ipv4Addr::LOCALHOST, transport.tcp_port));
        let mut stream = TcpStream::connect(address).unwrap();
        let agent_id = "agent-test";
        let nonce = "nonce-test";
        let hello = serde_json::json!({
            "type": "hello",
            "protocol": AGENT_PROTOCOL_VERSION,
            "agent_id": agent_id,
            "session": session,
            "nonce": nonce,
            "version": "test",
            "pid": 42,
            "proof": proof(token, &["hello", "1", agent_id, session, nonce]),
        });
        write_json_line(&mut stream, &hello).unwrap();
        stream
    }

    fn connect_agent(transport: &NetworkTransport, session: &str, token: &str) -> TcpStream {
        let mut stream = open_agent(transport, session, token);
        stream
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        let welcome = read_line(&mut stream);
        assert_eq!(welcome["type"], "welcome");
        stream
    }

    fn read_line(stream: &mut TcpStream) -> serde_json::Value {
        let mut body = Vec::new();
        let mut byte = [0_u8; 1];
        loop {
            stream.read_exact(&mut byte).unwrap();
            if byte[0] == b'\n' {
                break;
            }
            body.push(byte[0]);
        }
        serde_json::from_slice(&body).unwrap()
    }

    fn python_interpreter() -> Option<String> {
        for candidate in [
            "C:/Python27/python.exe",
            "/opt/homebrew/bin/python2.7",
            "/usr/local/bin/python2.7",
        ] {
            if Path::new(candidate).is_file() {
                return Some(candidate.to_string());
            }
        }
        Command::new("python3")
            .arg("--version")
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map(|_| "python3".to_string())
    }

    #[test]
    fn authenticated_agent_round_trips_and_deduplicates_replay() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let transport = start_transport(Arc::clone(&events));
        let mut agent = connect_agent(&transport, "session-a", TOKEN);

        let deadline = Instant::now() + Duration::from_secs(1);
        while Instant::now() < deadline
            && !events
                .lock()
                .unwrap()
                .iter()
                .any(|event| matches!(event, ServerEvent::Hello { .. }))
        {
            thread::sleep(Duration::from_millis(5));
        }
        assert!(events
            .lock()
            .unwrap()
            .iter()
            .any(|event| matches!(event, ServerEvent::Hello { remote: false, .. })));

        let rx = transport.request(InFrame::Exec {
            id: "r1".to_string(),
            code: "21 * 2".to_string(),
        });
        let request = read_line(&mut agent);
        assert_eq!(request["type"], "exec");
        write_json_line(
            &mut agent,
            &serde_json::json!({
                "session": "session-a",
                "seq": 1,
                "type": "result",
                "id": "r1",
                "ok": true,
                "repr": "42"
            }),
        )
        .unwrap();
        let reply = rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(matches!(reply, OutFrame::Result { repr: Some(value), .. } if value == "42"));
        let ack = read_line(&mut agent);
        assert_eq!(ack["seq"], 1);

        write_json_line(
            &mut agent,
            &serde_json::json!({
                "session": "session-a",
                "seq": 1,
                "type": "stdout",
                "stream": "stdout",
                "text": "duplicate"
            }),
        )
        .unwrap();
        let duplicate_ack = read_line(&mut agent);
        assert_eq!(duplicate_ack["seq"], 1);
        assert!(!events.lock().unwrap().iter().any(|event| matches!(
            event,
            ServerEvent::Log { lines }
                if lines.iter().any(|line| line.text == "duplicate")
        )));
        transport.stop();
    }

    #[test]
    fn first_agent_remains_active_while_later_agents_are_ignored() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let transport = start_transport(Arc::clone(&events));
        let mut first = connect_agent(&transport, "first-session", TOKEN);
        let mut second = open_agent(&transport, "second-session", TOKEN);
        second
            .set_read_timeout(Some(Duration::from_millis(300)))
            .unwrap();
        let mut byte = [0_u8; 1];
        let second_read = second.read(&mut byte);
        let second_was_rejected = second_read.is_err() || second_read.unwrap_or(0) == 0;

        let _rx = transport.request(InFrame::Exec {
            id: "first-wins".to_string(),
            code: "21 * 2".to_string(),
        });
        first
            .set_read_timeout(Some(Duration::from_millis(300)))
            .unwrap();
        let mut received = Vec::new();
        let first_received_request = read_available_frames(&mut first, &mut received)
            .map(|(frames, _closed)| frames.iter().any(|line| line.contains("first-wins")))
            .unwrap_or(false);
        let hello_count = events
            .lock()
            .unwrap()
            .iter()
            .filter(|event| matches!(event, ServerEvent::Hello { .. }))
            .count();
        drop(first);
        let release_deadline = Instant::now() + Duration::from_secs(1);
        while Instant::now() < release_deadline && transport.writer.lock().unwrap().is_some() {
            thread::sleep(Duration::from_millis(5));
        }
        assert!(transport.writer.lock().unwrap().is_none());
        let _next = connect_agent(&transport, "second-retry", TOKEN);
        let takeover_hello_count = events
            .lock()
            .unwrap()
            .iter()
            .filter(|event| matches!(event, ServerEvent::Hello { .. }))
            .count();
        transport.stop();

        assert!(second_was_rejected, "second agent received a welcome");
        assert!(
            first_received_request,
            "request was not routed to first agent"
        );
        assert_eq!(hello_count, 1);
        assert_eq!(takeover_hello_count, 2);
    }

    #[test]
    fn invalid_token_is_rejected() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let transport = start_transport(Arc::clone(&events));
        let address = SocketAddr::from((Ipv4Addr::LOCALHOST, transport.tcp_port));
        let mut stream = TcpStream::connect(address).unwrap();
        let hello = serde_json::json!({
            "type": "hello",
            "protocol": 1,
            "agent_id": "bad",
            "session": "bad-session",
            "nonce": "bad-nonce",
            "proof": proof("wrong-token", &["hello", "1", "bad", "bad-session", "bad-nonce"]),
        });
        write_json_line(&mut stream, &hello).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_millis(300)))
            .unwrap();
        let mut byte = [0_u8; 1];
        let read = stream.read(&mut byte);
        assert!(read.is_err() || read.unwrap_or(0) == 0);
        assert!(events.lock().unwrap().is_empty());
        transport.stop();
    }

    #[test]
    fn anonymous_agent_is_accepted_only_when_token_is_not_required() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let transport = start_transport_with_security(Arc::clone(&events), false);
        let address = SocketAddr::from((Ipv4Addr::LOCALHOST, transport.tcp_port));
        let mut stream = TcpStream::connect(address).unwrap();
        write_json_line(
            &mut stream,
            &serde_json::json!({
                "type": "hello",
                "protocol": AGENT_PROTOCOL_VERSION,
                "agent_id": "anonymous-agent",
                "session": "anonymous-session",
                "nonce": "anonymous-nonce",
                "version": "test",
                "pid": 42
            }),
        )
        .unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        let welcome = read_line(&mut stream);
        assert_eq!(welcome["type"], "welcome");
        assert_eq!(welcome["secure"], false);

        let deadline = Instant::now() + Duration::from_secs(1);
        while Instant::now() < deadline
            && !events
                .lock()
                .unwrap()
                .iter()
                .any(|event| matches!(event, ServerEvent::Hello { .. }))
        {
            thread::sleep(Duration::from_millis(5));
        }
        assert!(events
            .lock()
            .unwrap()
            .iter()
            .any(|event| matches!(event, ServerEvent::Hello { .. })));
        transport.stop();

        let secure_events = Arc::new(Mutex::new(Vec::new()));
        let secure_transport = start_transport_with_security(Arc::clone(&secure_events), true);
        let secure_address = SocketAddr::from((Ipv4Addr::LOCALHOST, secure_transport.tcp_port));
        let mut rejected = TcpStream::connect(secure_address).unwrap();
        write_json_line(
            &mut rejected,
            &serde_json::json!({
                "type": "hello",
                "protocol": AGENT_PROTOCOL_VERSION,
                "agent_id": "anonymous-agent",
                "session": "anonymous-session",
                "nonce": "anonymous-nonce"
            }),
        )
        .unwrap();
        rejected
            .set_read_timeout(Some(Duration::from_millis(300)))
            .unwrap();
        let mut byte = [0_u8; 1];
        let read = rejected.read(&mut byte);
        assert!(read.is_err() || read.unwrap_or(0) == 0);
        assert!(secure_events.lock().unwrap().is_empty());
        secure_transport.stop();
    }

    #[test]
    fn request_without_agent_fails_immediately() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let transport = start_transport(events);
        let rx = transport.request(InFrame::Inspect {
            id: "no-agent".to_string(),
            expr: "x".to_string(),
        });
        assert!(matches!(
            rx.recv_timeout(Duration::from_millis(100)),
            Err(RecvTimeoutError::Disconnected)
        ));
        transport.stop();
    }

    #[test]
    fn authenticated_udp_discovery_returns_the_tcp_endpoint() {
        let probe = UdpSocket::bind("127.0.0.1:0").unwrap();
        let discovery_address = probe.local_addr().unwrap();
        drop(probe);
        let events = Arc::new(Mutex::new(Vec::new()));
        let transport = NetworkTransport::start_with_addresses(
            TOKEN.to_string(),
            true,
            "127.0.0.1:0".parse().unwrap(),
            Some(discovery_address),
            Arc::new(move |event| events.lock().unwrap().push(event)),
        )
        .unwrap();
        let client = UdpSocket::bind("127.0.0.1:0").unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        let agent_id = "discovering-agent";
        let nonce = "discover-nonce";
        let request = serde_json::json!({
            "type": "discover",
            "protocol": AGENT_PROTOCOL_VERSION,
            "agent_id": agent_id,
            "nonce": nonce,
            "proof": proof(TOKEN, &["discover", agent_id, nonce]),
        });
        client
            .send_to(&serde_json::to_vec(&request).unwrap(), discovery_address)
            .unwrap();
        let mut buffer = [0_u8; 4096];
        let (size, _) = client.recv_from(&mut buffer).unwrap();
        let offer: serde_json::Value = serde_json::from_slice(&buffer[..size]).unwrap();
        assert_eq!(offer["type"], "offer");
        assert_eq!(offer["tcp_port"], transport.tcp_port);
        assert_eq!(offer["agent_id"], agent_id);
        assert_eq!(offer["secure"], true);
        transport.stop();
    }

    #[test]
    fn anonymous_udp_discovery_is_answered_only_when_token_is_not_required() {
        let probe = UdpSocket::bind("127.0.0.1:0").unwrap();
        let discovery_address = probe.local_addr().unwrap();
        drop(probe);
        let events = Arc::new(Mutex::new(Vec::new()));
        let transport = NetworkTransport::start_with_addresses(
            TOKEN.to_string(),
            false,
            "127.0.0.1:0".parse().unwrap(),
            Some(discovery_address),
            Arc::new(move |event| events.lock().unwrap().push(event)),
        )
        .unwrap();
        let client = UdpSocket::bind("127.0.0.1:0").unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        let request = serde_json::json!({
            "type": "discover",
            "protocol": AGENT_PROTOCOL_VERSION,
            "agent_id": "anonymous-agent",
            "nonce": "anonymous-nonce"
        });
        client
            .send_to(&serde_json::to_vec(&request).unwrap(), discovery_address)
            .unwrap();
        let mut buffer = [0_u8; 4096];
        let (size, _) = client.recv_from(&mut buffer).unwrap();
        let offer: serde_json::Value = serde_json::from_slice(&buffer[..size]).unwrap();
        assert_eq!(offer["type"], "offer");
        assert_eq!(offer["secure"], false);
        assert_eq!(offer["tcp_port"], transport.tcp_port);
        transport.stop();
    }

    #[test]
    fn stop_releases_the_listener_for_immediate_reconnect() {
        let probe = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = probe.local_addr().unwrap();
        drop(probe);
        let sink: EventSink = Arc::new(|_| {});
        let first = NetworkTransport::start_with_addresses(
            TOKEN.to_string(),
            true,
            address,
            None,
            Arc::clone(&sink),
        )
        .unwrap();
        first.stop();
        let second =
            NetworkTransport::start_with_addresses(TOKEN.to_string(), true, address, None, sink)
                .unwrap();
        second.stop();
    }

    #[test]
    fn round_trips_with_the_real_python_agent() {
        let Some(python) = python_interpreter() else {
            eprintln!("skip: Python 2.7/3 not found");
            return;
        };
        let events = Arc::new(Mutex::new(Vec::new()));
        let transport = start_transport(Arc::clone(&events));
        let config_dir =
            std::env::temp_dir().join(format!("wms_network_agent_{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&config_dir).unwrap();
        fs::write(
            config_dir.join("agent-network.json"),
            serde_json::to_vec(&AgentNetworkConfig {
                token: TOKEN.to_string(),
                host: "127.0.0.1".to_string(),
                tcp_port: transport.tcp_port,
                discovery_port: AGENT_DISCOVERY_PORT,
            })
            .unwrap(),
        )
        .unwrap();
        let runner = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../mod/tests/run_standalone.py"
        );
        let mut child = Command::new(python)
            .arg(runner)
            .arg(&config_dir)
            .spawn()
            .expect("spawn real Python agent");

        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline
            && !events
                .lock()
                .unwrap()
                .iter()
                .any(|event| matches!(event, ServerEvent::Hello { .. }))
        {
            thread::sleep(Duration::from_millis(10));
        }
        assert!(events
            .lock()
            .unwrap()
            .iter()
            .any(|event| matches!(event, ServerEvent::Hello { .. })));

        let reply = transport
            .request(InFrame::Exec {
                id: "real-agent".to_string(),
                code: "21 * 2".to_string(),
            })
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        assert!(matches!(
            reply,
            OutFrame::Result {
                ok: true,
                repr: Some(value),
                ..
            } if value == "42"
        ));

        let _ = child.kill();
        let _ = child.wait();
        transport.stop();
        let _ = fs::remove_dir_all(config_dir);
    }

    #[test]
    fn round_trips_with_the_real_python_agent_without_a_token() {
        let Some(python) = python_interpreter() else {
            eprintln!("skip: Python 2.7/3 not found");
            return;
        };
        let events = Arc::new(Mutex::new(Vec::new()));
        let transport = start_transport_with_security(Arc::clone(&events), false);
        let config_dir =
            std::env::temp_dir().join(format!("wms_anonymous_agent_{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&config_dir).unwrap();
        fs::write(
            config_dir.join("agent-network.json"),
            serde_json::to_vec(&serde_json::json!({
                "host": "127.0.0.1",
                "tcp_port": transport.tcp_port,
                "discovery_port": AGENT_DISCOVERY_PORT
            }))
            .unwrap(),
        )
        .unwrap();
        let runner = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../mod/tests/run_standalone.py"
        );
        let mut child = Command::new(python)
            .arg(runner)
            .arg(&config_dir)
            .spawn()
            .expect("spawn anonymous Python agent");

        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline
            && !events
                .lock()
                .unwrap()
                .iter()
                .any(|event| matches!(event, ServerEvent::Hello { .. }))
        {
            thread::sleep(Duration::from_millis(10));
        }
        assert!(events
            .lock()
            .unwrap()
            .iter()
            .any(|event| matches!(event, ServerEvent::Hello { .. })));

        let reply = transport
            .request(InFrame::Exec {
                id: "anonymous-real-agent".to_string(),
                code: "6 * 7".to_string(),
            })
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        assert!(matches!(
            reply,
            OutFrame::Result {
                ok: true,
                repr: Some(value),
                ..
            } if value == "42"
        ));

        let _ = child.kill();
        let _ = child.wait();
        transport.stop();
        let _ = fs::remove_dir_all(config_dir);
    }
}

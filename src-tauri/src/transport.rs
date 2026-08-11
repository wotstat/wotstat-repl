//! Desktop end of the file-buffer transport.
//!
//! Mirrors the agent's `framebus.py`: append newline-JSON to `d2c`, drain `c2d`,
//! both guarded by exclusive-create `*.lock` files. A background thread polls
//! `c2d`, forwards stdout/hello frames to an event sink, and correlates replies
//! by id.
//!
//! The event sink is a plain closure, so the transport has no Tauri dependency
//! (DIP) and can be exercised in isolation (see the test at the bottom).

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use crate::process::is_process_alive;
use crate::protocol::{InFrame, LogLine, OutFrame, ServerEvent};

const LOCK_STALE: Duration = Duration::from_secs(5);
const POLL_INTERVAL: Duration = Duration::from_millis(50);

pub type EventSink = Arc<dyn Fn(ServerEvent) + Send + Sync>;

pub struct FileBufferTransport {
    dir: PathBuf,
    pending: Mutex<std::collections::HashMap<String, Sender<OutFrame>>>,
    running: Arc<AtomicBool>,
    sink: EventSink,
}

impl FileBufferTransport {
    pub fn start(dir: PathBuf, sink: EventSink) -> Arc<Self> {
        let transport = Arc::new(Self {
            dir,
            pending: Mutex::new(std::collections::HashMap::new()),
            running: Arc::new(AtomicBool::new(true)),
            sink,
        });
        transport.reset_desktop_buffer();
        transport.send(InFrame::Hello);
        let worker = Arc::clone(&transport);
        thread::spawn(move || worker.read_loop());
        transport
    }

    pub fn stop(&self) {
        self.running.store(false, Ordering::Relaxed);
    }

    /// Send a request and get a receiver that resolves with the matching reply.
    pub fn request(&self, frame: InFrame) -> Receiver<OutFrame> {
        let (tx, rx) = mpsc::channel();
        let id = frame.id().expect("control frames cannot be requested");
        self.pending.lock().unwrap().insert(id.to_string(), tx);
        self.send(frame);
        rx
    }

    fn send(&self, frame: InFrame) {
        if let Ok(line) = serde_json::to_string(&frame) {
            self.append("d2c", &line);
        }
    }

    /// Drop requests left by a previous desktop session. Keep `c2d`: a live
    /// agent may have already written its hello and startup logs there.
    fn reset_desktop_buffer(&self) {
        let path = self.dir.join("d2c");
        if !path.exists() {
            return;
        }
        let lock = self.dir.join("d2c.lock");
        if acquire(&lock) {
            let _ = fs::write(path, b"");
            release(&lock);
        }
    }

    fn append(&self, name: &str, line: &str) {
        let path = self.dir.join(name);
        let lock = self.dir.join(format!("{name}.lock"));
        if !acquire(&lock) {
            return;
        }
        if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&path) {
            let _ = file.write_all(line.as_bytes());
            let _ = file.write_all(b"\n");
        }
        release(&lock);
    }

    fn read_loop(&self) {
        let path = self.dir.join("c2d");
        let lock = self.dir.join("c2d.lock");
        let mut handshake_seen = false;
        let mut game_pid = None;
        while self.running.load(Ordering::Relaxed) {
            let mut batch = Vec::new();
            let mut disconnected = false;
            for line in drain_file(&path, &lock) {
                match serde_json::from_str::<OutFrame>(&line) {
                    Ok(OutFrame::Disconnected) => {
                        if handshake_seen {
                            disconnected = true;
                        }
                    }
                    Ok(OutFrame::Stdout {
                        stream,
                        level,
                        text,
                    }) => batch.push(LogLine {
                        stream,
                        level,
                        text,
                    }),
                    Ok(OutFrame::Hello { version, pid }) => {
                        handshake_seen = true;
                        game_pid = pid;
                        (self.sink)(ServerEvent::Hello { version, pid });
                    }
                    Ok(frame) => {
                        if let Some(id) = frame.correlation_id() {
                            let waiter = self.pending.lock().unwrap().remove(id);
                            if let Some(tx) = waiter {
                                let _ = tx.send(frame);
                            }
                        }
                    }
                    Err(_) => {}
                }
            }
            if !batch.is_empty() {
                (self.sink)(ServerEvent::Log { lines: batch });
            }
            if disconnected {
                (self.sink)(ServerEvent::Disconnected);
                self.running.store(false, Ordering::Relaxed);
                break;
            }
            if game_pid.map(|pid| !is_process_alive(pid)).unwrap_or(false) {
                (self.sink)(ServerEvent::Disconnected);
                self.running.store(false, Ordering::Relaxed);
                break;
            }
            thread::sleep(POLL_INTERVAL);
        }
    }
}

fn drain_file(path: &Path, lock: &Path) -> Vec<String> {
    if !path.exists() {
        return Vec::new();
    }
    if !acquire(lock) {
        return Vec::new();
    }
    let data = fs::read_to_string(path).unwrap_or_default();
    let _ = fs::write(path, b"");
    release(lock);
    data.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.to_string())
        .collect()
}

fn acquire(lock: &Path) -> bool {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        match OpenOptions::new().write(true).create_new(true).open(lock) {
            Ok(_) => return true,
            Err(_) => {
                if is_stale(lock) {
                    let _ = fs::remove_file(lock);
                    continue;
                }
                if Instant::now() >= deadline {
                    return false;
                }
                thread::sleep(Duration::from_millis(2));
            }
        }
    }
}

fn release(lock: &Path) {
    let _ = fs::remove_file(lock);
}

fn is_stale(lock: &Path) -> bool {
    fs::metadata(lock)
        .and_then(|m| m.modified())
        .map(|t| SystemTime::now().duration_since(t).unwrap_or_default() > LOCK_STALE)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::InFrame;
    use std::process::Command;

    fn python27() -> Option<&'static str> {
        let p = "C:/Python27/python.exe";
        if Path::new(p).exists() {
            Some(p)
        } else {
            None
        }
    }

    // Drives the REAL py2.7 agent over the file-buffer transport: Rust writes
    // d2c, the agent execs and writes c2d, Rust correlates the reply by id.
    #[test]
    fn loopback_exec_with_real_agent() {
        let py = match python27() {
            Some(p) => p,
            None => {
                eprintln!("skip: C:/Python27 not present");
                return;
            }
        };
        let runner = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../mod/tests/run_standalone.py"
        );
        let dir = std::env::temp_dir().join(format!("wms_rust_it_{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);

        let events: Arc<Mutex<Vec<ServerEvent>>> = Arc::new(Mutex::new(Vec::new()));
        let collected = Arc::clone(&events);
        let transport = FileBufferTransport::start(
            dir.clone(),
            Arc::new(move |ev| collected.lock().unwrap().push(ev)),
        );

        let mut child = Command::new(py)
            .arg(runner)
            .arg(dir.to_str().unwrap())
            .spawn()
            .expect("spawn agent");
        thread::sleep(Duration::from_millis(900));

        let rx = transport.request(InFrame::Exec {
            id: "t1".into(),
            code: "(__import__('sys').stdout.write('out'), __import__('sys').stderr.write('err'), 21 * 2)[-1]".into(),
        });
        let frame = rx.recv_timeout(Duration::from_secs(10));

        let got_hello = events
            .lock()
            .unwrap()
            .iter()
            .any(|e| matches!(e, ServerEvent::Hello { .. }));

        let _ = child.kill();
        transport.stop();
        let _ = fs::remove_dir_all(&dir);

        assert!(got_hello, "agent should send a hello handshake on start");
        match frame {
            Ok(OutFrame::Result {
                repr,
                ok,
                stdout,
                stderr,
                ..
            }) => {
                assert!(ok, "exec should succeed");
                assert_eq!(repr.as_deref(), Some("42"));
                assert_eq!(stdout, "out");
                assert_eq!(stderr, "err");
            }
            other => panic!("unexpected reply: {other:?}"),
        }
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn attaches_to_agent_that_sent_hello_before_transport_started() {
        let dir =
            std::env::temp_dir().join(format!("wms_rust_preconnected_{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        fs::write(
            dir.join("c2d"),
            format!(
                "{{\"type\":\"hello\",\"version\":\"test\",\"pid\":{}}}\n",
                std::process::id()
            ),
        )
        .unwrap();

        let events: Arc<Mutex<Vec<ServerEvent>>> = Arc::new(Mutex::new(Vec::new()));
        let collected = Arc::clone(&events);
        let transport = FileBufferTransport::start(
            dir.clone(),
            Arc::new(move |event| collected.lock().unwrap().push(event)),
        );

        let deadline = Instant::now() + Duration::from_secs(1);
        while Instant::now() < deadline
            && !events
                .lock()
                .unwrap()
                .iter()
                .any(|event| matches!(event, ServerEvent::Hello { .. }))
        {
            thread::sleep(Duration::from_millis(10));
        }

        let desktop_handshake = fs::read_to_string(dir.join("d2c")).unwrap();
        transport.stop();
        let _ = fs::remove_dir_all(&dir);

        assert!(desktop_handshake.contains("{\"type\":\"hello\"}"));
        assert!(events.lock().unwrap().iter().any(|event| matches!(
            event,
            ServerEvent::Hello {
                version: Some(version),
                pid: Some(pid),
            } if version == "test" && *pid == std::process::id() as i64
        )));
    }

    #[test]
    fn explicit_disconnect_emits_event() {
        let dir = std::env::temp_dir().join(format!("wms_rust_disconnect_{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);

        let events: Arc<Mutex<Vec<ServerEvent>>> = Arc::new(Mutex::new(Vec::new()));
        let collected = Arc::clone(&events);
        let transport = FileBufferTransport::start(
            dir.clone(),
            Arc::new(move |ev| collected.lock().unwrap().push(ev)),
        );
        transport.append("c2d", "{\"type\":\"hello\"}\n{\"type\":\"disconnected\"}");

        let deadline = Instant::now() + Duration::from_secs(1);
        while Instant::now() < deadline
            && !events
                .lock()
                .unwrap()
                .iter()
                .any(|event| matches!(event, ServerEvent::Disconnected))
        {
            thread::sleep(Duration::from_millis(10));
        }

        transport.stop();
        let _ = fs::remove_dir_all(&dir);

        assert!(events
            .lock()
            .unwrap()
            .iter()
            .any(|event| matches!(event, ServerEvent::Disconnected)));
    }

    #[test]
    fn start_ignores_disconnect_without_handshake() {
        let dir =
            std::env::temp_dir().join(format!("wms_rust_stale_session_{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        fs::write(dir.join("c2d"), b"{\"type\":\"disconnected\"}\n").unwrap();

        let events: Arc<Mutex<Vec<ServerEvent>>> = Arc::new(Mutex::new(Vec::new()));
        let collected = Arc::clone(&events);
        let transport = FileBufferTransport::start(
            dir.clone(),
            Arc::new(move |ev| collected.lock().unwrap().push(ev)),
        );

        thread::sleep(Duration::from_millis(100));
        transport.stop();
        let _ = fs::remove_dir_all(&dir);

        assert!(!events
            .lock()
            .unwrap()
            .iter()
            .any(|event| matches!(event, ServerEvent::Disconnected)));
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn dead_agent_pid_emits_disconnected() {
        let dir = std::env::temp_dir().join(format!("wms_rust_dead_pid_{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);

        let events: Arc<Mutex<Vec<ServerEvent>>> = Arc::new(Mutex::new(Vec::new()));
        let collected = Arc::clone(&events);
        let transport = FileBufferTransport::start(
            dir.clone(),
            Arc::new(move |ev| collected.lock().unwrap().push(ev)),
        );
        transport.append(
            "c2d",
            &format!("{{\"type\":\"hello\",\"pid\":{}}}", i64::MAX),
        );

        let deadline = Instant::now() + Duration::from_secs(1);
        while Instant::now() < deadline
            && !events
                .lock()
                .unwrap()
                .iter()
                .any(|event| matches!(event, ServerEvent::Disconnected))
        {
            thread::sleep(Duration::from_millis(10));
        }

        transport.stop();
        let _ = fs::remove_dir_all(&dir);

        assert!(events
            .lock()
            .unwrap()
            .iter()
            .any(|event| matches!(event, ServerEvent::Disconnected)));
    }
}

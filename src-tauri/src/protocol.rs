//! Wire frames shared with the in-game agent and the frontend over a Tauri
//! channel.

use serde::{Deserialize, Serialize};

/// desktop -> game. Serialized as `{ "type": "...", "id": "...", ... }`.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InFrame {
    Ready {
        id: String,
    },
    Exec {
        id: String,
        code: String,
    },
    Complete {
        id: String,
        prefix: String,
        budget: u32,
    },
    Inspect {
        id: String,
        expr: String,
    },
    Lint {
        id: String,
        code: String,
    },
    Screenshot {
        id: String,
        format: String,
        capture_id: String,
    },
    Mouse {
        id: String,
        action: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        x: Option<f64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        y: Option<f64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        button: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        wheel_delta: Option<i32>,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        modifiers: Vec<String>,
    },
    Keyboard {
        id: String,
        action: String,
        key: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        character: Option<String>,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        modifiers: Vec<String>,
    },
}

impl InFrame {
    pub fn id(&self) -> &str {
        match self {
            InFrame::Ready { id }
            | InFrame::Exec { id, .. }
            | InFrame::Complete { id, .. }
            | InFrame::Inspect { id, .. }
            | InFrame::Lint { id, .. }
            | InFrame::Screenshot { id, .. }
            | InFrame::Mouse { id, .. }
            | InFrame::Keyboard { id, .. } => id,
        }
    }
}

/// game -> desktop.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OutFrame {
    Disconnected,
    Stdout {
        stream: String,
        #[serde(default)]
        level: Option<String>,
        #[serde(default)]
        timestamp: Option<String>,
        #[serde(default)]
        source: Option<String>,
        text: String,
    },
    Ready {
        id: String,
        #[serde(default)]
        ok: bool,
        #[serde(default)]
        error: Option<String>,
    },
    Result {
        id: String,
        #[serde(default)]
        ok: bool,
        #[serde(default)]
        repr: Option<String>,
        #[serde(default)]
        exc: Option<String>,
        #[serde(default)]
        stdout: String,
        #[serde(default)]
        stderr: String,
    },
    Complete {
        id: String,
        candidates: Vec<Candidate>,
    },
    Inspect {
        id: String,
        #[serde(default)]
        signature: Option<String>,
        #[serde(default)]
        doc: Option<String>,
    },
    Lint {
        id: String,
        diagnostics: Vec<Diagnostic>,
    },
    ScreenshotStarted {
        id: String,
        #[serde(default)]
        ok: bool,
        #[serde(default)]
        width: Option<u32>,
        #[serde(default)]
        height: Option<u32>,
        #[serde(default)]
        error: Option<String>,
    },
    Input {
        id: String,
        #[serde(default)]
        ok: bool,
        #[serde(default)]
        x: Option<f64>,
        #[serde(default)]
        y: Option<f64>,
        #[serde(default)]
        width: Option<f64>,
        #[serde(default)]
        height: Option<f64>,
        #[serde(default)]
        key: Option<String>,
        #[serde(default)]
        error: Option<String>,
    },
}

impl OutFrame {
    /// The request id this frame answers, or `None` for async frames (stdout, hello).
    pub fn correlation_id(&self) -> Option<&str> {
        match self {
            OutFrame::Disconnected | OutFrame::Stdout { .. } => None,
            OutFrame::Ready { id, .. }
            | OutFrame::Result { id, .. }
            | OutFrame::Complete { id, .. }
            | OutFrame::Inspect { id, .. }
            | OutFrame::Lint { id, .. }
            | OutFrame::ScreenshotStarted { id, .. }
            | OutFrame::Input { id, .. } => Some(id),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Candidate {
    pub name: String,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub signature: Option<String>,
    #[serde(default)]
    pub doc: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Diagnostic {
    pub line: u32,
    pub col: u32,
    pub severity: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogLine {
    pub stream: String,
    pub level: Option<String>,
    pub timestamp: Option<String>,
    pub source: Option<String>,
    pub text: String,
}

/// Streamed to the frontend over a `tauri::ipc::Channel`.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ServerEvent {
    Log {
        lines: Vec<LogLine>,
    },
    Hello {
        version: Option<String>,
        pid: Option<i64>,
        capabilities: Vec<String>,
        remote: bool,
    },
    Disconnected,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn result_output_is_backward_compatible() {
        let old: OutFrame = serde_json::from_str(
            r#"{"type":"result","id":"old","ok":true,"repr":"42","exc":null}"#,
        )
        .unwrap();
        let new: OutFrame = serde_json::from_str(
            r#"{"type":"result","id":"new","ok":true,"stdout":"out","stderr":"err"}"#,
        )
        .unwrap();

        assert!(matches!(
            old,
            OutFrame::Result { stdout, stderr, .. } if stdout.is_empty() && stderr.is_empty()
        ));
        assert!(matches!(
            new,
            OutFrame::Result { stdout, stderr, .. } if stdout == "out" && stderr == "err"
        ));
    }

    #[test]
    fn stdout_accepts_optional_timestamp_metadata() {
        let frame: OutFrame = serde_json::from_str(
            r#"{"type":"stdout","stream":"python_log","level":"INFO","timestamp":"2026-08-20 04:31:08.295","source":"Main","text":"ready\n"}"#,
        )
        .unwrap();

        assert!(matches!(
            frame,
            OutFrame::Stdout {
                stream,
                level: Some(level),
                timestamp: Some(timestamp),
                source: Some(source),
                text,
            } if stream == "python_log"
                && level == "INFO"
                && timestamp == "2026-08-20 04:31:08.295"
                && source == "Main"
                && text == "ready\n"
        ));
    }

    #[test]
    fn virtual_input_omits_absent_optional_fields() {
        let frame = InFrame::Mouse {
            id: "mouse-1".into(),
            action: "click".into(),
            x: None,
            y: None,
            button: Some("left".into()),
            wheel_delta: None,
            modifiers: Vec::new(),
        };
        let value = serde_json::to_value(frame).unwrap();
        assert_eq!(value["type"], "mouse");
        assert_eq!(value["button"], "left");
        assert!(value.get("x").is_none());
        assert!(value.get("modifiers").is_none());
    }
}

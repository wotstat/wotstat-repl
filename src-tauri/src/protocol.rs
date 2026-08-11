//! Wire frames shared with the in-game agent (see docs/PROTOCOL.md) and the
//! events streamed to the frontend over a Tauri channel.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// desktop -> game. Serialized as `{ "type": "...", "id": "...", ... }`.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InFrame {
    Hello,
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
    Dump {
        id: String,
        expr: String,
        depth: u32,
    },
}

impl InFrame {
    pub fn id(&self) -> Option<&str> {
        match self {
            InFrame::Hello => None,
            InFrame::Exec { id, .. }
            | InFrame::Complete { id, .. }
            | InFrame::Inspect { id, .. }
            | InFrame::Lint { id, .. }
            | InFrame::Dump { id, .. } => Some(id),
        }
    }
}

/// game -> desktop.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OutFrame {
    Disconnected,
    Hello {
        #[serde(default)]
        version: Option<String>,
        #[serde(default)]
        pid: Option<i64>,
    },
    Stdout {
        stream: String,
        #[serde(default)]
        level: Option<String>,
        text: String,
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
    Dump {
        id: String,
        #[serde(default)]
        roots: serde_json::Value,
        #[serde(default)]
        errors: serde_json::Value,
        #[serde(default)]
        stubs: HashMap<String, String>,
    },
}

impl OutFrame {
    /// The request id this frame answers, or `None` for async frames (stdout, hello).
    pub fn correlation_id(&self) -> Option<&str> {
        match self {
            OutFrame::Disconnected | OutFrame::Hello { .. } | OutFrame::Stdout { .. } => None,
            OutFrame::Result { id, .. }
            | OutFrame::Complete { id, .. }
            | OutFrame::Inspect { id, .. }
            | OutFrame::Lint { id, .. }
            | OutFrame::Dump { id, .. } => Some(id),
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
    #[serde(default)]
    pub source: Option<String>,
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
}

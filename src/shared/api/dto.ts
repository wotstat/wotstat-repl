// Mirrors the Rust serde types in src-tauri/src/protocol.rs and install.rs.

export interface Candidate {
  name: string
  kind?: string | null
  signature?: string | null
  doc?: string | null
  source?: string | null
}

export interface Diagnostic {
  line: number
  col: number
  severity: string
  message: string
}

export interface LogLine {
  stream: string
  level?: string | null
  text: string
}

export interface GameInfo {
  path: string
  version: string
  modsVersion: string
  exe: string
  installed: boolean
}

export type McpStatus = 'disabled' | 'starting' | 'listening' | 'error'

export interface McpConnectionInfo {
  enabled: boolean
  url: string
  networkUrl: string
  status: McpStatus
  error: string | null
}

export interface McpCliState {
  installed: boolean
  configured: boolean
}

export interface McpCliStatus {
  codex: McpCliState
  claude: McpCliState
}

export type ServerEvent =
  | { kind: 'log'; lines: LogLine[] }
  | { kind: 'hello'; version?: string | null; pid?: number | null }
  | { kind: 'disconnected' }

export type OutFrame =
  | { type: 'hello'; version?: string | null; pid?: number | null }
  | { type: 'stdout'; stream: string; level?: string | null; text: string }
  | {
      type: 'result'
      id: string
      ok: boolean
      repr?: string | null
      exc?: string | null
      stdout?: string
      stderr?: string
    }
  | { type: 'complete'; id: string; candidates: Candidate[] }
  | { type: 'inspect'; id: string; signature?: string | null; doc?: string | null }
  | { type: 'lint'; id: string; diagnostics: Diagnostic[] }
  | { type: 'dump'; id: string; roots: unknown; errors: unknown; stubs: Record<string, string> }

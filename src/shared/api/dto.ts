// Mirrors the Rust serde types in src-tauri/src/protocol.rs and install.rs.

export interface Candidate {
  name: string
  kind?: string | null
  signature?: string | null
  doc?: string | null
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
  timestamp?: string | null
  source?: string | null
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
export type McpMode = 'full' | 'remoteRepl'

export interface McpConnectionInfo {
  enabled: boolean
  url: string
  networkUrl: string
  mode: McpMode
  status: McpStatus
  error: string | null
}

export interface AgentConnectionInfo {
  localAddress: string
  networkAddresses: string[]
  configPath: string
  clientConfig: string
}

export interface McpIntegrationState {
  available: boolean
  configured: boolean
  configPath: string | null
  error: string | null
}

export interface McpIntegrationStatus {
  chatgptCodex: McpIntegrationState
  claudeCode: McpIntegrationState
}

export type McpActivityStatus = 'pending' | 'success' | 'error'

export interface McpActivityEntry {
  id: number
  command: string
  status: McpActivityStatus
  startedAtMs: number
  finishedAtMs: number | null
  durationMs: number | null
  request: unknown
  response: unknown | null
}

export interface McpActivitySnapshot {
  revision: number
  entries: McpActivityEntry[] | null
}

export type ServerEvent =
  | { kind: 'log'; lines: LogLine[] }
  | { kind: 'hello'; version?: string | null; pid?: number | null; remote: boolean }
  | { kind: 'disconnected' }

export type OutFrame =
  | {
      type: 'stdout'
      stream: string
      level?: string | null
      timestamp?: string | null
      source?: string | null
      text: string
    }
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

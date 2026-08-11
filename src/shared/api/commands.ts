import { invoke, Channel } from '@tauri-apps/api/core'
import type { GameInfo, McpCliStatus, McpConnectionInfo, OutFrame, ServerEvent } from './dto'
import {
  COMPLETION_BUDGET_STORAGE_KEY,
  DEFAULT_COMPLETION_BUDGET,
  MAX_COMPLETION_BUDGET,
} from '@/shared/config'
import { loadState } from '@/shared/lib'

function completionBudget(): number {
  const value = loadState<unknown>(COMPLETION_BUDGET_STORAGE_KEY, DEFAULT_COMPLETION_BUDGET)
  return typeof value === 'number' && Number.isInteger(value)
    ? Math.min(MAX_COMPLETION_BUDGET, Math.max(0, value))
    : DEFAULT_COMPLETION_BUDGET
}

// Tauri v2 maps camelCase JS keys to snake_case Rust params automatically.
export const api = {
  ping: () => invoke<string>('ping'),
  mcpConnectionInfo: () => invoke<McpConnectionInfo>('mcp_connection_info'),
  mcpSetEnabled: (enabled: boolean) =>
    invoke<McpConnectionInfo>('mcp_set_enabled', { enabled }),
  mcpCliStatus: () => invoke<McpCliStatus>('mcp_cli_status'),
  mcpAddToCodex: () => invoke<string>('mcp_add_to_codex'),
  mcpAddToClaude: () => invoke<string>('mcp_add_to_claude'),
  mcpRemoveFromCodex: () => invoke<string>('mcp_remove_from_codex'),
  mcpRemoveFromClaude: () => invoke<string>('mcp_remove_from_claude'),
  defaultBufferDir: () => invoke<string>('default_buffer_dir'),
  stubsDir: () => invoke<string>('stubs_dir'),
  writeStubs: (stubs: Record<string, string>) => invoke<string>('write_stubs', { stubs }),

  detectGames: () => invoke<GameInfo[]>('detect_games'),
  inspectGameDir: (dir: string) => invoke<GameInfo | null>('inspect_game_dir', { dir }),
  installAgent: (gameDir: string, modsVersion: string) =>
    invoke<string>('install_agent', { gameDir, modsVersion }),
  launchGame: (gameDir: string, exe: string, replay?: string) =>
    invoke<void>('launch_game', { gameDir, exe, replay }),

  connect: (bufferDir: string, onEvent: Channel<ServerEvent>) =>
    invoke<void>('connect', { bufferDir, onEvent }),
  disconnect: () => invoke<void>('disconnect'),

  execCode: (code: string) => invoke<OutFrame>('exec_code', { code }),
  complete: (prefix: string, budget = completionBudget()) =>
    invoke<OutFrame>('complete', { prefix, budget }),
  inspect: (expr: string) => invoke<OutFrame>('inspect', { expr }),
  lintCode: (code: string) => invoke<OutFrame>('lint_code', { code }),
  dumpObject: (expr: string, depth = 2) => invoke<OutFrame>('dump_object', { expr, depth }),

  jediStart: (python: string, script: string, root: string, sysPath: string[]) =>
    invoke<unknown>('jedi_start', { python, script, root, sysPath }),
  jediComplete: (code: string, line: number, column: number) =>
    invoke<unknown>('jedi_complete', { code, line, column }),
  jediLint: (code: string) => invoke<unknown>('jedi_lint', { code }),
}

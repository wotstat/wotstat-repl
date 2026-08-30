import { invoke } from '@tauri-apps/api/core'
import type {
  AgentConnectionInfo,
  GameInfo,
  McpActivitySnapshot,
  McpIntegrationStatus,
  McpConnectionInfo,
} from './dto'

// Tauri v2 maps camelCase JS keys to snake_case Rust params automatically.
export const api = {
  ping: () => invoke<string>('ping'),
  mcpConnectionInfo: () => invoke<McpConnectionInfo>('mcp_connection_info'),
  mcpActivity: (sinceRevision?: number) =>
    invoke<McpActivitySnapshot>('mcp_activity', { sinceRevision }),
  mcpSetEnabled: (enabled: boolean) => invoke<McpConnectionInfo>('mcp_set_enabled', { enabled }),
  mcpIntegrationStatus: () => invoke<McpIntegrationStatus>('mcp_integration_status'),
  mcpAddToChatgptCodex: () => invoke<string>('mcp_add_to_chatgpt_codex'),
  mcpAddToClaude: () => invoke<string>('mcp_add_to_claude'),
  mcpRemoveFromChatgptCodex: () => invoke<string>('mcp_remove_from_chatgpt_codex'),
  mcpRemoveFromClaude: () => invoke<string>('mcp_remove_from_claude'),
  agentConnectionInfo: () => invoke<AgentConnectionInfo>('agent_connection_info'),

  detectGames: () => invoke<GameInfo[]>('detect_games'),
  inspectGameDir: (dir: string) => invoke<GameInfo | null>('inspect_game_dir', { dir }),
  installAgent: (gameDir: string, modsVersion: string) =>
    invoke<void>('install_agent', { gameDir, modsVersion }),
  launchGame: (gameDir: string, exe: string, replay?: string) =>
    invoke<void>('launch_game', { gameDir, exe, replay }),
}

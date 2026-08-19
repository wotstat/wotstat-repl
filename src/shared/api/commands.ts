import { invoke } from "@tauri-apps/api/core";
import type {
  AgentConnectionInfo,
  GameInfo,
  McpCliStatus,
  McpConnectionInfo,
} from "./dto";

// Tauri v2 maps camelCase JS keys to snake_case Rust params automatically.
export const api = {
  ping: () => invoke<string>("ping"),
  mcpConnectionInfo: () => invoke<McpConnectionInfo>("mcp_connection_info"),
  mcpSetEnabled: (enabled: boolean) =>
    invoke<McpConnectionInfo>("mcp_set_enabled", { enabled }),
  mcpCliStatus: () => invoke<McpCliStatus>("mcp_cli_status"),
  mcpAddToCodex: () => invoke<string>("mcp_add_to_codex"),
  mcpAddToClaude: () => invoke<string>("mcp_add_to_claude"),
  mcpRemoveFromCodex: () => invoke<string>("mcp_remove_from_codex"),
  mcpRemoveFromClaude: () => invoke<string>("mcp_remove_from_claude"),
  agentConnectionInfo: () =>
    invoke<AgentConnectionInfo>("agent_connection_info"),

  detectGames: () => invoke<GameInfo[]>("detect_games"),
  inspectGameDir: (dir: string) =>
    invoke<GameInfo | null>("inspect_game_dir", { dir }),
  installAgent: (gameDir: string, modsVersion: string) =>
    invoke<void>("install_agent", { gameDir, modsVersion }),
  launchGame: (gameDir: string, exe: string, replay?: string) =>
    invoke<void>("launch_game", { gameDir, exe, replay }),

};

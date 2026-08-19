### | EN | [RU](./README_RU.md) |

# WotStat WoT REPL

A desktop application for developing mods for **World of Tanks** and **Mir Tankov**. It connects to a running client, executes Python 2.7 code on the game thread, and displays live output.

![App window](./docs/img/hero.png)

## Features

- Support for both **World of Tanks** and **Mir Tankov**.
- Execute Python code in a live client.
- Stream `stdout`, `stderr`, and `BigWorld.log*` messages to the built-in console.
- Search logs and filter them by log level.
- Autocomplete based on live client objects, including their signatures.
- Launch a client or replay, or connect to an already running game.
- Automatically discover and remember added game clients.
- An embedded MCP server for controlling the client from AI agents.

## Installation

The [latest release](https://github.com/wotstat/wotstat-repl/releases/latest) provides two Windows downloads:

- `wotstat-repl.exe` — the ready-to-run application with no installation required. On a standard, unmodified Windows 10/11 system, download it and run it directly.
- `wotstat-repl-setup.exe` — an installer with shortcuts and standard uninstall support. Use it if Microsoft Edge WebView2 is missing from the system or if you prefer a regular installation.

## Usage

1. Launch the application. It automatically detects common game installations; if yours is not listed, click **Browse…** and select the client root folder.
2. Select a client and click **Launch Game**. To launch a replay, use the arrow beside that button and select a `.wotreplay` or `.mtreplay` file.
3. The application installs the agent into the selected client's `mods/<version>` directory, starts the game, and waits for the **Connected** status. If the game is already running and the agent was installed previously, click **Connect**.
4. Enter code in the editor and press `Ctrl/Cmd+Enter`. The selection is executed, or the entire editor when nothing is selected. Results and logs appear in the console on the right.

Autocomplete comes directly from the connected client. The in-game agent walks live objects with `dir()`/`getattr()`, reads typed native signatures from `__doc__`, and falls back to Python runtime inspection for regular functions and bound methods. Results are cached with bounded, short-lived caches to keep repeated completion responsive.

## MCP

On first launch, the application creates and enables a local Streamable HTTP MCP server by default. It listens on `0.0.0.0:8765`; the stable local URL looks like this:

```text
http://127.0.0.1:8765/mcp?token=<persistent-UUID>
```

The token and server state are stored in `%LOCALAPPDATA%\WotStatWoTREPL\mcp.json`. Click **MCP** in the status bar to enable or disable the server, copy a local or network URL, or add the `wot_repl` configuration to Codex or Claude Code. The corresponding CLI must be available on `PATH` for the add buttons to work.

To add the server to Codex manually:

```sh
codex mcp add wot_repl --url "http://127.0.0.1:8765/mcp?token=<token>"
```

The server exposes six tools:

| Tool               | Purpose                                                                                                  |
| ------------------ | -------------------------------------------------------------------------------------------------------- |
| `wot_list_clients` | Find supported game installations and report their status.                                               |
| `wot_start_client` | Install/connect the agent and launch a client; accepts `game_dir` and optional `replay_path`.            |
| `wot_close_client` | Gracefully close the active client; optional `timeout_ms` is limited to 0–60000.                         |
| `wot_kill_client`  | Force-terminate the active client after verifying its process.                                           |
| `wot_exec`         | Run Python 2.7 code on the game's main thread; accepts `code` and optional `timeout_ms` from 1 to 30000. |
| `wot_read_log`     | Read recent log messages; supports `cursor`, `limit`, and a short wait through `wait_ms`.                |

`wot_exec` can change the game state, while `wot_close_client` and `wot_kill_client` terminate the process. Start an MCP client session with `wot_list_clients`, and use `wot_close_client` before resorting to forced termination.

### | EN | [RU](./README_RU.md) |

# WotStat REPL

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

The [latest release](https://github.com/wotstat/wotstat-repl/releases/latest)
provides desktop builds and the game mod:

- Windows x64: portable `wotstat-repl_<version>.exe` and the
  `wotstat-repl_<version>_win64-setup.exe` installer.
- macOS: `wotstat-repl_<version>_macos.dmg`, a universal image for Apple
  Silicon and Intel Macs.
- Linux x64: `wotstat-repl_<version>_linux.AppImage` and
  `wotstat-repl_<version>_linux.deb` packages.
- `wotstat.repl_<version>.wotmod` for World of Tanks and
  `wotstat.repl_<version>.mtmod` for Mir Tankov. Both contain the same universal
  game agent with desktop TCP connectivity and a standalone browser REPL.

The same universal agent is embedded in every desktop build. When the application can
access a local game installation, it installs the embedded copy automatically;
the versioned release file is intended for a game on another machine or a manual
installation. macOS and Linux builds can connect to a remote/Proton game, but
do not directly launch a Windows game executable.

Internally the desktop embeds the agent as `wotstat.repl.mod`, then installs it
as `wotstat.repl_<version>.wotmod` for World of Tanks or
`wotstat.repl_<version>.mtmod` for Mir Tankov. Before installation it removes
older `wotstat.repl` builds from that game-version directory, so upgrading does
not leave duplicate mods.

## Usage

1. Launch the application. It automatically detects common game installations; if yours is not listed, click **Browse…** and select the client root folder.
2. Select a client and click **Launch Game**. To launch a replay, use the arrow beside that button and select a `.wotreplay` or `.mtreplay` file.
3. The application installs the agent into the selected client's `mods/<version>` directory, starts the game, and waits for the **Connected** status. If the game is already running and the agent was installed previously, click **Connect**.
4. Enter code in the editor and press `Ctrl/Cmd+Enter`. The selection is executed, or the entire editor when nothing is selected. Results and logs appear in the console on the right.

### Web REPL without the desktop application

1. Install `wotstat.repl_<version>.wotmod` for World of Tanks or
   `wotstat.repl_<version>.mtmod` for Mir Tankov into the client's
   `mods/<version>` directory.
2. Start the game.
3. Open [http://127.0.0.1:8768/](http://127.0.0.1:8768/) in a browser on the same machine.

The interface is available while the game is running. The browser page exposes
only the REPL, logs, diagnostics, and completion; it has no client process
controls, agent network settings, or MCP controls. The same mod simultaneously
keeps its TCP connection available, so the desktop application and its MCP
server can be used alongside the browser page. Its HTTP server listens only on
`127.0.0.1`; LAN access is deliberately disabled. The REPL still executes
arbitrary Python 2.7 code inside the game and must not be exposed to untrusted
users.

## Agent network

The universal in-game agent connects to the desktop over a persistent TCP connection.
Token authentication is enabled by default. Local sessions listen only on
`127.0.0.1:8766`. The agent reconnects automatically and retains up to 8 MiB of
unacknowledged output in memory, so the game and UI may start in either order
and startup logs arrive when the UI comes online while the game is still
running. A UI session controls only the first agent that connects; additional
game clients keep retrying and can take over only after the active client exits.

To run the game and UI on different machines, open **Agent LAN** in the status
bar and enable **Accept LAN connections**. Keep **Secure connection** enabled
(the default) and copy the remote game config to
`mods\configs\wotstat-repl\agent-network.json` under the Windows game root.
Alternatively, disable **Secure connection**: agents may connect anonymously
even if they still have a token saved for another desktop. Without an explicit
host, the agent discovers the UI over UDP port `8767`, then
opens an outgoing TCP connection to port `8766`. These are regular unprivileged
sockets; administrator rights are not required, although the UI machine's
firewall may ask whether to allow LAN traffic. In insecure mode, any reachable
agent can use the REPL. The popover lists every active
IPv4 listener address, with private LAN addresses first. If the network blocks broadcast,
an explicit UI IPv4 address is still required in the config. See
[the agent protocol](docs/AGENT_PROTOCOL.md) for the wire-level details.

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
| `wot_list_clients` | Report local installations and the currently connected remote game, including capabilities.             |
| `wot_start_client` | Install/connect the agent and launch a local client; accepts `game_dir` and optional `replay_path`.      |
| `wot_close_client` | Gracefully close the active local client; optional `timeout_ms` is limited to 0–60000.                   |
| `wot_kill_client`  | Force-terminate the active local client after verifying its process.                                     |
| `wot_exec`         | Run Python 2.7 code on the game's main thread; accepts `code` and optional `timeout_ms` from 1 to 30000. |
| `wot_read_log`     | Read recent log messages; supports `cursor`, `limit`, and a short wait through `wait_ms`.                |

`wot_list_clients` marks every entry as `local` or `remote` and returns explicit
`repl`, `start`, `close`, and `kill` capabilities. A remote entry intentionally
has no local `path` or `exe`: it supports `wot_exec` and `wot_read_log`, but the
desktop cannot launch or terminate its process. For local clients, use
`wot_close_client` before resorting to forced termination.

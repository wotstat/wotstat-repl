# Fuflo WoT REPL

A desktop IDE for World of Tanks Python mod development: a **live REPL into the
running game client**, with code completion and linting that no existing tool
(notably PJOrion) provides. A modern, cliche-free reimplementation of PJOrion's
"WOT-Client" workflow.

> Dev/private use only. Injecting a loader and running arbitrary code in the client
> is against WG ToS and detectable; hiding that is explicitly out of scope.

See [`docs/PLAN.md`](docs/PLAN.md) for the full design and
[`docs/PROTOCOL.md`](docs/PROTOCOL.md) for the wire protocol.

## Architecture

```
Desktop (Tauri 2 + React 19 + TS + Tailwind 4, FSD)
  Monaco editor  ── completion / lint / hover providers ─┐
  xterm console  ◄── Channel<LogBatch> ──────────────────┤
        │ invoke()                                        │
  Rust backend                                            │
    Tauri commands ─┐
                    ├─ ClientManager (one active client) ─ protocol ─ file-buffer
    MCP Streamable  ┘           │                              │
    HTTP /mcp            JediWorker (py2.7)            c2d / d2c + *.lock files
                                                               │
In-game agent (py2.7 / BigWorld)
  bw_site loader ─ capture (stdout + BigWorld.log*) ─ main-thread runner ─ handlers
```

Three channels over one file-buffer transport: continuous **stdout/log stream**,
**exec results** by id, and **complete / inspect / lint / dump** request/response.
Completion and lint are two-layer: static (jedi over the decompiled source, works
offline) merged with dynamic runtime introspection from the live game.

## Layout

| Path | What |
|---|---|
| `src/` | FSD frontend (`app` / `pages` / `widgets` / `features` / `entities` / `shared`) |
| `src-tauri/` | Rust backend (protocol, transport, jedi supervisor, commands) |
| `mod/` | universal `.mod` source tree, Python 2.7 builder, and agent tests |
| `tools/jedi_worker/` | CPython 2.7 jedi static worker (stdio JSON) |
| `docs/PLAN.md` | full implementation plan |

## Prerequisites

- Node 20+ and Rust 1.88+ (desktop app)
- CPython **2.7** for the in-game agent and the jedi static worker
  (`jedi==0.17.2` + `parso 0.7.x` are the last py2-capable releases; see
  `tools/jedi_worker/requirements.txt`). The `python27.dll` + stdlib bundled with
  PJOrion works as that interpreter.

## Develop

```sh
npm install
npm run dev            # frontend development server
npm run build          # tsc + vite production build
npm run lint:fsd       # steiger FSD boundary check
```

## MCP setup

The app includes an MCP Streamable HTTP server. It binds to `0.0.0.0:8765`; its
stable local URL is `http://127.0.0.1:8765/mcp?token=<persistent-uuid>`. On first
use it creates `%LOCALAPPDATA%/FufloWoTREPL/mcp.json`; the UUID token is generated
once and retained when the server is enabled or disabled.

Click the **MCP** badge in the lower status bar to inspect server status, enable or
disable the listener, and copy the local URL. The popover can also register or
unregister `wot_repl` through **Add to/Remove from Codex** and **Add to/Remove from
Claude**; the corresponding `codex` or `claude` CLI must be installed and available
on the app's `PATH`. Add actions register the stable local URL.

The popover also shows a **Network URL** using the advertised LAN IPv4. Use that
alternative when the app runs in a VM or another machine must connect. The host
must be able to reach the guest IP; firewall rules and NAT, host-only, or bridged
networking are user-managed, so choose a mode that permits host-to-guest traffic
on TCP port 8765.

The server exposes exactly six tools:

| Tool | Parameters and behavior |
|---|---|
| `wot_list_clients` | No parameters. Lists detected installations and their process/agent status. |
| `wot_start_client` | `game_dir`, optional `replay_path`. Installs/connects the agent as needed and starts that client; starting the already-active client is a no-op, and only one client may be active. |
| `wot_close_client` | Optional `timeout_ms` (default 10000, clamped to 0..60000). Requests graceful shutdown and waits; it never escalates automatically to kill. |
| `wot_kill_client` | No parameters. Force-terminates the saved active process after executable identity verification. |
| `wot_exec` | `code`, optional `timeout_ms` (default 30000, clamped to 1..30000). Runs arbitrary Python on the game main thread. A Python exception is a successful tool result with `ok: false`; connection, timeout, validation, and other tool failures return MCP `isError: true`. |
| `wot_read_log` | Optional `cursor`, `limit` (default 200, clamped to 1..1000), and `wait_ms` (default 0, clamped to 0..5000). Returns newer entries after a cursor, or the latest entries when omitted; history is bounded to 10,000 entries and 4 MiB of UTF-8 text, with `truncated` indicating lost history. |

Security is intentionally minimal: the URL token is a bearer-like gate, HTTP has
no TLS, and the listener binds on every interface. Use it only on a trusted
development/VM network.

## Build for Windows

```powershell
.\build-windows.ps1
.\build-windows.ps1 -Version v0.5.0
```

Without `-Version`, the version comes from `package.json`; an explicit version
may include the Git tag's `v` prefix. The same version is used for the universal `.mod`
and Tauri installers. The script also runs the Python 2.7 agent and Rust tests,
checks FSD boundaries, and produces the final Windows installers. The release
workflow calls the same script from a clean checkout.

## Status

| Milestone | State |
|---|---|
| M0 scaffold (Tauri + React + TS + Tailwind 4 + FSD) | done, builds green |
| M1 stdout stream | code complete; agent capture + Rust watcher + xterm wired |
| M2 exec round-trip | code complete; Monaco + main-thread runner |
| M3 static completion/lint | code complete; jedi worker + Monaco providers |
| M4 dynamic layer | code complete; runtime complete/inspect/stubgen + merge |
| M5 polish | command palette, connect controls, design system |

Verified without a game: frontend `tsc`+`vite` build, `cargo check`, steiger FSD
check, agent unit + integration tests (`mod/tests/selftest.py`, `mod/tests/itest.py`), jedi
worker protocol. **Needs a live WoT client** to validate `BigWorld.callback`
main-thread marshaling, the real log volume, and end-to-end injection.

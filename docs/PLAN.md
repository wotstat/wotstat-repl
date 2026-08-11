# WoT Mod Studio — Implementation Plan

A desktop IDE for World of Tanks Python mod development: a live REPL into the running
game client, with code completion and linting that no existing tool (notably PJOrion)
provides. Modern, cliche-free reimplementation of PJOrion's "WOT-Client" workflow.

> Status: planning. Working name "WoT Mod Studio". Targets private/dev use only
> (injection + arbitrary exec is against WG ToS and detectable; out of scope to hide).

---

## 1. Overview and goals

| Goal | Description |
|---|---|
| Live REPL | Execute Python in the running WoT client and see the result. |
| Live stdout | Stream the game's stdout/stderr and `BigWorld.log*` into the app. |
| Completion | Two layers: static (jedi over decompiled source) + dynamic (runtime introspection). |
| Linting | Static (parso 2.7 + pyflakes) + authoritative `compile()` in the game. |
| Quality | SOLID + YAGNI, FSD frontend architecture, anti-cliche dark devtool UI. |

Non-goals (see §11): obfuscation/recompile-to-pyc, debugger/breakpoints, multi-client,
detection evasion, support for other BigWorld titles.

---

## 2. Architecture at a glance

```
 ┌─────────────────────────── Desktop (Tauri) ───────────────────────────┐
 │  Webview: React + TS + Tailwind (FSD)                                  │
 │    Monaco editor ── completion/lint/hover providers                    │
 │    xterm.js console ── streamed log lines                              │
 │        │ invoke()                    ▲ Channel<LogEvent>               │
 │  ──────┼──────────────────────────── │ ─────────────────────────────  │
 │  Rust backend                                                          │
 │    commands ─ session ─ protocol(serde) ─ transport(trait)            │
 │                                   │                  │                 │
 │                       JediWorker (CPython 2.7)   FileBufferTransport   │
 └───────────────────────────────────────┬──────────────────┬───────────┘
                static layer ▲            │ notify+files     │
   U:\...\wot-src\...\scripts ┘           ▼                  ▼
                              shared dir: client_buffer / orion_buffer + *_mutex
                                          │
 ┌──────────────────────── Game: WoT client (embedded Python 2.7) ───────┐
 │  bw_site.py loader ─> agent: capture + main-thread runner + handlers   │
 └────────────────────────────────────────────────────────────────────────┘
```

### Three channels (all over the file-buffer transport, JSON-framed, newline-delimited)

1. **stdout/log stream** — continuous, game -> desktop, no id. Rendered in xterm.
2. **exec result** — game -> desktop, correlated by request `id`.
3. **request/response** — `complete` | `inspect` | `lint` | `stubgen`, correlated by `id`.

### Two completion/lint layers

- **Static** (instant, offline): jedi 0.17.2 worker over the decompiled WoT source. Knows
  *structure*.
- **Dynamic** (when game is running): `dir()`/`inspect` on live objects via the agent.
  Knows *state*, plus native C modules that have no `.py`. Candidates tagged `live`.

---

## 3. FSD frontend structure

Layers: `app` and `shared` hold segments directly; `pages`, `widgets`, `features`,
`entities` are slice-then-segment. Import rule: a slice imports only from layers strictly
below; cross-slice access only via the slice's public `index.ts`. Enforced by **steiger**.

```
src/
  app/                      # no slices; composition root
    providers/              # theme, store bootstrap, query client
    layout/                 # split-pane shell (editor | console | status)
    styles/                 # tailwind entry + tokens.css
    index.tsx
  pages/
    studio/                 # the single IDE page (MVP)
      ui/ StudioPage.tsx
      index.ts
  widgets/
    editor-panel/           # Monaco + provider registration
      ui/  model/  index.ts
    log-console/            # xterm + stream subscription
      ui/  model/  index.ts
    status-bar/             # connection badge, jedi index state, cursor pos
      ui/  index.ts
    command-palette/        # Ctrl/Cmd+K
      ui/  model/  index.ts
  features/
    connect-session/        # connect/disconnect, configure game/buffer dirs
    run-code/               # exec selection/buffer, correlate by id (Cmd+Enter)
    complete-code/          # Monaco CompletionItemProvider: merge static+live
    lint-code/              # debounced lint -> Monaco markers
    generate-stubs/         # trigger stubgen -> write .pyi for jedi
      <each>/ model/  lib/  ui/  index.ts
  entities/
    session/                # status, gameDir, bufferDir  (model + ConnectionBadge ui)
    repl-message/           # stdout|stderr|result|system  (model + LogLine ui)
    diagnostic/             # Diagnostic type + toMonacoMarker (model + lib)
    completion-item/        # Candidate {source: static|live} (model)
      <each>/ model/  index.ts
  shared/
    api/                    # THE Tauri boundary
      commands.ts           # invoke() wrappers (typed)
      channels.ts           # Channel<LogEvent> subscription
      dto.ts                # protocol DTOs (mirror Rust serde types)
      index.ts
    ui/                     # design-system kit (Button, Panel, Badge, ...)
    lib/                    # pure utils
    config/                 # constants, keybindings
```

State: lightweight per-slice stores in `model` segments (zustand), not one global store
(SRP, keeps slices independent). The only place that imports `@tauri-apps/api` is
`shared/api` (DIP: features depend on the typed wrapper, not the transport).

Enforcement config:

```jsonc
// steiger.config.js + eslint
plugins: ["@feature-sliced/steiger-plugin"],
// eslint-plugin-boundaries to forbid deep imports past index.ts
```

---

## 4. Rust backend modules and the Tauri boundary

```
src-tauri/src/
  lib.rs                # tauri::Builder, manage(State), register commands
  commands.rs           # #[tauri::command] connect/disconnect/exec/complete/lint/stubgen/configure
  session/mod.rs        # SessionManager: status, game dir, shared-buffer dir
  protocol/mod.rs       # Frame enum (serde, internally-tagged) — single source of truth
  transport/
    mod.rs              # trait Transport (one impl now; YAGNI on sockets)
    file_buffer.rs      # FileBufferTransport: notify watcher + mutex writes
  jedi/mod.rs           # JediWorker: spawn CPython 2.7, JSON-over-stdio, id correlation
  stream.rs             # log batching (coalesce ~33ms) -> Channel<LogEvent>
```

Key contracts:

```rust
// protocol/mod.rs — desktop<->game frames (serde tag = "type")
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InFrame  { Exec{id:String, code:String},
                    Complete{id:String, code:String, line:u32, col:u32},
                    Inspect{id:String, expr:String},
                    Lint{id:String, code:String},
                    Stubgen{id:String, modules:Vec<String>} }
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OutFrame { Stdout{stream:Stream, level:Option<String>, text:String},   // no id
                    Result{id:String, ok:bool, repr:Option<String>, stdout:String, exc:Option<String>},
                    Complete{id:String, candidates:Vec<Candidate>},
                    Inspect{id:String, signature:Option<String>, doc:Option<String>},
                    Lint{id:String, diagnostics:Vec<Diagnostic>},
                    Stubgen{id:String, stubs:HashMap<String,String>} }

// transport/mod.rs — the one seam we keep clean (DIP), one impl for now
pub trait Transport: Send + Sync {
    fn send(&self, frame: &InFrame) -> Result<()>;          // write orion_buffer under mutex
    fn subscribe(&self) -> mpsc::Receiver<OutFrame>;        // parsed client_buffer frames
}
```

- **Streaming**: `Stdout` frames go to a `tauri::ipc::Channel<LogEvent>` (Tauri docs: events
  are not for high-throughput; channels are). Coalesce in `stream.rs` on a ~33ms tokio
  interval so the webview is not flooded.
- **Watcher**: `notify` on `client_buffer`; keep a byte offset, read appended frames; on size
  shrink (game restart truncates the file) reset offset to 0.
- **Jedi worker**: long-lived child via `tauri-plugin-shell` (the `CommandEvent::Stdout`
  pattern) or `std::process`; one JSON request/response per line, correlated by id.

---

## 5. In-game py2.7 agent and wire protocol

Grounded in the reversed PJOrion `wottransmission`. Improvements: JSON frames, three
channels, and **main-thread marshaling** (PJOrion exec's on its daemon thread, which can
crash the client when touching render/entity objects).

```
mod/res/scripts/common (pure py2.7 stdlib only, shipped as a universal .mod)
  bw_site.py          # early loader (see injection below)
  wms_agent/__init__.py
  wms_agent/framebus.py  # buffer + file-mutex, newline-delimited JSON frames
  wms_agent/capture.py   # sys.stdout/stderr hijack + all 8 BigWorld.log* hooks
  wms_agent/runner.py    # main-thread marshaling via BigWorld.callback(0, fn)
  wms_agent/handlers.py  # exec | complete | inspect | lint | dump
  wms_agent/loop.py      # daemon: poll buffer, dispatch
```

**Injection (from PJOrion, kept):** loader `bw_site.pyc` imports the adjacent
`wms_agent` package, calls `wms_agent.start(buffer_dir)`, then `marshal.loads` the
original `bw_site.pyc` from `res/packages/scripts.pkg` (skip 8-byte header) and execs
it, so game startup is transparent.

**Capture (from PJOrion, kept):** wrap `sys.stdout`/`sys.stderr` (mirror to saved
original) and monkeypatch `logTrace/logDebug/logInfo/logNotice/logWarning/logError/`
`logCritical/logHack`; every write emits a `stdout` frame. Escape non-printable bytes.

**Exec (improved):** the poll thread reads an `exec`/`inspect`/`complete` frame, enqueues a
closure, and schedules it with `BigWorld.callback(0, runner)`. The runner executes on the
game main thread, captures stdout + repr + traceback, and hands the result back to the poll
thread (Event), which writes the `result` frame. This fixes PJOrion's footgun.

Message schema (newline-delimited JSON; `orion_buffer` = desktop->game, `client_buffer` =
game->desktop):

```jsonc
// -> game
{"id":"u1","type":"exec","code":"player = BigWorld.player(); print player"}
{"id":"u2","type":"complete","code":"BigWorld.","line":1,"col":9}
{"id":"u3","type":"inspect","expr":"BigWorld.player()"}
{"id":"u4","type":"lint","code":"print x"}
{"id":"u5","type":"stubgen","modules":["BigWorld","Math","ResMgr","Account"]}
// -> desktop
{"type":"stdout","stream":"log","level":"INFO","text":"Avatar.init\n"}
{"id":"u1","type":"result","ok":true,"repr":"<Avatar>","stdout":"<Avatar>\n","exc":null}
{"id":"u2","type":"complete","candidates":[{"name":"player","kind":"function","sig":"player()","doc":"...","source":"live"}]}
{"id":"u4","type":"lint","diagnostics":[{"line":1,"col":1,"severity":"error","message":"invalid syntax"}]}
```

Lock semantics: keep PJOrion's file-mutex (lock = create `*_mutex`, unlock = delete). It is
adequate for one local client; the race is bounded because each side appends whole frames
and the reader clears under lock. (Atomic-rename hardening is deferred, §11.)

---

## 6. Completion + lint subsystem

```
complete request ─┬─ STATIC: JediWorker (CPython 2.7 + jedi 0.17.2)
                  │     jedi.Project(path=wot-src/.../scripts,
                  │                  added_sys_path=[common, client, client_common, <stubs>])
                  │     + bundled .pyi stubs for native modules
                  └─ DYNAMIC: agent {type:complete} on live objects (tag source="live")
                        merge: static first (instant), live merged on top, deduped by name
```

- **Version pin (decisive):** jedi `0.17.2` is the last release supporting Python 2;
  `0.18.0` requires py3.6+. So: **CPython 2.7 + jedi==0.17.2 + parso>=0.7,<0.8**. The
  worker process itself runs on py2.7 so parso parses py2 grammar (`print` statements,
  `except X, e`). Reuse PJOrion's bundled `python27.dll`/`python27.zip` as that interpreter.
- **sys_path roots:** `scripts/common`, `scripts/client`, `scripts/client_common`, plus the
  generated stubs dir, via `jedi.Project(added_sys_path=[...])`.
- **Native-module stubs:** native C modules (`BigWorld`, `Math`, `ResMgr`, `Account`) have no
  `.py`, so they are invisible to static jedi. `generate-stubs` asks the agent to
  `dir()`/`inspect` them at runtime and emits `.pyi` files placed on the jedi sys_path. One
  runtime introspection feeds offline completion afterward.
- **Lint:** static `parso(2.7)` + `pyflakes` (last py2-compatible release) resolved against
  the source tree -> undefined names, unused, bad imports; PLUS authoritative `compile()` in
  the agent for syntax truth. Both map to Monaco markers (`diagnostic` entity -> `toMonacoMarker`).
- **SOLID/YAGNI:** static and dynamic sit behind one `CompletionSource` interface in
  `features/complete-code`; no workspace-wide go-to index for MVP (jedi handles local
  completion first).

---

## 7. Design system (anti-cliche, dark devtool)

Adapted from taste-skill, retargeted from landing pages to a dense, keyboard-driven IDE.
UI copy follows the bans: no em-dash, no "Jane Doe"/lorem, real icons (lucide), no decorative
gradients/glow, left-aligned dense empty states (not centered hero blocks).

**Palette — intentional cool slate "instrument panel", explicitly not beige/brass/oxblood.**
Justification: a desaturated blue-slate base reduces eye strain over long sessions and reads
as a technical instrument; a single teal accent signals the "live runtime" link without the
overused VS-Code blue or any banned warm-premium palette.

```css
/* tokens.css */
--bg-base:#0E1116; --bg-panel:#151A21; --bg-elevated:#1B222B; --border:#232B36;
--fg:#C9D3DF; --fg-muted:#7F8B9A; --fg-faint:#4C586A;
--accent-live:#3FB9B0;            /* connected / runtime candidates */
--ok:#4CAF7D; --warn:#D8A657; --error:#E5484D; --info:#5B9BD5;
```

- **Type:** UI sans = Inter 13px; editor/console mono = JetBrains Mono 13px. Scale 12/13/14/16/20.
- **Density:** 4px spacing base; 28-32px control/row height; 1px borders over shadows.
- **Motion (restrained):** 80-120ms ease on hover/focus, panel resize, completion popup fade +
  4px translate. Banned: scroll-jacking, marquees, parallax, per-element entrance animations,
  decorative spinners. Connection badge uses a calm pulse only while `connecting`.
- **Components:** editor pane, xterm console (level-colored), status bar (connection badge +
  jedi index state + cursor pos), command palette, completion popup (static vs `live` icon),
  diagnostics gutter.

---

## 8. Tech stack and key version pins

| Area | Choice | Pin / note |
|---|---|---|
| Shell | Tauri | 2.x; `tauri::ipc::Channel` for log stream |
| UI | React + TS + Vite | React 18+, TS 5.x, Vite 5/6 |
| Styling | Tailwind | 3.4+ with the token set above |
| State | zustand | per-slice model stores |
| Editor | monaco-editor | provider-based completion/lint/hover |
| Console | @xterm/xterm + addon-fit | batched writes |
| Motion/icons | motion + lucide-react | restrained policy |
| FSD lint | steiger + @feature-sliced/steiger-plugin, eslint-plugin-boundaries | |
| Rust | notify, serde/serde_json, tokio, uuid, tauri-plugin-shell | |
| Static worker | CPython 2.7 + jedi==0.17.2 + parso 0.7.x + pyflakes (py2) | **last py2 jedi** |
| In-game agent | pure py2.7 stdlib only | shipped as a universal .mod |

---

## 9. Milestone roadmap (MVP first)

| # | Milestone | Deliverable (acceptance) | Depends on |
|---|---|---|---|
| M0 | Skeleton | Tauri+React+TS+Tailwind+FSD scaffold, steiger config, tokens, empty 3-pane layout renders | - |
| M1 | **stdout vertical slice** | Agent capture -> client_buffer; Rust watcher + Channel; xterm shows live game logs when WoT runs | M0 |
| M2 | exec round-trip | orion_buffer + main-thread runner; Monaco + Cmd+Enter; result rendered by id | M1 |
| M3 | static completion+lint | py2.7 jedi 0.17.2 worker over wot-src; Monaco completion + markers (parso/pyflakes + compile) | M2 |
| M4 | dynamic layer | complete/inspect/stubgen via agent; merge `live` candidates; native `.pyi` generated | M3 |
| M5 | polish | command palette, settings (dirs), reconnect UX, universal .mod build + installer | M4 |

M1 is the "see the game's stdout in the window" goal and is the first proof the whole
transport works end to end.

---

## 10. Risks and open questions

- **ToS / anti-cheat:** bw_site injection + arbitrary exec is against WG ToS and detectable.
  Dev/private use only; no evasion in scope.
- **File-mutex races** under heavy log volume; may need atomic-rename or a named pipe later.
- **jedi 0.17.2 perf** over ~8066 files: cold completion latency; needs caching/warm worker.
- **BigWorld.callback** threading: confirm main-thread scheduling exists in this client build
  and that result hand-back (Event) does not deadlock the poll thread.
- **py2.7 interpreter sourcing** on user machines (reuse PJOrion's bundled python27).
- **parso 2.7 on decompiled source:** imperfect decompilation may produce unparseable files;
  lint must degrade gracefully per-file.
- **Tauri capabilities:** fs access to the game dir + shared-buffer dir + worker spawn must be
  whitelisted in the capability config.

---

## 11. Explicitly deferred (YAGNI)

- Pluggable socket transport (keep the `Transport` trait, ship one file-buffer impl).
- Multi-session / multi-client.
- Hot module reload (PJOrion's `module.py` / Twisted rebuild).
- Workspace-wide go-to-definition index.
- Obfuscation / recompile-to-pyc (PJOrion scope, not ours).
- Debugger / breakpoints.
- Other BigWorld titles.
```

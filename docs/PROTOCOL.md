# File-buffer wire protocol

Both sides (desktop Rust backend and in-game py2.7 agent) speak newline-delimited
JSON frames over two append files in a shared directory.

## Files

| File | Writer | Reader | Lock |
|---|---|---|---|
| `c2d` | game agent | desktop | `c2d.lock` |
| `d2c` | desktop | game agent | `d2c.lock` |

Lock = create `<name>.lock` with `O_CREAT \| O_EXCL`; unlock = delete it. A lock
older than 5s is force-broken (peer crash recovery). The writer holds the lock for
the whole append; the reader holds it for the whole read+truncate. One JSON object
per line.

## Desktop -> game (`d2c`)

```jsonc
// request/repeat the agent handshake (used when reconnecting)
{ "type": "hello" }
{ "id": "<uuid>", "type": "exec",    "code": "player = BigWorld.player()" }
{ "id": "<uuid>", "type": "complete","prefix": "BigWorld.pla", "budget": 120 }
{ "id": "<uuid>", "type": "inspect", "expr": "BigWorld.player()" }
{ "id": "<uuid>", "type": "lint",    "code": "print x" }
{ "id": "<uuid>", "type": "dump",    "expr": "BigWorld.player()", "depth": 3 }
```

`complete.budget` is optional for older clients and defaults to `120`; it limits
how many live candidates are inspected for kind, documentation, and signatures.

## Game -> desktop (`c2d`)

```jsonc
// continuous stream, no id
{ "type": "stdout", "stream": "stdout|stderr|log", "level": "INFO", "text": "...\n" }
// shutdown, no id
{ "type": "disconnected" }
// correlated by id
{ "id": "<uuid>", "type": "result",  "ok": true, "repr": "<Avatar>", "exc": null, "stdout": "printed output\n", "stderr": "" }
{ "id": "<uuid>", "type": "complete", "candidates": [{"name":"player","source":"live"}] }
{ "id": "<uuid>", "type": "inspect",  "signature": "player()", "doc": "..." }
{ "id": "<uuid>", "type": "lint",     "diagnostics": [{"line":1,"col":1,"severity":"error","message":"..."}] }
{ "id": "<uuid>", "type": "dump",     "roots": [ ... ], "errors": [ ... ], "stubs": { "Avatar": "<.pyi text>" } }
```

`result.stdout` and `result.stderr` contain output correlated to that `exec`
request while the same writes continue through the global stdout/log stream. The
desktop accepts older result frames that omit either field and deserializes the
missing value as an empty string.

## Threading

`exec`, `complete`, `inspect`, `dump` run on the game **main thread** via
`BigWorld.callback(0, ...)`. `lint` is pure and runs on the agent poll thread.
Captured stdout/log is queued on the game thread and shipped by the poll thread.

The agent sends `hello` on startup and responds to the desktop's `hello` request,
so a desktop reconnect does not depend on the original startup frame still being
in the buffer. The frame contains the game process PID. The desktop checks that
this process is still alive and emits `disconnected` when it exits. A normal
`fini()` also sends `disconnected` immediately. Before the first `hello`, the
desktop waits indefinitely for the game.

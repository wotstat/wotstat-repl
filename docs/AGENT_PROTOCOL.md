# Agent network protocol v1

The in-game Python 2.7 agent connects out to the Rust desktop backend. TCP
frames are UTF-8 newline-delimited JSON with a 2 MiB frame limit. UDP discovery
datagrams are single JSON objects without newline framing.

## Endpoints

- TCP `8766`: persistent agent session, authenticated when a token is present.
- UDP `8767`: LAN discovery, enabled only when the UI's LAN mode is enabled.
- Local mode binds TCP to `127.0.0.1`; LAN mode binds TCP and UDP to `0.0.0.0`.

Both ports are unprivileged. The implementation uses ordinary `SOCK_STREAM`
and `SOCK_DGRAM` sockets, never raw sockets.

## Optional shared configuration

Secure connection is enabled by default. Both peers then use the same
`agent-network.json`:

```json
{
  "token": "persistent-UUID",
  "host": "auto",
  "tcp_port": 8766,
  "discovery_port": 8767
}
```

Local installation writes a copy under
`mods/configs/wotstat-repl/agent-network.json`, which is visible from native
Windows and from a Proton prefix. For a remote game, copy the configuration
shown by the UI to the same location. Set `host` to a literal UI address when
UDP broadcast is unavailable.

When the UI's **Secure connection** option is disabled, an agent without this
file uses `host: "auto"` and the default ports `8766`/`8767`. If broadcast is
unavailable, a tokenless file may still provide a literal endpoint:

```json
{
  "host": "192.168.0.30",
  "tcp_port": 8766,
  "discovery_port": 8767
}
```

## Discovery

With `host: "auto"`, the agent first tries `127.0.0.1`, then broadcasts:

```json
{
  "type": "discover",
  "protocol": 1,
  "agent_id": "...",
  "nonce": "...",
  "proof": "..."
}
```

With a token, the UI validates `proof = HMAC-SHA256(token,
"discover|agent_id|nonce")` and replies to the source address with its TCP port,
server id, `secure: true`, and `HMAC-SHA256(token,
"offer|agent_id|nonce|tcp_port|server_id")`. A config-free agent sends an empty
proof; it receives `secure: false` only when the UI allows insecure connections.
An agent with a saved token also accepts such an insecure offer, allowing it to
move from a previously paired desktop to an explicitly insecure listener.
The agent connects to the source IP of the offer, not to an address supplied
inside the payload.

## TCP authentication

The agent starts every TCP connection with `hello`, including a process-local
session id. A configured agent also sends `HMAC-SHA256(token,
"hello|protocol|agent_id|session|nonce")`. The desktop answers with `welcome`, a
`secure` flag, and `HMAC-SHA256(token,
"welcome|protocol|agent_id|session|nonce|server_id")`. When secure mode is
enabled, an invalid or missing proof is rejected before application frames are
accepted. When it is disabled, anonymous connections are allowed. A configured
agent authenticates when its token matches, but accepts an explicit
`secure: false` welcome when the token belongs to another desktop.

The token authenticates peers but does not encrypt code or logs. Anonymous mode
allows any reachable agent to use the REPL. LAN mode is for trusted networks;
use secure mode plus a VPN or tunnel across untrusted networks.

## Delivery and reconnect

Every agent-to-desktop application frame contains `session` and a monotonic
`seq`. The desktop acknowledges processed frames with:

```json
{ "type": "ack", "session": "...", "seq": 42 }
```

The agent retains unacknowledged frames in an 8 MiB in-memory deque and replays
them after reconnect. It never blocks the game thread on socket I/O. When the
limit is exceeded, oldest frames are removed; the desktop detects the sequence
gap and emits a warning. The buffer intentionally disappears when the game
process exits.

The desktop accepts only the first authenticated/allowed agent as the active
session. Later TCP handshakes are closed without a `welcome` while that session
is alive. Rejected agents keep their normal reconnect loop, so one can become
active after the first agent disconnects.

Desktop requests keep the existing `exec`, `complete`, `inspect`, and `lint`
shapes and are correlated by UUID `id`. The desktop sends `ping`; the agent
answers `pong`. A silent connection is closed after 20 seconds.

## Local screenshot capture

MCP screenshot capture requires the Rust desktop and game to run on the same
computer. The desktop sends a `screenshot` request containing a generated
32-character hexadecimal `capture_id` and `jpg` or `png` format. The game calls
`BigWorld.screenShot` with a `wotstat-repl-<capture_id>` filename prefix and
returns only the window dimensions. The Rust desktop waits for the matching file
under the selected installation's `screenshots` directory, verifies that it has
finished growing, reads and validates it, then removes it. Image bytes never
travel through this JSON protocol.

## Virtual input delivery

Successful `mouse` and `keyboard` responses use `ok: true` to confirm that the
event was delivered through the game's input pipeline. The agent intentionally
ignores the boolean returned by BigWorld's event handlers: it indicates whether
a particular UI handler consumed the event, not whether delivery succeeded. The
public MCP result exposes this unambiguously as `delivered: true`.

A composite mouse `click` is deliberately spread across game ticks: cursor
movement is applied first, button down runs on the next tick, and button up on
the tick after that. The input response is emitted only after button up. This
matches BigWorld UI processing and prevents down/up from collapsing into a
hover-only event.

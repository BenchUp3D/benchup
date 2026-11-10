# benchup-mcp

An [MCP](https://modelcontextprotocol.io) server that lets an AI agent (Claude Code, Claude Desktop, any MCP client) build in BenchUp. It exposes the tools from `benchup-tools` (lengths in mm, Z up; `get_scene` first). Every mutating call is one undo step labelled `Agent: <tool>`.

It speaks newline-delimited JSON-RPC 2.0 on stdin/stdout (logs go to stderr) and has two modes.

## File mode

```
benchup-mcp file project.bup [--allow-shared]
```

Loads the `.bup`, applies each tool call, records an undo step and saves atomically after every call (symlinks are followed and file permissions kept). If the save fails, the change is rolled back and the call returns an error. If the file changed on disk since this session loaded or last saved it (for example BenchUp saved it), nothing is written, the change is rolled back and the call returns "The project file changed on disk"; reopen it with a new session. Shared (server-linked) projects are refused unless `--allow-shared`, because local edits would diverge from the server. Do not edit a file that BenchUp has open.

## Live mode

```
benchup-mcp live [--port 47615] [--token T] [--allow-origin ORIGIN]...
```

The agent talks stdio to this process; BenchUp (desktop or the web app) connects to it over a WebSocket and executes the calls in the open document. In BenchUp: Settings → Add-ons → Agent connection → Allow agent control.

Token: set `BENCHUP_AGENT_TOKEN` (at least 16 characters). `--token` also works but is discouraged: command-line arguments are visible to other users in process lists.

```
claude mcp add benchup -e BENCHUP_AGENT_TOKEN=<TOKEN> -- benchup-mcp live
```

```json
{
  "mcpServers": {
    "benchup": {
      "command": "benchup-mcp",
      "args": ["live"],
      "env": { "BENCHUP_AGENT_TOKEN": "<TOKEN>" }
    }
  }
}
```

### App protocol v2 (JSON text frames)

The raw token is never sent; both sides prove they know it with HMAC-SHA256 (`benchup_tools::pairing`).

1. App connects to `ws://127.0.0.1:<port>` and sends a hello: `{"type":"authenticate","ticket":"","connection":<string>}` (`ticket` is ignored).
2. Bridge replies `{"type":"challenge","nonce":<64 hex>,"server":"benchup-mcp","version":"...","protocol":2}`.
3. App sends `{"type":"prove","nonce":<64 hex app nonce>,"proof":hex(HMAC(token, "benchup-agent-link/app\n" + bridge_nonce + "\n" + app_nonce))}`. A bad nonce or proof gets `{"type":"error","message":"pairing token does not match"}` and the socket is closed. Any other frame after the hello is a protocol error.
4. Bridge replies `{"type":"ready","server":"benchup-mcp","version":"...","proof":hex(HMAC(token, "benchup-agent-link/bridge\n" + bridge_nonce + "\n" + app_nonce))}`; the app must verify this proof before trusting the bridge.
5. Bridge to app: `{"type":"call","id":"c1","name":<tool>,"arguments":{...},"client":<agent name>,"deadline_ms":<unix epoch ms when the bridge gives up>}`.
6. App to bridge: `{"type":"result","id":"c1","ok":true,"result":{...}}`. The app may also send `{"type":"status","message":"..."}` (logged only).
7. On timeout the bridge sends `{"type":"cancel","id":"c1"}` and the agent gets an `isError` result telling it to call `get_scene` before retrying. A result for a cancelled id is ignored.

The whole hello-to-prove exchange (including the WebSocket handshake) must finish within 5 s regardless of traffic. Until authenticated, messages are limited to 1 MiB (text frames to 64 KiB) and at most 8 unauthenticated connections are held; extra ones are closed immediately. Authenticated messages may be up to 32 MiB. The bridge refuses to start without OS randomness.

One app is connected at a time; a newer authenticated connection replaces (and closes) the older one. `tools/list` is answered locally. A forwarded call waits up to 120 s; on timeout, when no app is connected, or when the app drops mid-call, the agent gets an `isError` result with a hint.

### Security model

- Binds `127.0.0.1` only; there is no option to bind elsewhere.
- WebSocket `Origin` must be absent (native app), `https://benchup3d.github.io`, `http://localhost[:port]`, `http://127.0.0.1[:port]`, or an `--allow-origin` value. Anything else gets HTTP 403. This stops arbitrary web pages from reaching the bridge.
- The pairing token is never transmitted; mutual HMAC challenge-response with fresh nonces on both sides. Treat it like a password: use the env var, not `--token` (visible in the process list).

## Tests

```
cargo test -p benchup-mcp
```

Tests use only loopback (`127.0.0.1`, port 0) and temp directories.

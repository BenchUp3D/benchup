# benchup-mcp

An [MCP](https://modelcontextprotocol.io) server that lets an AI agent (Claude Code, Claude Desktop, any MCP client) build in BenchUp. It exposes the tools from `benchup-tools` (lengths in mm, Z up; `get_scene` first). Every mutating call is one undo step labelled `Agent: <tool>`.

It speaks newline-delimited JSON-RPC 2.0 on stdin/stdout (logs go to stderr) and has two modes.

## File mode

```
benchup-mcp file project.bup [--allow-shared]
```

Loads the `.bup`, applies each tool call, records an undo step and saves atomically after every call. If the save fails, the change is rolled back and the call returns an error. Shared (server-linked) projects are refused unless `--allow-shared`, because local edits would diverge from the server. Do not edit a file that BenchUp has open.

## Live mode

```
benchup-mcp live [--port 47615] [--token T] [--allow-origin ORIGIN]...
```

The agent talks stdio to this process; BenchUp (desktop or the web app) connects to it over a WebSocket and executes the calls in the open document. In BenchUp: Settings → Add-ons → Agent connection → Allow agent control.

Token: `--token` or `BENCHUP_AGENT_TOKEN`, at least 16 characters.

```
claude mcp add benchup -- benchup-mcp live --token <TOKEN>
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

### App protocol (JSON text frames)

1. App connects to `ws://127.0.0.1:<port>` and, within 5 s, sends `{"type":"authenticate","ticket":<token>,"connection":<string>}`.
2. Bridge replies `{"type":"ready","server":"benchup-mcp","version":"..."}`, or `{"type":"error","message":"pairing token does not match"}` and closes.
3. Bridge to app: `{"type":"call","id":"c1","name":<tool>,"arguments":{...},"client":<agent name>}`.
4. App to bridge: `{"type":"result","id":"c1","ok":true,"result":{...}}`. The app may also send `{"type":"status","message":"..."}` (logged only).

One app is connected at a time; a newer authenticated connection replaces (and closes) the older one. `tools/list` is answered locally. A forwarded call waits up to 120 s; on timeout, when no app is connected, or when the app drops mid-call, the agent gets an `isError` result with a hint.

### Security model

- Binds `127.0.0.1` only; there is no option to bind elsewhere.
- WebSocket `Origin` must be absent (native app), `https://benchup3d.github.io`, `http://localhost[:port]`, `http://127.0.0.1[:port]`, or an `--allow-origin` value. Anything else gets HTTP 403. This stops arbitrary web pages from reaching the bridge.
- The pairing token is compared in constant time. Treat it like a password: prefer the env var over `--token` (which is visible in the process list).

## Tests

```
cargo test -p benchup-mcp
```

Tests use only loopback (`127.0.0.1`, port 0) and temp directories.

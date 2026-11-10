# Use BenchUp from your own AI agent (MCP)

BenchUp includes an [MCP](https://modelcontextprotocol.io) server, `benchup-mcp`. It lets an AI agent that you already use (Claude Code, Cursor, or any MCP client) read and edit a BenchUp project. You bring the agent and the model. BenchUp does not send anything to an AI service for this.

The agent gets the same 27 tools as [Build with MIRA](assistant.md). Lengths are in millimetres and Z is up. The tool list is in [mira/tools.md](mira/tools.md).

MCP and MIRA are two separate add-ons. You can use either, both, or neither.

## Two modes

| Mode | Command | Use it when |
|---|---|---|
| File | `benchup-mcp file <project.bup>` | You want the agent to edit a project file on disk. BenchUp does not need to be running. |
| Live | `benchup-mcp live` | You want the agent to edit the project that is open in BenchUp, on the desktop or in the browser. You watch the changes appear. |

## Install the server

`benchup-mcp` is a native program. Build it from this repository (Rust 1.95 or newer):

```sh
# Install it into ~/.cargo/bin
cargo install --path crates/benchup-mcp

# Or build it in place
cargo build -p benchup-mcp --release     # produces target/release/benchup-mcp
```

If you build in place, give your agent the full path to the binary instead of `benchup-mcp`.

## File mode

```sh
benchup-mcp file ~/projects/side-table.bup
```

The server loads the project, runs each tool call, and saves the file after every call. Each call that changes the project is one undo step, labelled `Agent: <tool>`. If a save fails, the change is rolled back and the agent gets an error.

* Close the project in BenchUp first. Do not edit a file that BenchUp has open.
* If the file changes on disk while a session is open (for example you save it in BenchUp), the next change is **not saved**. It is rolled back and the agent gets "The project file changed on disk (was it saved in BenchUp?). Reopen it with a new benchup-mcp session." Start a new session to continue. This protects your work from being overwritten.
* Saving keeps symlinks and the file's permissions.
* A shared (server-linked) project is refused, because edits made offline would diverge from the server. `--allow-shared` edits the local copy anyway. There is no edit lease in file mode.

To add it to Claude Code:

```sh
claude mcp add benchup-file -- benchup-mcp file ~/projects/side-table.bup
```

## Live mode: pair BenchUp with your agent

In live mode the agent starts `benchup-mcp live`, and BenchUp connects to it. Both stay on your computer. Pairing uses a secret token.

1. Open BenchUp. Choose **Settings → Add-ons → Agent connection**.
2. Turn on **Allow agent control**. BenchUp creates a pairing token and stores it on this device. The status line reads "Connecting to the agent bridge…" and then retries until an agent starts the bridge.
3. Press **Copy Claude Code command** or **Copy MCP config (JSON)**.
4. Add the server to your agent:
   * **Claude Code.** Paste and run the command. It looks like this:

     ```sh
     claude mcp add benchup -e BENCHUP_AGENT_TOKEN=<TOKEN> -- benchup-mcp live
     ```

     The token travels in the environment, not in the program's arguments. `--token <TOKEN>` also works, but other users on the machine can see command-line arguments, so avoid it.
   * **Cursor and other clients.** Paste the JSON into the client's MCP configuration (for Cursor, `.cursor/mcp.json`). The token travels in the environment:

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
5. Start or restart the agent. When it starts the server, BenchUp connects within a few seconds. The status line reads "Connected to <agent name>", and the status bar shows "Agent connected".
6. Ask the agent to describe or build something. A good first request is "read the BenchUp scene".

Options of `benchup-mcp live`:

| Option | Meaning |
|---|---|
| `--port <n>` | Port to listen on. Default 47615. Set the same value in the **Port** field in BenchUp. The copied command and JSON include `--port` when it is not the default. |
| `--token <T>` | Pairing token. Discouraged, because command-line arguments are visible to other users. Prefer `BENCHUP_AGENT_TOKEN`, which the server reads when `--token` is absent. At least 16 characters. |
| `--allow-origin <origin>` | Allow another web origin to connect (repeatable). See [Browser notes](#browser-notes). |

Other controls in the section:

* **Show / Hide** reveals or masks the token. It is not remembered.
* **Copy token** copies only the token.
* **Regenerate** creates a new token. The old one stops working, and BenchUp drops the current connection. Restart your agent with the new token.
* **Port** changes the port. BenchUp reconnects when you change it.

The token and the on/off switch are remembered. Agent control is **off by default** and stays off until you turn it on. Build with MIRA has its own switch in **Settings → Add-ons** (on by default). The two do not depend on each other: the agent connection works with MIRA turned off, and then no Agent activity cards are shown.

## What an agent can do

Everything the 27 tools allow: read the scene, find and inspect objects, create boards, move, rotate, scale and resize, push and pull faces, group and ungroup, rename, set material, grain and tags, paint, add joints, dimensions and text, erase, select, and enter or leave an object. Start with [mira/tools.md](mira/tools.md) for the arguments, limits and errors of each tool. It is the same tool set that MIRA uses.

The server tells the agent to call `get_scene` first. It returns the model in millimetres.

In live mode every result also names the project that answered: `"document": {"name": "...", "epoch": N}`. The epoch changes when you open or replace a project. If the project changed while a call was waiting, the call is refused ("The open project changed before this call ran") and the agent should call `get_scene` again.

**Selection.** An external agent always sees your current selection (`get_selection`, `get_scene`). The "Include selection" setting of MIRA does not apply to it. If you do not want an agent to see what you have selected, do not connect it.

## Undo

Each call that changes the project is **one undo step**, labelled `Agent: <tool> (N new, M changed)` in live mode and `Agent: <tool>` in file mode. Use Undo in BenchUp as usual. An agent that makes ten changes makes ten undo steps, so you can step back through them one at a time.

Read-only calls (such as `get_scene`) add no undo step. A call that fails changes nothing.

## Working together with MIRA

You can use MIRA and an external agent in the same project.

* While a MIRA turn is running, agent calls **wait** in a queue. They run in order when the turn has finished. Nothing is interleaved.
* The same wait applies while BenchUp is opening or replacing a project.
* The queue holds 64 calls. A call that waits about 100 seconds gets a "BenchUp is busy with a MIRA turn, retry" error, so the agent can try again.
* Each agent change appears in the MIRA transcript as an **Agent activity** card. It has the same rows (click to select or zoom) and an Undo button for the latest step. Cards only appear when the Build with MIRA add-on is on.
* MIRA's next turn sees the agent's changes, because it reads the current scene.

## Shared projects

In live mode BenchUp applies its normal shared-project rules to the agent:

* If you only have read access, the agent can read but every edit is refused.
* If you need an edit lease, the agent can change only the objects you hold. Others are refused with `not-leased`. Select an object to take its lease.
* The agent cannot enter a group you do not hold or create geometry inside one. After every change BenchUp checks that nothing outside your lease was altered, and rolls the call back if it was. MIRA follows the same rules.
* If your edit access lapses in the middle of a call, the call is rolled back and the agent gets the reason.

File mode has no lease, which is why it refuses shared projects unless you pass `--allow-shared`.

## Security model

* **Local only.** The server listens on `127.0.0.1`. There is no option to listen on another address. BenchUp connects out to it.
* **Pairing token, never sent.** The token itself does not cross the connection. The server sends a random challenge, and each side proves it knows the token with an HMAC-SHA256 answer over both sides' random values. A wrong proof is rejected and the connection is closed. Pairing must finish within 5 seconds. The token is stored on this device only (a private file written atomically with mode `0600` on the desktop, `localStorage` in the browser) and is never saved in settings or project files.
* **Mutual authentication.** BenchUp also checks the server's proof before it accepts any call. A program that merely listens on the port cannot learn the token or send calls that BenchUp will run. It can only block the port; if pairing fails, change the port.
* **Limits.** Before pairing a connection may send at most 1 MiB (64 KiB per text frame), and at most 8 unpaired connections are held. After pairing, messages may be up to 32 MiB (BenchUp accepts up to 16 MiB).
* **Origin check.** For browser connections, the server accepts only these origins: none (the desktop app), `https://benchup3d.github.io`, `http://localhost:*`, `http://127.0.0.1:*`, and any `--allow-origin` values. Other web pages get HTTP 403. This stops an unrelated website from reaching the server.
* **Trust.** Any program running as your user can read the token file, so it is trusted in the same way. BenchUp does not defend against other programs that already run as you.
* **One agent connection.** The server accepts one BenchUp at a time. A newer authenticated connection replaces the older one.
* **Same limits as MIRA.** The agent cannot touch locked or hidden objects, cannot exceed the size limits, and cannot reach objects outside the current edit scope. A failed call changes nothing.
* **Timeouts do not leave surprises.** Each call carries a deadline. If the server gives up on a call it tells BenchUp to cancel it, and a late result is ignored, so a call the agent was told had failed is not applied afterwards. (A call that is already running finishes; call `get_scene` to check.)
* **Client names.** The agent's name shown in BenchUp is stripped of control and invisible characters and cut to 64 characters.
* **Edit scope.** If the agent enters an object (`enter_object`), BenchUp puts you back in the scope you were in after every call.
* **Off by default.** Turn **Allow agent control** off to stop it. Waiting calls are answered with "Agent control was turned off" and the connection is closed.
* **Threat model.** See [mira/persistence-and-security.md](mira/persistence-and-security.md#71-agent-bridge-threat-model).

## Browser notes

The web editor at <https://benchup3d.github.io> can connect to the server on your computer. The page opens `ws://127.0.0.1:47615`.

* Chrome may ask for permission to access devices on your local network the first time. Allow it, or the connection fails. If you refused, change the site permission in the address bar.
* Safari has not been tested.
* For `trunk serve` (development), the origins `http://localhost:*` and `http://127.0.0.1:*` are already allowed.
* If you host the web app on another origin, start the server with `--allow-origin https://your.host`.
* The browser keeps the token in `localStorage` for that origin. A token set up in the desktop app is not shared with the browser: open the web app and copy its own token.

## Troubleshooting

| What you see | Cause and fix |
|---|---|
| Agent says "The open project changed before this call ran" | You opened or replaced a project while the call was waiting. Call `get_scene` and repeat the request. |
| BenchUp says "The agent bridge uses an incompatible protocol. Update benchup-mcp…" | `benchup-mcp` is older or newer than BenchUp. Rebuild and reinstall it from the same version. |
| BenchUp says "The program on this port did not prove the pairing token" | Something other than your `benchup-mcp` is using the port, or the agent has an old token. Check that nothing else listens on the port, regenerate the token, or pick another port. |
| BenchUp says it could not create a pairing token (no secure random numbers) | The device could not provide secure randomness. Nothing connects until it can. |
| File mode: "The project file changed on disk" | The file was saved by someone else (for example BenchUp) after the session loaded it. Start a new session. |
| Agent says "BenchUp is not connected" | BenchUp is closed, **Allow agent control** is off, or the ports differ. Open **Settings → Add-ons → Agent connection**, turn it on, and check that **Port** matches `--port`. |
| BenchUp says "Not connected: The bridge refused the connection: pairing token does not match. Retrying." | The agent is using an old token. Copy the command or JSON again (after **Regenerate**) and restart the agent. |
| BenchUp keeps showing "Connecting to the agent bridge…" or "Not connected … Retrying" | No agent has started the server yet, or another program uses the port. Start the agent. BenchUp retries every 1 to 10 seconds. |
| `benchup-mcp` exits with "cannot listen on 127.0.0.1:47615" | The port is in use, often by another agent session. Close it, or pick a different `--port` and set the same value in BenchUp. |
| `benchup-mcp` exits with "a pairing token is required" or "must be at least 16 characters" | Set `BENCHUP_AGENT_TOKEN` with the copied value (or pass `--token`). |
| Agent says "BenchUp did not answer in time" | A call took longer than 120 seconds, or BenchUp is blocked by a dialog. Retry. Check the project with `get_scene`, because the change may have been applied. |
| Agent says "BenchUp is busy with a MIRA turn, retry" | A MIRA turn or a project load ran for more than about 100 seconds. Wait for it to finish, or press Stop in MIRA, and retry. |
| Agent says "Too many agent calls are waiting, retry" | More than 64 calls are queued behind a MIRA turn. Slow down and retry. |
| Agent says "BenchUp disconnected before answering" | BenchUp was closed, or you turned agent control off or regenerated the token. Reconnect, then check the project with `get_scene`. |
| Edits are refused with `read-only` or `not-leased` | The project is shared and you have no edit access or lease. See [Shared projects](#shared-projects). |
| In the browser, nothing connects | Check the Chrome permission above, and that the page origin is allowed. |
| File mode: "is a shared project" | The file is linked to a server. Use live mode, or pass `--allow-shared` and accept that the copy can diverge. |

## Where this is implemented

For contributors. Details are in the [contributor documentation](mira/README.md).

| Part | Location |
|---|---|
| Shared tool layer (tools, scene summary, `.bup` files) | `crates/benchup-tools/src/{ops,scene,files}.rs` |
| MCP server (stdio protocol, file mode, live bridge) | `crates/benchup-mcp/src/{protocol,file_mode,live}.rs`, `crates/benchup-mcp/README.md` |
| App side (host, connection, token) | `crates/benchup-app/src/{agent_host,credentials}.rs`, `crates/benchup-app/src/agent_link/` |
| Architecture and diagram | [mira/architecture.md](mira/architecture.md#31-mira-and-mcp-as-two-add-ons-on-one-tool-layer) |
| Threat model | [mira/persistence-and-security.md](mira/persistence-and-security.md#71-agent-bridge-threat-model) |

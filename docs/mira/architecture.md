# Build with MIRA: architecture

[Docs index](../README.md) · [User guide](../assistant.md) · Architecture · [Module reference](module-reference.md) · [Tool catalog](tools.md) · [LLM protocol](llm-protocol.md) · [UI/UX](ui-ux.md) · [Persistence and security](persistence-and-security.md) · [Testing](testing.md) · [Contributing](contributing.md) · [Development history](development-history.md) · [Roadmap](roadmap.md)


**Citation convention.** Every `path:line` is relative to `crates/benchup-app/src/` unless it starts with `benchup-core/`, `benchup-tools/` or `benchup-mcp/` (those are `crates/<name>/src/`). Citations in `benchup-tools`, `benchup-mcp`, `agent_host.rs`, `agent_link/` and `credentials.rs` are for branch `feature/agent-tools` at commit `91c017f`. The other lines were taken on `feature/build-with-mira` after the commit "Fix issues found while documenting MIRA" (parent `44a7cc9`); the agent commits moved a few lines in `assistant/mod.rs` and `assistant/panel.rs`. All of them will drift as code changes, so search for the symbol name if a line is off.

The end-user view of the feature is in [../assistant.md](../assistant.md). This page is about how it is built. The tool layer is shared with an MCP server for external agents; section 3.1 shows how the two fit together, and the user guide for that side is [../mcp.md](../mcp.md).

## 1. Purpose, goals, non-goals

"Build with MIRA" lets a designer type a plain-language request. An LLM reached over an OpenAI-compatible Chat Completions API answers with *tool calls*; the editor executes those calls against the live `Model` and shows what changed (module doc, `assistant/mod.rs:-1`).

Goals (each one is visible in code):

| Goal | Where it shows |
|---|---|
| The LLM changes the model only through a fixed tool catalog, never by free-form output | `tools::definitions` `benchup-tools/src/ops.rs:224`; `SYSTEM_PROMPT` `assistant/turn.rs:27` |
| One request equals exactly one undo step | `Assistant::poll`/`stop` return a `TurnResult`; `assistant_commit_turn` calls `commit_edit` (the logic of `finish_edit`) once (`assistant/mod.rs:458`, `app.rs:772`) |
| The UI thread never blocks | `ChatTransport` returns an `mpsc::Receiver` polled per frame (`assistant/client.rs:232`); `Turn::poll` uses `try_recv` (`assistant/turn.rs:203`) |
| Any OpenAI-compatible provider works; default preset is endlesstokens.ai | `DEFAULT_BASE_URL` `assistant/client.rs:9`, `valid_base_url` `assistant/client.rs:26` |
| A bad or hostile model reply cannot corrupt the model | per-call backup/rollback, argument validation, bounds, lease (`benchup-tools/src/ops.rs:1120`) |
| Works on desktop and in the browser (wasm32) | `cfg(target_arch)` split in `assistant/client.rs:285-357` and `assistant/credentials.rs:63-102` |
| The API key never lands in a project file or in preferences | `Preferences` has no key field (`assistant/mod.rs:40`); `assistant/credentials.rs:1` |
| Shared (collaborative) projects stay safe | `ToolEnv::read_only` / `lease` (`benchup-tools/src/ops.rs:33,45`), `Gate` (`assistant/turn.rs:69`), `discard_turn` (`assistant/mod.rs:307`) |

Non-goals (things the code does *not* do; verified by absence):

* No streaming: `ChatRequest.stream` is always `false` (`assistant/turn.rs:194`).
* No retry or back-off: nothing in the module retries a failed request (grep for `retry`/`429` finds only a tool hint string, `benchup-tools/src/ops.rs:501`).
* No cancellation of an in-flight HTTP request: `Stop` drops the receiver; the worker thread or fetch finishes on its own and its reply is discarded (`Turn::stop` `assistant/turn.rs:284`; the `let _ = sender.send(..)` at `assistant/client.rs:313,354`).
* No persistence of the transcript or of the model-facing history: both live in `Assistant` memory only (`assistant/mod.rs:90,95`).
* No tool-call memory across turns: `history` stores only user prompt text and the assistant's final summary, or a `(stopped)` / `(error: ...)` placeholder when there is none, so roles keep alternating (`Assistant::record` `assistant/mod.rs:391-406`).
* No image/vision input, no web access, no file access. The tools are listed in [tools.md](tools.md).
* No model-level (core) knowledge of the assistant: `benchup-core` has no assistant code and was not changed on this branch (`git diff 64b2a44..HEAD --stat` shows no `crates/benchup-core` files).

## 2. Where the code lives, and the layering

```
crates/
  benchup-core/            geometry + document model (no egui, no networking)
  benchup-app/src/
    app.rs                 BenchUpApp (the eframe::App), declares most modules via #[path]
    assistant/             <- this feature
      mod.rs               Assistant state machine + commit glue (impl BenchUpApp)
      client.rs            OpenAI-compatible HTTP client (+ test fake)
      credentials.rs       per-server API key slot (delegates to ../credentials.rs)
      tools.rs             OpenAI adapter + text fallback parser (LLM-specific only)
      turn.rs              one request: the round loop
      panel.rs             chat UI (impl BenchUpApp)
      window.rs            docked / detached hosting (impl BenchUpApp)
      tests.rs  ui_tests.rs
    agent_host.rs          shared by both add-ons: ToolEnv from app state, one external call = one undo step
    agent_link/            Agent connection add-on (app connects out to benchup-mcp)
    credentials.rs         secrets on this device (API key, pairing token)
  benchup-tools/src/
    ops.rs                 tool definitions + executor (JSON -> Model edits); was assistant/tools.rs
    scene.rs               model -> JSON for an agent (mm); was assistant/scene.rs
    files.rs               .bup encode/decode + atomic save; was in project_files.rs
    ops_tests.rs           tool tests; was assistant/tools_tests.rs
  benchup-mcp/src/         MCP server binary (stdio), file mode and live bridge
```

Layers:

| Layer | Crate / module | Knows about | Does not know about |
|---|---|---|---|
| Core | `benchup-core` | `Model`, entities, geometry, `JointSettings`, `tr!` | egui, HTTP, any agent |
| Tools | `benchup-tools` | core `Model`, `serde_json`, gzip | egui, HTTP, WebSocket, any LLM wire format |
| App | `benchup-app` (`BenchUpApp`, `finish_edit`, history, sharing, sidebar) | core, egui, wgpu | LLM wire format |
| Assistant module | `benchup-app/src/assistant/` | tools, a few `BenchUpApp` private fields, egui | rendering, geometry kernels |
| Agent host | `agent_host.rs`, `agent_link/` | tools, `BenchUpApp` private fields, the WebSocket client | the MCP protocol (the bridge speaks it) |
| MCP server | `benchup-mcp` (separate process) | tools, MCP, WebSocket server | egui, the app |

Inside the module the dependency direction is: `panel`/`window` (UI) -> `mod.rs` (`Assistant`) -> `turn` -> `benchup_tools` (via the thin `tools` adapter) + `client`. `credentials` is used only by `mod.rs`. `benchup-tools` knows nothing about `client`: the adapter `assistant/tools.rs:8` turns the OpenAI `ToolCall` into a neutral `Call`.

### Why the module is declared from `app.rs` via `#[path]`

`app.rs` is the file that defines `BenchUpApp` (`app.rs:334`) and all its fields are private to the `app` module. Rust privacy lets a *descendant* module read private items of its ancestors. The assistant is therefore declared as a child of `app`:

```rust
// app.rs:99-100
#[path = "assistant/mod.rs"]
mod assistant;
```

`app.rs` declares its siblings the same way (flat `#[path = "fasteners.rs"] mod fasteners;` etc., for example `app.rs:106-107`), because the crate root `lib.rs:4` has plain `mod app;` and `app.rs` is not a `mod.rs` file. The `#[path]` is what lets the module live in a sub-directory while still being a child of `app`. The consequences, all used by the code:

* `assistant/mod.rs:6` does `use super::*;`, which brings every `app.rs` import and private item into scope (for example `Model`, `BTreeSet`, `egui`, `BenchUpApp`).
* The module can implement `impl BenchUpApp` blocks that touch private fields: `self.doc.model`, `self.selected`, `self.general_settings`, `self.assistant`, `self.history_preview` etc. (`assistant/mod.rs:410-490`, `assistant/panel.rs:152`, `assistant/window.rs:78`).
* Panel code reaches sibling app modules with `super::super::` (for example `super::super::sidebar`, `super::super::searchable_selector`, `assistant/panel.rs:4-8`): `panel` -> `assistant` -> `app`.
* Submodules inside `assistant/` use `#[path = "client.rs"]` again (`assistant/mod.rs:12-27`) so each file keeps its flat name next to `mod.rs`.

The cost: the assistant is tightly coupled to `BenchUpApp` internals. The pure parts (`client`, `benchup-tools`, `turn`, `credentials`) deliberately avoid that coupling and take explicit arguments (`&mut Model`, `Gate`, `TurnSetup`), which is what makes them unit-testable without an app.

## 3. Component diagram

```mermaid
flowchart LR
  subgraph Core["benchup-core"]
    M["Model\n(geometry, objects, tags, joints)"]
  end
  subgraph App["benchup-app (app.rs and friends)"]
    A["BenchUpApp\n(doc.model, selected, history)"]
    FE["finish_edit\n(app.rs:772)"]
    SH["sharing.rs\nshared_edit_allowed, shared_lease,\napply_shared_remote"]
    SB["sidebar.rs\nInspectorPreferences.assistant"]
  end
  subgraph MIRA["assistant module"]
    MOD["mod.rs\nAssistant + commit glue"]
    TURN["turn.rs\nTurn (round loop)"]
    TOOLS["tools.rs\nOpenAI adapter"]
    BT["benchup-tools\nops.rs, scene.rs"]
    CLIENT["client.rs\nChatTransport / HttpTransport"]
    CRED["credentials.rs\nkey store"]
    PANEL["panel.rs\nchat UI"]
    WIN["window.rs\ndock / detach"]
  end
  LLM[("OpenAI-compatible\nserver")]
  FS[("data dir file /\nlocalStorage")]

  PANEL --> MOD
  WIN --> PANEL
  A -->|"poll_assistant each frame"| MOD
  MOD --> TURN
  TURN --> TOOLS
  TOOLS --> BT
  TURN --> CLIENT
  BT --> M
  CLIENT <-->|"HTTPS JSON"| LLM
  MOD --> CRED
  CRED --> FS
  MOD -->|"assistant_commit_turn"| FE
  FE --> A
  SH -->|"discard_turn, Gate inputs"| MOD
  SB -->|"restore / preferences"| MOD
```

### 3.1 MIRA and MCP as two add-ons on one tool layer

External agents (Claude Code, Cursor, any MCP client) edit the same model through the same tools. Both add-ons call `benchup_tools::execute`; neither owns the tools. The app-side glue they share is `agent_host.rs`.

```mermaid
flowchart LR
  subgraph Ext["Outside the app"]
    AG["MCP client\n(Claude Code, Cursor, ...)"]
    MCP["benchup-mcp\nstdio MCP server\nmodes: file, live"]
    BUP[("project.bup")]
  end
  subgraph App["benchup-app"]
    LINK["agent_link/\nAgent connection add-on"]
    MIRA["assistant/\nBuild with MIRA add-on"]
    HOST["agent_host.rs\ntool_env, run_agent_call,\ncommit_edit (one undo step)"]
    M2[("Model")]
  end
  subgraph Tools["benchup-tools (shared crate)"]
    OPS["ops.rs\ndefinitions + execute"]
    SC["scene.rs"]
    FI["files.rs"]
  end
  LLM2[("LLM endpoint")]
  AG <-->|"MCP over stdio"| MCP
  MCP -->|"file mode"| FI
  FI <--> BUP
  MCP -->|"file mode"| OPS
  MCP <-->|"live mode: ws://127.0.0.1\n(app connects out)"| LINK
  LINK --> HOST
  MIRA --> HOST
  MIRA <-->|"HTTPS"| LLM2
  HOST --> OPS
  OPS --> SC
  OPS --> M2
```

* **Crates.** `benchup-tools` (library, builds for `wasm32`) holds the 27 tools, the scene summary and `.bup` reading/writing. `benchup-mcp` (binary, native only) wraps it in an MCP server. The app depends on `benchup-tools` and does not depend on `benchup-mcp`.
* **File mode** runs entirely in the `benchup-mcp` process: decode the `.bup`, run each call against the document, record one history step `Agent: <tool>`, save atomically (`benchup-mcp/src/file_mode.rs:84-114`).
* **Live mode.** `benchup-mcp live` listens on `127.0.0.1` only (`Bridge::start`, `benchup-mcp/src/live.rs`). The app connects out to it with `realtime::Socket::connect_limited` (`pump`, `agent_link/mod.rs`). The handshake is protocol v2 and mutual (`benchup-tools/src/pairing.rs`): the app sends a hello with an empty ticket, the bridge answers with a `challenge` (nonce, protocol 2), the app sends `prove` (its own nonce and an HMAC-SHA256 proof over both nonces), and the bridge answers `ready` with its own proof, which the app must verify. The raw token is never sent. The bridge then forwards each `tools/call` as a `call` frame carrying `deadline_ms`; the app answers with a `result` frame. On timeout the bridge sends `cancel` and ignores a late result. The app is the WebSocket client so that the browser build works too: a web page can open `ws://127.0.0.1` but cannot listen. Frame sizes and the unauthenticated-connection cap are in [persistence-and-security.md](persistence-and-security.md#71-agent-bridge-threat-model).
* **Stale calls.** Each queued call remembers the document epoch (`BenchUpApp::doc_epoch`) at arrival. If the open project changed before it runs, it is refused with "The open project changed before this call ran" and the hint to call `get_scene`. Every agent result carries `"document": {"name", "epoch"}` (`AgentLink::result_frame`). The app drops a call whose deadline has almost passed and removes a queued call when `cancel` arrives.
* **One call, one undo step.** `run_agent_call` (`agent_host.rs:88`) snapshots the model, runs `benchup_tools::execute`, and commits through `commit_edit` with the label `Agent: <summary>`. If the commit is refused (shared edit access lapsed), the edit is rolled back and the reason is returned (`agent_host.rs:122-132`).
* **Cooperation with MIRA.** `poll_agent_link` (`agent_link/mod.rs:552`) does not run a call while `assistant.busy()` or a file operation is replacing the model. Calls wait in a FIFO queue (limit 64, `:41`) and run in order afterwards. A call that waits more than 100 s is answered with a "busy, retry" error before the bridge's 120 s timeout (`:40`, `expire` `:526`). Each applied call is recorded in the MIRA transcript as an "Agent activity" card (`Assistant::record_agent`, `assistant/mod.rs:343`), when MIRA is on, with the same select/zoom rows and Undo as a MIRA change card. MIRA's next turn sees the agent's changes through the scene summary.
* **Same rules for both.** `tool_env` (`agent_host.rs:46`) builds the environment once: the purchasing catalog, `read_only` from `shared_edit_allowed`, the lease from `shared_lease`, and the selection. Limits, locks, lease and scope checks live in `ops.rs`, so they cannot differ between the add-ons.
* **Boundary: Cargo features and runtime toggles.**
  * *Features.* `benchup-app` has the Cargo features `assistant` and `agent-link`, both on by default (`Cargo.toml`). `app.rs:92-105` declares either the real module or a stub with the same small surface: `assistant/stub.rs` or `agent_link/stub.rs`. Call sites need no `cfg`. In a stub build the panel, menu items and polling are empty functions, and `agent_link_settings` draws nothing.
  * *Saved preferences survive.* The preference types (`AssistantPreferences`, `WindowPreferences`, `AgentPreferences`) live in the always-compiled `addons.rs`, so a build without an add-on reads its saved settings and writes them back unchanged instead of dropping them.
  * *Shared types.* `Changes` (the change-card data) moved to `agent_host.rs:15` so both add-ons and the stubs use it.
  * *Runtime toggles* in Settings > Add-ons (`settings.rs:428-441`). "Build with MIRA" is `GeneralSettings.assistant_enabled`, default on (`settings.rs:189`). "Allow agent control" is `AgentPreferences.enabled`, default off. Both are in `GeneralSettings`, which is saved without a model edit.
  * *MIRA off* (`BenchUpApp::assistant_available`, `agent_host.rs:35`): a running turn is stopped, and what it already applied stays as one "MIRA (stopped)" undo step. The View menu item, the detached window, the context-menu item and the shortcut row are hidden. The shortcut chord is still consumed but does nothing. The Assistant inspector section is skipped when drawing, but not removed (`section_hidden`, `agent_host.rs:40`), so the saved section order does not change.
  * *Agent link with MIRA off.* It keeps working. It records no transcript, because there is none.
  * *Add-on compiled out.* Its row in Settings > Add-ons is hidden. With both out, the section shows "No add-ons are included in this build." (`settings.no_add_ons`).

## 4. One turn, end to end

```mermaid
sequenceDiagram
  autonumber
  actor U as Designer
  participant P as panel.rs (composer)
  participant AP as BenchUpApp (mod.rs impl)
  participant AS as Assistant
  participant T as Turn (turn.rs)
  participant C as ChatTransport
  participant L as LLM server
  participant X as tools::execute
  participant H as finish_edit / History

  U->>P: types text with @Name, presses Enter
  P->>P: rewrite_prompt: "@Name" -> "@Name [#id]", collect ids
  P->>AP: assistant_submit -> assistant_send(Prompt)
  AP->>AS: send(model, selection, catalog, read_only, lease, prompt)
  AS->>T: Turn::start (builds scene + references JSON)
  T->>C: chat(request: 2 system msgs + history + user, tools, tool_choice=auto)
  C-->>L: POST /chat/completions (thread or fetch)
  loop each frame
    AP->>AS: poll(model, Gate)
    AS->>T: poll: try_recv
  end
  L-->>C: choices[0].message {content, tool_calls}
  C-->>T: reply on mpsc channel
  T->>T: gate.block? (shared project) -> finish(stopped)
  loop up to 32 calls in the reply
    T->>T: mutating call? gate.recheck() -> stop if access lapsed
    T->>X: execute(model, call, env)
    X->>X: read-only / lease checks, backup = model.clone()
    X->>X: run tool, range check, rollback on failure
    X-->>T: ToolOutcome (result JSON, ok, mutated, ids)
    T->>T: append tool message (<= 8000 chars, valid JSON)
  end
  T->>C: chat(...) again (re-request) until text-only reply or 12 rounds
  T-->>AS: Poll{finished: TurnResult{before, summary, ids}}
  AS->>AS: turn = None  (busy gate released)
  AS-->>AP: Poll
  AP->>AP: assistant_commit_turn(result)
  AP->>H: commit_edit("MIRA: <summary>", before)  (exactly one step, or rolled back)
  AP->>AP: select touched entities, assistant.record(result)
  AP->>P: transcript Entry with Changes (change card, history position)
```

Step-by-step with citations:

1. **Prompt and references.** `assistant_submit` (`assistant/panel.rs:189`) calls `assistant_compose` (`:178`) -> `rewrite_prompt` (`:105`). Typed `@Name` that matches a chip becomes `@Name [#id]`; chips never typed into the text are appended as `Also referenced: ...` (`:136-143`). The id set (every chip, `:142`) becomes `Prompt.references` (`assistant/mod.rs:83`).
2. **Pre-flight.** `Assistant::send` (`assistant/mod.rs:260`) fails fast with a user-facing string if busy, prompt empty, URL invalid, no key, or no model (`:309-321`). Failures are shown as `ui.notice` (`assistant/panel.rs:205`) and **do not** create a transcript entry.
3. **Setup.** `BenchUpApp::assistant_send` (`assistant/mod.rs:446`) snapshots the stock catalog from `general_settings.stock.wood`, `read_only = !shared_edit_allowed(ctx)` and `lease = shared_lease()` (`sharing.rs:738,752`). `Turn::start` (`assistant/turn.rs:129`) merges selection into references when `include_selection`; when it is off the selection is left out of the scene summary too and `ToolEnv.hide_selection` makes `get_selection` answer with a note (`:138-141`). It renders `references` within `REFERENCE_BUDGET` (`:25`) and `scene::summary` within the remainder, and builds the first request. Details: [llm-protocol.md](llm-protocol.md).
4. **Request.** `Turn::request` (`assistant/turn.rs:188`) clones `messages` + `tools` into a `ChatRequest` and calls `transport.chat`, which returns a `Receiver` at once.
5. **Poll.** Every frame `poll_assistant` (`assistant/mod.rs:412`, called from `app.rs:6455`) builds a `Gate`, calls `Assistant::poll` -> `Turn::poll`. If the reply is not ready it returns idle and `Assistant::poll` schedules `request_repaint_after(100 ms)` (`assistant/mod.rs:313-315`).
6. **Execute.** For each call (max 32 per reply, `MAX_CALLS_PER_REPLY` `assistant/turn.rs:23`) `tools::execute` (`benchup-tools/src/ops.rs:1120`) applies it; the JSON result goes back as a `tool` message; one longer than `MAX_TOOL_RESULT_CHARS` is replaced by `{"ok":..,"truncated":true,"preview":"..."}`, which is still valid JSON (`truncated_result`, `assistant/turn.rs:20,115`). Before each mutating call the turn asks `Gate.recheck` whether editing is still allowed (`:252-256`). `mutated` makes `poll_assistant` call `self.changed(ctx)` so the viewport refreshes (`assistant/mod.rs:439`).
7. **Re-request.** After the batch, `rounds += 1` and either the round cap ends the turn or `request` is called again with the grown message list (`assistant/turn.rs:274-280`). The *scene JSON in the second system message is not refreshed*; the model learns about changes from tool results or by calling `get_scene`.
8. **Finish.** A reply with no tool calls ends the turn with that text as the summary (`:242-244`). `Turn::finish` (`:289`) builds the `TurnResult`: ids filtered to what still exists, ids created-then-erased dropped, default summary "Done." when the text is empty, and `before` taken from `ToolEnv`.
9. **Commit.** `assistant_commit_turn` (`assistant/mod.rs:458`) puts the user back in the edit scope they had when the turn began (`restore_scope`, `:434`), calls `commit_edit(label, before)` once (a wrapper of `finish_edit` that reports whether the step was recorded), selects touched top-level entities, records the transcript and history, and stores the history position on the card, only when this turn produced a card (`:538-551`). If the edit was refused and rolled back, the card is dropped and an error entry says so.
10. **Change card.** `change_card` (`assistant/panel.rs:894`) lists created/modified/joints and erased names (max 12 rows in total, `CARD_ROWS` `:64`; a final "+N more" counts every category not shown) and enables "Undo turn" only if `turn_undoable` (`:986`).

## 5. Turn state machine

`Turn` has no state enum. The states are implied by two facts: whether `Assistant.turn` is `Some` (`assistant/mod.rs:96`), and whether `Turn.pending` is `Some` (`assistant/turn.rs:95`). Everything between "reply arrived" and "next request sent / turn finished" happens inside a single call to `Turn::poll`, so it is atomic from the UI's point of view.

```mermaid
stateDiagram-v2
  [*] --> Idle
  Idle --> AwaitingReply: Assistant.send -> Turn.start -> request
  AwaitingReply --> AwaitingReply: poll, channel empty (repaint in 100 ms)
  AwaitingReply --> Processing: reply received
  Processing --> Finished: transport error (edits kept)
  Processing --> Finished: no tool calls (summary = text)
  Processing --> Finished: gate.block set and calls present (stopped, reason as error)
  Processing --> Finished: rounds >= MAX_ROUNDS (12) after executing the batch (error)
  Processing --> AwaitingReply: batch executed, rounds < 12, request again
  AwaitingReply --> Finished: Stop / Esc / banner button (Turn.stop)
  AwaitingReply --> Discarded: discard_turn (shared project replaced)
  AwaitingReply --> Finished: new_model / open project -> assistant_stop
  Finished --> Idle: assistant_commit_turn + record
  Discarded --> Idle: nothing committed, error entry
```

| Transition | Code | Notes |
|---|---|---|
| Start | `Turn::start` `assistant/turn.rs:129`, `Assistant::send` `assistant/mod.rs:260` | `started_at` is `ctx.input(\|i\| i.time)` (`:182`) |
| Poll, no reply | `TryRecvError::Empty` -> idle `assistant/turn.rs:205` | |
| Channel dropped | `TryRecvError::Disconnected` -> `ChatError::Network("The request was interrupted")` `:206-208` | e.g. worker thread panicked |
| Transport/parse error | `finish(.., Some(error.user_message()), false)` `:214` | Tool edits already applied are **kept** and still committed |
| Text-only reply | `finish(model, content, None, false)` `:243` | Empty text becomes "Done." (`:303-305`) |
| Gate blocked | `:229-234` | Returns `stopped = true` with the block reason as the error. Evaluated when the reply has tool calls |
| Gate re-check | `:252-256` | Before each mutating call of a reply, `gate.recheck` is asked again; a reason ends the turn as stopped before that call runs |
| Round limit | `MAX_ROUNDS = 12` (`:17`); check at `:275-278` | The 12th batch **is executed** before stopping. Error text from `assistant.error.round_limit` |
| Stop | `Assistant::stop` `assistant/mod.rs:321` -> `Turn::stop` `assistant/turn.rs:284` | `pending = None`; result carries `stopped = true`; edits kept |
| `discard_turn` | `assistant/mod.rs:328` | Calls `turn.stop`, puts the live model back in its starting scope, clears `before`, `selection` and all id lists, sets the error, records. Nothing reaches `finish_edit` |

Where Stop comes from: the composer Stop button (`assistant/panel.rs:767`), the busy banner button (`:1019`), Esc while busy (`app.rs:2179-2182`), `new_model` (`app.rs:1014`) and `open_project`-style loading (`projects.rs:166`).

**Gate re-check.** While busy, `poll_assistant` computes a `Gate` *every frame* (`assistant/mod.rs:412-437`): `block` is `Some(shared_edit_block_reason)` when `shared_edit_allowed` is false, `lease` is the current `shared_lease()`, and `recheck` is a closure that asks the same question again. Because the closure needs `&self` while the assistant and the model are borrowed mutably, `poll_assistant` temporarily moves both out of `self` (`std::mem::take`) for the duration of `Assistant::poll`; the check reads only sharing state, never the assistant or the model. `Turn::poll` applies the gate in three places: when a reply with tool calls arrives and `gate.block` is set, the turn ends instead of executing; otherwise `env.lease` is overwritten with the fresh lease so a lease that changed between rounds is enforced; and before *each mutating call* in the batch `recheck` is asked again, so access that lapses inside a 32-call batch stops the turn before the next edit. Remaining limitation: the lease itself is refreshed once per reply (see [section 12](#12-gaps--unclear)).

## 6. Threading model

Everything that reads or writes the `Model` runs on the UI thread, inside `Turn::poll` and `tools::execute`. Only the network I/O is off-thread.

```mermaid
flowchart TB
  subgraph UI["UI thread (egui frame loop)"]
    PA["poll_assistant"] --> TP["Turn::poll\ntry_recv"]
    TP --> EX["tools::execute\nmutates Model"]
    TP --> RQ["Turn::request"]
  end
  subgraph Native["native"]
    TH["std::thread::spawn\nureq call"]
  end
  subgraph Web["wasm32"]
    FT["js fetch Promise\nwasm_bindgen_futures::spawn_local"]
  end
  RQ -->|"spawn()"| TH
  RQ -->|"spawn()"| FT
  TH -->|"mpsc::Sender.send"| TP
  FT -->|"mpsc::Sender.send"| TP
  TH -->|"ctx.request_repaint()"| PA
  FT -->|"ctx.request_repaint()"| PA
```

| | Native | Web (wasm32) |
|---|---|---|
| Worker | `std::thread::spawn` per request (`assistant/client.rs:286`) | JS `fetch` wrapped by `js_sys::Function`, awaited in `wasm_bindgen_futures::spawn_local` (`:321-356`) |
| Result delivery | `sender.send(result)` then `ctx.request_repaint()` (`:313-314`) | same, from the async block (`:354-355`) |
| Timeout | `ureq` `timeout_global` 120 s (`:289`) | `AbortSignal.timeout(120000)` (`:329`) |
| Redirects | `max_redirects(0)` (`:290`) | `redirect: 'error'` (`:329`) |
| Size cap | `.limit(RESPONSE_LIMIT)` 4 MiB (`:308`) | `Content-Length` check + body length check at 4194304 (`:327-330`) |
| Panic in a tool | `catch_unwind` -> rollback (`benchup-tools/src/ops.rs:1152`) | A panic aborts; **no recovery** (comment `benchup-tools/src/ops.rs:1149-1151`) |

The receiver is the only synchronisation primitive. Because the poll loop is `try_recv`, there is no lock, no `Arc<Mutex<Model>>`, and no way for a worker thread to see the model. `Model` is also never sent to a worker: `Turn` holds `messages: Vec<Message>` (JSON-ish data) and a cloned `ChatRequest` crosses the thread boundary.

`Assistant` is also polled for the model-list request (`poll_models`, `assistant/mod.rs:196`), through the same channel pattern (`models_receiver`, `:136`).

The detached window on desktop is an *immediate viewport* (`ctx.show_viewport_immediate`, `assistant/window.rs:258`) so the closure can borrow `&mut self`; everything still runs on the UI thread.

## 7. Undo and commit semantics

**Exactly one `finish_edit` per turn**, and none when the turn changed nothing.

```mermaid
flowchart TD
  C["tools::execute (mutating call)"] --> B["backup = model.clone()"]
  B --> R["run tool inside catch_unwind"]
  R -->|"Err or panic"| RB["*model = backup\nerror outcome"]
  R -->|"Ok, out of range"| RB2["*model = backup\n'result out of range'"]
  R -->|"Ok"| S{"scene changed?\n!backup.same_scene(model)"}
  S -->|"yes and env.before is None"| SET["env.before = Some(backup)\n(first change only)"]
  S -->|"yes, before already set"| DROP["backup dropped"]
  S -->|"no"| DROP
  SET --> FIN["Turn::finish: before = env.before.take()"]
  DROP --> FIN
  FIN --> CM["assistant_commit_turn:\nfinish_edit(label, before)"]
```

* **Lazy `before` snapshot.** `ToolEnv.before` (`benchup-tools/src/ops.rs:42`) is `None` until the first call that actually changes the scene: `mutated = !backup.same_scene(model)` and `if mutated && env.before.is_none() { env.before = Some(backup) }` (`benchup-tools/src/ops.rs:1182-1186`). Read tools, failed calls and no-op calls never set it. `Model::same_scene` (`benchup-core/src/document.rs:113`) deliberately ignores the editing scope (`edit_context`).
* **Per-tool rollback.** Mutating tools take `model.clone()` before running (`benchup-tools/src/ops.rs:1148`). On `Err`, panic (native only), or out-of-range result, `*model = backup` restores it exactly (`:1159-1174`). `tools_tests.rs::err` asserts the model is unchanged on every error.
* **Commit.** `assistant_commit_turn` (`assistant/mod.rs:458`):
  1. `erased_names` are read from `before` (the entities are gone from the live model) (`:490-494`).
  2. `restore_scope(&mut self.doc.model, result.scope)` (`:497`) puts the designer back in the scope they had at turn start; the snapshot `before` already carries that scope (`Turn::finish`, `assistant/turn.rs:300`), so the recorded step has no scope change and undoing it leaves the designer where they were.
  3. Label: `assistant.history_stopped` ("MIRA (stopped)") if `result.stopped`, else `assistant.history_label` = "MIRA: {summary}" with the summary cut to 60 chars and trimmed (`:500-505`).
  4. `commit_edit(&label, before, ctx)` (`:506`, definition `app.rs:777`; `finish_edit` is its `()`-returning wrapper) records a single `history.record_applied_attributed` and returns `true`. It refuses and reverts (returning `false`) when `assistant.busy()` - safe here because the turn was already removed (`self.turn = None` in `poll`/`stop`) - and when `!shared_edit_allowed`. If the edit was real (`before != model`) but not recorded, the turn's id lists and selection are cleared and an error entry (`assistant.error.rolled_back`) is appended, so no change card claims the rolled-back edits (`:507-516`).
  5. Touched top-level, visible entities become the selection (`:518-526`); a `select` tool call (`TurnResult.selection`) then overrides it, filtered to existing, visible ids (`:528-532`).
  6. `assistant.record` appends transcript + history and returns whether it added a change card; only then does the newest card get `position = history.position()` (if `recorded`), so a later turn with no changes leaves earlier cards, and their Undo turn, intact (`:538-551`).
* **Stop** (button, banner, Esc): `Turn::stop` -> `finish(.., stopped = true)`. Edits already applied are committed as one step labelled "MIRA (stopped)". If nothing changed, `before` is `None`, nothing is recorded in history, and the transcript only gets an entry if there is a summary, changes or error (`Assistant::record` `assistant/mod.rs:358-406`).
* **Errors mid-turn** (network failure, round limit) also keep the partial work as one step with the normal "MIRA: ..." label (summary may be empty -> label "MIRA:").
* **Discard** (remote replacement): no commit at all (section 9).
* **Undo turn** in the card calls the normal `undo` (`assistant/panel.rs:550`), which goes through `navigate_history`. It is offered only while `changes.position == history.position() == latest` (`turn_undoable` `:986`). `navigate_history` itself refuses while busy (`app.rs:833`).
* **`catch_unwind` and wasm.** `std::panic::catch_unwind(AssertUnwindSafe(..))` wraps `run` (`benchup-tools/src/ops.rs:1152`). The code comment states that `catch_unwind` cannot help on wasm32, where a panic aborts (`benchup-tools/src/ops.rs:1149`), so every tool validates counts, ranges and lengths *before* touching the model, and results are range-checked afterwards (`MAX_RESULT_M`). The user guide says the page must be reloaded after such a crash ([../assistant.md](../assistant.md)). On native a caught panic returns the error "internal error while applying the tool" (`:1152`) after restoring the backup, but the default panic hook may still print to stderr.
* **Edit scope is restored.** `enter_object` / `exit_to_root` write `model.edit_context` directly and are classified non-mutating (`benchup-tools/src/ops.rs:122-123`), so they take no snapshot. `Turn` records the scope at start (`Turn.scope`, `assistant/turn.rs:174`); when the turn ends, is stopped or is discarded the app puts the user back in it (`restore_scope`, called from `assistant_commit_turn` and `discard_turn`), and `before.edit_context` is set to it, so undoing a MIRA turn leaves the user in the scope they were in. If that object no longer exists the user lands at the top level. Tests: `a_turn_puts_the_designer_back_in_their_edit_scope_and_undo_keeps_it`.

## 8. The busy gate

"Busy" is simply `Assistant::busy() = turn.is_some()` (`assistant/mod.rs:139`). While busy:

| What | Where |
|---|---|
| Editor-driven model changes are reverted with a message `assistant.busy_blocked` | `finish_edit` `app.rs:781-785` |
| Undo/redo/history navigation refused | `navigate_history` `app.rs:833` |
| Keyboard: only Esc (stops the turn) is processed; the toggle shortcut is handled before | `app.rs:2175-2183` |
| Pointer: primary-button gestures are ignored unless the tool is Select or Orbit (camera, middle button and selecting remain) | `app.rs:4531-4537` |
| Composer Enter does not submit; `send` returns "still working" | `assistant/panel.rs:654`, `assistant/mod.rs:271` |
| Model picker cannot change the model; "New chat" is disabled | `assistant/panel.rs:370`, `:296`; `Assistant::new_chat` `assistant/mod.rs:189` |
| A banner with a spinner and Stop is drawn | `assistant_busy_banner` `assistant/panel.rs:1002` |
| Starting a turn cancels in-progress tool gestures so they cannot collide | `assistant_cancel_interactions` `assistant/panel.rs:209` (called from `assistant_submit` `:195`) |

The gate is implemented at the *app entry points* listed above, not inside `Model`. I did not audit every code path that can mutate `doc.model` (for example sidebar edits that bypass `finish_edit`); see [Gaps](#12-gaps--unclear).

## 9. Shared-project behaviour

Three independent mechanisms, all on the UI thread:

1. **Read-only.** `assistant_send` sets `read_only = !shared_edit_allowed(ctx)` (`assistant/mod.rs:448`). `shared_edit_allowed` (`sharing.rs:738`) is true for non-shared projects, otherwise requires role owner/edit, no conflict, a baseline, a server acknowledgement in the last 10 s, and the selection to equal the confirmed lease selection. When `read_only`, every mutating tool returns `{"ok":false,"error":"read-only","hint":...}` before anything else (`benchup-tools/src/ops.rs:1127-1132`). Read tools still work.
2. **Lease.** `shared_lease()` (`sharing.rs:752`) is `Some(sharing.confirmed.clone())` for shared projects, `None` otherwise. `execute` rejects a mutating call whose *existing* target ids (`ids`, `id`, `face_id`, `a`, `b`) are outside the lease, with `"not-leased"` and the hint "select it first so you hold the edit lease" (`benchup-tools/src/ops.rs:1135-1139`). An id is leased if it or any ancestor is in the lease, or was created (or released by `ungroup`) by this turn (`leased` `:1030`, `env.created` `:1176`). Pure creators (`CREATING`, `:1015`) are exempt. The non-mutating `select` tool is held to the same rule: with a lease, it refuses ids that are neither leased nor created this turn (`:963`); `create_copy` can still *read* an unleased source, and the copy is new.
3. **Remote replacement.** `apply_shared_remote` calls `assistant.discard_turn(...)` *before* replacing `doc.model` (`sharing.rs:376`). The turn is stopped, its snapshot, selection and id lists are cleared, the error "The shared project changed; MIRA stopped" is recorded, and nothing is committed (`assistant/mod.rs:304-313`), so a stale `before` can never land in the remote history; the live model's edit scope is restored first so the scope the turn opened is not carried into the replaced project. Test: `sharing.rs:1204-1232` (`a_remote_replacement_discards_the_running_mira_turn`).

External agents get the same three mechanisms. `run_agent_call` builds its environment with the same `tool_env` (read-only and lease included), so a read-only shared project refuses agent edits and a lease limits which entities an agent may touch (tests `a_shared_project_without_edit_access_refuses_agent_edits_but_allows_reads` and `the_edit_lease_is_enforced_for_agent_calls`, `agent_link/tests.rs:500,483`). There is no turn to discard: each call is committed or rolled back on its own.

In addition the per-frame `Gate` (section 5) stops a running turn when edit access lapses, and it is re-checked before each mutating call. If access lapses between the last edit and the commit, `commit_edit` reverts the model, the change card is not shown, and the transcript gets the error `assistant.error.rolled_back` (test `a_rolled_back_turn_does_not_claim_its_edits_on_the_change_card`).

## 10. Error-handling philosophy

* **Tool failures are data for the model, not exceptions.** Every failure becomes `{"ok":false,"error":..,"hint":..}` and goes back as the tool message so the model can self-correct in the same turn (`error_outcome` `benchup-tools/src/ops.rs:997`; module doc `:1-4`). Hints name the next tool to call.
* **Fail closed, restore exactly.** A failed or out-of-range mutating call restores a full model clone. The cost is one `Model::clone` per mutating call; this is a deliberate trade for correctness (unmeasured here).
* **Validate before touching the model.** Counts, lengths, finite numbers and coordinate bounds are checked up front (`MAX_IDS`, `MAX_POINTS`, `MAX_TEXT_CHARS`, `MAX_COORDINATE_MM`, scale bounds; `benchup-tools/src/ops.rs:17-27`), because on wasm there is no panic recovery.
* **Transport errors are typed** (`ChatError`, `assistant/client.rs:42`) and mapped to user-facing English strings via `tr!` (`user_message` `:53`). They end the turn but keep partial work.
* **Server error text is untrusted:** cut to 300 chars (`MAX_ERROR_CHARS` `assistant/client.rs:17`).
* **User-input problems never start a turn:** `send` returns `Err(String)` shown as a notice (`assistant/mod.rs:260-283`).
* **Credentials errors are non-fatal:** if saving the validated key fails the user is still connected for the session and gets an error entry (`assistant/mod.rs:227-234`).
* **Untrusted shape:** unknown tool names, extra argument fields (`deny_unknown_fields`), string-or-object `arguments`, missing tool-call ids, and fenced-JSON tool calls are all tolerated or refused explicitly (`assistant/client.rs:76-83`, `assistant/turn.rs:224-228`, `benchup-tools/src/ops.rs:1222`).
* All user-facing text goes through `tr!` and lives in `assets/i18n/en.json` (86 `assistant.*` keys at this commit). Tool `error`/`hint` strings sent to the model are *not* translated (they are plain Rust strings).

## 11. Extension points

| You want to... | Touch | Notes |
|---|---|---|
| Add a tool | `benchup-tools/src/ops.rs`: `TOOLS` (`:96`), `definitions()` (`:224`), an `*Args` struct (`:328-472`), the tool fn, the `run` match (`:914`) | `tool_definitions_are_strict_and_match_the_executor` (`benchup-tools/src/ops_tests.rs:304`) fails if schema and executor disagree. Read the lease rule: if the tool only creates, add to `CREATING` (`benchup-tools/src/ops.rs:1015`); if it takes ids under new key names, extend `target_ids` (`benchup-tools/src/ops.rs:1018`). Full procedure in [contributing.md](contributing.md) and [tools.md](tools.md) |
| Change the prompt | `SYSTEM_PROMPT` (`assistant/turn.rs:27`) | quoted in [llm-protocol.md](llm-protocol.md) |
| Change what the model sees of the scene | `scene::summary`, `object_entry`, `describe` (`benchup-tools/src/scene.rs:129,71,217`) and the budgets (`:9-12`) | budgets are characters, not tokens |
| Support a provider quirk | `client.rs`: `ResponseMessage`/`ToolCall` serde (`:161,95`), `check_status` (`:194`), `says_tools_unsupported` (`:182`), `fallback_tool_call` (`assistant/tools.rs:16`) | keep the `ChatTransport` trait stable for tests |
| Replace the transport (for tests, proxies, streaming) | implement `ChatTransport` (`assistant/client.rs:232`) and pass to `Assistant::with_transport` (`assistant/mod.rs:115`) | the test `FakeTransport` is at `assistant/client.rs:371` |
| Change limits | `MAX_ROUNDS`, `MAX_CALLS_PER_REPLY`, `MAX_TOOL_RESULT_CHARS` (`assistant/turn.rs:17-23`); `HISTORY_*` (`assistant/mod.rs:32-33`) | update the user guide troubleshooting numbers |
| Add a persisted option | `Preferences` (`assistant/mod.rs:40`) with `#[serde(default)]`; sanitise in `sanitized` (`:69`) | the repo rule is to persist every option via `InspectorPreferences` and save through `persist_inspector_changes` (see `AGENTS.md`) |
| Add a panel control | `panel.rs`; session-only state goes in `UiState` (`assistant/panel.rs:46`) | modal footers declare least-impactful first (repo rule; see the Connect dialog `:476-481`) |
| Change docking or window behaviour | `window.rs` | |
| New strings | `assets/i18n/en.json` with `assistant.` prefix, use `tr!` | English only (repo rule) |
| Change history/undo behaviour | `assistant_commit_turn` (`assistant/mod.rs:458`) | keep the single-`finish_edit` invariant |

## 12. Gaps / unclear

Items 1-3, 6 and 7 of the earlier list and the per-call part of item 4 were fixed in the commit "Fix issues found while documenting MIRA" (see [roadmap.md](roadmap.md#23-fixed-in-fix-issues-found-while-documenting-mira)). What remains:

1. **The edit lease is refreshed once per reply** (`assistant/turn.rs:235`), not per call; edit *access* is re-checked before every mutating call. A lease change within a 32-call batch is not seen until the next reply.
2. **Stale scene (documented design choice).** The scene system message is built once per turn (`assistant/turn.rs:147-150`) and never refreshed; a multi-round turn relies on tool results, and the model can call `get_scene`. The fixed part of the scene (selection, tags, stock catalog, joints: at most 200, 100, 40 and 100 entries) is count-capped rather than budgeted; only the object list is filled up to the character budget.
3. **The history keeps the *rewritten* prompt** with `[#id]` markers, clipped to 2000 characters, and no tool calls (by design).
4. **Busy gate coverage not audited (documented design choice).** I only verified the entry points listed in section 8, not every direct mutation of `doc.model`.
5. **Provider facts are maintainer-supplied.** The endlesstokens.ai CORS preflight date (the preflight `Date` header was Mon, 09 Nov 2026 04:15:31 GMT, which is the evening of 2026-11-08 in US Pacific time), 2 parallel sessions per key, model names and `et:auto` tool support cannot be verified from the code; see [llm-protocol.md](llm-protocol.md).
6. **Credential file permissions are only enforced on unix** (`save_in` in `credentials.rs`); behaviour on Windows is whatever the OS default is.

# Build with MIRA: roadmap, known issues and good first issues

[Contributor docs](README.md) · [Architecture](architecture.md) · [Module reference](module-reference.md) · [UI and UX](ui-ux.md) · [Persistence and security](persistence-and-security.md) · [Testing](testing.md) · [Contributing](contributing.md) · [Development history](development-history.md)

Paths are under `crates/benchup-app/src/` unless noted; line numbers refer to `feature/build-with-mira` after the commit "Fix issues found while documenting MIRA" (parent `44a7cc9`). There are no `TODO`/`FIXME` markers in `assistant/` (searched), so the items below come from the maintainers' list, from the "Gaps / unclear" sections of [architecture.md](architecture.md) (cited as A1-A10) and [module-reference.md](module-reference.md), and from my own reading for these documents (marked "found while documenting"). Severity words are my judgement, not the review's.

## 1. Known limitations

| # | Limitation | Detail / pointer |
|---|---|---|
| L1 | **The browser build is only compile-checked** | There is no automated run of the web build. CI runs `cargo check` for wasm32 and the local rustup check passes with 0 warnings since `44a7cc9` ([testing.md](testing.md#9-local-wasm32-check)), but nothing exercises the `fetch` wrapper (`assistant/client.rs:316-357`), `localStorage` credentials (`credentials.rs:76-102`) or the floating window in a browser. Use the manual checklist. |
| L2 | **Native multi-monitor was never run in a real window by an automated test** | The pure helpers are tested (`position_usable`, `default_position`, debounce); `show_viewport_immediate` and a real second monitor were not ([ui-ux.md](ui-ux.md#8-latch-and-unlatch), [testing.md](testing.md#11-manual-test-checklist) items 8-10). |
| L3 | **No live smoke test** | Nothing talks to a real endpoint; behaviour against real model output (tool-call formats, errors, rate limits) is untested. A plan exists ([testing.md](testing.md#12-planned-live-smoke-test-not-written-yet)). |
| L4 | **Responses are not streamed** | `ChatRequest.stream` is always `false` (`assistant/turn.rs:194`); the UI shows "Working… N s" until the whole reply arrives (limit 120 s, `client.rs:12`). |
| L5 | **Outward-normal heuristic for concave parts** | `scene::outward_sign` orients face normals "away from the owning part's center" (`benchup-tools/src/scene.rs:179-193`); for concave or L-shaped parts the sign can be wrong, so `push_pull` may extrude the wrong way. Documented as a heuristic only in a doc comment. |
| L6 | **Outliner drag-in works only when docked** | The drop target is the composer; it does not receive the Outliner's drag payload while the panel is in a separate OS window ([ui-ux.md](ui-ux.md#42-outliner-drag-in-docked-only)). Use `@` or "Ask MIRA about this" instead. |
| L7 | **The transcript is not persisted** | Closing the app loses the chat and the model-facing history (`assistant/mod.rs:90,95`); by design today, for privacy ([persistence-and-security.md](persistence-and-security.md#3-what-is-deliberately-not-persisted)). |
| L8 | **The busy gate does not freeze inspector widgets** | Inspector controls remain interactive during a turn; their edits are refused only if they go through `finish_edit` (`app.rs:772-792`). Not every direct mutation of `doc.model` was audited (A8). |
| L9 | **Off-screen detection is a heuristic** | A saved window position within about 2.5 monitor sizes of the main window is trusted even if that monitor is unplugged; egui does not expose the monitor list ([ui-ux.md](ui-ux.md#84-off-screen-heuristic)). |
| L10 | **Tool support of some models is unknown** | For example the `et:auto` model name of endlesstokens: whether it supports tool calls cannot be told from the code; unsupported models fail with "This model doesn't support tool calls" (`client.rs:180-204`) and the text-JSON fallback (`assistant/tools.rs:16`) is only a partial remedy. |

## 2. Known issues (defects and design gaps)

Most defects found while writing these documents were fixed in the commit "Fix issues found while documenting MIRA" ([development-history.md](development-history.md)); they are listed in section 2.3 so that old references (A1-A10, I1-I14) still resolve. What is open is below.

### 2.1 Open, from architecture.md "Gaps / unclear"

| ID | Issue | Code |
|---|---|---|
| A4 | **The edit lease is refreshed once per reply**, not per call. Edit access itself is now re-checked before every mutating call (fixed, see 2.3), but a lease change in the middle of a 32-call batch is not noticed until the next reply. | `assistant/turn.rs:235` |
| A5 | **The scene message is built once per turn** and can go stale during a long turn; the model relies on tool results and can call `get_scene`. A documented design choice, not a defect. The fixed part of the scene is count-capped rather than budgeted, also by design. | `assistant/turn.rs:147-150` |
| A8 | The busy gate coverage was **not audited** beyond the listed entry points (see L8). A documented design choice. | see L8 |
| A9 | **Provider facts are maintainer-supplied** (CORS date, 2 sessions, model names, `et:auto`). | see L10 |
| A10 | **Credential file permissions are tightened only on Unix**. | `assistant/credentials.rs:33-45` |

### 2.2 Open, found while documenting

| ID | Issue | Code | Severity (my estimate) |
|---|---|---|---|
| I6 | **Failed sends still cancel in-progress gestures** (`assistant_cancel_interactions` runs before validation). | `assistant/panel.rs:195` | P3 |
| I9 | **Privacy notice wording understates what is sent** (positions, tags, joints, materials as well as names and dimensions). | `assets/i18n/en.json` (`assistant.privacy`), `benchup-tools/src/scene.rs` | P3 |
| I11 | **Key file writes are not atomic** (truncate in place), so a crash can leave an empty file (read as "no key"). | `credentials.rs` (`save_in`) | Fixed in `dd59d55`: temporary file, `0600`, rename |
| I13 | **Esc to stop and "typing never triggers shortcuts" are unverified for the detached OS window.** Esc is handled in the root window's keyboard handler. | `app.rs:2179-2182` | unknown |
| I14 | **Mention popup and drag-in cover groups and components only**; tags, joints, faces and edges cannot be referenced, unlike the original proposal. | `assistant/panel.rs:556-576,154-166` | feature gap |
| I15 | **`create_copy` can copy entities outside the edit lease** in a shared project (the copy is new, so it is exempt). `select` is now restricted. | `benchup-tools/src/ops.rs:628` | P3 (shared projects) |

### 2.3 Fixed in "Fix issues found while documenting MIRA"

| ID | Was | Now |
|---|---|---|
| A1 | Include selection off still sent the selection in the scene summary and tool environment. | The scene summary lists no selection and `get_selection` returns `{"selection":[],"note":"The designer has turned off sharing their selection."}` (`ToolEnv.hide_selection`, `benchup-tools/src/ops.rs:38-38`; `assistant/turn.rs:139-142`). The lease logic still uses the real lease. |
| A2 | A change card could describe edits that were rolled back. | `commit_edit` (`app.rs:777`) returns whether the step was recorded; if not, the card is dropped and an error entry says the changes were rolled back (`assistant/mod.rs:477-486`). Test `a_rolled_back_turn_does_not_claim_its_edits_on_the_change_card`. |
| A3 | `enter_object` / `exit_to_root` leaked the edit scope out of a turn. | The scope at turn start is recorded (`Turn.scope`), put back when the turn ends, is stopped or is discarded, and the undo snapshot carries it (`assistant/turn.rs:296-300`, `assistant/mod.rs:410`). |
| A4 (part) | The gate was checked once per reply. | `Gate.recheck` is asked before each mutating call (`assistant/turn.rs:252-256`, closure built in `poll_assistant`, `assistant/mod.rs:412-437`). |
| A6 | Truncated tool results were not valid JSON. | An oversized result becomes `{"ok":...,"truncated":true,"preview":"..."}` (`truncated_result`, `assistant/turn.rs:115-126`). |
| A7 | History could hold consecutive user messages. | A stopped or failed turn adds an assistant placeholder `(stopped)` / `(error: ...)` (`assistant/mod.rs:393-403`). |
| I1 | Shift+Enter submitted the prompt. | `consume_plain_enter` (`assistant/panel.rs:884-892`); test `enter_sends_the_prompt_and_shift_enter_inserts_a_newline`. |
| I2 | Undo turn was lost on an earlier card. | `record` returns whether it added a card; only then is the position stamped (`assistant/mod.rs:498-511`). |
| I3 | CI filter missed tests. | `ci.yml` runs the substring filters `'assistant'` and `'mira'` (`.github/workflows/ci.yml:50-56`). |
| I4 | A typed model was replaced when the list omitted it. | A default is chosen only when the model is empty (`assistant/mod.rs:237-245`). |
| I5 | The base URL was saved before key validation. | `Assistant::connect_to` keeps the new address in `pending_base` and adopts it only on success (`assistant/mod.rs:163-225`, `assistant/panel.rs:485-500`). |
| I7 | "Bring to front" did nothing on the web. | `assistant_raise` calls `ctx.move_to_top(embedded_layer())` (`assistant/window.rs:116-125`). |
| I8 | Erased rows past the card limit were dropped uncounted. | One "+N more" line counts every category (`assistant/panel.rs:928-969`). |
| I10 | `.env.local` was not ignored. | `/.env.local` is in `.gitignore`. |
| I12 | `select` could select entities outside the lease. | With a lease, only leased or turn-created entities can be selected (`benchup-tools/src/ops.rs:963-970`). |
| T7, T8 | Tag name limit (500 vs 128 bytes) and `scale` schema (`exclusiveMinimum`) disagreed with the executor. | Both match the executor now ([tools.md](tools.md)). |

## 3. Future work

| Item | Notes / where it would go |
|---|---|
| **Streaming responses** | Add SSE streaming to `ChatTransport` (a new method or a chunk channel), show tokens in the transcript, keep Stop responsive. Touches `client.rs` (native `ureq` body reading, web `fetch` streams), `turn.rs` state machine, `panel.rs`. |
| **Preview and approve mode** | Apply a turn on a scratch copy of the model, show a diff in the change card, and commit only on approval; the current design is apply-immediately with one undo ([development-history.md](development-history.md)). The lazy `before` snapshot (`benchup-tools/src/ops.rs:1183-1186`) is a starting point. |
| **Vision input with screenshots** | Send a viewport capture with the prompt for models that accept images; must respect the privacy notice and a size budget; the project already has a screenshot module (`crate::screenshots`). |
| **Cut-list and estimate tools** | Read-only (and maybe mutating) tools that expose the existing cut-list, cutting-plan and time/cost estimate data; follow [contributing.md](contributing.md#31-add-a-new-tool). |
| **Golden-prompt evaluation harness** | A set of prompts with expected tool-call properties run against a real or recorded model to detect prompt/tool regressions; ties in with the live smoke test (L3). |
| **Anthropic Messages API transport** | A second `ChatTransport` implementation for providers that expose that API; needs a provider selector in `Preferences` and the connect dialog ([contributing.md](contributing.md#32-add-a-provider-preset-or-a-default-model)). |
| **OS keychain storage** | Replace the plain file/localStorage with Keychain, Credential Manager or Secret Service where available. |
| **Persisted transcripts (opt-in)** | Per-project chat history, off by default for privacy. |
| **Lease check per call**, **scene refresh between rounds** | Open A4 (lease part) and A5. |
| **Real second-monitor test coverage** | A scripted or recorded manual procedure, or a headless window test, for L2. |

## 3.1 MCP and agent connection: follow-ups

Added with the MCP server and the Agent connection add-on (`feature/agent-tools`, commit `91c017f`). Paths are under `crates/`.

| Item | Notes / where it would go |
|---|---|
| **Batch and undo grouping for agents** | Today every mutating call is one undo step (`benchup-app/src/agent_host.rs`, `run_agent_call`), so a 30-call build is 30 steps. Add an optional `begin_batch` / `end_batch` pair, or a tool-call argument, that folds a run of calls into one history step. Needs a rule for what happens if the batch is interrupted by a user edit or a MIRA turn |
| **Persistent edit scope per agent session** | After each call the user's edit scope is restored (`agent_host.rs`, `restore_scope`), so `enter_object` has no lasting effect and an agent must pass ids that are reachable from the user's scope. Keep a scope per agent session and apply it for that agent's calls only |
| **Streamable HTTP transport** | `benchup-mcp` speaks stdio only (`benchup-mcp/src/protocol.rs`). Clients that need a URL would need Streamable HTTP, with the same loopback and token rules |
| **Safari and other browsers** | Chrome behaviour is described in [../mcp.md](../mcp.md#browser-notes). Safari and Firefox have not been tested for `ws://127.0.0.1` from the hosted page |
| **Back-off after failed pairing attempts** | Protocol v2 never sends the token and caps unauthenticated connections at 8 with a 5 s window, but the bridge does not slow down repeated wrong proofs (B3) |
| **Several apps or several agents at once** | One app connection at a time; a newer one replaces the older (`live.rs:326-333`). A second MCP client needs its own bridge process and port |
| **Agent activity without MIRA** | The "Agent activity" card is shown only in the MIRA transcript. With the MIRA add-on off, there is no card, only the status-bar indicator and the history entries |
| **Live end-to-end test** | Tests use a fake bridge and a fake app. Nothing drives a real agent. A scripted smoke test (start `benchup-mcp live`, connect a headless app) would close that gap, as for L3 |

## 4. Good first issues

Small, well-bounded, each with tests and the files to start from. (The earlier list items for I1-I5, I7, I8, I10, I12 and A6 were done in "Fix issues found while documenting MIRA"; see section 2.3.)

1. **Reword the privacy notice** (I9) in `assets/i18n/en.json` and update the tests that assert the sentence (`ui_tests.rs:93-118`, search for the English text).
2. **Do not cancel gestures on a failed send** (I6): move `assistant_cancel_interactions` after validation in `assistant_submit` (`panel.rs:195`).
3. ~~Make key-file writes atomic (I11)~~ Done in `dd59d55` (`credentials.rs`, `save_in`).
4. **Write the `#[ignore]` live smoke test** (L3) following the plan in [testing.md](testing.md#12-planned-live-smoke-test-not-written-yet). `.env.local` is already ignored.
5. **Add tests for untested paths**: `@` popup keyboard navigation, `Esc` to stop, the Outliner drop (`OutlinerDrag` payload), the busy-gate blocking of undo (`app.rs:834`).
6. **Lease check for `create_copy`** (I15): require the source to be leased, or accept the exemption and say so in the tool description.
7. **Add a tag and joint reference** (I14), a larger task: follow [contributing.md](contributing.md#35-add-a-reference-type).
8. **Refresh the lease before every mutating call** (A4): pass the lease through `Gate.recheck` as well as the access check.

## Gaps / unclear

* Which of these items maintainers consider in scope for the upstream PR is unknown.
* Severity labels in section 2 ("Found while documenting") are mine; I did not reproduce the open items by running the app. The fixed items I1-I5, I7, I8, I10 and I12 each have a regression test (section 2.3).
* `et:auto` and the other provider-side facts cannot be verified from this repository.

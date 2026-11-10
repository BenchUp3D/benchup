# Build with MIRA: LLM protocol

[Docs index](README.md) · [User guide](../assistant.md) · [Architecture](architecture.md) · [Module reference](module-reference.md) · [Tool catalog](tools.md) · LLM protocol · [UI/UX](ui-ux.md) · [Persistence and security](persistence-and-security.md) · [Testing](testing.md) · [Contributing](contributing.md) · [Development history](development-history.md) · [Roadmap](roadmap.md)


**Citation convention.** `path:line` is relative to `crates/benchup-app/src/` unless it starts with `benchup-core/` or `benchup-tools/` (those are `crates/<name>/src/`). The scene summary and tool schemas now live in `benchup-tools`; their lines are for branch `feature/agent-tools` at commit `91c017f`. Lines are for branch `feature/build-with-mira` after the commit "Fix issues found while documenting MIRA" (parent `44a7cc9`). `client.rs`, `turn.rs`, `scene.rs`, `tools.rs` mean the files under `assistant/`.

Contents: [Endpoints and transport](#endpoints-and-transport) · [Request](#request) · [Response parsing and the tool loop](#response-parsing-and-the-tool-loop) · [System prompt](#system-prompt) · [Scene summary](#scene-summary) · [References](#references) · [History](#conversation-history) · [Fallback parser](#fallback-tool-call-parser) · [Error mapping](#error-mapping) · [URL guard](#url-guard) · [Providers](#provider-compatibility) · [Gaps](#gaps--unclear)

## Endpoints and transport

Two calls, both relative to the configured base URL (`normalize_base` trims blanks and trailing `/`, `client.rs:20`):

| Purpose | Method and URL | Code |
|---|---|---|
| List models (also validates a key) | `GET {base}/models` | `HttpTransport::models` `client.rs:247-250` |
| Chat | `POST {base}/chat/completions` | `HttpTransport::chat` `client.rs:251-267` |

Defaults: base `https://api.endlesstokens.ai/v1` (`DEFAULT_BASE_URL`, `client.rs:9`); key page `https://portal.endlesstokens.ai/key` (`KEY_PORTAL_URL`, `client.rs:10`).

| Aspect | Native (ureq thread) | Web (wasm32 `fetch`) |
|---|---|---|
| Headers | `Authorization: Bearer <key>`; POST adds `Content-Type: application/json` (`client.rs:294-301`) | Same two headers (`:326-327`) |
| Other headers | none set by the code (ureq defaults apply, e.g. User-Agent) | none set by the code |
| Timeout | 120 s global (`TIMEOUT_SECS`, `:12,289`) | `AbortSignal.timeout(120000)` (`:329`) |
| Response cap | 4 MiB (`RESPONSE_LIMIT`, `:14,308`); exceeding it is a `Network` error | `Content-Length` > 4194304 or body `text.length` > 4194304 throws `Response too large` (`:330-333`); the body check happens after `r.text()` |
| Redirects | `max_redirects(0)` (`:290`); a 3xx is not followed and surfaces as a non-2xx status | `redirect: 'error'` (`:329`) |
| Status handling | `http_status_as_error(false)`: statuses are parsed by `check_status` (`:291`) | status and body returned in JSON, same parser (`:334,348-351`) |
| Credentials | n/a | `credentials: 'omit'`, `cache: 'no-store'` (`:329`). The comment at `:319-320` explains: a wildcard CORS origin is only accepted for requests without ambient credentials |
| Failure text | `ChatError::Network(<ureq error>)` | `Cannot reach the assistant server` for any JS rejection (`:341-345`) |
| Delivery | `mpsc` send then `ctx.request_repaint()` (`:313-314`) | same, in `spawn_local` (`:354-355`) |

Other facts: no streaming (`stream: false`), no retry or back-off, no cancellation of an in-flight request (Stop only drops the receiver). The request body is `serde_json::to_string(request).unwrap_or_default()` (`client.rs:258`), so a serialisation failure would send an empty body (not expected for this type).

## Request

`ChatRequest` (`client.rs:140`) serialises to:

| Field | Value |
|---|---|
| `model` | the selected model id (`Preferences.model`) |
| `messages` | see below |
| `tools` | the 27 schemas from `tools::definitions()`, which is `benchup_tools::openai_tools` (`benchup-tools/src/ops.rs:208`), omitted if empty |
| `tool_choice` | the string `"auto"` (`turn.rs:193`) |
| `stream` | `false` |

No `temperature`, `max_tokens`, `top_p` or other sampling parameter is sent.

**Message order** (`Turn::start`, `turn.rs:151-156`):

1. `system`: `SYSTEM_PROMPT` (below).
2. `system`: `"Current model state (JSON):\n"` + compact JSON `{"scene": <scene summary>, "references": <references>}` (`turn.rs:147-154`).
3. Up to 20 earlier `user`/`assistant` text messages (see [History](#conversation-history)).
4. `user`: the prompt after `@Name [#id]` rewriting.

Each model round appends (`turn.rs:236-241, 238`): an `assistant` message with `content` (possibly `""`) and `tool_calls`, then one `tool` message per call with `tool_call_id` and the result text. `Message` skips `tool_calls` when empty and `tool_call_id` when `None` (`client.rs:120-123`); `content` is always present as a string.

Realistic second-round request (abridged; the real scene JSON is single-line):

```json
{
  "model": "Qwen3.8-27B",
  "messages": [
    {"role": "system", "content": "You are MIRA, a woodworking design assistant inside BenchUp, ..."},
    {"role": "system", "content": "Current model state (JSON):\n{\"references\":{\"4\":{\"id\":4,\"kind\":\"group\",\"name\":\"Leg 2\",\"faces\":[{\"id\":31,\"normal\":[1.0,0.0,0.0],\"area_mm2\":28000.0,\"center_mm\":[40.0,20.0,350.0],\"facing\":\"+x\"}]}},\"scene\":{\"units\":\"mm\",\"up_axis\":\"z\",\"edit_path\":[],\"selection\":[4],\"object_count\":2,\"objects\":[...]}}"},
    {"role": "user", "content": "Make @Leg 2 [#4] 40 mm longer"},
    {"role": "assistant", "content": "", "tool_calls": [
      {"id": "call_0", "type": "function", "function": {"name": "resize_board", "arguments": "{\"id\":4,\"size_mm\":[40,40,740]}"}}
    ]},
    {"role": "tool", "tool_call_id": "call_0", "content": "{\"bounds_mm\":{\"max\":[40,40,740],\"min\":[0,0,0],\"size\":[40,40,740]},\"created\":[],\"modified\":[4],\"ok\":true,\"objects\":[...]}"}
  ],
  "tools": [ {"type": "function", "function": {"name": "get_scene", "description": "...", "parameters": {"type": "object", "properties": {}, "required": [], "additionalProperties": false}}}, ... ],
  "tool_choice": "auto",
  "stream": false
}
```

(Key order inside objects depends on `serde_json`'s map ordering; the example is illustrative. The model id `Qwen3.8-27B` is the provider-supplied name given in the task brief; the code never hard-codes it.)

## Response parsing and the tool loop

Parsing: `parse_chat` (`client.rs:221`) = `check_status` (JSON parse of 2xx) -> `ChatResponse { choices: [{ message: { content?, tool_calls? } }] }`. Only `choices[0]` is used (`turn.rs:212`). Empty `choices` is `BadResponse("The server returned no answer")` (`client.rs:225-227`). Other fields (`finish_reason`, `usage`, reasoning fields) are ignored.

Tolerances (`client.rs:76-101`):

* `tool_calls[].id` and `.type` may be missing (`type` defaults to `"function"`).
* `function.arguments` may be a JSON string, a JSON object (re-serialised), or `null` (-> `"{}"`).
* `content` may be `null`.

Loop (`Turn::poll`, `turn.rs:200-281`):

1. Take `content` and `tool_calls`. If no calls, try the [fallback parser](#fallback-tool-call-parser) on `content`.
2. Calls with an empty id get `call_{round}_{index}` (`turn.rs:224-228`); the fallback call is `call_fallback`.
3. If calls exist and the gate says editing is blocked, finish as stopped (`:229-234`).
4. Push the assistant message, including the raw `content`.
5. No calls: finish; `content` is the summary (empty -> "Done.").
6. Execute up to 32 calls; extra calls get `{"ok":false,"error":"too many calls in one reply"}` (`:247-251`). Before each *mutating* call the gate's `recheck` is asked again; a reason finishes the turn as stopped without running that call (`:252-256`). Each result is `result.to_string()`; one over 8000 chars is replaced by valid JSON, `{"ok":<bool>,"truncated":true,"preview":"<first ~7000 chars>"}` (`truncated_result`, `:115`), with the preview shrunk until the whole message is within 8000 chars.
7. `rounds += 1`; at 12 rounds finish with the round-limit error, else request again.

Calls inside one reply are executed sequentially in order; the model does not see results until the next round.

## System prompt

Verbatim from `SYSTEM_PROMPT`, `turn.rs:27-34` (Rust string with `\` line continuations, so the sent text is one paragraph with single spaces between sentences):

```rust
pub(crate) const SYSTEM_PROMPT: &str = "You are MIRA, a woodworking design assistant inside BenchUp, a 3D modeler. \
You change the user's model only by calling the provided tools. \
All lengths you read or write are in millimetres. The Z axis points up; boards are axis-aligned boxes unless rotated. \
Refer to entities by their numeric id, and use the ids in the scene and in the references block. \
Call get_object to read a board's faces before push_pull or any edit that needs a face id; never guess ids. \
Prefer high-level tools (create_board, move, resize_board, add_joint, set_material) over drawing raw geometry. \
Check tool results: if a call fails, read the error and hint, then correct it. \
When the work is complete, answer with one short sentence summarising what you did, without calling more tools.";
```

## Scene summary

Built by `scene::summary(model, selection, catalog, budget)` (`benchup-tools/src/scene.rs:129`). All lengths in mm, rounded to 0.1 mm (`benchup-tools/src/scene.rs:15`).

| Key | Content | Cap |
|---|---|---|
| `units` | `"mm"` | |
| `up_axis` | `"z"` | |
| `edit_path` | ids of open groups from root | |
| `selection` | selected ids (editor selection); always `[]` when "Include selection" is off | 200 (`MAX_LISTED_SELECTION`); `selection_total` added when more |
| `tags` | `{id, name, visible}` | 100 (`MAX_LISTED_TAGS`) |
| `stock_catalog` | `{id, name, thickness_mm, width_mm, length_mm}` | 40 |
| `joints` | `{id, a, b, kind}`; `kind` is the `JointKind` debug name (e.g. `MortiseTenon`) or `"Fasteners"` | 100 (first 100 by key) |
| `loose_geometry` | `{edges, faces}` counts of root-level loose entities | |
| `object_count` | total objects | |
| `objects` | `object_entry` list (below), selection first, then in-scope, then the rest | by budget |
| `omitted`, `hint` | present only when objects were cut; hint: "Scene truncated: use find_objects{query} or get_object{id} to inspect omitted objects." | |

`object_entry` (`benchup-tools/src/scene.rs:71`): `id`, `name`, `kind` (`group`/`component`), `parent`, `tag`, `bounds_mm {min,max}`, `size_mm`, `material {id,name}`, `grain`, `hidden`, `locked` (optional keys appear only when applicable).

Example (pretty-printed; real output is compact):

```json
{
  "units": "mm",
  "up_axis": "z",
  "edit_path": [],
  "selection": [4],
  "tags": [{"id": 2, "name": "Legs", "visible": true}],
  "stock_catalog": [{"id": "6f1c...", "name": "Maple 2x4", "thickness_mm": 38.0, "width_mm": 89.0, "length_mm": 2400.0}],
  "joints": [{"id": 9, "a": 4, "b": 6, "kind": "MortiseTenon"}],
  "loose_geometry": {"edges": 0, "faces": 0},
  "object_count": 2,
  "objects": [
    {"id": 4, "name": "Leg 2", "kind": "group", "tag": "Legs",
     "bounds_mm": {"min": [340, 0, 0], "max": [380, 40, 700]}, "size_mm": [40, 40, 700],
     "material": {"id": "6f1c...", "name": "Maple 2x4"}, "grain": "z"},
    {"id": 6, "name": "Apron", "kind": "group", "size_mm": [300, 40, 80],
     "bounds_mm": {"min": [20, 0, 500], "max": [320, 40, 580]}}
  ]
}
```

**Budgets** (characters of JSON text, not tokens):

* `SCENE_BUDGET = 24_000` for scene + references together (`benchup-tools/src/scene.rs:9`).
* References get at most a third: `REFERENCE_BUDGET = SCENE_BUDGET / 3 = 8000` (`turn.rs:25`). The scene gets `SCENE_BUDGET - references.to_string().len()` (`turn.rs:146`).
* In `summary`, `used` starts at the length of the fixed part (selection, tags, stock, joints, counts) plus 160 reserved for `omitted`/`hint`; objects are appended in order until the next entry would exceed `budget` (`benchup-tools/src/scene.rs:156-166`). The fixed part itself is not budgeted, only count-capped; if it already exceeds the budget no objects are listed and all are `omitted`.
* The `get_scene` tool calls `summary` with the whole 24 000 and no references (`benchup-tools/src/ops.rs:918`).
* Tool results are capped separately at 8000 chars (`turn.rs:19`).

Face detail (`describe`, `benchup-tools/src/scene.rs:217`): object entries add `children` and `faces` (max 80, `faces_omitted`); each face: `id`, outward `normal` (3 decimals), `area_mm2`, `center_mm`, `facing`. See [tools.md](tools.md#outward-normals-and-push_pull).

## References

* **Rendering in the prompt.** `rewrite_prompt` (`assistant/panel.rs:105`) rewrites a typed `@Name` that equals a chip label (longest labels first, not followed by an alphanumeric, preceded by start/whitespace) into `@Name [#id]`. Chips never typed into the text are appended: `"\n\nAlso referenced: @A [#1], @B [#2]"` (`assistant.prompt.also_referenced`).
* **Attached data.** The set of all chip ids is `Prompt.references`. `Turn::start` adds the editor selection when `include_selection` is on (`turn.rs:139-142`); when it is off the selection is also omitted from the scene summary and `get_selection` answers `{"selection":[],"note":"The designer has turned off sharing their selection."}` and renders `scene::references(model, ids, 8000)` (`benchup-tools/src/scene.rs:257`): a JSON map `"<id>": describe(id)` (so objects with faces, faces, joints, other entities), `{"id":N,"kind":"missing"}` for vanished ids. Entries are added in ascending id order; the first is always kept, and when the running size (entry length + 8) exceeds the budget the rest are counted in `"omitted": <n>` (`benchup-tools/src/scene.rs:260-267`). The `omitted` key sits in the same map as the id keys.
* References are by id, so renaming after attaching does not break them (test `references_describe_entities_by_id_even_after_a_rename`, `tests.rs:432`).
* Chips can only be objects (groups/components): `assistant_add_chips` ignores other ids (`panel.rs:154`); the *selection* may contain any entity type.

## Conversation history

* `Assistant.history: Vec<Message>` holds only text: after each turn `record` pushes the (rewritten) user prompt and then the assistant summary or, when there is none, a placeholder, each clipped to `HISTORY_CHARS = 2000` chars, and keeps the last `HISTORY_MESSAGES = 20` messages (`assistant/mod.rs:32-33,391-405`).
* Tool calls and tool results of earlier turns are not kept; the scene is rebuilt fresh each turn.
* A stopped or failed turn adds the user prompt and an assistant placeholder, `(stopped)` or `(error: <message>)` (`(no reply)` as a last resort), so `user` and `assistant` messages always alternate.
* `New chat` clears transcript and history (`mod.rs:227`). History is memory only.
* Within a turn the `messages` vector grows with every round with no trimming other than the per-result 8000-char cap.

## Fallback tool-call parser

`fallback_tool_call(content)` (`assistant/tools.rs:16`) is tried only when the reply has no structured `tool_calls`. Accepted shapes, after trimming the whole content:

1. Exactly one fenced block: content starts with ```` ``` ```` and ends with ```` ``` ````, with no further ```` ``` ```` inside. An optional leading `json` label (case-insensitive) is stripped.
2. A bare object: content starts with `{` and ends with `}`.

Inside, the JSON is either a call object or `{"function": {...}}` (the `function` value is used if it is an object). The name comes from `name` or `tool` and must be a known tool; the arguments come from `arguments` or `parameters`, defaulting to `{}`; if `arguments` is a string it must itself parse as JSON. The resulting call has id `call_fallback`. Anything else (prose containing JSON, several blocks, unknown tool) returns `None` and the content is treated as the final answer. Only one call per reply can come from the fallback. Tests: `tools_tests.rs:327,344`; `tests.rs:250`.

## Error mapping

`check_status(status, body)` (`client.rs:194`):

| Condition | `ChatError` | User text (`en.json`) |
|---|---|---|
| 2xx, JSON parses | ok | |
| 2xx, JSON invalid | `BadResponse(serde error)` | "Unexpected response from the assistant server: {e}" |
| 2xx chat, `choices` empty | `BadResponse` | "The server returned no answer" |
| 2xx models, no `data` array | `BadResponse` | "The server did not return a model list" |
| 401 | `InvalidKey` | "Invalid API key" |
| 400 and message says tools unsupported | `ToolsUnsupported` | "This model doesn't support tool calls. Pick another model." |
| any other non-2xx (400, 403, 429, 5xx, 3xx) | `Http{status, message}` | "The assistant server returned an error ({status}): {message}" |
| connection/TLS/timeout/oversize | `Network(e)` | "Cannot reach the assistant server: {e}" (web: fixed text) |
| channel closed | `Network` | "The request was interrupted" |
| invalid base URL | `InvalidUrl` | "The assistant server address must use https, or http on localhost." |

* `message` = `error.message` if present, else `error` if a string, else the raw body, truncated to 300 chars (`client.rs:168-178`).
* **Tools-unsupported heuristic** (`client.rs:182-191`): lowercase message; refusal phrases `not support`, `unsupported`, `does not allow`, `doesn't allow`, `not allowed`, `no support`; subject phrases `tool`, `function calling`, `function call`; true if any refusal and any subject start within 40 bytes of each other. So "Unsupported parameter: temperature" or "max_tokens is not supported" stay `Http`.
* **Truncation.** Error text is cut at 300 chars; scene by budgets; tool results at 8000 chars, as valid JSON with a `preview` (see above).
* An error during a turn ends it but keeps edits already applied ([architecture.md](architecture.md#7-undo-and-commit-semantics)). A 429 or 5xx is not retried.

## URL guard

`valid_base_url` (`client.rs:26`) accepts a string only if all hold:

* contains none of `?`, `#`, `@`, space;
* either starts with `https://` and has a non-empty authority, or starts with `http://` and the authority (text before the next `/`) is exactly `localhost`, `127.0.0.1`, or those followed by `:port` (`starts_with("localhost:")` / `"127.0.0.1:"`).

So rejected: plain `http://` to other hosts, `http://localhost.evil.com`, credentials in the URL, query strings, other schemes, `https://` with nothing after. `[::1]` and `0.0.0.0` are not accepted (inference from the code; `client.rs:31-36`). The guard runs when saving preferences (`Preferences::sanitized`, `assistant/mod.rs:49-52`), in the Connect dialog (`panel.rs:422`), in `Assistant::send` (`mod.rs:315`) and again inside `spawn` (`client.rs:278`). Tests: `client.rs:421-444`.

## Provider compatibility

The client speaks the OpenAI Chat Completions shape (`/models`, `/chat/completions`, function tools, `tool_calls`, `tool` role messages). Anything that implements those can be used by changing the server address (Connect dialog > Advanced > Server address, or **Change endpoint...** in the menu; `panel.rs:285,443-464`). Keys are stored per normalised base URL (`credentials.rs:19`), so switching servers does not reuse a key.

**endlesstokens.ai (default preset).** Facts below marked *(supplied)* come from the maintainer's task brief and the user guide and cannot be verified from the code:

* Base `https://api.endlesstokens.ai/v1` and key portal `https://portal.endlesstokens.ai/key` are in code (`client.rs:9-10`).
* CORS is `*`, verified with a preflight whose HTTP `Date` header was Mon, 09 Nov 2026 04:15:31 GMT *(supplied)*. That is the evening of 2026-11-08 in US Pacific time, the same period as the commit times in [development-history.md](development-history.md), so the two dates agree. This is why the web build must not send cookies: see `credentials: 'omit'` (`client.rs:319-329`).
* Two parallel sessions per key *(supplied; also in [../assistant.md](../assistant.md) troubleshooting)*. The code makes one request at a time per turn and has no cross-app throttling.
* Model Qwen3.8-27B supports tool calling *(supplied)*. The code only prefers a model id containing `qwen` (case-insensitive) when the current one is missing from the list (`DEFAULT_MODEL_HINT`, `assistant/mod.rs:34,238-245`); the list is sorted alphabetically (`client.rs:216`), so the first match alphabetically wins.
* The `et:auto` router: tool support is undocumented *(supplied)*. The code treats it like any model id; if the router rejects tools with a 400 that matches the heuristic the user sees the "doesn't support tool calls" message.

**Other servers.**

* Hosted OpenAI-compatible services: set the base URL to the `/v1` root (the client appends `/models` and `/chat/completions`). The service must allow the browser origin via CORS for the web build; the native build is not subject to CORS.
* Localhost (Ollama, llama.cpp server, LM Studio, vLLM, etc.): `http://localhost:11434/v1` style addresses pass the guard (`http` is allowed for `localhost` and `127.0.0.1` only). A key is still required by the UI (empty key is rejected, `assistant/mod.rs:166-169`); servers that ignore auth accept any non-empty string. From the hosted web build, calling `http://localhost` is subject to browser mixed-content/CORS rules (not verified here).
* The model must support function/tool calling; otherwise either a 400 (mapped to the tools message) or plain text answers that cannot edit the model. Models that print the call as JSON in the message body are handled by the [fallback parser](#fallback-tool-call-parser).
* The server must accept `tool_choice: "auto"`, `stream: false`, assistant messages with `content: ""` plus `tool_calls`, and `role: "tool"` messages.
* Unsupported by design: streaming responses, non-JSON bodies, replies over 4 MiB.

## Gaps / unclear

Fixed in "Fix issues found while documenting MIRA": the selection being sent with "Include selection" off, truncated tool results that were not valid JSON, and consecutive `user` messages in the history. Open or by design:

1. **Scene is not refreshed between rounds** (`turn.rs:147-150`); only tool results update the model's view, and the model can call `get_scene`. A documented design choice.
2. **Fixed part of the scene is count-capped rather than budgeted** (`benchup-tools/src/scene.rs:136-156`): selection, tags, stock, joints are count-capped only, so the total can exceed 24 000 chars when they are large (not measured). A documented design choice.
3. **`Content-Length`/size check on the web is after reading the body** for chunked responses (`client.rs:332`), and counts UTF-16 units, not bytes.
4. **No 429/5xx retry**, no `Retry-After` handling, no streaming, no cancellation of in-flight requests.
5. **History stores the rewritten prompt**, so `[#id]` markers from an earlier turn persist, and may go stale after ids change (e.g. `group`).
6. **Provider facts** (2 sessions per key, Qwen3.8-27B tool support, `et:auto`) are supplied, not verified from the repository. The only repository evidence is the defaults at `client.rs:9-10`, the `qwen` hint at `assistant/mod.rs:34`, the `credentials: 'omit'` comment at `client.rs:319-320`, and the user guide.
7. **Localhost from the web build** (mixed content, private-network CORS) is untested here; `[::1]` is not accepted by the guard.
8. The example request/scene above are hand-written illustrations built from the schemas in the code, not captured traffic.

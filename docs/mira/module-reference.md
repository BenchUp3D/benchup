# Build with MIRA: module reference

[Docs index](../README.md) · [User guide](../assistant.md) · [Architecture](architecture.md) · Module reference · [Tool catalog](tools.md) · [LLM protocol](llm-protocol.md) · [UI/UX](ui-ux.md) · [Persistence and security](persistence-and-security.md) · [Testing](testing.md) · [Contributing](contributing.md) · [Development history](development-history.md) · [Roadmap](roadmap.md)


**Citation convention.** `path:line` is relative to `crates/benchup-app/src/` unless it starts with `benchup-core/` (`crates/benchup-core/src/`). Lines are for branch `feature/build-with-mira` after the commit "Fix issues found while documenting MIRA" (parent `44a7cc9`). Visibility: `pub(crate)` and `pub(super)` are written as in the source; "private" means no modifier. In `assistant/mod.rs` the module items are `pub(super)` i.e. visible to `app`, and sub-modules are `pub(crate)`.

Contents: [mod.rs](#assistantmodrs) · [client.rs](#assistantclientrs) · [credentials.rs](#assistantcredentialsrs) · [scene.rs](#assistantsceners) · [tools.rs](#assistanttoolsrs) · [turn.rs](#assistantturnrs) · [panel.rs](#assistantpanelrs) · [window.rs](#assistantwindowrs) · [tests](#test-files) · [Touchpoints outside the module](#touchpoints-outside-the-module) · [Gaps](#gaps--unclear)

Module declaration: `app.rs:88-89` (`#[path = "assistant/mod.rs"] mod assistant;`). Why `#[path]`: [architecture.md section 2](architecture.md#why-the-module-is-declared-from-apprs-via-path).

---

## assistant/mod.rs

Responsibility: the module root. Declares the sub-modules, owns the session state (`Assistant`), persisted `Preferences`, the model-list/connect flow, `send`/`poll`/`stop`/`discard_turn`/`record`, and the `impl BenchUpApp` glue that commits a finished turn as one history step. Doc comment `:1-5`.

Sub-module declarations `:11-27`: `client`, `credentials`, `panel`, `scene`, `tools`, `turn`, `window` (all `pub(crate)`, each with `#[path = "x.rs"]`), and `#[cfg(test)] mod tests` (`tests.rs`). Imports `use super::*;` (`:6`), `tr`, `client::{ChatError, ChatTransport, Message, Role}`, `turn::{Gate, Turn, TurnResult, TurnSetup}` (`:30`).

### Constants

| Item | Line | Value / meaning |
|---|---|---|
| `HISTORY_MESSAGES` | `:32` | `20`; max messages kept in the model-facing chat history |
| `HISTORY_CHARS` | `:33` | `2000`; each history message is clipped to this many chars (`record` `:413`) |
| `DEFAULT_MODEL_HINT` | `:34` | `"qwen"`; substring (case-insensitive) used to pre-select a model, but only when no model is chosen yet (`poll_models` `:276-283`) |

### Types

| Item | Line | Description |
|---|---|---|
| `struct WindowPreferences { detached: bool, position: Option<[f32;2]>, size: Option<[f32;2]> }` | `:40` | `pub(super)`, `Default + Serialize + Deserialize`, `#[serde(default)]`. Panel placement; position = outer corner, size = inner size, both points |
| `struct Preferences { base_url, model, include_selection, privacy_dismissed, window }` | `:49` | `pub(super)`, serialisable, `#[serde(default)]`. Persisted by the sidebar preferences. **No API key field.** Default: `DEFAULT_BASE_URL`, empty model, `include_selection: true`, `privacy_dismissed: false` (`impl Default` `:57`) |
| `enum Connection { Disconnected, Connecting, Connected, Failed(String) }` | `:87` | UI-visible connection status |
| `enum EntryRole { User, Assistant, Error }` | `:95` | `Copy` |
| `struct Changes { created, modified, erased, joints: Vec<u64>, stopped: bool, position: Option<usize>, erased_names: BTreeMap<u64,String> }` | `:102` | What a finished turn changed, for the change card. `position` = history position right after this turn's step (`None` if nothing was recorded) |
| `struct Entry { role, text, changes: Option<Changes> }` | `:114` | One transcript row |
| `struct Prompt { prompt: String, references: BTreeSet<u64> }` | `:121` | Composer output passed to `send` |
| `struct Assistant` | `:126` | Session state. Public fields: `settings: Preferences`, `transcript: Vec<Entry>`, `key: Option<String>`, `models: Vec<String>`, `connection: Connection`, `ui: panel::UiState`. Private: `history: Vec<Message>`, `turn: Option<Turn>`, `models_receiver: Option<(String, Receiver<Result<Vec<String>, ChatError>>)>`, `key_candidate: Option<String>`, `pending_base: Option<String>`, `transport: Box<dyn ChatTransport>` |

### Functions on `Preferences`

| Signature | Line | Description |
|---|---|---|
| `fn sanitized(self) -> Self` (private) | `:69` | Drops non-finite window position/size, clamps size to `[window::MIN_SIZE, 8192]`, normalises `base_url` and falls back to the default if invalid, trims the model to 200 chars |

### Functions on `Assistant`

| Signature | Line | Description |
|---|---|---|
| `impl Default` -> `with_transport(Box::new(client::HttpTransport))` | `:146` | Production constructor |
| `pub(super) fn with_transport(transport: Box<dyn ChatTransport>) -> Self` | `:153` | Test seam |
| `pub(super) fn preferences(&self) -> Preferences` | `:170` | Clone of `settings` for the sidebar save |
| `pub(super) fn restore(&mut self, preferences: &Preferences)` | `:173` | Load + `sanitized` |
| `pub(super) fn busy(&self) -> bool` | `:177` | `turn.is_some()`; the busy gate |
| `pub(super) fn progress(&self) -> Option<(&[String], f64)>` | `:181` | Tool names run so far and `started_at` for the composer status line |
| `pub(super) fn load_key(&mut self)` | `:186` | Reads the stored key for `settings.base_url` once; sets `Connected` |
| `pub(super) fn connect(&mut self, ctx, key: &str)` | `:195` | Connects to the current `settings.base_url`; delegates to `connect_to` |
| `pub(super) fn connect_to(&mut self, ctx, base: &str, key: &str)` | `:201` | Empty key -> `Failed`; else `Connecting`, remembers `key_candidate`, requests `/models` from `base`. If `base` differs from `settings.base_url` it is kept only in `pending_base`; the address, the key, the model list and an empty model are adopted by `poll_models` only on success, so a failed attempt leaves the old server settings alone |
| `pub(super) fn refresh_models(&mut self, ctx)` | `:214` | Re-lists models with the stored key (no candidate) |
| `pub(super) fn forget_key(&mut self) -> Result<(), String>` | `:220` | Clears key, models, state; deletes the stored key |
| `pub(super) fn new_chat(&mut self)` | `:227` | Clears transcript and history; no-op while busy |
| `fn poll_models(&mut self)` (private) | `:234` | Receives the model list. Ignores replies for a different server than the one expected (`pending_base` if set, else `settings.base_url`, normalised) (`:243-251`); on success adopts a pending address (`:257`), saves a candidate key only after the server accepted it, and picks a default model only if `settings.model` is empty (`:276`); `InvalidKey` clears a stored key from memory only |
| `pub(super) fn send(&mut self, ctx, model: &Model, selection: &BTreeSet<u64>, catalog: BTreeMap<MaterialId, StockMaterial>, read_only: bool, lease: Option<BTreeSet<u64>>, request: Prompt) -> Result<(), String>` | `:298` | Validates (busy, empty, URL, key, model), pushes the user entry, builds `TurnSetup` with a clone of `history`, starts a `Turn` |
| `pub(super) fn poll(&mut self, ctx, model: &mut Model, gate: &Gate) -> turn::Poll` | `:345` | Polls models and the turn; clears `turn` when finished; schedules a 100 ms repaint while waiting |
| `pub(super) fn stop(&mut self, model: &Model) -> Option<TurnResult>` | `:359` | Takes the turn and stops it, keeping edits |
| `pub(super) fn discard_turn(&mut self, model: &mut Model, error: String) -> bool` | `:366` | Stops the turn, puts the live model back in the scope it started in, wipes `before`, `selection` and the id lists, records the error. Returns whether a turn existed |
| `pub(super) fn record(&mut self, result: &TurnResult) -> bool` | `:380` | Appends the assistant entry (+ change card), error entry, and user/assistant history messages (an assistant placeholder `(stopped)` / `(error: ...)` / `(no reply)` when there is no summary, so roles alternate); trims history to `HISTORY_MESSAGES`. Returns whether a change card was added |

### `impl BenchUpApp` in mod.rs

| Signature | Line | Description |
|---|---|---|
| `pub(super) fn poll_assistant(&mut self, ctx)` | `:440` | Per frame (`app.rs:6433`). When busy builds a `Gate` (block, lease, per-call `recheck`), moving the assistant and the model out of `self` with `mem::take` while the recheck closure borrows `&self`; polls, calls `self.changed(ctx)` on `mutated`, commits a finished turn |
| `pub(super) fn assistant_send(&mut self, ctx, request: Prompt) -> Result<(), String>` | `:468` | Collects the stock catalog, `read_only`, `lease`; calls `Assistant::send` |
| `pub(super) fn assistant_stop(&mut self, ctx)` | `:481` | Stop + commit |
| `pub(super) fn assistant_commit_turn(&mut self, mut result: TurnResult, ctx)` | `:488` | Restores the edit scope, one `commit_edit` (drops the card and adds an error entry if the step was refused and rolled back), selection, `record`, and the card position and erased names only when `record` added a card |
| `fn restore_scope(model: &mut Model, scope: Option<u64>)` (free, private) | `:434` | Sets `edit_context` to `scope` if that object still exists, else the top level |

---

## assistant/client.rs

Responsibility: the OpenAI-compatible HTTP client, the wire types, the URL guard and the test fake. Requests run off the UI thread and answer through an `mpsc` channel (doc `:-3--1`). Protocol details: [llm-protocol.md](llm-protocol.md).

### Constants

| Item | Line | Value |
|---|---|---|
| `DEFAULT_BASE_URL` | `:9` | `"https://api.endlesstokens.ai/v1"` (`pub(crate)`) |
| `KEY_PORTAL_URL` | `:10` | `"https://portal.endlesstokens.ai/key"` (`pub(crate)`) |
| `TIMEOUT_SECS` | `:12` | `120`; `#[cfg(any(not(wasm32), test))]` |
| `RESPONSE_LIMIT` | `:14` | `4 * 1024 * 1024`; same cfg |
| `MAX_ERROR_CHARS` | `:17` | `300` |

### Functions

| Signature | Line | Description |
|---|---|---|
| `pub(crate) fn normalize_base(base: &str) -> String` | `:20` | Trim blanks and trailing `/` |
| `pub(crate) fn valid_base_url(base: &str) -> bool` | `:26` | URL guard: https with non-empty authority, or http for `localhost` / `127.0.0.1` (optional `:port`); rejects `? # @` and space |
| `fn arguments_text<'de, D>(d: D) -> Result<String, D::Error>` | `:76` | serde helper: string stays, `null` -> `"{}"`, object/other -> its JSON text |
| `fn function_kind() -> String` | `:84` | `"function"` default for `ToolCall.kind` |
| `fn error_message(body: &str) -> String` | `:168` | `error.message`, or `error` as a string, or the raw body; cut to 300 chars |
| `fn says_tools_unsupported(message: &str) -> bool` | `:182` | Heuristic for 400s: a refusal phrase within 40 chars of a "tool"/"function call(ing)" phrase |
| `pub(crate) fn check_status(status: u16, body: &str) -> Result<Value, ChatError>` | `:194` | 2xx -> parse JSON; 401 -> `InvalidKey`; 400 + heuristic -> `ToolsUnsupported`; else `Http` |
| `pub(crate) fn parse_models(status: u16, body: &str) -> Result<Vec<String>, ChatError>` | `:206` | `data[].id`, sorted and deduplicated |
| `pub(crate) fn parse_chat(status: u16, body: &str) -> Result<ChatResponse, ChatError>` | `:221` | Typed parse; empty `choices` is `BadResponse` |
| `fn spawn<T: Send + 'static>(ctx, url: String, base: &str, key: &str, body: Option<String>, parse: fn(u16,&str)->Result<T,ChatError>) -> Receiver<Result<T,ChatError>>` | `:270` | Re-checks the URL guard, then runs the request on a thread (native, `:285-315`) or via `fetch` + `spawn_local` (wasm, `:316-357`) |

### Types

| Item | Line | Description |
|---|---|---|
| `enum ChatError { InvalidUrl, InvalidKey, ToolsUnsupported, Network(String), Http{status:u16,message:String}, BadResponse(String) }` | `:42` | `Clone + PartialEq`. `fn user_message(&self) -> String` at `:53` maps to `tr!` strings |
| `enum Role { System, User, Assistant, Tool }` | `:69` | serde `rename_all = "lowercase"` |
| `struct FunctionCall { name: String, arguments: String }` | `:89` | `arguments` deserialised with `arguments_text` |
| `struct ToolCall { id: String, kind: String (serde "type"), function: FunctionCall }` | `:95` | `id` and `type` default. `fn new(id:&str, name:&str, arguments:&Value) -> Self` at `:103` |
| `struct Message { role, content: String, tool_calls: Vec<ToolCall>, tool_call_id: Option<String> }` | `:116` | `tool_calls` and `tool_call_id` skipped when empty/None. `Message::text(role, content)` `:126`, `Message::tool(call_id, content)` `:129` |
| `struct ChatRequest { model, messages, tools: Vec<Value>, tool_choice: Option<&'static str>, stream: bool }` | `:140` | `Serialize` only; `tools` skipped when empty, `tool_choice` when `None` |
| `struct ChatResponse { choices: Vec<Choice> }` | `:151` | |
| `struct Choice { message: ResponseMessage }` | `:156` | |
| `struct ResponseMessage { content: Option<String>, tool_calls: Option<Vec<ToolCall>> }` | `:161` | |
| `trait ChatTransport { fn models(&self, ctx, base, key) -> Receiver<Result<Vec<String>,ChatError>>; fn chat(&self, ctx, base, key, request: &ChatRequest) -> Receiver<Result<ChatResponse,ChatError>> }` | `:232` | Seam for tests |
| `struct HttpTransport` | `:244` | `impl ChatTransport` at `:246`: `models` = `GET {base}/models`, `chat` = `POST {base}/chat/completions` |

### Test-only

| Item | Line | Description |
|---|---|---|
| `mod fake` (`#[cfg(test)]`, `pub(crate)`) | `:362` | Scripted transport |
| `struct FakeTransport { replies, requests, models }` | `:371` | Replies popped from a queue (empty queue -> `Network("no scripted reply")`); every request is recorded as JSON in `requests` |
| `FakeTransport::push_tool_calls(calls)` / `push_text(text)` | `:377` / `:384` | Queue a reply |
| `mod tests` (`#[cfg(test)]`) | `:418` | URL guard, status mapping, truncation, tools-unsupported heuristic, argument forms, wire format (`:422-527`) |

---

## assistant/credentials.rs

Responsibility: per-server API key storage. Native: a private file in the data dir. Web: `localStorage`. Never goes to eframe storage, preferences, or project files (doc `:1-3`).

| Signature | Line | Description |
|---|---|---|
| `fn base_hash(base: &str) -> u64` | `:10` | FNV-1a over the normalised base URL; `cfg(not wasm32 or test)` |
| `fn key_file(dir: &Path, base: &str) -> PathBuf` | `:19` | `assistant-{hash:016x}.key` |
| `fn storage_name(base: &str) -> String` | `:23` | `benchup.assistant.key.{normalised base}`; `cfg(wasm32 or test)` |
| `fn save_in(dir, base, key) -> Result<(), String>` | `:28` | Creates dir; file mode `0o600` on unix, also tightened when the file exists; writes the trimmed key |
| `fn load_in(dir, base) -> Option<String>` | `:49` | Trimmed, non-empty |
| `fn forget_in(dir, base) -> Result<(), String>` | `:55` | Missing file is success |
| `pub(crate) fn load(base: &str) -> Option<String>` | native `:64`, wasm `:86` | Uses `crate::storage::data_dir()` natively |
| `pub(crate) fn save(base: &str, key: &str) -> Result<(), String>` | native `:68`, wasm `:92` | |
| `pub(crate) fn forget(base: &str) -> Result<(), String>` | native `:72`, wasm `:98` | |
| `fn local_storage() -> Result<web_sys::Storage, String>` | `:77` | wasm only; errors via `tr!` |
| `mod tests` | `:105` | Round-trip per server, trailing-slash equivalence, `0o600`, tightening of an existing `0o644` file |

---

## assistant/scene.rs

Responsibility: render the `Model` as compact JSON in millimetres for the LLM (doc `:3-4`). Schema and budgets: [llm-protocol.md](llm-protocol.md).

| Item | Line | Description |
|---|---|---|
| `pub(crate) const SCENE_BUDGET: usize = 24_000` | `:9` | Characters for scene + references |
| `const MAX_FACES: usize = 80` | `:10` | Faces listed by `describe` |
| `const MAX_LISTED_SELECTION: usize = 200` | `:11` | |
| `const MAX_LISTED_TAGS: usize = 100` | `:12` | |
| `pub(crate) fn mm(metres: f64) -> f64` | `:15` | Metres to mm, rounded to 0.1 mm |
| `pub(crate) fn from_mm(millimetres: f64) -> f64` | `:18` | mm to metres |
| `pub(crate) fn point_mm(p: DVec3) -> Value` | `:21` | `[x,y,z]` in mm |
| `fn unit(v: DVec3) -> Value` | `:24` | Direction, rounded to 0.001 |
| `pub(crate) fn kind_of(model, id) -> &'static str` | `:29` | `group`, `component`, `face`, `edge`, `dimension`, `text`, `guide`, `joint`, or `missing` |
| `pub(crate) fn bounds_json(model, ids: &BTreeSet<u64>) -> Option<Value>` | `:52` | `{min, max, size}` in mm via `Model::selection_bounds` |
| `fn axis_name(v: DVec3) -> Value` | `:57` | `"x"/"y"/"z"` if within 0.999 of an axis, else the vector |
| `fn material_json(model, object) -> Option<Value>` | `:66` | `{id, name}` |
| `pub(crate) fn object_entry(model, id) -> Option<Value>` | `:71` | One object: id, name, kind, parent, tag, `bounds_mm`, `size_mm`, material, grain, hidden, locked |
| `pub(crate) fn stock_list(model, catalog) -> Vec<Value>` | `:104` | Model stock materials overlaid with the purchasing catalog; dimensions in tenths of mm to mm |
| `fn joint_json(model, id) -> Option<Value>` | `:118` | `{id, a, b, kind}` |
| `pub(crate) fn summary(model, selection, catalog, budget: usize) -> Value` | `:129` | Whole-scene summary, objects ordered selection, in-scope, rest; truncates with `omitted` and `hint` |
| `pub(crate) fn outward_sign(model, face_id) -> f64` | `:183` | `+1` or `-1` mapping the stored face normal to the outward one |
| `fn face_json(model, id) -> Option<Value>` | `:195` | id, outward `normal`, `area_mm2`, `center_mm`, `facing` |
| `pub(crate) fn describe(model, id) -> Option<Value>` | `:217` | Object (with children and up to 80 faces), face, joint, or other entity |
| `pub(crate) fn references(model, ids, budget: usize) -> Value` | `:257` | Map id -> `describe`, within budget, with `omitted` count |

---

## assistant/tools.rs

Responsibility: the tool catalog (schemas) and the executor that applies a tool call to a `Model` with validation, lease checks, rollback and bounded results (doc `:1-4`). Per-tool behaviour: [tools.md](tools.md).

### Constants

| Item | Line | Value |
|---|---|---|
| `MAX_IDS` | `:20` | `500` ids per call |
| `MAX_POINTS` | `:21` | `1000` polyline points |
| `MAX_TEXT_CHARS` | `:22` | `500` (tag names, text notes) |
| `MAX_COORDINATE_MM` | `:24` | `1.0e5` (100 m) |
| `MAX_RESULT_M` | `:26` | `1000.` (1 km) |
| `MIN_SCALE` / `MAX_SCALE` | `:29` / `:30` | `1.0e-3` / `1.0e3`; also the `minimum` / `maximum` of the `scale` schema |
| `MAX_TAG_BYTES` | `:28` | `128`; core's tag-name limit, also `maxLength` of the `tag` schema properties |
| `TOOLS: &[(&str, bool)]` | `:99` | Names and mutating flag, 27 entries |
| `CREATING: &[&str]` | `:961` | Pure-creator tools exempt from the lease |

### Types

| Item | Line | Description |
|---|---|---|
| `pub(crate) struct ToolEnv` | `:34` | Per-turn state: `read_only`, `selection`, `hide_selection` (the designer turned off "Include selection": `get_scene` and `get_selection` do not reveal `selection`), `catalog`, `before: Option<Model>`, `lease: Option<BTreeSet<u64>>`, `created: BTreeSet<u64>`. `#[derive(Default)]` |
| `pub(crate) struct ToolOutcome` | `:54` | `result: Value`, `ok`, `mutated`, `created`, `modified`, `erased`, `joints` |
| `struct Fail { error: String, hint: Option<String> }` | `:66` | `From<&str>` `:70`, `From<String>` `:75` |
| `fn fail(error, hint) -> Fail` | `:80` | |
| `struct Done` + `type Res = Result<Done, Fail>` | `:85`, `:92` | Tool return: id lists plus `extra: Map` |
| `enum AxisArg {X,Y,Z}` + `fn axis(&self) -> Axis` | `:240`, `:246` | |
| `enum JointArg {...}` + `fn kind(&self) -> JointKind` | `:256`, `:265` | |
| `type V3 = [f64; 3]` | `:277` | |
| Argument structs (all `#[serde(deny_unknown_fields)]`) | `:279-423` | `Empty`, `FindArgs`, `IdArg`, `IdsArg`, `CreateBoardArgs`, `CopyArgs`, `PolylineArgs`, `MoveArgs`, `RotateArgs`, `ScaleArgs`, `ResizeArgs`, `PushPullArgs`, `RenameArgs`, `MaterialArgs`, `GrainArgs`, `GroupArgs`, `TagArgs`, `PaintArgs`, `JointParams`, `JointArgs`, `DimensionArgs`, `TextArgs` |

### Functions

| Signature | Line | Description |
|---|---|---|
| `fn modified(ids: &BTreeSet<u64>) -> Done` | `:94` | |
| `pub(crate) fn is_mutating(name: &str) -> bool` | `:129` | |
| `pub(crate) fn is_known(name: &str) -> bool` | `:132` | |
| `fn vec3()`, `ids()`, `id()`, `text()`, `number()`, `axis_enum()` | `:138-161` | JSON-Schema fragments |
| `fn def(name, description, properties, required) -> Value` | `:162` | Builds a function tool with `additionalProperties:false` |
| `pub(crate) fn definitions() -> Vec<Value>` | `:175` | The 27 tool schemas sent to the model |
| `fn parse<T: DeserializeOwned>(arguments: &str) -> Result<T, Fail>` | `:425` | Empty text -> `{}`; error "invalid arguments: ..." |
| `fn point(v: V3) -> Result<DVec3, Fail>` | `:435` | The mm to metre conversion with the 100 m bound |
| `fn finite(value: f64, what: &str) -> Result<f64, Fail>` | `:441` | |
| `fn unreachable_hint(model, id) -> String` | `:447` | Suggests `enter_object`/`exit_to_root` |
| `fn entities(model, ids, mutating: bool) -> Result<BTreeSet<u64>, Fail>` | `:458` | Count, existence, hidden, and (mutating) locked + in-scope checks |
| `fn objects(model, ids, mutating)` | `:483` | `entities` + must be group/component |
| `fn one_object(model, id) -> Result<u64, Fail>` | `:490` | |
| `fn find_material(model, env, id: MaterialId) -> Result<StockMaterial, Fail>` | `:494` | Catalog first, then model |
| `fn assign_material(model, env, object, material: Option<MaterialId>)` | `:501` | Copies the definition into the model |
| `fn tag_named(model, name) -> Result<u64, Fail>` | `:511` | Trims, rejects names over 128 *bytes* (`MAX_TAG_BYTES`, core's limit), then case-insensitive lookup or `add_tag` |
| `fn checked_name(name) -> Result<String, Fail>` | `:522` | 1-128 chars, trimmed |
| `fn read(value: Value) -> Res` | `:532` | Wrap a read result |
| Tool functions | | `create_board` `:540`, `create_copy` `:579`, `draw_polyline` `:592`, `move_objects` `:612`, `rotate` `:617`, `scale` `:623`, `resize_board` `:632`, `push_pull` `:653`, `rename` `:697`, `set_material` `:703`, `set_grain` `:713`, `group` `:718`, `ungroup` `:737`, `tag` `:746`, `paint` `:757`, `erase` `:770`, `add_joint` `:779`, `add_dimension` `:832`, `add_text` `:836`, `find_objects` `:844` |
| `fn run(model, env, name, arguments) -> Res` | `:865` | Dispatch; also inlines `get_scene`, `get_object`, `get_selection` (empty with a note when `hide_selection`), `list_stock_materials`, `select` (lease-restricted), `enter_object`, `exit_to_root` |
| `fn error_outcome(error: String, hint: Option<String>) -> ToolOutcome` | `:943` | |
| `fn target_ids(name, arguments) -> Vec<u64>` | `:964` | Existing ids named by a mutating call (`ids`, `id`, `face_id`, `a`, `b`) |
| `fn leased(model, env, id) -> bool` | `:976` | Walks up to 256 ancestors |
| `fn out_of_range(model, done: &Done) -> bool` | `:989` | Created/modified bounds non-finite or beyond 1 km |
| `pub(crate) fn execute(model: &mut Model, call: &ToolCall, env: &mut ToolEnv) -> ToolOutcome` | `:998` | The executor |
| `pub(crate) fn fallback_tool_call(content: &str) -> Option<ToolCall>` | `:1088` | Fenced-JSON / bare-JSON tool call |
| `pub(super) mod tests` | `:1118-1120` | `#[path = "tools_tests.rs"]` |

---

## assistant/turn.rs

Responsibility: one designer request: send scene + prompt, apply returned tool calls, feed results back, repeat until the model answers in text. A state machine advanced by `poll` once per frame; nothing blocks (doc `:1-4`).

| Item | Line | Description |
|---|---|---|
| `pub(crate) const MAX_ROUNDS: usize = 12` | `:17` | Max tool-call replies per turn |
| `const MAX_TOOL_RESULT_CHARS: usize = 8_000` | `:19` | Tool result message cap |
| `pub(crate) const MAX_CALLS_PER_REPLY: usize = 32` | `:23` | |
| `const REFERENCE_BUDGET: usize = scene::SCENE_BUDGET / 3` | `:25` | 8000 chars |
| `pub(crate) const SYSTEM_PROMPT: &str` | `:27-34` | Quoted in [llm-protocol.md](llm-protocol.md#system-prompt) |
| `pub(crate) struct TurnSetup { prompt, references, selection, include_selection, catalog, read_only, lease, history }` | `:37` | Captured when the user sends |
| `pub(crate) struct TurnResult { prompt, summary, created, modified, erased, joints, before: Option<Model>, stopped, error: Option<String>, selection: Option<BTreeSet<u64>>, scope: Option<u64> }` | `:50` | Handed to the app to commit; `scope` is the edit scope at turn start |
| `#[derive(Default)] pub(crate) struct Gate<'a> { block: Option<String>, lease: Option<BTreeSet<u64>>, recheck: Option<&'a dyn Fn() -> Option<String>> }` | `:69` | Per-frame shared-project conditions; `recheck` is asked before each mutating call |
| `pub(crate) struct Poll { finished: Option<TurnResult>, mutated: bool }` | `:79` | |
| `pub(crate) struct Turn` | `:85` | Private fields: `base`, `key`, `model_name`, `prompt`, `messages`, `tools`, `env: ToolEnv`, `selection_used`, `scope`, `pending: Option<Receiver<Result<ChatResponse, ChatError>>>`, `rounds`, id lists. Public: `activity: Vec<String>`, `started_at: f64` |
| `pub(crate) fn truncated_result(text: &str, ok: bool) -> String` | `:115` | Valid-JSON replacement for an oversized tool result: `{"ok":..,"truncated":true,"preview":"..."}` with a preview shrunk until the whole is within `MAX_TOOL_RESULT_CHARS` |
| `fn push_unique(list: &mut Vec<u64>, ids: &[u64])` | `:106` | |
| `pub(crate) fn start(ctx, transport: &dyn ChatTransport, (base, key, model_name): (&str,&str,&str), model: &Model, setup: TurnSetup) -> Self` | `:129` | Builds messages and sends the first request |
| `fn request(&mut self, ctx, transport)` | `:188` | |
| `pub(crate) fn poll(&mut self, ctx, transport, model: &mut Model, gate: &Gate) -> Poll` | `:200` | Advances at most one model reply |
| `pub(crate) fn stop(&mut self, model: &Model) -> TurnResult` | `:284` | |
| `fn finish(&mut self, model: &Model, summary: String, error: Option<String>, stopped: bool) -> TurnResult` | `:289` | |

---

## assistant/panel.rs

Responsibility: the chat UI: connect flow, transcript with change cards, composer with `@` references, busy banner. The same code renders docked and detached (doc `:1-3`). UI behaviour is described in [ui-ux.md](ui-ux.md).

### Constants

| Item | Line | Value |
|---|---|---|
| `COMPOSER_ID` | `:62` | `"assistant_composer"` |
| `MENTION_LIMIT` | `:63` | `8` |
| `CARD_ROWS` | `:64` | `12` |
| `CONNECT_ADVANCED_KEY` | `:65` | `"assistant_connect_advanced"` (stable section key) |

### Types

| Item | Line | Description |
|---|---|---|
| `enum Host { Docked, Window }` | `:22` | Windows fill their height |
| `struct Chip { label: String, id: u64 }` | `:30` | Reference pill; id authoritative |
| `struct ConnectDialog { key, base_url, submitted }` | `:36` | |
| `struct UiState` | `:46` | Session-only: `text`, `chips`, `last_prompt`, `notice`, `connect`, `model_filter`, `mention_index`, `mention_dismissed`, `focus_prompt`, `scroll_to_panel`, `key_checked`, `models_requested`, `geometry: window::Geometry` |
| `enum CardAction { Focus(u64), Undo }` (private) | `:147` | Deferred card clicks |

### Free functions

| Signature | Line | Description |
|---|---|---|
| `pub(crate) fn entity_label(model, id) -> (String, &'static str)` | `:68` | Object name or `"{kind} #{id}"` |
| `pub(crate) fn host_of(base_url: &str) -> String` | `:78` | Host part for the privacy note |
| `pub(crate) fn mention_query(text, cursor_chars) -> Option<(usize, String)>` | `:89` | `@query` before the caret (<= 40 chars, preceded by whitespace/start) |
| `pub(crate) fn rewrite_prompt(text, chips: &[Chip]) -> (String, BTreeSet<u64>)` | `:105` | `@Name` -> `@Name [#id]`, longest labels first, unmentioned chips appended |
| `fn icon_button(ui, icon, tooltip, color) -> bool` | `:796` | |
| `fn chip_pill(ui, text, exists, dimmed, colors) -> bool` | `:811` | Returns true when removed |
| `fn transcript_entry(ui, entry, model, position, latest, actions)` | `:841` | |
| `fn consume_plain_enter(input: &mut egui::InputState) -> bool` | `:882` | Removes and reports an Enter press without Shift; Shift+Enter is left to the text box (egui's `consume_key` ignores Shift) |
| `fn change_card(ui, changes, model, position, latest, actions)` | `:892` | Rows (created, modified, joints, then erased) up to `CARD_ROWS`, then "+N more" counting every category |
| `pub(crate) fn turn_undoable(changes: &Changes, position: usize, latest: usize) -> bool` | `:986` | |
| `pub(crate) fn erased_names(before: &Model, erased: &[u64]) -> BTreeMap<u64, String>` | `:991` | |

### `impl BenchUpApp` (`:152`, `:998`)

| Signature | Line | Description |
|---|---|---|
| `pub(crate) fn assistant_add_chips(&mut self, ids: impl IntoIterator<Item=u64>)` | `:154` | Objects only; duplicates skipped |
| `pub(crate) fn assistant_focus_entity(&mut self, id, ctx)` | `:169` | Select + `fit_selection` |
| `pub(crate) fn assistant_compose(&self) -> (String, BTreeSet<u64>)` | `:178` | Drops chips whose object no longer exists |
| `pub(crate) fn assistant_submit(&mut self, ctx)` | `:189` | Cancels interactions, sends, clears composer on success, sets notice on error |
| `pub(crate) fn assistant_cancel_interactions(&mut self)` | `:209` | Clears move/draw/measure/paste gestures |
| `pub(crate) fn assistant_panel(&mut self, ui, host: Host)` | `:224` | Whole body; lazily loads key and model list |
| `fn assistant_toolbar(&mut self, ui, host)` | `:256` | Latch/unlatch, menu (Change endpoint, Forget key), New chat, model picker |
| `fn assistant_model_picker(&mut self, ui)` | `:331` | Searchable selector, typed model ids allowed |
| `fn assistant_disconnected(&mut self, ui)` | `:378` | |
| `pub(crate) fn assistant_open_connect(&mut self, ctx, show_endpoint: bool)` | `:395` | |
| `pub(crate) fn assistant_dialogs(&mut self, ctx)` | `:408` | Connect modal (Cancel declared before Connect). A different address is validated through `Assistant::connect_to` and saved only after a successful connect |
| `fn assistant_transcript(&mut self, ui, height, ctx)` | `:514` | |
| `fn assistant_mention_candidates(&self, query) -> Vec<(u64, String, &'static str)>` | `:556` | Selected first, max 8 |
| `fn assistant_composer(&mut self, ui)` | `:578` | Chips, `@` popup, Enter/Up handling, drop target for `OutlinerDrag`, privacy note, include-selection checkbox, Send/Stop |
| `pub(crate) fn assistant_busy_banner(&mut self, ctx)` | `:1000` | Overlay with spinner + Stop |

---

## assistant/window.rs

Responsibility: latch / unlatch hosting (docked section, native OS window, or in-canvas floating window) and persisted geometry (doc `:1-7`).

| Item | Line | Description |
|---|---|---|
| `pub(crate) const DEFAULT_SIZE: [f32;2] = [440., 640.]` | `:16` | |
| `pub(crate) const MIN_SIZE: [f32;2] = [340., 300.]` | `:17` | |
| `const SETTLE_SECONDS: f64 = 0.4` | `:20` | Debounce for saving geometry |
| `const SECTION_KEY: &str = "assistant"` | `:21` | Inspector section key |
| `fn embedded_id() -> egui::Id` | `:23` | `Id::unique("benchup.assistant.window")`, shared by the embedded window and its layer |
| `pub(crate) fn embedded_layer() -> egui::LayerId` | `:27` | The embedded window's `Order::Middle` layer; "Bring to front" raises it |
| `pub(crate) fn viewport_id() -> egui::ViewportId` | `:31` | Hash of `"benchup.assistant"` |
| `pub(crate) struct Geometry { seen, since, frames }` | `:37` | Debounce state; `Default` |
| `pub(crate) fn position_usable(position, size, main: Option<Rect>, monitor: Option<Vec2>) -> bool` | `:50` | Heuristic against off-screen saved positions; `cfg(not wasm32 or test)` |
| `pub(crate) fn default_position(main, size) -> Option<Pos2>` | `:72` | Near the main window's top-right |
| `assistant_section_contents(&mut self, ui)` | `:80` | Docked panel, or "Open in a separate window" with Latch back / Bring to front |
| `assistant_set_detached(&mut self, ctx, detached: bool)` | `:102` | |
| `fn assistant_raise(&self, ctx)` | `:116` | Native: `ViewportCommand::Focus` on the detached window. Embedded (web): `ctx.move_to_top(embedded_layer())`. The prompt is focused by the callers via `focus_prompt` |
| `assistant_visible(&self, ctx) -> bool` | `:128` | |
| `assistant_reveal(&mut self, ctx)` | `:134` | |
| `assistant_toggle(&mut self, ctx)` | `:147` | |
| `assistant_shortcut(&self) -> egui::KeyboardShortcut` | `:159` | Command+Shift+key from settings; falls back to `M` |
| `assistant_toggle_shortcut(&mut self, ctx) -> bool` | `:165` | |
| `assistant_view_menu_item(&mut self, ui)` | `:174` | |
| `assistant_window(&mut self, ctx)` | `:185` | Per-frame; owns the detached window and dialogs |
| `fn assistant_embedded_window(&mut self, ctx, size)` | `:202` | In-canvas `egui::Window` (web) |
| `fn assistant_native_window(&mut self, ctx, size)` | `:229` | `show_viewport_immediate`; `cfg(not wasm32)` |
| `fn track_geometry(&mut self, ctx, position, size)` | `:286` | Debounced save into `Preferences.window` |

All except the free functions and the private fns are `pub(crate)` methods of `impl BenchUpApp` (`:78`).

---

## Test files

Run the module's tests with `cargo test -p benchup-app --lib assistant` and `... mira` (as `.github/workflows/ci.yml` does; the substring filters also select `assistant_ui`, the docking test and the sharing test); details in [testing.md](testing.md).

| File | Compiled from | Contents (selected helpers and tests) |
|---|---|---|
| `assistant/tests.rs` (1112 lines) | `assistant/mod.rs:25-27`, module `app::assistant::tests` | Helpers `in_temp_data_dir` `:11` (re-runs one test in a child process with `BENCHUP_DATA_DIR` set to a temp dir), `tool` `:32`, `new_board` `:35`, `connected_app` `:39`, `ask` `:46`, `finish` `:49`, `script_table` `:60`, `shared_app` `:696`, `x_of` `:704`, `Storage` mock `:340`. Tests cover full scripted turns, undo-step count, Stop, error handling, read-only, fenced fallback, round cap, `send` validation, connect/key flow, preference persistence and old-save defaults, scene units/truncation, references by id, chips/prompt rewrite, busy gate, remote replacement, undo-turn availability, lease, 32-call cap, shared budget, key-per-server, hidden selection, rolled-back turns, edit-scope restore, per-call access re-check, valid-JSON truncation, history roles, no-change turns, typed models, deferred base-URL save, Bring to front |
| `assistant/tools_tests.rs` (521 lines) | `assistant/tools.rs:1118-1120` (`pub(super) mod tests`) | Helpers `call` `:5`, `run` `:8`, `ok` `:11`, `err` `:16`, `board` `:25`, `bounds` `:28`, `near` `:31`, `first_face` `:359`. Tests per tool family, structured errors, lock/hidden/read-only, schema-vs-executor consistency (`:305`), fallback parser (`:327,344`), lease (`:364`), bounded output (`:404`), push_pull owner chain (`:436`), selection hiding, tag-name and `scale` schema limits and lease-restricted `select` (`:456` to the end) |
| `assistant/ui_tests.rs` (303 lines) | `app_tests.rs:493-495` as `mod assistant_ui` | Uses the shared UI `Harness`. Helpers `object` `:10`, `open_panel` `:19`. Tests: context-menu entry, disabled without selection, Connect modal button order, disconnected/connected panel, change cards and undo, Enter-to-send, Shift+Enter and blocked-send notice, the "+N more" card line, shortcut toggle, drop helpers, pointer tools paused while busy (`Stall` transport `:207`), shortcut key collisions |
| Inline tests | `assistant/client.rs:418`, `assistant/credentials.rs:105`, `inspector_docking.rs:944`, `sharing.rs:1201`, `sidebar.rs` preference tests | See the sections above and the touchpoints below |

---

## Touchpoints outside the module

From `git diff 64b2a44..HEAD --stat` (28 files; 11 of them are `assistant/*.rs`, the rest are listed below). Everything below is read from the diff.

### Application code

| File | Lines (HEAD) | Change | Why |
|---|---|---|---|
| `app.rs` | `:88-89` | `#[path = "assistant/mod.rs"] mod assistant;` | Declare the module as a child of `app` for private-field access |
| `app.rs` | `:322`, `:503` | Field `assistant: assistant::Assistant`, initialised with `Default` in `with_document` | Session state lives on the app |
| `app.rs` | `:753-792` (`finish_edit`, `commit_edit`) | Early return reverting `doc.model` and saying `assistant.busy_blocked` when `edited && assistant.busy()`; `commit_edit` returns whether the step was recorded and `finish_edit` wraps it | Busy gate: user edits cannot interleave with MIRA; the commit path can tell a rolled-back turn |
| `app.rs` | `:814-815` (`navigate_history`) | Refuse undo/redo/history jumps while busy | Same gate |
| `app.rs` | `:995-996` (`new_model`) | `self.assistant_stop(ctx)` first | A stale turn must not edit a new project |
| `app.rs` | `:2156-2164` (keyboard handler) | `assistant_toggle_shortcut` first; while busy only Esc (stop) is handled | Shortcut + keyboard gate |
| `app.rs` | `:4509-4515` (pointer press) | While busy, primary-button gestures ignored unless tool is Select or Orbit | Pointer gate; camera and selecting stay available |
| `app.rs` | `:6433` | `self.poll_assistant(&ctx)` in the frame loop | Drive the turn each frame |
| `app.rs` | `:6462-6463` | `assistant_busy_banner`, `assistant_window` | Overlay and detached window |
| `projects.rs` | `:166` | `self.assistant_stop(ctx)` at the start of project loading | Stop turn before the document is replaced |
| `sharing.rs` | `:372` | `self.assistant.discard_turn(&mut self.doc.model, tr!("assistant.error.shared_changed"))` in `apply_shared_remote` (`:345`) before `doc.model` is replaced; the edit scope is read after the discard | Never commit a stale snapshot against remote history |
| `sharing.rs` | `:750-755` | `pub(super) fn shared_lease(&self) -> Option<BTreeSet<u64>>` | Provide the lease to `assistant_send` and `Gate` |
| `sharing.rs` | `:756-780` | `#[cfg(test)] test_share_with_lease` (`:758`), `test_expire_lease` (`:778`) | Test hooks |
| `sharing.rs` | `:1202-1233` | Test `a_remote_replacement_discards_the_running_mira_turn` | Regression |
| `sidebar.rs` | `:14` | `pub(super) use outliner::{OutlinerCache, OutlinerDrag};` | Panel needs the drag payload type |
| `sidebar.rs` | `:64`, `:104` | `InspectorPreferences.assistant: assistant::Preferences` and its default | Persisted preferences (`#[serde(default)]` for old saves) |
| `sidebar.rs` | `:569-577` | `pub(super) fn section_open(ctx, key, default_open) -> bool` | Read a section's open state (used by `assistant_visible`) |
| `sidebar.rs` | `:613`, `:715` | `self.assistant.restore(&preferences.assistant)` / `assistant: self.assistant.preferences()` | Restore and capture |
| `sidebar.rs` | `:831`, `:869`, `:4001`, `:4014`, `:4032`, `:4050` | `InspectorSection::Assistant` variant (before the legacy `Fasteners`), added to `restore_order`, `COUNT = 11`, `ALL`, title `assistant.name`, key `"assistant"` | New last section; saved layouts keep order |
| `sidebar.rs` | `:4181` | Section colour for `"assistant"` | Visual |
| `sidebar.rs` | `:4913` | `self.assistant_view_menu_item(ui)` in the View menu | Menu entry with shortcut text |
| `sidebar.rs` | preference tests (`3396-3800` region) | Expected section order now ends with `Assistant` | Test updates |
| `inspector_docking.rs` | `:634` | `InspectorSection::Assistant => self.assistant_section_contents(ui)` | Render the section |
| `inspector_docking.rs` | `:943-968` | Test `saved_layouts_without_the_assistant_section_gain_it_at_the_end` | Layout migration regression |
| `outliner.rs` | `:12-13` | `OutlinerDrag` and its `ids` field made `pub(in crate::app)` | Panel accepts Outliner drops |
| `context_menu.rs` | `:28`, `:53-57`, `:326`, `:413-414` | `ContextAction::AskAssistant`: adds chips for the selection and reveals the panel; icon `"assistant"`; separator + menu item `assistant.ask_about_this` | "Ask MIRA about this" |
| `settings.rs` | `:158`, `:177` | `GeneralSettings.assistant_key: String` (default `"M"`) | Remappable shortcut |
| `settings.rs` | `:208-210` | Reset to default when not allowed | Sanitise on load |
| `settings.rs` | `:237-241` | `RESERVED_ASSISTANT_KEYS` (11 letters/Space) and `assistant_key_allowed` | Avoid Ctrl+Shift collisions with built-in commands |
| `settings.rs` | `:554-582`, `:586-590` | Settings > Shortcuts row for MIRA with rejection message; "restore defaults" also resets this key | UI |
| `shortcuts.rs` | `:98-101`, `:284` | Shortcut help row "MIRA: Cmd/Ctrl + Shift + key" | Documentation in-app |
| `icons.rs` | `:1485`, `:1500` | Line icons `"assistant"` (speech bubble) and `"latch"`/`"unlatch"` | Panel icons |
| `app_tests.rs` | `:494-495`, `:525`, `:5583` | `mod assistant_ui` (`assistant/ui_tests.rs`); `Assistant` in the harness section order and expected order | Tests |

### Non-code files

| File | Change |
|---|---|
| `assets/i18n/en.json` | 86 `assistant.*` keys added (one existing line gains a trailing comma) |
| `.github/workflows/ci.yml` | New CI: `cargo check --workspace --all-targets --locked`, core tests, `cargo test -p benchup-app --lib --locked 'assistant'` and `... 'mira'`, release-critical module tests, and a `wasm32-unknown-unknown` check |
| `README.md` | "Build with MIRA" section linking `docs/assistant.md` |
| `docs/README.md` | Index entry for `assistant.md` |
| `docs/sidebar.md` | One paragraph about the new section |
| `docs/assistant.md` | New user guide |
| `Cargo.toml` / `Cargo.lock` | **Not changed.** `ureq`, `tempfile`, `wasm-bindgen-futures`, `js-sys`, `web-sys` were already dependencies (`crates/benchup-app/Cargo.toml:24-33`) |
| `crates/benchup-core/**` | **Not changed** |

---

## Gaps / unclear

* `sidebar.rs` preference-test edits are spread over many hunks (`3396-3800` region in the original numbering); I listed them as one row rather than citing each assertion.
* `assistant/mod.rs:1-5` calls `window` a host "docked in the inspector or detached", but `window.rs` also implements the web in-canvas floating window; this is a doc-comment simplification, not a bug.
* `ui_tests.rs` is compiled from `app_tests.rs` rather than from `assistant/mod.rs`, unlike `tests.rs`; it relies on the `Harness` type defined there (`app_tests.rs`, `struct Harness` at `:497`).

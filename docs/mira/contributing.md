# Build with MIRA: contributing

[Contributor docs](README.md) · [Architecture](architecture.md) · [Module reference](module-reference.md) · [Tools](tools.md) · [LLM protocol](llm-protocol.md) · [UI and UX](ui-ux.md) · [Persistence and security](persistence-and-security.md) · [Testing](testing.md)

This page tells you how to set up the fork and gives step-by-step recipes for the common changes. Paths are under `crates/benchup-app/src/` unless noted (`benchup-tools/` and `benchup-mcp/` are `crates/<name>/src/`); line numbers refer to branch `feature/agent-tools` at commit `91c017f` (earlier ones to `feature/build-with-mira`, parent `44a7cc9`) and will drift, so search for the function name.

## 1. Fork quick start

```sh
git clone https://github.com/aarvee11/editor.git benchup
cd benchup
git remote add upstream https://github.com/BenchUp3D/editor.git
git fetch upstream
git switch feature/build-with-mira          # the feature branch lives on the fork
git switch -c my-change                     # then branch from it
```

Remote names are a convention: after a plain clone `origin` is the fork and `upstream` is BenchUp3D. (On the maintainer's machine the names are the other way round: `origin` = BenchUp3D, `fork` = aarvee11.) Pull requests for the feature target `BenchUp3D/editor` `main`; see [section 5](#5-pull-request-process).

**Prerequisites** (from the root `README.md`, "Prerequisites"):

* Rust 1.95 or newer through rustup (the repository was checked with 1.97.1).
* A C/C++17 compiler for the native SketchUp reader (not needed for browser builds).
* A GPU/driver supported by wgpu (the browser prefers WebGPU, then WebGL2).
* Linux: `sudo apt install build-essential pkg-config libx11-dev libxcb1-dev libxkbcommon-dev libwayland-dev libegl1-mesa-dev`. Windows: MSVC toolchain and Visual Studio C++ build tools. macOS: Xcode Command Line Tools.

**Run the desktop app:** `cargo run -p benchup-app`.

**Run in a browser:** `rustup target add wasm32-unknown-unknown`, `cargo install --locked trunk`, then `trunk serve` and open <http://127.0.0.1:8080> (keep the same origin to keep the saved model and, for MIRA, the saved key; `README.md` "Run in a browser").

**Try the assistant:** View → Build with MIRA (Ctrl/Cmd+Shift+M), Connect, paste a key from <https://portal.endlesstokens.ai/key> or point Advanced → Server address at any OpenAI-compatible server (`http://localhost:...` is allowed). To work without a key, read the tests: everything is covered with `FakeTransport` ([testing.md](testing.md#5-faketransport)).

**Verify:** `cargo test -p benchup-tools`, `cargo test -p benchup-app --lib assistant` and `cargo test -p benchup-core`. Expect the 31 failures that exist on `main` when running the whole suite ([testing.md](testing.md#2-the-31-pre-existing-failures-on-main)).

## 2. Repository conventions that apply here

All of these come from the root `AGENTS.md`; read it in full once.

| Rule | What it means for MIRA code |
|---|---|
| **Persistent options** | Every option, tool choice or expandable section must remember its last state. Put assistant preferences in `assistant::Preferences` (included in `InspectorPreferences`, `sidebar.rs:64`); use stable keys that do not depend on titles or order; keep loading backward compatible with `#[serde(default)]`; save through `persist_inspector_changes` (automatic once the field is in `Preferences`); add regression tests including "saved without a model edit". Recipe: [section 3.3](#33-add-a-persisted-assistant-setting). |
| **Disclosure state** | For a custom `CollapsingHeader` use `sidebar::restored_header_open` and `sidebar::remember_section` with a stable key, as the connect dialog does for `assistant_connect_advanced` (`assistant/panel.rs:65,443-465`). |
| **Modal actions** | Use the shared `Modal` (`modal.rs`). The footer renders right to left; declare actions from least to most impactful so Cancel/Close is on the right (`assistant/panel.rs:476-481`; test `assistant/ui_tests.rs:69`). |
| **Application language** | All user-facing text is English and goes through `tr!` and `assets/i18n/en.json`. Keep internal keys stable. Recipe: [section 3.4](#34-add-a-ui-string). |
| **Inspector tables** | Right-sidebar tables use `crate::inspector_table::InspectorTable`, vertically centred. The assistant panel has no inspector table; if you add one, do not use `egui::Grid`. |
| **GPU viewport rendering** | Anything drawn in the viewport must be GPU-first. The MIRA busy banner is an egui `Area` overlay, not scene geometry (`assistant/panel.rs:1002`). Do not generate per-frame CPU meshes for viewport features, and keep the assistant's work off the render path: tools run on the CPU model, then the normal pipeline refreshes the GPU scene through `changed()` (`assistant/mod.rs:438-440`). Verify on a large scene such as Rooftop. |
| **PDF export header** | Not used by the assistant; if you add any export, use `pdf_header.rs`. |
| **Release / deploy requests** | A "deploy" request means follow the release workflow in AGENTS.md. Do not run local builds, tests or benchmarks for such a request; GitHub Actions does that. This is separate from ordinary development. |
| **Compilation check** | At the end of each change, check the running BenchUp process and its trunk log (`/tmp/ketchup-trunk.log`) for new compile errors, once per fix, without starting a new build. |
| **Project names** | "BenchUp" is the Rust client in this repo; "Ketchup Server" is a separate PHP repo. The assistant never talks to Ketchup Server. |

## 3. Recipes

Each recipe lists the files and functions to touch and the tests to add. After any recipe run `cargo test -p benchup-app assistant`.

### 3.1 Add a new tool

A tool is added once, in the shared crate `benchup-tools`. Build with MIRA and the [MCP server](../mcp.md) both pick it up: MIRA through `openai_tools()`, MCP through `mcp_tools()`. You do not edit `assistant/` or `benchup-mcp` to add a tool, except for the wire-format fixture below. Everything is in `benchup-tools/src/ops.rs` unless noted; the old `assistant/tools.rs` is now only the OpenAI adapter. The existing `rename` tool is a compact example.

1. **Argument struct.** Add a `#[derive(Deserialize)] #[serde(deny_unknown_fields)] struct FooArgs { ... }` in the "arguments" section (near `RenameArgs`, `benchup-tools/src/ops.rs:404`). Use `u64` for ids, `Vec<u64>` for id lists, `[f64; 3]` (alias `V3`, `benchup-tools/src/ops.rs:326`) for mm vectors, `Option<T>` for nullable values. Lengths are millimetres on the wire.
2. **Schema.** Add a `def("foo", "description", json!({...properties...}), &["required", ...])` entry to `definitions()` (`benchup-tools/src/ops.rs:224`). Use the helpers `vec3()`, `ids()`, `id()`, `text()`, `number()`, `axis_enum()`. `def` sets `additionalProperties: false`. Every property in `required` must exist in `properties` (checked by test `tool_definitions_are_strict_and_match_the_executor`). Keep the description short and precise; it is the model's only documentation. State in the schema the range the executor enforces (`minimum`/`maximum`, `maxLength`), as `scale` and the tag name do; a schema that disagrees with the executor misleads the model. A tool that reveals the editor selection must honour `ToolEnv.hide_selection` ("Include selection" off).
3. **Register the name and classification.** Add `("foo", true)` (mutating) or `("foo", false)` to the `TOOLS` table (`benchup-tools/src/ops.rs:96`). **This flag decides**:
   * read-only projects: mutating tools are refused when `env.read_only` (`benchup-tools/src/ops.rs:1127-1132`);
   * the edit lease: mutating tools are checked with `leased()` against the ids found by `target_ids` (`benchup-tools/src/ops.rs:1135-1139`);
   * rollback: a backup is taken only for mutating tools (`benchup-tools/src/ops.rs:1148`) and `env.before` (the undo snapshot) is set only for them.
   A tool that changes the model must be `true`.
4. **Lease targets.** `target_ids` (`benchup-tools/src/ops.rs:1018-1025`) reads the existing ids from the raw arguments under the keys `ids`, `id`, `face_id`, `a`, `b`. If your tool names an existing entity under **any other key**, extend `target_ids` or the lease will not be enforced for it. If the tool only adds new entities and does not touch existing ones, add its name to `CREATING` (`benchup-tools/src/ops.rs:1015`).
5. **Executor function.** Write `fn foo(model: &mut Model, a: FooArgs) -> Res`. Validate **before** touching the model, because wasm cannot recover from a panic ([persistence-and-security.md](persistence-and-security.md#q04-wasm-panics)): use `entities(model, &ids, true)` / `objects(...)` / `one_object(...)` for ids (they check count <= `MAX_IDS`, existence, hidden, locked, editing scope; `benchup-tools/src/ops.rs:507-541`), `point(v)` and `finite(x, name)` for numbers (`benchup-tools/src/ops.rs:484-493`), `checked_name` for names (1-128 chars), and limit lengths with `MAX_TEXT_CHARS`/`MAX_POINTS`/`MAX_COORDINATE_MM`/`MIN_SCALE`/`MAX_SCALE` (`benchup-tools/src/ops.rs:17-27`). Return `Err(fail("what went wrong", "what to do instead"))` so the model can recover. Return `Ok(Done { created, modified, erased, joints, extra })`; `modified(&set)` is a shortcut. Use `scene::from_mm`/`mm` to convert units (`benchup-tools/src/scene.rs:15-19`).
6. **Dispatch.** Add `"foo" => foo(model, parse(arguments)?),` to the `match` in `run` (`benchup-tools/src/ops.rs:915`).
7. **Limits.** If the tool accepts lists or loops, add a constant and test it. Results are range-checked automatically for mutating tools (`out_of_range`, 1 km, `benchup-tools/src/ops.rs:1111`).
8. **Tests** (`benchup-tools/src/ops_tests.rs`, use `board()`, `run()`, `ok()`, `err()` helpers at the top; run them with `cargo test -p benchup-tools`): a success case with expected geometry; the failure cases (unknown id, locked, bad numbers); lease refusal if it targets existing entities (extend `shared_lease_refuses_entities_outside_it`, `:329`); bounds if it takes lists (`model_output_is_bounded`, `:369`). `tool_definitions_are_strict_and_match_the_executor` (`:304`) automatically checks schema/executor agreement and unknown-field rejection, and the count assertion in `scripted_turn_applies_every_call_and_reports_changes` (`tests.rs:75`) compares against `definitions().len()`, so it adapts.
   **Wire fixture in the assistant.** `tool_definitions_sent_to_the_server_are_unchanged` (`assistant/tools.rs:86`) compares `openai_tools()` with `assistant/tools_wire_fixture.json` and asserts `definitions().len() == 27`. A new tool changes both on purpose: add its entry to the fixture and bump the count, so a change to what MIRA sends is always visible in review. The MCP side needs no edit: `tools_list_has_every_tool_with_schema` (`benchup-mcp/tests/mcp.rs:70`) compares against `definitions()`.
9. **Docs.** Add an entry to [tools.md](tools.md) (and its counts, here and in [../mcp.md](../mcp.md)), mention it in [llm-protocol.md](llm-protocol.md) if it changes the prompt, and update the system prompt only if the model needs a hint ([section 3.6](#36-change-the-system-prompt-safely)). The user guide does not list tools.
10. **Not needed:** an i18n key (tool names and descriptions are for the model, not the user), a UI change (the change card lists created/modified/erased/joint ids automatically from `Done`).

### 3.2 Add a provider preset or a default model

There is **no preset registry**. The "preset" is a handful of constants and strings:

| What | Where |
|---|---|
| Default base URL | `DEFAULT_BASE_URL` (`assistant/client.rs:9`), used as the `Preferences` default (`assistant/mod.rs:40`), the sanitiser fallback (`:78-80`) and the hint of the Server address field (`assistant/panel.rs:451`) |
| "Get a key" link | `KEY_PORTAL_URL` (`assistant/client.rs:10`, used at `assistant/panel.rs:442`) |
| Default model hint | `DEFAULT_MODEL_HINT = "qwen"` (`assistant/mod.rs:34`, used at `:276-283`: first listed model whose id contains it, else the first; only applied when no model is chosen yet) |
| Dialog wording | `assistant.connect_title` ("Connect to endlesstokens"), `assistant.connect_subtitle`, `assistant.get_a_key`, `assistant.explainer` in `assets/i18n/en.json`; also `assistant.name` ("Build with MIRA") |
| Wire format | OpenAI Chat Completions: `GET {base}/models`, `POST {base}/chat/completions` (`client.rs:246-267`); no provider-specific code |

To change the default model, edit `DEFAULT_MODEL_HINT`, then update the test `connecting_saves_a_valid_key_and_rejects_an_invalid_one` (`assistant/tests.rs:308`), which expects `Qwen3.8-27B`. To change the default server, edit the two constants and the i18n strings above, and update `ui_tests.rs:192-199` (it asserts the host `api.endlesstokens.ai`) and the privacy-sentence assertion at `ui_tests.rs:93-118`.

To support **several** presets (a dropdown in the connect dialog) you would need a new struct, a `Preferences` field ([section 3.3](#33-add-a-persisted-assistant-setting)) and UI in `assistant_dialogs`; per-endpoint key storage already works for any number of servers (`credentials.rs:19-25`). A provider that is not OpenAI-compatible (for example the Anthropic Messages API) needs a new `ChatTransport` implementation and is listed as future work in [roadmap.md](roadmap.md). Only add a base URL that must pass `valid_base_url` (HTTPS, or loopback HTTP). Do not hard-code keys.

### 3.3 Add a persisted assistant setting

1. Add the field to `Preferences` (`assistant/mod.rs:40-40`), or to `WindowPreferences` (`:40-44`) if it is about placement. The struct is `#[serde(default)]`, so old data loads. If the type has no `Default`, give one.
2. Set its default in `impl Default for Preferences` (`:57-67`).
3. If the value can be hostile or corrupt, normalise it in `Preferences::sanitized` (`:69-83`), as is done for URL, model, position and size.
4. Bind it to the UI. For a checkbox follow the Include selection example: `ui.checkbox(&mut self.assistant.settings.include_selection, ...)` (`assistant/panel.rs:760`). Nothing else is needed for saving: `Assistant::preferences()` clones the whole struct into `InspectorPreferences` (`sidebar.rs:715`) and `persist_inspector_changes` saves when it differs (`sidebar.rs:793`).
5. For a collapsible section, use a stable key with `restored_header_open` / `remember_section` (see `CONNECT_ADVANCED_KEY`, `panel.rs:65,443-465`); those keys land in `InspectorPreferences.sections`.
6. For a keyboard shortcut, store it in `GeneralSettings` like `assistant_key` (`settings.rs:162,187,221-223`), and check collisions with the reserved list (`settings.rs:240`).
7. **Never** store a secret in `Preferences`. Keys go through `credentials`.
8. Tests: extend `old_preferences_without_assistant_fields_load_with_defaults` (`assistant/tests.rs:379`) with a JSON that lacks the field, and `settings_persist_without_a_model_edit_and_the_key_never_does` (`:355`) or `window_privacy_and_selection_preferences_persist_without_a_model_edit` (`:449`) with a changed value that survives `restore_inspector_preferences` without `project.modified`. If you sanitise, add an assertion like those in `saved_window_geometry_is_sanitized_and_old_saves_default` (`:477`).
9. Document it in [persistence-and-security.md](persistence-and-security.md#21-fields).

### 3.4 Add a UI string

1. Pick a key under `assistant.` (for example `assistant.my_label`). Keys are stable identifiers; do not rename existing ones. Errors use `assistant.error.*`, change-card text `assistant.card.*`.
2. Add `"assistant.my_label": "English text"` to `assets/i18n/en.json`. Templates use Rust format placeholders: `"{count} items"`; for plural quantities see the README "Application messages" section (`one` / `other`).
3. Use it with `tr!("assistant.my_label")` or `tr!("assistant.my_label", count = n)` (`use benchup_core::tr;`). The catalog is embedded at build time (README "Application messages"), so a missing key is a build-time problem, not a runtime one.
4. English only, even if the request comes in another language (AGENTS.md "Application language"). No hard-coded user-visible strings. Tool names shown in the progress line are the model's raw tool names and are not translated.
5. Add the key to the table in [ui-ux.md](ui-ux.md#12-i18n-keys-added-by-the-feature) and, if the text is asserted in a UI test, use the exact English string there as the existing tests do.

### 3.5 Add a reference type

Today references are objects (groups and components) only. To let `@` and drag-in attach another kind (for example tags or joints):

1. **Candidates.** `assistant_mention_candidates` (`assistant/panel.rs:556`) iterates `model.objects`; add your entities and a kind label key. Keep `MENTION_LIMIT` and the ordering rule in mind.
2. **Chip creation.** `assistant_add_chips` (`:154`) and `assistant_compose` (`:178`) skip ids that are not in `model.objects`; widen both checks (use `scene::kind_of`, `benchup-tools/src/scene.rs:29`, which already knows face, edge, dimension, text, guide and joint). `entity_label` (`:68`) derives the name from `model.objects`; give it a source for the new kind.
3. **Chip display.** `assistant_composer` marks chips whose id is gone (`:598-603`) by checking `model.objects`; update that check.
4. **Rewriting.** `rewrite_prompt` (`:105`) works on `Chip{label,id}` and needs no change as long as ids are `u64`.
5. **What the model receives.** `scene::references` (`benchup-tools/src/scene.rs:257`) calls `describe` (`benchup-tools/src/scene.rs:217`); extend `describe`/`object_entry` so the new kind produces useful JSON, and count it in the budget.
6. **Selection chip.** `live_selection` (`panel.rs:586-590`) counts only objects.
7. **Tools.** Make sure a tool can act on the kind (`entities()` in `benchup-tools/src/ops.rs:507` accepts any entity id but `objects()` accepts only groups/components).
8. Tests: add a case to `chips_rewrite_the_prompt_with_ids_and_collect_references` (`tests.rs:539`), `references_describe_entities_by_id_even_after_a_rename` (`:432`) and a UI test in `ui_tests.rs`. Update [ui-ux.md](ui-ux.md#4-references) and [llm-protocol.md](llm-protocol.md).

Drag sources other than the Outliner need their own payload type; the composer only reads `OutlinerDrag` (`assistant/panel.rs:679-691`).

### 3.6 Change the system prompt safely

The prompt is the constant `SYSTEM_PROMPT` in `assistant/turn.rs:27-34`, sent as the first message of every request (`turn.rs:152`). The second system message holds the scene JSON (`turn.rs:147-154`). No test asserts the prompt text.

1. Keep the **facts** true: units are millimetres, Z is up, boards are axis-aligned boxes unless rotated, ids come from the scene or the references block, face ids come from `get_object`. If you change a tool's name or arguments, update the prompt and [tools.md](tools.md) together.
2. Keep it short; it is sent on every round (up to 12 per turn) and counts against the model's context.
3. Never put secrets, user data or instructions that weaken safety ("always apply without checking") in it. Remember the model treats names in the scene as data, but cannot be forced to; see the prompt-injection row in the [threat model](persistence-and-security.md#7-threat-model).
4. Avoid wording that makes the model emit a tool call as plain text; the text fallback only accepts a whole-message JSON (`assistant/tools.rs:16`).
5. Manually run a representative set of prompts before and after (there is no automated evaluation; a golden-prompt harness is listed in [roadmap.md](roadmap.md)): "make a 600 x 400 x 750 mm side table with four 40 mm legs", an edit that needs `get_object` then `push_pull`, an `@` reference rename, a request the tools cannot do, and a request in a read-only shared project. Compare the number of rounds and error results, not the prose.
6. Update [llm-protocol.md](llm-protocol.md), which quotes the prompt.

## 4. Code style and commits

* **Format and lint:** the repository has no `rustfmt.toml` or clippy configuration checked in, and CI runs neither. Match the surrounding style (the assistant module uses rustfmt-like formatting with 4-space indentation, `//!` module docs and `///` doc comments that explain *why*). Run `cargo fmt` only on files you changed and avoid reformatting unrelated code in the same commit. Fix new compiler warnings; the wasm build is warning-free at `44a7cc9` and should stay so.
* **Keep invariants in comments.** The module already documents safety reasoning inline (for example `benchup-tools/src/ops.rs:1149-1151`, `credentials.rs:17-24`). Do the same.
* **Tests with the code.** New behaviour needs a test in the same commit; tests must follow the no-network and temporary-data-directory rules ([testing.md](testing.md)).
* **No secrets.** Never commit keys. Put them in an ignored file: `/.env.local` is in `.gitignore` ([testing.md](testing.md#12-planned-live-smoke-test-not-written-yet)).
* **Stage files by name** (`git add path/to/file`), never `git add .` or `-A`.
* **Commit messages** follow the history of this branch: an imperative one-line summary of at most about 72 characters ("Add Build with MIRA, an optional LLM design assistant", "Quote CI test filter so the workflow parses"), a blank line, then a short body of bullets explaining what and why, and a reference to the issue when relevant (`Relates to #1`). One logical change per commit.

## 5. Pull request process

1. Branch from `feature/build-with-mira` (or from `main` once the feature is merged). Keep changes focused.
2. Rebase on the latest base before opening: `git fetch upstream && git rebase upstream/main` (or the feature branch).
3. Open a **draft** pull request against `BenchUp3D/editor` `main` from your fork's branch. The feature itself is in the draft PR <https://github.com/BenchUp3D/editor/pull/3>. Link the proposal issue (`Relates to #1`, <https://github.com/BenchUp3D/editor/issues/1>) in the description.
4. Describe what changed, why, how to try it, and **testing honestly**: list the commands you ran, state that the full suite has **31 pre-existing failures on `main`**, give the before/after failing-set diff result ([testing.md](testing.md#21-how-to-diff-failing-sets-before-and-after-a-change)), say what you did **not** test (for example the native second monitor, the web build in a browser, live provider calls), and attach the wasm check result if you touched shared code.
5. Expect first-time contributors to need a maintainer to approve the CI run ("Approve and run workflows"); CI also needs the account's GitHub Actions billing to be in good standing ([testing.md](testing.md#10-ci)).
6. Mark ready for review only when CI is green (or the only red items are explained, pre-existing failures) and you have worked through the checklist below. Respond to review with new commits; squash only if the maintainers ask.
7. Do not push release tags or edit `assets/release-notes.json` unless a maintainer asks; the release process is described in AGENTS.md and the README.

## 6. Pre-PR checklist

- [ ] Branch is rebased; the diff only contains intended files; no stray formatting changes.
- [ ] `cargo test -p benchup-app assistant`, `cargo test -p benchup-app mira` and `cargo test -p benchup-core` pass locally (CI runs the same two substring filters; keep `assistant` or `mira` in new test names or paths).
- [ ] Failing-set diff against a clean `main` worktree is identical (31 known failures), or every new failure is explained.
- [ ] Local wasm checks pass with no warnings: `benchup-app` with default features and with `--no-default-features`, and `benchup-tools` ([testing.md](testing.md#9-local-wasm32-check) has the commands for rustup and for Homebrew Rust).
- [ ] New or changed tool: schema, args struct, `run` arm, `TOOLS` flag, `target_ids`/`CREATING`, validation before mutation, tests in `benchup-tools`, wire fixture and count in `assistant/tools.rs`, [tools.md](tools.md) updated.
- [ ] `cargo test -p benchup-tools --locked`, `cargo test -p benchup-mcp --locked` and `cargo check -p benchup-app --locked --no-default-features` pass (CI runs them; see [testing.md](testing.md#13-tools-mcp-and-agent-link-tests)).
- [ ] New persisted field: default, sanitiser, old-data test, "no model edit" test, stable key, docs.
- [ ] New strings: in `assets/i18n/en.json`, English only, via `tr!`.
- [ ] New modal: shared `Modal`, least impactful action on the right.
- [ ] No secret, key or personal data in code, tests, logs or docs.
- [ ] No test touches the network or the real data directory; tests that commit edits use `in_temp_data_dir`.
- [ ] Viewport work stays GPU-first; large scene (Rooftop) checked if you touched viewport or scene code.
- [ ] Manual checklist items relevant to your change were run ([testing.md](testing.md#11-manual-test-checklist)).
- [ ] User guide ([../assistant.md](../assistant.md)) updated if behaviour users see changed; contributor docs updated if internals changed.
- [ ] PR description is honest about what was and was not tested and links issue #1.

## Gaps / unclear

* There is no `CONTRIBUTING.md`, PR template, issue template, `rustfmt.toml` or clippy config in the repository at `44a7cc9`; the style and PR expectations above are inferred from the branch history and AGENTS.md, not from an upstream policy.
* Which remote is called `origin` differs between the maintainer's checkout and a fresh clone (section 1).
* Whether upstream wants squash merges, signed commits or a CLA is unknown.
* Homebrew's Rust has no wasm32 target. [testing.md](testing.md#9-local-wasm32-check) shows how to add rustup next to it for the browser check, and how to remove it without deleting the shared `~/.cargo` cache.

# Build with MIRA: tool catalog

[Docs index](README.md) · [User guide](../assistant.md) · [Architecture](architecture.md) · [Module reference](module-reference.md) · Tool catalog · [LLM protocol](llm-protocol.md) · [UI/UX](ui-ux.md) · [Persistence and security](persistence-and-security.md) · [Testing](testing.md) · [Contributing](contributing.md) · [Development history](development-history.md) · [Roadmap](roadmap.md)


**Citation convention.** `path:line` is relative to `crates/benchup-app/src/` unless it starts with `benchup-core/` (`crates/benchup-core/src/`). Lines are for branch `feature/build-with-mira` after the commit "Fix issues found while documenting MIRA" (parent `44a7cc9`). `tools.rs` means `assistant/tools.rs`.

There are 27 tools (`TOOLS`, `assistant/tools.rs:99`): 19 mutating and 8 non-mutating. The non-mutating set is `get_scene`, `find_objects`, `get_object`, `get_selection`, `list_stock_materials`, `select`, `enter_object`, `exit_to_root`. The schemas are built in `definitions()` (`tools.rs:175`); each uses `additionalProperties: false` (`def`, `tools.rs:162`) and the matching argument struct uses `#[serde(deny_unknown_fields)]` (`tools.rs:279-423`), so unknown fields are rejected with "invalid arguments: ...".

## Contents

[Shared behaviour](#shared-behaviour) · [Global limits](#global-limits) · [Units](#unit-conversion) · [Lease and read-only](#lease-read-only-and-created-ids) · [Result shape](#result-shapes) · [Read tools](#read-tools) · [Create tools](#create-tools) · [Transform tools](#transform-tools) · [Edit tools](#edit-tools) · [Scope and selection tools](#scope-and-selection-tools) · [Special topics](#special-topics) · [Gaps](#gaps--unclear)

## Shared behaviour

Order of checks in `execute` (`tools.rs:998`):

1. Unknown tool name -> `{"ok":false,"error":"unknown tool X","hint":"Available tools: ..."}` (`:1000-1003`).
2. Mutating and `env.read_only` -> error `read-only` (`:1005-1010`).
3. Mutating and any existing target id is outside the lease -> error `not-leased` (`:1012-1016`).
4. `backup = model.clone()` for mutating tools (`:1017`).
5. `run` under `catch_unwind` (`:1021`). A panic becomes "internal error while applying the tool" (`:1021`).
6. `Err` -> `*model = backup`, error outcome (`:1027-1032`).
7. `Ok` but `out_of_range` -> `*model = backup`, error `result out of range` (`:1033-1039`).
8. `Ok` -> record ids, set `env.before` on first real change, build the result (`:1040-1082`).

Argument parsing: `parse` (`tools.rs:425`) treats an empty/blank argument string as `{}`; a parse failure returns `invalid arguments: <serde error>` with the hint "Send a JSON object that matches the tool schema exactly: required fields, no extra fields, lengths in mm." Ids are `u64` and all tools using `entities` first check count, existence and hidden state (`tools.rs:458`):

| Check (in order) | Error | Hint |
|---|---|---|
| ids empty | `ids must not be empty` | Pass at least one entity id from get_scene. |
| more than 500 ids | `too many ids` | Pass at most 500 ids per call. |
| id not in model | `unknown id N` | Call get_scene or find_objects to see valid ids. |
| hidden | `entity N is hidden` | Hidden entities cannot be used. |
| mutating: locked | `entity N is locked` | Locked entities cannot be changed; tell the designer to unlock it. |
| mutating: `entity_in_context(id) != Some(id)` | `entity N is not editable in the current scope` | `unreachable_hint` (`tools.rs:447`): `N is inside object P: call enter_object{id:P} first, or exit_to_root ...`, else `Call exit_to_root, then retry.` |

`objects()` (`:483`) additionally requires group/component: `N is not a group or component` / "Use ids of objects listed under objects in get_scene." These checks are referred to below as **E** (entities, mutating) and **O** (objects, mutating). Core functions that return `Err(&'static str)`/`String` are surfaced as the error text with no hint (`From<&str>`/`From<String>` for `Fail`, `tools.rs:70-79`).

## Global limits

| Limit | Value | Where | Failure |
|---|---|---|---|
| Tool calls executed per model reply | 32 (`MAX_CALLS_PER_REPLY`) | `assistant/turn.rs:23,247-251` | Calls 33+ get a tool message `{"ok":false,"error":"too many calls in one reply"}` and are not executed |
| Model replies with tool calls per turn | 12 (`MAX_ROUNDS`) | `assistant/turn.rs:17,275` | Turn ends with an error; the 12th batch is executed |
| Ids per call | 500 (`MAX_IDS`) | `tools.rs:20,462` | `too many ids` |
| Polyline points | 1000 (`MAX_POINTS`), min 2 | `tools.rs:21,593-598` | `points_mm needs at least 2 points` / `too many points` |
| Text length | 500 chars (`MAX_TEXT_CHARS`) for `add_text.text` | `tools.rs:22,825` | `text too long` |
| Tag name length | 128 *bytes* after trim (`MAX_TAG_BYTES`, core's limit); `maxLength: 128` in the schema | `tools.rs:28,513` | `tag name too long` |
| Name length | 1-128 chars after trim (`checked_name`) | `tools.rs:522` | `name must be 1-128 characters` |
| Input coordinate / vector | finite and each \|component\| <= 1e5 mm = 100 m (`MAX_COORDINATE_MM`) | `tools.rs:24,435-439` | `coordinate out of range` / "Use finite values within 100 m (100000 mm) of the origin, in millimetres." Applies to every `*_mm` vector via `point()`, including `size_mm`, `offset_mm`, `delta_mm` |
| `create_board` extents | 0.1 mm to 100000 mm each | `tools.rs:540` (checked in metres: `< 1.0e-4` or `> 100.`) | `size_mm out of range` |
| `resize_board` extents | each >= 0.1 mm (upper bound only the 100 m vector bound) | `tools.rs:632` | `size_mm out of range` |
| `scale` factor | 0.001 to 1000 (`MIN_SCALE`/`MAX_SCALE`); the schema says `minimum 0.001, maximum 1000` | `tools.rs:29-30,626` | `factor out of range` |
| Result extent | all created/modified entities must have finite bounds within 1000 m (1 km) of the origin (`MAX_RESULT_M`) | `tools.rs:26,989-994,1033` | `result out of range` + rollback |
| `find_objects.limit` | default 40, clamped 1-100 | `tools.rs:846` | |
| `add_joint` clamps | `fingers`, `dovetails` 1-64; `taper_degrees` 0-30; `clearance_mm` 0-5; ratios must be in (0, 1] | `tools.rs:786-812` | ratios: `<name> must be between 0 and 1`; others are clamped silently |
| Tool result text sent to the model | 8000 chars; a longer result is replaced by `{"ok":<bool>,"truncated":true,"preview":"<first ~7000 chars>"}` (the preview shrinks until the whole message fits, because JSON escaping can lengthen it) | `assistant/turn.rs:20,115` | Always valid JSON; the model should call a narrower tool (`find_objects`, `get_object`) |
| `objects` summaries in a mutating result | first 20 touched entities | `tools.rs:1069` | |
| Faces listed by `get_object` | 80, `faces_omitted` gives the rest | `assistant/scene.rs:10,226-231` | |
| Stock list in scene summary | 40 | `assistant/scene.rs:143` | `list_stock_materials` is unbounded (`tools.rs:888`) |

## Unit conversion

* Wire: millimetres, rounded to 0.1 mm on output (`scene::mm`, `assistant/scene.rs:15`). Core: metres.
* Input conversion happens only in `point()` (`tools.rs:435`) via `from_mm` (`scene.rs:18`, divide by 1000), plus `from_mm` for `distance_mm` (`tools.rs:674`) and `clearance_mm` (`:811`).
* `area_mm2` is `face_area * 1e6` rounded (`scene.rs:205`); stock dimensions are stored in tenths of mm and divided by 10 (`scene.rs:112`).
* Angles are degrees on the wire (`rotate.degrees`, `taper_degrees`); passed through unchanged to core.
* Z is up (`up_axis: "z"` in the scene, `scene.rs:139`).

## Lease, read-only and created ids

* **Read-only** (`ToolEnv.read_only`, `tools.rs:36`): refuses all 19 mutating tools. The 8 non-mutating tools, including `select`, `enter_object` and `exit_to_root`, still run.
* **Lease** (`ToolEnv.lease`, `tools.rs:48`; `None` outside shared projects). `target_ids` (`tools.rs:964`) reads the raw JSON for keys `ids`, `id`, `face_id`, `a`, `b`. For tools in `CREATING` (`create_board`, `create_copy`, `draw_polyline`, `add_dimension`, `add_text`, `tools.rs:961`) it returns nothing. An existing target id is *leased* if it or any ancestor (walk up to 256 steps via `Model::parent`) is in the lease or in `env.created` (`leased`, `tools.rs:976`). Ids that do not exist are skipped here and fail later as `unknown id`. Failure: `{"ok":false,"error":"not-leased","hint":"select it first so you hold the edit lease"}`.
* **Created-ids exemption.** After each successful call `env.created` is extended with `done.created` (`tools.rs:1041`), and after `ungroup` also with the released ids `done.modified` (`:1042-1045`). Those entities (and anything inside them) are leased for the rest of the turn.
* **Lease is refreshed every reply** from the app's `Gate` (`assistant/turn.rs:235`).
* **Read tools are not lease-checked.** `select` is non-mutating but is restricted when a lease is present: it refuses ids that are neither leased (`leased`, which includes created ids) nor created this turn, with `entity N is not in your edit lease` (`tools.rs:914`). Outside shared projects it is unchanged.

## Result shapes

Every successful result is a JSON object with `"ok": true` (`tools.rs:1053-1054`).

Mutating tools add (`tools.rs:1055-1071`):

```json
{
  "ok": true,
  "created": [12],
  "modified": [],
  "erased": [3],
  "bounds_mm": {"min": [0,0,0], "max": [40,40,700], "size": [40,40,700]},
  "objects": [ { "id": 12, "name": "Leg", "kind": "group", "size_mm": [40,40,700], "bounds_mm": {"min": [..], "max": [..]} } ]
}
```

* `created` / `modified` are filtered to ids that still exist; `erased` appears only when non-empty.
* `bounds_mm` covers created+modified entities (omitted if none have bounds).
* `objects` = `scene::object_entry` for up to 20 of those (omitted when empty; faces/edges do not produce entries because `object_entry` only handles objects, `scene.rs:71-73`).
* Tool-specific extras are merged in (`new_face_id`, `members`, `note`, `tag_id`, `joint_id`).

Error: `{"ok": false, "error": "...", "hint": "..."}` (`error_outcome`, `tools.rs:943`; `hint` optional).

`object_entry` fields (`assistant/scene.rs:71-101`): `id`, `name`, `kind` (`group`|`component`), optional `parent`, `tag` (inherited tag name), `bounds_mm {min,max}`, `size_mm`, `material {id,name}`, `grain` (`"x"|"y"|"z"` or a unit vector), `hidden: true`, `locked: true`.

---

## Read tools

All read tools run without a backup and cannot change the model.

### get_scene

* Kind: read. Purpose: the full scene summary again.
* Args: none (`{}`; extra fields rejected, `Empty`, `tools.rs:281`).
* Core calls: `scene::summary` (`scene.rs:129`) with the full `SCENE_BUDGET` (24 000) and the *current* selection (`tools.rs:870`), or an empty selection when `ToolEnv.hide_selection` is set ("Include selection" off).
* Result: the scene object documented in [llm-protocol.md](llm-protocol.md#scene-summary) plus `ok:true`.
* Errors: `invalid arguments` only.

### find_objects

* Kind: read. Purpose: search groups/components when the summary was truncated.
* Args: `query` string (required), `limit` integer 1-100 (optional, default 40).
* Matching (`tools.rs:847-857`), on objects only, case-insensitive: empty query matches all; exact id; name substring; kind equal to `group` or `component`; inherited-tag name substring; material name substring. Hidden objects are included.
* Core: `Model::inherited_tag` (`benchup-core/src/tags.rs:91`), `Model::object_stock_material` (`benchup-core/src/objects.rs:118`), `scene::kind_of`, `scene::object_entry`.
* Result: `{"ok":true,"total":N,"objects":[object_entry...]}` (first `limit`).
* Errors: `invalid arguments`.

### get_object

* Kind: read. Purpose: details of one entity; faces for `push_pull`.
* Args: `id` integer (required).
* Core: `scene::describe` (`scene.rs:217`).
* Result by entity type:
  * Object: `object_entry` fields + `children` (ids), `faces` (up to 80 of `{id, normal, area_mm2, center_mm?, facing?}`), `faces_omitted` if more. `normal` is the **outward** normal (see [outward normals](#outward-normals-and-push_pull)); `facing` is `"+x"|"-x"|"+y"|"-y"|"+z"|"-z"` when within 0.999 of an axis. Faces come from `Model::expand_entities` (`benchup-core/src/objects.rs:224`), so nested objects' faces are included.
  * Face: the face entry + `kind:"face"` + `parent`.
  * Joint: `{id, a, b, kind}`.
  * Other entity: `{id, kind, bounds_mm?}`.
* Errors: `unknown id N` / "Call get_scene or find_objects." Does **not** check hidden or locked.

### get_selection

* Kind: read. Args: none.
* Result: `{"ok":true,"selection":[ids that still exist],"objects":[object_entry...]}` from `env.selection` (`tools.rs:882-886`), which is the editor selection at send time, possibly changed by `select`. With "Include selection" off (`ToolEnv.hide_selection`) it returns `{"ok":true,"selection":[],"note":"The designer has turned off sharing their selection."}` (`tools.rs:880`).

### list_stock_materials

* Kind: read. Args: none.
* Core: `scene::stock_list` (`scene.rs:104`): model stock materials overlaid with the purchasing catalog from settings.
* Result: `{"ok":true,"materials":[{"id":"<uuid>","name":..,"thickness_mm":..,"width_mm":..,"length_mm":..}]}`. `id` is a UUID string (`MaterialId = uuid::Uuid`, `benchup-core/src/lib.rs:291`).

---

## Create tools

### create_board

* Kind: mutating (in `CREATING`: lease exempt). Purpose: one rectangular board as a named group.
* Args (`CreateBoardArgs`, `tools.rs:300`):

| Field | Type | Req | Notes |
|---|---|---|---|
| `name` | string | yes | 1-128 chars |
| `origin_mm` | [x,y,z] number | yes | minimum corner, mm |
| `size_mm` | [x,y,z] number | yes | extents 0.1 mm-100000 mm |
| `material_id` | string (UUID) | no | from `list_stock_materials`; schema says string, parsed as `Uuid` |
| `grain_axis` | `"x"\|"y"\|"z"` | no | |
| `tag` | string (`maxLength` 128) | no | created if missing; blank ignored; at most 128 bytes |

* Procedure (`tools.rs:540-577`): build a rectangle in a scratch `Model` with `draw_polyline` (`benchup-core/src/document.rs:209`), extrude the face along +Z by `size.z` with `push_pull_along` (`benchup-core/src/modeling.rs:444`), `paste_entities` into the live model (`benchup-core/src/objects.rs:445`), `make_object` with `component=false` (`benchup-core/src/objects.rs:638`), then `assign_material` (tools-level), `set_grain_direction` (`benchup-core/src/objects.rs:74`) and `tag_named` + `assign_tag` (`benchup-core/src/tags.rs:13,66`).
* Result: standard mutating shape; `created: [group id]`.
* Errors: `name must be 1-128 characters`; `coordinate out of range`; `size_mm out of range`; `unknown material_id X` ("Call list_stock_materials for valid ids."); `the board could not be created` ("The current editing scope may be locked; try exit_to_root."); `tag name too long`; core messages (for example from `add_tag`, which rejects blank, over 128 bytes, or "Untagged", `benchup-core/src/tags.rs:13-17`).

### create_copy

* Kind: mutating, `CREATING` (the source is only read). Purpose: copy an object/group and offset it.
* Args: `id` integer, `offset_mm` [x,y,z] (both required).
* Source check: `entities(.., mutating=false)`: exists and visible, locked allowed, scope not required (`tools.rs:580`).
* Core: `copy_entities` (`benchup-core/src/objects.rs:391`), `paste_entities` (`:445`). New ids = pasted roots not previously in `root_entities`.
* Result: standard shape, `created` = new ids.
* Errors: id errors; `coordinate out of range`; `nothing was copied`.

### draw_polyline

* Kind: mutating, `CREATING`. Purpose: loose connected edges through points; a closed loop makes a face. Not grouped.
* Args: `points_mm` array of [x,y,z] (2-1000, required), `close` boolean (optional; appends the first point).
* Core: `Model::draw_polyline(&points, false)` (`benchup-core/src/document.rs:209`).
* Result: standard shape; `created` = new edge and face ids (diff of edge+face key sets, `tools.rs:605-609`).
* Errors: `points_mm needs at least 2 points` ("Give the corners in order."); `too many points`; `coordinate out of range`; core errors.

### add_dimension

* Kind: mutating, `CREATING`. Args: `start_mm`, `end_mm`, `offset_mm` (all [x,y,z], required).
* Core: `Model::add_dimension` (`benchup-core/src/measurements.rs:217`).
* Result: `created: [dimension id]`. Errors: `coordinate out of range`, core errors.

### add_text

* Kind: mutating, `CREATING`. Args: `point_mm`, `offset_mm` ([x,y,z]), `text` (string <= 500 chars); all required.
* Core: `Model::add_text` (`benchup-core/src/annotations.rs:31`).
* Result: `created: [text id]`. Errors: `text too long` ("Use at most 500 characters."), `coordinate out of range`, core errors.

---

## Transform tools

All use **E** (ids exist, visible, unlocked, direct children of the current edit scope) and are lease-checked.

### move

* Args: `ids` integer array, `delta_mm` [x,y,z]. Core: `Model::move_selection` (`benchup-core/src/modeling.rs:145`). Result: `modified` = ids.
* Errors: E errors, `coordinate out of range`, core errors, `result out of range`.

### rotate

* Args: `ids`, `pivot_mm` [x,y,z], `axis` `"x"|"y"|"z"`, `degrees` number (finite; no other bound). Core: `rotate_selection` (`modeling.rs:166`) with the world axis vector. Result: `modified`.
* Errors: E, `degrees must be a finite number`, `coordinate out of range`, core.

### scale

* Args: `ids`, `center_mm`, `factor` (number, 0.001-1000; the schema says `minimum 0.001`, `maximum 1000`, the same range the executor enforces). Uniform. Core: `scale_selection` (`modeling.rs:225`). Result: `modified`.
* Errors: E, `factor must be a finite number`, `factor out of range` ("Use a factor between 0.001 and 1000; scale in several steps if needed."), `coordinate out of range`, `result out of range`.

### resize_board

* Args: `id`, `size_mm` [x,y,z] (world-axis extents). Uses **O** on one id.
* Procedure (`tools.rs:632-652`): compare the world-axis-aligned bounds (`selection_bounds`, `benchup-core/src/modeling.rs:25`) with the target; for each differing axis call `scale_selection_constrained(&set, lo, new/old, Some(AxisConstraint::Axis(axis)))` (`modeling.rs:248`) about the minimum corner `lo`. Axes within 1e-9 are skipped.
* Result: `modified: [id]`.
* Errors: O errors; `size_mm out of range` ("Each extent must be at least 0.1 mm."); `the object has no geometry`; `the object is flat along that axis and cannot be resized`; core; `result out of range`.
* Note: works on world-axis bounds, so a rotated board is resized along world axes, not its own.

### push_pull

* Args: `face_id` integer, `distance_mm` number. Positive = outward, negative = into the part.
* Checks (`tools.rs:653-674`): is a face (`{face} is not a face`, "Use a face id from get_object."); not hidden/locked (`face N is hidden or locked`); `entity_in_context(face)` exists (`face N is not editable in the current scope` + hint); that top entity passes **E**; every ancestor object is visible and unlocked (`face N is inside hidden or locked object P`); `distance_mm` finite.
* Distance: `from_mm(distance_mm) * scene::outward_sign(model, face)` (see below). Core: `Model::push_pull(face, distance)` (`benchup-core/src/modeling.rs:423`; rejects non-finite, `|d| < EPS` or `|d| > 1e7` m).
* Scope: if the face's owner is not the current edit scope, `model.edit_context` is temporarily set to the owner and restored afterwards (`tools.rs:675-686`).
* Result: standard shape; `modified: [owner object]` when the owner is an object; extra `new_face_id` when the returned face still exists.
* Lease: the check uses `face_id` and walks up ancestors, so owning an enclosing board suffices.

### set_grain

* Args: `id` integer, `axis` `"x"|"y"|"z"` or `null` (required, may be null). **O**. Core: `set_grain_direction` (`benchup-core/src/objects.rs:74`). Result: `modified: [id]`. Errors: O, core.

---

## Edit tools

### rename

* Args: `id`, `name` (1-128 chars). **O**. Sets `object.name` directly (no core function, `tools.rs:700`). Result: `modified: [id]`.

### set_material

* Args: `ids`, `material_id` string UUID or `null` (clears). **O**. `find_material` looks in the catalog first, then the model (`tools.rs:494`); `assign_material` copies the definition into `model.stock_materials` and sets `stock_material_id`, clearing any inline `stock_material` (`tools.rs:501-510`). Result: `modified`. Errors: O, `unknown material_id X`.

### group

* Args: `ids`, `name`, `component` (boolean, all required). **E** (any entity type, not only objects). Core: `Model::make_object` (`benchup-core/src/objects.rs:638`).
* Result: standard shape with `created: [new group id]`, `erased: [input ids that no longer exist]`, plus `members` (object entries of the group's direct `entities` that are objects) and `note: "Member ids may have changed; use the ids in members."` See [group id remapping](#group-id-remapping).
* Errors: E, name errors, `the selection could not be grouped` ("Locked or empty selections cannot be grouped.").

### ungroup

* Args: `ids`. **O**. Core: `Model::dissolve_objects(&set, false)` (`benchup-core/src/objects.rs:738`): one level, nested groups preserved.
* Result: `modified` = released ids that exist, `erased` = input ids that no longer exist. Released ids are added to `env.created` (lease exemption).

### tag

* Args: `ids`, `tag` string (at most 128 bytes) or `null`/blank (clears). **E**. `tag_named` (`tools.rs:511`) finds a tag case-insensitively or creates it with `add_tag` (`benchup-core/src/tags.rs:13`); `assign_tag` (`tags.rs:66`).
* Result: `modified` plus `tag_id` (number or `null`). Errors: E, `tag name too long`, core (blank, >128 bytes, "Untagged").

### paint

* Args: `ids`, `rgb` [r,g,b] integers 0-255 (parsed as `[u8;3]`). **E**. Objects use `paint_object` (`benchup-core/src/modeling.rs:350`), faces `paint_face` (`modeling.rs:282`).
* Errors: E; `N cannot be painted` ("Paint objects or faces.") for other entity types; core. Note the loop aborts on the first bad id and the whole call is rolled back.

### erase

* Args: `ids`. **E**. Core: `Model::erase` (`benchup-core/src/document.rs:279`). Result: `erased` = ids no longer present. Error `nothing was erased` if none disappeared.

### add_joint

* Args (`JointArgs`, `tools.rs:404`): `a` and `b` integers (different boards), `kind` one of `mortise_tenon`, `half_lap`, `tongue_groove`, `rabbet`, `dovetail`, `finger`; optional `params` object (no extra keys): `width_ratio`, `height_ratio` (0-1], `fingers`, `dovetails` (integers, clamped 1-64; negative values fail parsing), `taper_degrees` (clamped 0-30), `clearance_mm` (clamped 0-5, converted to metres), `depth_axis`, `width_axis` (integers 0-2).
* Both ids must pass **O** (so groups/components). Core: `Model::add_connection` with `ConnectionOperation::Joinery`, `entry_face: None` (`benchup-core/src/connections.rs:279`). Core rejects an existing joint between the pair, hidden/protected members, members not in scope, and evaluation errors.
* Result: `modified: [a, b]`, `joints: [joint id]` (surfaces in the change card, not in the standard `created`), extra `joint_id`.
* Errors: `a and b must differ`; O; `<ratio> must be between 0 and 1`; `axis indexes are 0, 1 or 2`; `taper_degrees must be a finite number`; core messages (for example "these pieces already have a wood joint ...", "select two visible editable boards in the current ...", or joinery evaluation errors, joined with the core separator).
* Tip from the schema description: the boards must already overlap where they meet.

---

## Scope and selection tools

### select

* Kind: non-mutating (editor state only). Args: `ids`. Uses `entities(.., mutating=false)`: exist, visible, <= 500; locked and out-of-scope allowed. In a shared project (lease present) every id must also be leased or created this turn, else the whole call fails and the selection is unchanged.
* Effect: replaces `env.selection` (`tools.rs:921`). At the end of a turn that used `select`, `TurnResult.selection` is applied to the editor selection, filtered to existing visible ids (`assistant/turn.rs:316`, `assistant/mod.rs:528-533`). It overrides the automatic "select touched entities".
* Result: `{"ok":true,"selection":[ids]}`.
* Note: `selection_used` is only set when the call succeeded (`assistant/turn.rs:266`).

### enter_object

* Kind: non-mutating by classification, but it changes `model.edit_context`. Args: `id`.
* Core: `Model::enter_object` (`benchup-core/src/edit_context.rs:214`): the id must be a visible, unlocked group/component that is a direct child of the current scope. Result: `{"ok":true,"edit_path":[ids from root]}` (`edit_path`, `edit_context.rs:201`).
* Error: `cannot open N` ("It must be an unlocked, visible group or component in the current scope.").

### exit_to_root

* Args: none. Sets `model.edit_context = None` (`tools.rs:936`). Result: `{"ok":true,"edit_path":[]}`.

---

## Special topics

### Outward normals and push_pull

Face normals in the document follow drawing winding, so a closed part may store inward normals (comment `assistant/scene.rs:179-182`). `outward_sign` (`scene.rs:183`) returns `-1` when the vector from the owning object's center (`Model::selection_center`, `benchup-core/src/modeling.rs:20`) to the face center (`face_center`, `benchup-core/src/faces.rs:181`) points against the stored normal (`dot < -1e-9`), else `+1`; it defaults to `+1` when the plane, owner or center cannot be found. `get_object` reports `normal * sign` (`scene.rs:197`) and `push_pull` multiplies the distance by the same sign (`tools.rs:674`), so a positive `distance_mm` always moves the face away from the part. The heuristic uses the owning part's center, so it can be wrong for strongly non-convex parts (inference from the code; not tested for that case).

### Group id remapping

`make_object` removes the original entities and re-pastes them with new ids (`benchup-core/src/objects.rs:638-700`: `faces.retain`, `objects.retain`, `paste_mapped`), updating joint member ids via the remap. After `group`, input ids may therefore no longer exist (reported in `erased`) and the members have new ids (reported in `members`). The system prompt does not mention this; the tool result `note` does. Because the old ids vanish, `Turn::finish` filters `modified` ids by existence and drops ids created and later erased in the same turn (`assistant/turn.rs:291-295`). With a lease, the new group id is in `env.created` and therefore leased (`tools.rs:1041`).

### enter_object, exit_to_root and the edit context

* Mutating tools require their targets to be direct children of the current `edit_context` (`entity_in_context`, `benchup-core/src/edit_context.rs:189`). The scene summary includes `edit_path` so the model knows where it is (`scene.rs:140`), and in-scope objects are listed before out-of-scope ones (`scene.rs:151-155`).
* Core functions such as `draw_polyline`, `make_object`, `paste_entities`, `add_text` and `push_pull` redirect through `edit_in_context` when a scope is open, so created entities land inside the open group.
* `edit_context` is not part of `Model::same_scene` (`benchup-core/src/document.rs:111-125`), so entering or leaving a group never sets the undo snapshot. Instead the turn restores it: `Turn` remembers the scope at start and the app puts the user back in it when the turn ends, is stopped or is discarded (`restore_scope`), and the history snapshot carries that scope, so "Undo turn" leaves the user in the scope they were in before the turn. Tests: `a_turn_puts_the_designer_back_in_their_edit_scope_and_undo_keeps_it`.

---

## Gaps / unclear

Fixed in the commit "Fix issues found while documenting MIRA": edit scope leaking out of a turn (the scope is restored and undo keeps it), the tag-length mismatch (the tool and schema now use core's 128-byte limit), the `scale` schema (`minimum`/`maximum` replace `exclusiveMinimum`), truncated results (now valid JSON), and `select` outside the lease. Still open:

1. **`draw_polyline` lease exemption is an assumption.** It is treated as "only adds entities" (`tools.rs:961`), but I did not verify that `Model::draw_polyline` (`benchup-core/src/document.rs:209`) never splits or merges existing edges/faces owned by someone else's lease. The out-of-range check covers created/modified ids only (`tools.rs:989`), not erased or implicitly altered ones.
2. **`create_copy` copies a source the caller may not lease** (by design, `tools.rs:961` comment) and only checks visibility. The copy is new and therefore exempt.
3. **`enter_object` / `exit_to_root` are allowed in read-only mode**, since they only move the edit scope, which is restored at the end of the turn.
4. **`resize_board` operates on world-axis bounds** (`tools.rs:632`), not the part's local axes.
5. **`push_pull` outward sign** relies on a center-based heuristic (`scene.rs:183-193`); behaviour on concave parts is untested here.
6. **`group` accepts any entity** (E, not O) while the system prompt only mentions boards; faces/edges are allowed if `make_object` accepts them.
7. **Core messages are not translated into hints.** Many core `Err` strings reach the model with no hint; their exact wording lives in `benchup-core` and may change.
8. **Error/hint strings** shown to the model are plain English literals, not `tr!` keys.
9. Tests for the above are in `assistant/tools_tests.rs`; see [testing.md](testing.md).

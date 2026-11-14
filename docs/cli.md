# Command line and AI agents

[Documentation](README.md) · [Agent guide](agent-guide.md) · [Developer README](../README.md)

`benchup` reads and edits BenchUp `.bup` projects from a shell or an AI coding agent (Claude Code, Codex, Cursor…), without opening the app. It reads every project format the app saves, including files with a viewport preview. Lengths are millimetres and Z is up.

## Installation

The command is built from this repository with Rust 1.95 or newer:

```sh
cargo install --locked --path crates/benchup-cli   # installs `benchup` in ~/.cargo/bin
```

Make sure `~/.cargo/bin` is on your `PATH`. The installed command does not update itself: run the same command again after pulling changes. The desktop app is installed separately.

## Commands

```sh
benchup guide                               # conventions, workflow and woodworking recipes
benchup new shelf.bup --name "Shelf"        # create an empty project
benchup tools                               # list the tools
benchup tools create_board                  # one tool's JSON schema
benchup scene shelf.bup                     # scene summary: objects, sizes, grain, stock, joints
benchup scene shelf.bup '{"depth":0,"fields":["name","size_mm"]}'   # a filtered summary (get_scene's arguments)
benchup get shelf.bup 79                    # one entity in detail, with its faces, profiles and finish
benchup render shelf.bup --view front       # a PNG of the model: shelf.front.png (-o OUT.png, --size 1200x900)

benchup call shelf.bup create_board '{"name":"Side","origin_mm":[0,0,0],"size_mm":[18,300,900],"grain_axis":"z"}'
echo '{"ids":[79],"delta_mm":[0,0,10]}' | benchup call shelf.bup move -

benchup call shelf.bup --batch ops.jsonl    # several calls as ONE undo step, all-or-nothing
benchup call shelf.bup --batch ops.jsonl --dry-run --brief   # try it without saving, compact output
benchup call shelf.bup check_model          # overlaps without joints, joints that cannot be cut, ...
benchup call shelf.bup get_cut_list         # boards by material and size
benchup call shelf.bup get_estimate         # finishing, joinery and router time; wood by material
benchup undo shelf.bup
benchup redo shelf.bup

benchup mcp shelf.bup                       # the same tools over MCP (stdio), plus run_batch, undo and render_view

benchup open shelf.bup                      # open it in the BenchUp desktop app
benchup open shelf.bup --app target/debug/benchup-app   # or in a specific executable
```

A batch file has one call per line. Blank lines and lines starting with `#` are skipped:

```jsonl
{"tool": "create_board", "arguments": {"name": "Left",  "origin_mm": [0, 0, 0],    "size_mm": [18, 300, 900]}}
{"tool": "create_board", "arguments": {"name": "Right", "origin_mm": [782, 0, 0],  "size_mm": [18, 300, 900]}}
{"tool": "create_board", "arguments": {"name": "Shelf", "origin_mm": [18, 0, 400], "size_mm": [764, 300, 18]}}
```

In a batch, `"$N.field"` is a value from the result of call N (counting from 1): `"$1.created[0]"`, `"$3.joint_id"`, `"$2.face_id"`. A reference to a list of ids inside a list is spliced in, so `"ids": ["$1.created", "$2.created"]` joins both; other lists, such as a point, stay one item. Text that is not exactly a reference, such as `"$12.50 hardware"`, is kept as written, and a single call never resolves references:

```jsonl
{"tool": "create_board", "arguments": {"name": "Side",  "origin_mm": [0, 0, 0],   "size_mm": [18, 300, 900]}}
{"tool": "create_board", "arguments": {"name": "Shelf", "origin_mm": [9, 0, 400], "size_mm": [500, 300, 18]}}
{"tool": "add_joint", "arguments": {"a": "$2.created[0]", "b": "$1.created[0]", "kind": "tongue_groove"}}
```

## Behaviour

- **Output:** JSON on stdout. Exit status is `0` on success, `1` when a tool reported an error (`{"ok": false, "error", "hint"}`, file unchanged), and `2` for a usage or I/O error (message on stderr).
- **MCP:** each tool call is applied and saved on its own. `run_batch {calls, dry_run}` runs several calls as one undo step with `"$N.field"` references, like `--batch` (with `dry_run: true`, like `--dry-run`); `undo` (`redo: true`) steps through the project history; `render_view` returns an image.
- **Previews:** `benchup render` draws the model from a standard view (`iso`, `front`, `back`, `left`, `right`, `top`, `bottom`) with joints and router profiles cut, edges, and dimensions with their values, without a display or GPU. Its JSON output lists dimensions whose labels overlap and dimensions that point along the view. Over MCP, the `render_view` tool returns the same image.
- **History:** each `call` is one entry in the project history, labelled “Agent edit” (in the app's language); a batch is one entry. You can undo it in BenchUp or with `benchup undo`.
- **Atomic saves:** the file is written through a temporary file and a rename. If the file changed on disk since it was read (for example, BenchUp saved it), the command refuses to overwrite it. `benchup mcp` reads the file again before each call, so it carries on from what BenchUp saved.
- **Views and cameras:** the CLI keeps the camera of each view saved in the file, so editing a project does not move the user's views. It cannot list or change saved views yet, and `benchup render` draws the standard views (iso, front, back, left, right, top, bottom), not saved cameras; use BenchUp's [views](controls.md#views-and-cameras) and Tools → Screenshot for those.
- **Shared projects:** local copies of shared projects are refused unless you pass `--allow-shared`, because edits would diverge from the server.
- **Opening in the app:** `benchup open` hands the file to the installed desktop app (by bundle id on macOS, through the `.bup` file association on Linux and Windows). On macOS, when BenchUp is not running yet, it starts the app with the file as a launch argument. `--app <executable>`, or the `BENCHUP_APP` environment variable, starts that executable instead, which is handy for a development build. The app must be installed separately: [download it](https://github.com/BenchUp3D/editor/releases/latest).
- **Not live:** the CLI edits the file, not a running BenchUp window. Run `benchup open` again after a change to reload it; if the project has unsaved changes in the app, BenchUp asks first. Saving in BenchUp overwrites the file without checking for outside changes, so save or discard your app edits before editing with the CLI. Live editing through a local bridge is planned ([#13](https://github.com/BenchUp3D/editor/issues/13)).

## Using it from an agent

Agents should read `benchup guide` first (over MCP, the `get_guide` tool); it is the [agent guide](agent-guide.md). It explains the conventions (the front faces -Y), which board is `a` in a joint, the batch workflow, woodworking recipes and current limitations.

To make Claude Code reach for the CLI on its own, install the skill:

```sh
cp -r crates/benchup-cli/skill/benchup ~/.claude/skills/
```

With Claude Code, either let it call the CLI directly ("use `benchup` to add a shelf to shelf.bup"), or register the MCP server:

```sh
claude mcp add benchup -- benchup mcp /path/to/shelf.bup
```

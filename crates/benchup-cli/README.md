# benchup (CLI)

Read and edit BenchUp `.bup` projects from a shell or an AI coding agent (Claude Code, Codex, Cursor…), without opening the app.

```sh
cargo install --path crates/benchup-cli   # installs the `benchup` binary
```

The tools come from [`benchup-tools`](../benchup-tools), the same operation layer an MCP server or in-app integration can use. Lengths are millimetres and Z is up.

## Commands

```sh
benchup new shelf.bup --name "Shelf"        # create an empty project
benchup tools                               # list the tools
benchup tools create_board                  # one tool's JSON schema
benchup scene shelf.bup                     # scene summary: objects, sizes, grain, stock, joints
benchup get shelf.bup 79                    # one entity in detail, with its faces

benchup call shelf.bup create_board '{"name":"Side","origin_mm":[0,0,0],"size_mm":[18,300,900],"grain_axis":"z"}'
echo '{"ids":[79],"delta_mm":[0,0,10]}' | benchup call shelf.bup move -

benchup call shelf.bup --batch ops.jsonl    # several calls as ONE undo step, all-or-nothing
benchup undo shelf.bup
benchup redo shelf.bup

benchup mcp shelf.bup                       # the same tools over MCP (stdio)
```

A batch file has one call per line. Blank lines and lines starting with `#` are skipped:

```jsonl
{"tool": "create_board", "arguments": {"name": "Left",  "origin_mm": [0, 0, 0],    "size_mm": [18, 300, 900]}}
{"tool": "create_board", "arguments": {"name": "Right", "origin_mm": [782, 0, 0],  "size_mm": [18, 300, 900]}}
{"tool": "create_board", "arguments": {"name": "Shelf", "origin_mm": [18, 0, 400], "size_mm": [764, 300, 18]}}
```

## Behaviour

- **Output:** JSON on stdout. Exit status is `0` on success, `1` when a tool reported an error (`{"ok": false, "error", "hint"}`, file unchanged), and `2` for a usage or I/O error (message on stderr).
- **History:** each `call` is one entry in the project history, labelled `Agent: <tool>`, or `Agent: N changes` for a batch. You can undo it in BenchUp or with `benchup undo`.
- **Atomic saves:** the file is written through a temporary file and a rename. If the file changed on disk since it was read (for example, BenchUp saved it), the command refuses to overwrite it.
- **Shared projects:** local copies of shared projects are refused unless you pass `--allow-shared`, because edits would diverge from the server.
- **Not live:** the CLI edits the file, not a running BenchUp window. Reopen the project to see the changes. Live editing through a local bridge is planned (see #13).

## Using it from an agent

With Claude Code, either let it call the CLI directly ("use `benchup` to add a shelf to shelf.bup"), or register the MCP server:

```sh
claude mcp add benchup -- benchup mcp /path/to/shelf.bup
```

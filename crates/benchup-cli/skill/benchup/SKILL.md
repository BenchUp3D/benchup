---
name: benchup
description: Design or edit furniture and woodworking projects in BenchUp (.bup files) with the `benchup` command-line tool. Use when the user mentions BenchUp, a .bup file, or asks to model, change, dimension or cost a piece of furniture (shelves, cabinets, tables, drawers, joints, edge profiles, finishes, cut lists).
---

# BenchUp projects

Use the `benchup` CLI to read and edit `.bup` projects. It edits the file, not an open BenchUp window.

1. Run `benchup guide` once and follow it. It explains the axes (the front faces -Y), joint roles, the batch workflow and recipes.
2. Read the project with `benchup scene file.bup` (on big projects, filter it: `benchup scene file.bup '{"depth":0}'`). Create a new one with `benchup new file.bup --name "..."`.
3. Make each change as one `benchup call file.bup --batch -` (one undo step), generating the JSON lines with a script for repeated parts. Use `--dry-run` to try first, ending the batch with `check_model {"only_new": true}`.
4. Verify with `benchup call file.bup check_model`, look at it with `benchup render file.bup --view front`, and for materials and time use `get_cut_list` and `get_estimate`.
5. Show the result with `benchup open file.bup`.

If a size does not fit or the request is ambiguous, say so and propose an interpretation instead of forcing it. `benchup tools <name>` shows any tool's exact arguments.

# Round 5 (14 November 2026)

First round with the tools of [#42](https://github.com/BenchUp3D/benchup/issues/42) (router profiles, joint trimming and `set_joint`, finishes and `get_estimate`, `create_profile_board`, `split_face`/`merge_faces`, `distribute`, `change_thickness`, scene filters, `measure_space`/`find_surfaces`, joint geometry, `check_model only_new`, `render`/`render_view`) and with five new cases (13–17). Each cell is the number of commands the agent ran (MCP calls for case 17), with those that returned an error in brackets. Results were checked independently with `check_model` and an iso render of each final file.

| # | Case | R4 | R5 | Result in R5 | `focus` tools used |
|---|---|---|---|---|---|
| 01 | Lengthen the desk | ~61 (0) | **23 (1)** | ✅ lengthened at the front, notches kept | `split_face`, `push_pull`, `merge_faces`, `measure_space` |
| 02 | Five shelves to four | 14 (0) | 17 (0) | ✅ equal gaps of 345.6 mm, joints flipped in a separate batch | `distribute`, `flip_joint` |
| 03 | Dining table | 15 (0) | 18 (0) | ✅ 8/8 joints | `create_board`, `add_joint`, `render` |
| 04 | Chest of drawers | 14 (0) | 20 (0) | ✅ 24/24 joints, one component | `group`, `create_copy`, `render` |
| 05 | Back in a rabbet and plinth | 18 (0) | 43 (2) | ⚠️ builds and checks, but the `rabbet` joint is not the rabbet the user means (see findings) | `add_joint`, `set_joint`, `render` |
| 06 | Solid oak 19 mm | 17 (1) | 25 (3) | ✅ after working around two `change_thickness` bugs | `change_thickness` |
| 07 | Bedside table with curved sides | 36 (4) | **16 (0)** | ✅ first try, 6/6 joints, 2 profiles | `create_profile_board`, `add_profile` |
| 08 | Leaning ladder shelf | 22 (0) | 27 (1) | ✅ | `create_profile_board`, `rotate`, `render` |
| 09 | Workshop dimensions | 11 (0) | 18 (0) | ✅ no overlapping labels in the front view | `add_dimension`, `render` |
| 10 | Modules, components and tags | 19 (~2) | 25 (2) | ⚠️ partial on purpose, as the guide says (housed shelves cannot be one component); alternative saved | `group`, `tag` |
| 11 | Cut list and sheets | 8 (0) | 12 (0) | ✅ 2 sheets, checked by hand layout | `get_cut_list` |
| 12 | A shelf that does not fit | 10 (0) | 14 (0) | ✅ refused, 491 mm alternative saved | `measure_space`, `check_model only_new` |
| | **Total 01–12** | **~245 (~7)** | **258 (9)** | 10 ✅, 2 ⚠️ | |
| 13 | Edge profiles | — | 13 (0) | ✅ 12 profiles | `add_profile` |
| 14 | Bench with through tenons | — | 15 (0) | ✅ 8/8 joints; sized the rails flush instead of trimming, so `trim` was not exercised | `add_joint` |
| 15 | Finish and time | — | 13 (0) | ✅ ~4 h 50 min work, 30 h drying | `set_finish`, `get_estimate` |
| 16 | Shelf above the desk, checked visually | — | 20 (0) | ✅ notched around the posts, no clash, 4 renders | `get_scene` filters, `measure_space`, `render` |
| 17 | Five shelves to four, over MCP | — | 16 (0) | ✅ same result as 02 | `distribute`, `render_view` |

The total for 01–12 is flat, but it hides two opposite moves. The cases the new tools were made for dropped sharply: 01 (61 → 23, no more probing walls one by one) and 07 (36 → 16, first try). Elsewhere agents now spend 2–5 commands on `render` to look at their result, which they could not do before, and two cases grew because of real bugs or a misleading guide row: 05 (rabbet geometry, 43) and 06 (`change_thickness`, 25). 13–17 all succeeded with no tool errors.

## Findings

### Tool bugs (benchup-tools)
1. **`merge_faces` does not open the board's group** like the other editing tools; it fails with "not editable in the current scope" unless wrapped in `enter_object` (01).
2. **`change_thickness`** decides which board goes into which by the joint's `a`/`b` roles: with reversed joints it silently adjusts nothing. It also ignores boards that only butt against a thickened board, which then overlap it (06).
3. **`check_model only_new`** compares issues by ids; `group` changes member ids, so existing issues are reported as new (10).
4. **`cut_in_b`** in joint geometry describes the overlap region, not the mortise or groove actually cut, so a closed mortise reads as `stopped` (14).
5. **`render`**: dimensions with `axis` are drawn diagonally between their points instead of along the axis (08); text notes are drawn as a marker without their content, and the output does not say so (03, 08, 09); arc facets are drawn as edges and look toothed (07).
6. **Failed batch output** buries the error after a long `earlier_results`, and `--brief` does not trim `earlier_results` (01, 06).
7. **`set_grain`** takes only `id`, unlike `set_material` and `tag` (08).
8. **`split_face`** returns its pieces in an undocumented order (01).
9. **`check_model`** ignores router profiles when looking for overlaps (05).

### Tool improvements asked for
- `get_estimate`: totals per category (finishing, joinery, router) and elapsed time (work + wait); the step defaults in the `set_finish` schema (15).
- `measure_space`: say what `null` means (nothing in that direction) (02, 16).
- `render`: zoom/region, hide or isolate objects, several views per call (04, 05, 13); document the 2400 px limit (05).
- `distribute`: a `round_mm` option; changing the number of shelves in one call (02, 17).
- `get_object` on a board: whether it is shaped or notched (16).
- `check_model only_new`: summarise existing issues by kind (09).

### MCP
- **No batch, dry run or undo over MCP** (17). Each call writes the file; the agent made backup copies by hand. The guide is written for the CLI (`--batch`, `--dry-run`, `$N.field`, `benchup tools`) and is misleading over MCP. Proposal: MCP tools `run_batch {calls, dry_run}` and `undo`, and a short MCP section in the guide.

### Guide
- **"Back panel in a rabbet" is misleading** (05): the `rabbet` joint cuts both parts and leaves a 3 mm lip behind the back, not an open rabbet in the side. Explain it, or recommend a router `rabbet` profile on the sides' back edges with the back left whole.
- A table per plane of `create_profile_board`: which side it is seen from and where u and v point (07).
- `distribute` limits when there is no top or bottom; distribute each column separately (02, 17).
- The tension between "fixing pre-existing problems is reasonable" and "do not depart from the request" when no one can be asked (02, 05, 13).
- Drawer back joint, and whether it may sit on the bottom when both joints meet in one plane (04).
- `change_thickness` needs correct joint roles; butting parts need a check (06, until fixed).
- `render` does not draw text content (until fixed).

### Core limits
- **A router profile on a side with five through grooves fails** ("Each body must be a closed solid"), while one groove works. Found while writing the reference for case 13; needs a minimal reproduction before an issue.
- A classic rabbet joint (only `b` cut, `a` whole) does not exist; the `rabbet` kind cuts both parts (05).

### Missing features
- Cutting plan from the CLI (11; still out of scope, see #42).
- A dry-run render (render the result of a batch without applying it) (12).
- Moving an object between groups (`reparent`) (10).
- Reading the guide by section; it is about 38 KB with the tool list (11).

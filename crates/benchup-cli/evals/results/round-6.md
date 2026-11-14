# Round 6 (14 November 2026)

A short round to confirm the fixes made after [round 5](round-5.md), on the six cases where those findings came from. Each cell is the number of commands (MCP calls for 17), with errors in brackets; results checked independently with `check_model` and an iso render.

| # | Case | R5 | R6 | Result in R6 |
|---|---|---|---|---|
| 01 | Lengthen the desk | 23 (1) | 32 (1) | ✅ `merge_faces` worked inside the group without `enter_object`; the agent also filled the front corner |
| 05 | Back in a rabbet and plinth | 43 (2) ⚠️ | **23 (2)** | ✅ routed rabbet with the back whole, as the guide now says; the errors were a deliberate `reverse` test |
| 06 | Solid oak 19 mm | 25 (3) | **20 (1)** | ✅ `change_thickness` adjusted the shelves and the butting divider without workarounds |
| 08 | Leaning ladder shelf | 27 (1) | 24 (0) | ✅ dimensions along their axis and text readable in the render |
| 10 | Modules, components and tags | 25 (2) ⚠️ | 25 (1) ⚠️ | partial on purpose, as in every round; `only_new` no longer reports existing issues after grouping |
| 17 | Five shelves to four, over MCP | 16 (0) | **11 (0)** | ✅ used `run_batch` with a dry run and a separate batch for the joint fix, no manual backups |
| | **Total** | **159 (9)** | **135 (5)** | 5 ✅, 1 ⚠️ |

## Fixed after this round

- A one-call `--batch` answered with the bare result instead of `results`; `--batch` and `run_batch` now always answer with `results` (05).
- `get_scene{sections}` accepts `stock_catalog`, the name the output uses, as well as `stock` (06).
- `merge_faces` accepts a board's id inside `ids` (01).
- `add_joint` on a component instance explains that the board inside can only be joined within the instance (10).
- `ungroup` lists the released objects in `members` (10).
- `benchup tools --names` lists the tool names only; the full list is cut off in agent consoles (08).

## Still open

- `add_profile` does not say which face got `width_mm` and which `depth_mm` (05).
- A router profile that cannot be cut does not name the joint or profile it collides with (05).
- Blind (stopped) tongue and groove joints are not available in the core (17).
- `distribute` with a `round_mm` option; changing the number of shelves in one call (17).
- `render`: sections or hidden edges to see joints inside parts (08).

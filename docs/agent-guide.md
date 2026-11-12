# Building furniture with the BenchUp tools

[Documentation](README.md) · [Command line](cli.md)

This guide is written for AI agents that edit BenchUp projects through the [`benchup` command line](cli.md) or its MCP server. Agents read it with `benchup guide` (over MCP, the `get_guide` tool), and it always matches the installed version.

## Conventions

- Lengths are millimetres. Z is up. X runs left to right (width), Y is depth, and **the front of a piece faces -Y**: the BenchUp front view looks from -Y, so a wall behind a piece is on its +Y side.
- `create_board` takes the minimum corner (`origin_mm`) and the size along X, Y and Z (`size_mm`). Prefer it to drawing.
- Set the grain along a board's longest side: Z for sides and legs, X for shelves, tops and rails.
- Common thicknesses: 16, 18 or 19 mm for panels, 12 to 15 mm for drawer sides, 6 mm (3 to 9 mm) for backs and drawer bottoms. Legs are 40 to 80 mm square.

## Workflow

1. Read the model first with `get_scene` (`benchup scene`). It lists the project name, objects with their sizes, joints, dimensions, texts, loose geometry and stock materials.
2. Plan the whole change. For anything with repeated parts, write a small script that computes the positions and prints a batch file.
3. Apply it as **one batch**: one undo step, and all-or-nothing.
4. Verify:
   - `check_model` finds overlaps without a joint, joints that cannot be cut, joints whose roles look reversed and loose geometry.
   - `get_object` on a joint shows its settings and status.
   - `get_cut_list` lists the boards to cut.
5. With the CLI, `benchup open file.bup` shows the result in the desktop app. The CLI edits the file, not an open window: run `open` again to reload, and save or discard changes in the app before editing the file.

### Batches, results and dry runs

- Every call that changes the model returns `created`, `modified` and `erased` ids. Some return more: `joint_id` (`add_joint`), `material_id` (`create_stock_material`), `face_id` (`draw_polyline` and `push_pull`), and `members` (`group`). `benchup tools <name>` describes each tool.
- Ids are not consecutive, so always take them from results. In a batch, `"$N.field"` is a value from call N's result, counting from 1. Any field works, including nested ones: `"$1.created[0]"`, `"$3.joint_id"`, `"$2.material_id"`, `"$4.members[0].id"`, `"$5.created[-1]"`. A reference to a list inside a list is spliced in, so `"ids": ["$1.created", "$2.created"]` joins both.
- With the CLI, `--dry-run` reports every result without changing the file; ids made in a dry run differ from the real run. **End a dry-run batch with `check_model` (and `get_object` on new joints) to validate the change before applying it.** Read tools inside a batch see the batch's earlier changes. `--brief` leaves out object summaries and bounds.

## Groups, components and scope

- Every board is a group. To change a part inside another group (a module, a component), open each level with `enter_object`, from the outside in. With the CLI, the opened group lasts only for that command, so put `enter_object` and the edits in one `--batch` and finish with `exit_to_root`. `push_pull` works on faces inside groups without opening them.
- `group` with several ids makes ONE group, or one component with `component: true`. Grouping keeps existing joints.
- Components suit identical parts whose joints stay inside them, such as a drawer box. Build one completely, including its joints, make it a component, then `create_copy` it to each position. Instances of one component share a `definition` id in the scene.
- A joint can only be added between boards at the same level. A board inside a component instance cannot be joined to a board outside it, and `create_copy` does not copy joints with boards outside the component. Parts joined to outside boards (shelves housed in sides) should stay separate boards; name and tag them alike instead.
- Erasing a board also removes its joints. `remove_joint` deletes a joint alone. Moving, resizing or rotating a board keeps its joints.

## Joints

A joint joins two boards that **overlap** where they meet; the overlap is the joint's depth. Make the overlap first by placing one board partly inside the other, then call `add_joint`. Joints are cut when BenchUp evaluates the model, so the scene still shows plain boxes. `get_object` on the joint reports `"status": "ok"` or the reason it cannot be cut, and `add_joint` refuses joints that cannot be cut.

- **`a` carries the tenon, tongue, tails, fingers or lap**: the board that goes into the other (shelf, apron, drawer side, back panel).
- **`b` receives** the mortise, groove or sockets (side, leg, drawer front).
- Leave `depth_axis`/`width_axis` out. BenchUp picks the woodworking orientation: depth is the direction `a` goes into `b`. Tongues, tenons and rabbets are spread across `a`'s thickness; dovetails, fingers and laps across its broad side. The result reports the chosen orientation.

| Situation | Overlap | Joint |
|---|---|---|
| Shelf housed in a side | Shelf enters the side by about half the side's thickness (9 mm into 18) | `tongue_groove`, a = shelf, b = side |
| Apron into a leg | Apron enters the leg 25 to 35 mm, set back 5 to 10 mm from the leg's outer face | `mortise_tenon`, a = apron, b = leg, `width_ratio` about 0.33 (tenon one third of the apron's thickness), `height_ratio` about 0.75 |
| Back panel in a groove | Back (6 mm) enters each side by about half the side's thickness, 6 to 10 mm in from the back edge | `tongue_groove`, a = back, b = side |
| Back panel in a rabbet | Back enters each side by about half the side's thickness, flush with the back edge. Only when nothing else is housed in the sides near the back (see below) | `rabbet`, a = back, b = side |
| Drawer side into the front (half-blind dovetail) | Side enters the front from behind, leaving 4 to 6 mm of the front | `dovetail`, a = drawer side, b = front, `dovetails` 3 to 5 |
| Drawer bottom | Bottom (6 mm) enters the drawer sides by 6 mm and stops at the back face of the front | `tongue_groove`, a = bottom, b = side |
| Frame corners | Rails and stiles cross by their full width | `half_lap`, a = rail, b = stile |
| Box corners | Two panels overlap by the other's thickness | `finger`, a and b either way |

### Several joints in one board

Joints on the same board must not cross, touch, or end in the same plane; the joint is refused with a "closed solid" or "valid solid geometry" error. Keep at least 1 to 2 mm of wood between their regions:

- **Shelves and a back:** stop shelves, top and bottom 1 to 2 mm short of the back, not flush with it. With shelves, top or bottom housed in the sides, put the back in a groove set in from the back edge rather than a rabbet.
- **Drawer bottom:** groove it into the drawer sides only. Where the sides already enter the front with dovetails, a groove in the front would meet them.
- **Two aprons meeting in one leg:** their tenons must not meet inside the leg. With leg width P, apron setback s from the leg's outer face and apron thickness t, keep the tenon depth below P − s − t (70 − 10 − 22 = 38 mm, so use 30).
- **A central divider:** shelves on both sides at the same height would cut it through with two 9 mm grooves. Let them enter by a third of its thickness each, stagger the heights, or butt them against it without a joint.

## Recipes

- **Carcass (cabinet, bookcase):** two sides, a top and a bottom (between the sides, or the top over them), shelves, and a back in a groove. Stop everything inside 1 to 2 mm short of the back.
- **Even shelves:** with an inner height H (between the bottom and the top, or the full side height if there are none), n shelves of thickness t and equal gaps above, below and between, the gap is (H - n·t)/(n + 1). Shelf k (counting from 1) has its bottom at k·gap + (k - 1)·t above the inner bottom. To change the number of shelves, erase or copy shelves, then `move` them to the new heights; their joints follow.
- **Plinth:** a board 60 to 100 mm high between the sides, set back 20 to 50 mm from the front. A divider that reaches the floor splits it in two.
- **Table:** legs 60 to 80 mm square; aprons 80 to 120 mm high and 20 to 25 mm thick, their tops flush with the legs, tenons as above; a top 25 to 40 mm thick resting on the frame with a 20 to 50 mm overhang. Ask whether the size given is the top or the frame, and whether the height includes the top.
- **Drawer:** inset fronts leave about 2 mm at each side and 3 to 4 mm between drawers. With metal runners, make the box about 12.5 mm narrower on each side, or as the runner specifies.
- **Lengthening a piece:** `resize_board` the boards that run in that direction, choosing the end with `anchor`, and `move` the parts at that end. `resize_board` stretches the whole board, notches included, so rebuild a shaped board from its new outline instead.
- **Changing the stock thickness while keeping the outside size:** resize each board's thickness with `resize_board`, using `anchor` to keep the outer faces in place (`["max", "min", "min"]` for a right side, `"center"` for dividers), and keep each joint's overlap at about half the new thickness. Create the new material and assign it.
- **A leaning piece (ladder shelf):** build each upright standing vertically, from its outline with the ends already cut at θ (see shaped boards), and set its grain to z. Then `rotate` it by −θ about x, pivoting on the back edge of its foot, so its top leans towards the wall at +Y. For height H, board width w and angle θ, the length is (H - w·sin θ)/cos θ and the foot sits L·sin θ from the wall. Keep shelf edges a few millimetres away from the uprights' sloping faces; a near-coincident edge leaves slivers smaller than the model precision.
- **Materials and cut list:** `create_stock_material` (name, thickness, width, length; the length along the grain), then `set_material` on the boards. The material's thickness should match the boards'. `get_cut_list` groups boards by material and size; `minimum_stock_pieces`, an area lower bound, appears once boards have a material with sizes. To answer "how many sheets do I need" without changing the project, create and assign the material and call `get_cut_list` in one `--dry-run` batch.
- **Dimensions for the workshop:** put overall height, width and depth, and a chain of shelf gaps, in the front plane (y = 0) so they read in the front view. Dimension points snap to nearby vertices, so measure between real corners. `offset_mm` pushes the line away from the piece: chain dimensions about 100 mm out, overall ones about 250 mm out, so they do not overlap. `get_scene` lists each dimension with its `length_mm`. Ask whether shelf spacing means clear space or centre to centre.

## Shaped boards (curves, chamfers, angled ends)

There are no arc, fillet or chamfer tools. Draw the board's outline as one closed `draw_polyline`, using many short segments for a curve, on the plane where the board starts. Then `push_pull` its face to the thickness and `group` everything the two calls created. Finish with `set_grain` and a name. In one batch:

```jsonl
{"tool": "draw_polyline", "arguments": {"points_mm": [[0,0,0],[440,0,0],[450,10,0],[450,350,0],[0,350,0]], "close": true}}
{"tool": "push_pull", "arguments": {"face_id": "$1.face_id", "distance_mm": 18}}
{"tool": "group", "arguments": {"ids": ["$1.created", "$2.created"], "name": "Top", "component": false}}
{"tool": "set_grain", "arguments": {"id": "$3.created[0]", "axis": "x"}}
```

A loose face's normal follows the order of its points: counter-clockwise seen from the side it faces, so the outline above extrudes towards +z. Check the direction in the result's `bounds_mm` with `--dry-run`. The cut list measures a board along its grain, so draw a part in its natural orientation, set the grain, then rotate it into place.

Avoid cutting through a finished board with a negative `push_pull` of its full thickness. It currently leaves stray faces and edges, and `check_model` reports an open solid. Rebuild the part from its outline instead.

## When to stop and ask

- A requested size does not fit, such as a shelf longer than its opening. Measure the free space from `bounds_mm`: from one board's inner face to the next. A housed shelf is that gap plus both overlaps. Try the change in a `--dry-run` batch ending with `check_model`, report what you measured, and ask instead of forcing it.
- Ambiguous terms. Ask whether "two columns" means two bays, and whether shelf spacing is clear space or centre to centre. Ask whether "1000 mm from the floor" is the top or the bottom face. Ask whether a front is inset or overlaid, which end of a piece to lengthen, and whether a size is the top or the frame.
- In each case, propose an interpretation and say which one you used.

## Limitations

- Joints are cut from the overlap when BenchUp evaluates the model. The tools show the boards before cutting; use the joint's `status` to confirm.
- There is no fixing hardware (screws, dowels, metal runners). Leave a top resting on its frame, and note the fixing with `add_text` if it matters.

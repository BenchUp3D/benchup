# Building furniture with the BenchUp tools

## Conventions

- Lengths are millimetres. Z is up. X runs left to right (width), Y is depth, and **the front of a piece faces -Y**: the BenchUp front view looks from -Y.
- `create_board` takes the minimum corner (`origin_mm`) and the size along X, Y and Z (`size_mm`). Prefer it to drawing.
- Set the grain along a board's longest side: Z for sides and legs, X for shelves, tops and rails.
- Common thicknesses: 16, 18 or 19 mm for panels, 12 to 15 mm for drawer sides, 6 mm (3 to 9 mm) for backs and drawer bottoms. Legs are 40 to 80 mm square.

## Workflow

1. Read the model first (`get_scene`, or `benchup scene`). It lists objects with their sizes, joints, dimensions, texts and stock materials.
2. Plan the whole change. For anything with repeated parts, write a small script that computes the positions and prints a batch file.
3. Apply it as **one batch**: one undo step, and all-or-nothing. In a batch, refer to an earlier call's result as `"$N.field"`, counting calls from 1. For example, `"$1.created[0]"` is the board created by call 1, and `"$5.joint_id"` is the joint created by call 5. Ids are not consecutive, so always take them from results. With the CLI, `--dry-run` shows the results without changing the file.
4. Verify:
   - `check_model` finds overlaps without a joint, joints that cannot be cut and loose geometry.
   - `get_object` on a joint shows its settings and status.
   - `get_cut_list` lists the boards to cut.
5. With the CLI, `benchup open file.bup` shows the result in the desktop app. The CLI edits the file, not an open window: run `open` again to reload, and save or discard changes in the app before editing the file.

## Groups, components and scope

- Every board is a group. To change a part inside another group (a module, a component), open each level with `enter_object`, from the outside in. With the CLI, the opened group lasts only for that command, so put `enter_object` and the edits in one `--batch` and finish with `exit_to_root`. `push_pull` works on faces inside groups without opening them.
- `group` with several ids makes ONE group, or one component with `component: true`. For identical parts, build one completely, including its joints, make it a component, then `create_copy` it to each position. Instances of one component share a `definition` id in the scene. Avoid editing a board inside a component after it has copies.
- Erasing a board also removes its joints. `remove_joint` deletes a joint alone.

## Joints

A joint joins two boards that **overlap** where they meet; the overlap is the joint's depth. Make the overlap first by placing one board partly inside the other, then call `add_joint`. Joints are cut when BenchUp evaluates the model, so the scene still shows plain boxes. `get_object` on the joint reports `"status": "ok"` or the reason it cannot be cut, and `add_joint` refuses joints that cannot be cut.

- **`a` carries the tenon, tongue, tails, fingers or lap**: the board that goes into the other (shelf, apron, drawer side, back panel).
- **`b` receives** the mortise, groove or sockets (side, leg, drawer front).
- Leave `depth_axis`/`width_axis` out. BenchUp picks the woodworking orientation: depth is the direction `a` goes into `b`. Tongues, tenons and rabbets are spread across `a`'s thickness; dovetails, fingers and laps across its broad side. The result reports the chosen orientation.

| Situation | Overlap | Joint |
|---|---|---|
| Shelf housed in a side | Shelf enters the side by about half the side's thickness (9 mm into 18) | `tongue_groove`, a = shelf, b = side |
| Apron into a leg | Apron enters the leg 25 to 35 mm, set back 5 to 10 mm from the leg's outer face | `mortise_tenon`, a = apron, b = leg, `width_ratio` about 0.33 (tenon one third of the apron's thickness), `height_ratio` about 0.75 |
| Back panel in the sides | Back (6 mm) enters each side by about half its thickness, flush with the back edge | `rabbet`, a = back, b = side |
| Drawer side into the front (half-blind dovetail) | Side enters the front from behind, leaving 4 to 6 mm of the front | `dovetail`, a = drawer side, b = front, `dovetails` 3 to 5 |
| Drawer bottom | Bottom (6 mm) enters the front and sides by 6 mm | `tongue_groove`, a = bottom, b = front or side |
| Frame corners | Rails and stiles cross by their full width | `half_lap`, a = rail, b = stile |
| Box corners | Two panels overlap by the other's thickness | `finger`, a and b either way |

Several joints in one board must not cross. Two aprons meeting in one leg need shorter tenons, so their overlaps do not touch: `check_model` reports them as an overlap without a joint. A central divider with shelves on both sides at the same height would be cut through by two 9 mm grooves. Let the shelves enter it by a third of its thickness each, stagger the heights, or butt them against it without a joint.

## Recipes

- **Carcass (cabinet, bookcase):** two sides, a top and a bottom (between the sides, or the top over them), shelves, and a back. Shorten everything inside by the back's thickness so it does not hit the back; `check_model` reports it if it does.
- **Even shelves:** with an inner height H, n shelves of thickness t and equal gaps above, below and between, the gap is (H - n·t)/(n + 1). Shelf k (counting from 1) has its bottom at k·gap + (k - 1)·t above the inner bottom.
- **Plinth:** a board 60 to 100 mm high between the sides, set back 20 to 50 mm from the front.
- **Table:** legs 60 to 80 mm square; aprons 80 to 120 mm high and 20 to 25 mm thick, their tops flush with the legs; a top 25 to 40 mm thick resting on the frame with a 20 to 50 mm overhang.
- **Drawer:** inset fronts leave about 2 mm at each side and 3 to 4 mm between drawers. With metal runners, make the box about 12.5 mm narrower on each side, or as the runner specifies.
- **Changing the stock thickness while keeping the outside size:** resize each board's thickness with `resize_board`, using its `anchor` to keep the outer faces in place, and keep each joint's overlap at about half the new thickness.
- **A leaning piece (ladder shelf):** create the board upright, then `rotate` it about its foot. For height H, board width w and angle θ from vertical, the length is (H - w·sin θ)/cos θ and the foot sits L·sin θ from the wall.
- **Materials:** `create_stock_material` (name, thickness, width, length; the length along the grain), then `set_material` on the boards. The material's thickness should match the boards'.

## When to stop and ask

- A requested size does not fit, such as a shelf longer than its opening. Report the free space you measured and ask instead of forcing it.
- Ambiguous terms. Ask whether "two columns" means two bays, and whether shelf spacing is clear space or centre to centre. Ask whether "1000 mm from the floor" is the top or the bottom face. Ask whether a front is inset or overlaid, and which end of a piece to lengthen.
- In each case, propose an interpretation and say which one you used.

## Shaped boards (curves, chamfers, angled ends)

There are no arc, fillet or chamfer tools. Draw the board's outline as one closed `draw_polyline`, using many short segments for a curve, on the plane where the board starts. Then `push_pull` its face to the thickness and `group` everything the two calls created. Finish with `set_grain` and a name. In one batch:

```jsonl
{"tool": "draw_polyline", "arguments": {"points_mm": [[0,0,0],[440,0,0],[450,10,0],[450,350,0],[0,350,0]], "close": true}}
{"tool": "push_pull", "arguments": {"face_id": "$1.face_id", "distance_mm": 18}}
{"tool": "group", "arguments": {"ids": ["$1.created", "$2.created"], "name": "Top", "component": false}}
{"tool": "set_grain", "arguments": {"id": "$3.created[0]", "axis": "x"}}
```

Avoid cutting through a finished board with a negative `push_pull` of its full thickness. It currently leaves stray faces and edges, and `check_model` reports an open solid. Rebuild the part from its outline instead.

## Limitations

- A rotated board keeps square ends. For a level foot or top, build the part from its outline as shown above.
- Joints are cut from the overlap when BenchUp evaluates the model. The tools show the boards before cutting; use the joint's `status` to confirm.

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
   - `check_model` finds boards that pass through others, overlaps without a joint, joints that cannot be cut or look reversed, boards thicker or thinner than their material, and loose geometry. It also counts the joints that cut. Read `passed` (no errors or warnings); `ok` only says the check ran.
   - `get_object` on a joint shows its settings and status.
   - `get_cut_list` lists the boards to cut.
5. With the CLI, `benchup open file.bup` shows the result in the desktop app. The CLI edits the file, not an open window: run `open` again to reload, and save or discard changes in the app before editing the file.

### Batches, results and dry runs

- Every call that changes the model returns `created`, `modified` and `erased` ids. Some return more: `joint_id` (`add_joint`), `material_id` (`create_stock_material`), `face_id` (`draw_polyline` and `push_pull`), and `members` (`group`). `benchup tools <name>` describes each tool.
- Ids are not consecutive, so always take them from results. In a batch, `"$N.field"` is a value from call N's result, counting from 1. Any field works, including nested ones: `"$1.created[0]"`, `"$3.joint_id"`, `"$2.material_id"`, `"$4.members[0].id"`, `"$5.created[-1]"`. A reference may stand in for any argument, even where the schema says integer or uuid. A reference to a list of ids inside a list is spliced in, so `"ids": ["$1.created", "$2.created"]` joins both; other lists, such as a point in `points_mm`, stay one item. Text that is not exactly a reference, such as `"$12.50 hardware"`, is kept as written.
- With the CLI, `--dry-run` reports every result without changing the file; ids made in a dry run differ from the real run. **End a dry-run batch with `check_model` to validate the change before applying it.** Read tools inside a batch see the batch's earlier changes. `--brief` leaves out the object summaries of changing calls and keeps ids and bounds.
- When a call in a batch fails, nothing is changed. The output names the failed call and shows `earlier_results`, what the calls before it returned.

## Groups, components and scope

- Every board is a group. Editing tools (`move`, `resize_board`, `rename`, `tag`, `erase`, `add_joint`, `push_pull`…) reach parts inside groups and components by themselves when all the ids in a call share one group. Ids from different groups need separate calls.
- To **create** a part inside a group, open it with `enter_object` (nested groups from the outside in). With the CLI, the opened group lasts only for that command, so put `enter_object` and the creating calls in one `--batch` and finish with `exit_to_root`.
- `group` with several ids makes ONE group, or one component with `component: true`. With one board and `component: true`, it wraps that board in a new component. Grouping keeps existing joints. Two components with the same name are still different definitions; only `create_copy` makes linked instances.
- Components suit identical parts whose joints stay inside them, such as a drawer box. Build one completely, including its joints, make it a component, then `create_copy` it to each position. Instances of one component share a `definition` id in the scene.
- A joint can only be added between boards at the same level. A board inside a component instance cannot be joined to a board outside it. Making a board that already has joints into a component keeps those joints, but `create_copy` does not copy joints with boards outside the component, and they cannot be added to the copies.
- **When the user asks for housed parts to be one component** (shelves in grooves in the sides), it cannot keep the grooves. By default keep them as separate identical boards, with the same name and tag, and say why. Offer the alternative: one component of shelves resting on pins or cleats, without joints to the sides.
- A tag on a group applies to the boards inside it (the scene shows the inherited tag). Tag boards directly when parts of one group need different tags.
- Erasing a board also removes its joints. `remove_joint` deletes a joint alone. Moving, resizing or rotating a board keeps its joints.

## Joints

A joint joins two boards that **overlap** where they meet; the overlap is the joint's depth. Make the overlap first by placing one board partly inside the other, then call `add_joint`. Joints are cut when BenchUp evaluates the model, so the scene still shows plain boxes. `get_object` on the joint reports `"status": "ok"` or the reason it cannot be cut, and `add_joint` refuses joints that cannot be cut.

- **`a` carries the tenon, tongue, tails, fingers or lap**: the board that goes into the other (shelf, apron, drawer side, back panel).
- **`b` receives** the mortise, groove or sockets (side, leg, drawer front).
- Leave `depth_axis`/`width_axis` out. BenchUp picks the woodworking orientation: depth is the direction `a` goes into `b`. Tongues, tenons and rabbets are spread across `a`'s thickness; dovetails, fingers and laps across its broad side. The result reports the chosen orientation.
- A joint with `a` and `b` reversed would cut the tongue into the side and the groove into the shelf. `check_model` reports it, and `flip_joint` swaps the roles while keeping the joint's settings.
- Settings by kind: `mortise_tenon` uses `width_ratio` (tenon thickness as a share of `a`'s thickness) and `height_ratio`; `tongue_groove` and `rabbet` use `width_ratio`; `dovetail` uses `dovetails`, `width_ratio` (share of the width taken by tails) and `taper_degrees`; `finger` uses `fingers`. All use `clearance_mm`.

| Situation | Overlap | Joint |
|---|---|---|
| Shelf housed in a side | Shelf enters the side by about half the side's thickness (9 mm into 18) | `tongue_groove`, a = shelf, b = side |
| Apron into a leg | Apron enters the leg 25 to 35 mm, set back 5 to 10 mm from the leg's outer face | `mortise_tenon`, a = apron, b = leg, `width_ratio` about 0.33 (tenon one third of the apron's thickness), `height_ratio` about 0.75 |
| Back panel in a groove | Back (6 mm) enters each side by about half the side's thickness, 6 to 10 mm in from the back edge | `tongue_groove`, a = back, b = side |
| Back panel in a rabbet | Back enters each side by about half the side's thickness, flush with the back edge. Stop housed shelves 2 mm short of it (see below) | `rabbet`, a = back, b = side |
| Drawer side into the front (half-blind dovetail) | Side enters the front from behind, leaving 4 to 6 mm of the front | `dovetail`, a = drawer side, b = front, `dovetails` 3 to 5 |
| Drawer bottom | Bottom (6 mm) enters the drawer sides by 6 mm and stops 1 to 2 mm behind the front, clear of the dovetails | `tongue_groove`, a = bottom, b = side |
| Frame corners | Rails and stiles cross by their full width | `half_lap`, a = rail, b = stile |
| Box corners | Two panels overlap by the other's thickness | `finger`, a and b either way |
| Plinth between the sides | Plinth enters each side by about half the side's thickness | `tongue_groove`, a = plinth, b = side; or butt it without a joint |
| Top on the sides | Sides enter the top from below; the groove shows at the top's edges unless the sides stop short of them | `tongue_groove`, a = side, b = top; or rest the top on the sides and note the fixing |

### Several joints in one board

Two joints cut into the same board must not cross, and must not meet face to face inside it. For example, a shelf's groove that ends exactly where the back's rabbet begins leaves a sliver of zero thickness. The joint is then refused with a "closed solid" or "valid solid geometry" error, and the error names the other joints on those boards that touch it. Joints that only share the board's outer face are fine, such as a back and a plinth both reaching the floor. Keep at least 1 to 2 mm of wood between joint regions inside a board:

- **Shelves and a back:** stop shelves, top and bottom 1 to 2 mm short of the back, not flush with it. Then a rabbet or a groove for the back both work.
- **Drawer bottom:** groove it into the drawer sides only, and stop it 1 to 2 mm behind the front. Where the sides already enter the front with dovetails, a groove in the front would meet them.
- **The same error with no other joint nearby** comes from the board's own geometry: a shaped board with many small faces, or vertices very close to the joint's planes. Change the number of segments of a curve (24 and 48 are known to work) or move the joint a few millimetres, and test in a `--dry-run`.
- **Two aprons meeting in one leg:** their tenons must not meet inside the leg. With leg width P, apron setback s from the leg's outer face and apron thickness t, keep the tenon depth below P − s − t (70 − 10 − 22 = 38 mm, so use 30).
- **A central divider:** shelves on both sides at the same height would cut it through with two 9 mm grooves. Let them enter by a third of its thickness each, stagger the heights, or butt them against it without a joint.

## Recipes

- **Carcass (cabinet, bookcase):** two sides, a top and a bottom (between the sides, or the top over them), shelves, and a back in a groove. Stop everything inside 1 to 2 mm short of the back.
- **Even shelves:** with an inner height H (between the bottom and the top, or the full side height if there are none), n shelves of thickness t and equal gaps above, below and between, the gap is (H - n·t)/(n + 1). Shelf k (counting from 1) has its bottom at k·gap + (k - 1)·t above the inner bottom. To change the number of shelves, erase or copy shelves, then `move` them to the new heights; their joints follow.
- **Plinth:** a board 60 to 100 mm high between the sides, set back 20 to 50 mm from the front, usually under a bottom panel resting on it. A divider that reaches the floor splits it in two; butt each half against the divider.
- **A back behind a central divider:** stop the divider at the back's front face so one back runs behind both bays.
- **A top with chamfered or rounded front corners over the sides:** let the top overhang the sides at the front by at least the chamfer, or set the sides back, so their corners do not show under the cut.
- **Table:** legs 60 to 80 mm square; aprons 80 to 120 mm high and 20 to 25 mm thick, their tops flush with the legs, tenons as above; a top 25 to 40 mm thick resting on the frame with a 20 to 50 mm overhang. Ask whether the size given is the top or the frame, and whether the height includes the top.
- **Drawer:** build one drawer as a component (front, sides, back, bottom and their joints), then `create_copy` it up by the front's height plus the gap. Inset fronts leave about 2 mm at each side and 3 to 4 mm between drawers. The box (sides, back and bottom) has sides 12 to 15 mm thick and 30 to 50 mm lower than the front, and is 10 to 20 mm shorter than the carcass's inner depth. The bottom is 6 mm, grooved into the sides about 10 mm above their lower edge, and the back sits on the bottom. With metal runners, make the box about 12.5 mm narrower on each side, or as the runner specifies; with wooden runners, about 2 mm.
- **Lengthening a piece:** `resize_board` the boards that run in that direction, choosing the end with `anchor`, and `move` the parts at that end. `resize_board` stretches the whole board, notches included. For a board with notches or a shape, `push_pull` its end face by the extra length instead; that keeps the notches where they are. Faces in one plane merge after a push. To push only part of a face, split it first with a `draw_polyline` line across it (inside the board, with `enter_object`); the result lists each new face with its `center_mm`. The seam edges left in a flat face are harmless. Do not `erase` them: erasing an edge removes the faces on it.
- **Before choosing which end to lengthen,** look for walls and other parts in the way. The room in an example project can be a group of plain faces (a "Floor" with walls), whose faces `get_object` lists with their positions.
- **Changing the stock thickness while keeping the outside size:** resize each board's thickness with `resize_board`, using `anchor` to keep the outer faces in place (`["max", "min", "min"]` for a right side, `"center"` for dividers and shelves), and keep each joint's overlap at about half the new thickness. Keeping the shelves' centres keeps the gaps symmetric, but a change of an odd number of millimetres gives half millimetres; anchor the shelves at `"min"` for whole numbers. Recompute equal gaps only if asked. Create the new material and assign it; `check_model` reports boards whose thickness does not match their material.
- **A leaning piece (ladder shelf):** work out each upright's outline as it will stand: back edge leaning θ from vertical and touching the wall at the top, foot and top cut level. Rotate that outline by +θ about x around the foot of its back edge, which stands it upright, and draw it there (see shaped boards). Set its grain to z, then `rotate` it by −θ about x around the same point, so its top leans towards the wall at +Y. An upright of constant width w (measured square to its edges) and height H has edges H/cos θ long and level cuts w/cos θ long, and the foot of its back edge sits H·tan θ from the wall. Uprights of constant width give every shelf the same depth. For shelves that get shallower towards the top, taper the uprights (wider at the foot), or let the shelves stick out in front of narrow uprights. Keep shelf edges a few millimetres away from the uprights' sloping faces; a near-coincident edge leaves slivers smaller than the model precision. In the scene, a rotated board's grain shows as a vector, and the cut list measures it unrotated.
- **Materials and cut list:** `create_stock_material` (name, thickness, width, length; the length along the grain), then `set_material` on the boards. The material's thickness should match the boards'. `get_cut_list` groups boards by material and size; `minimum_stock_pieces`, an area lower bound, appears once boards have a material with sizes. To answer "how many sheets do I need" without changing the project, create and assign the material and call `get_cut_list` in one `--dry-run` batch.
- **Dimensions for the workshop:** put overall height and width, and a chain of shelf gaps, in the front plane (y = 0) so they read in the front view; put the depth in the side plane (x = the width), offset upwards. A dimension measures the straight distance between its points. Pass `axis` to measure only along x, y or z, so a slightly misplaced point cannot tilt it. Points exactly on a corner, edge or face attach to it and follow it. Nothing snaps, so use real corners: where a shelf enters a side, its visible corner is on the side's inner face. `offset_mm` pushes the line away from the piece: chain dimensions about 100 mm out, overall ones about 250 mm out, so they do not overlap. `add_dimension` returns `length_mm`, and `get_scene` lists every dimension. Put the piece's name in a text above it. Ask whether shelf spacing means clear space or centre to centre.

## Shaped boards (curves, chamfers, angled ends)

The tools have no arc tool and do not expose BenchUp's Router yet (edge chamfers, bevels, roundovers, coves, rabbets, grooves and other profiles). For a profile along an edge, say that the user can apply it with the Router in BenchUp, or cut the shape into the outline as below. Draw the board's outline as one closed `draw_polyline`, using many short segments for a curve, on the plane where the board starts. Then `push_pull` its face to the thickness and `group` everything the two calls created. Finish with `set_grain` and a name. In one batch:

```jsonl
{"tool": "draw_polyline", "arguments": {"points_mm": [[0,0,0],[440,0,0],[450,10,0],[450,350,0],[0,350,0]], "close": true}}
{"tool": "push_pull", "arguments": {"face_id": "$1.face_id", "distance_mm": 18}}
{"tool": "group", "arguments": {"ids": ["$1.created", "$2.created"], "name": "Top", "component": false}}
{"tool": "set_grain", "arguments": {"id": "$3.created[0]", "axis": "x"}}
```

A loose face's normal follows the order of its points: counter-clockwise seen from the side it faces. The outline above extrudes towards +z. A side panel drawn in the plane x = 0 extrudes towards +x when its points run counter-clockwise seen from +x, that is with y and z as the axes on the page: [[0,0,0],[0,350,0],[0,350,532],[0,0,532]]. Check the direction in the result's `bounds_mm` with `--dry-run`. The cut list measures a board along its grain, so draw a part in its natural orientation, set the grain, then rotate it into place. Joints work on shaped boards too.

For a curved edge with chord c and depth s (how far the curve goes in at its middle), the radius is R = (c²/4 + s²)/(2s). Generate 24 to 48 points along the arc in a script. Parts behind a curved front (shelves, a bottom) must be set back far enough to stay behind the curve at their height.

Avoid cutting through a finished board with a negative `push_pull` of its full thickness. It currently leaves stray faces and edges, and `check_model` reports an open solid. Rebuild the part from its outline instead.

## When to stop and ask

- A requested size does not fit, such as a shelf longer than its opening. Measure the free space from `bounds_mm`: from one board's inner face to the next. A housed shelf is that gap plus both overlaps. Try the change in a `--dry-run` batch ending with `check_model`, report what you measured, and ask instead of forcing it.
- Ambiguous terms. Ask whether "two columns" means two bays, and whether shelf spacing is clear space or centre to centre. Ask whether "1000 mm from the floor" is the top or the bottom face. Ask whether a front is inset or overlaid, which end of a piece to lengthen, and whether a size is the top or the frame. "The sides" of a bay can include a divider.
- A request that conflicts with how the piece is built, such as housed shelves as one component. Keep the construction, do what can be done, and offer the alternative.
- Problems already in the model that the user did not mention, such as joints `check_model` reports as reversed. Report them and offer to fix them. If the change touches those joints anyway, fixing them is reasonable; do it as a separate batch so it can be undone on its own.
- In each case, propose an interpretation and say which one you used.
- When you cannot ask (no one to answer), do not apply a change that departs from the request. Leave the project as it was, save the tested alternative as a batch file, and report both.

## Limitations

- Joints are cut from the overlap when BenchUp evaluates the model. The tools show the boards before cutting; use the joint's `status` to confirm.
- There is no fixing hardware (screws, dowels, metal runners). Leave a top resting on its frame, and note the fixing with `add_text` if it matters.
- The tools cannot read, set or render views and cameras, so you cannot look at the model. Check it with `get_scene`, `get_object` and `check_model`, and ask the user to look in BenchUp when the result must be seen. Saved views are left as they were.
- Some BenchUp features are not available through the tools yet: the Router's edge profiles, the Cutting Plan (stock layouts with saw kerf and grain), Workshop Labels and the Time & Cost Estimates PDFs, trimming excess material at joints, and wood finishes. `get_cut_list` gives an area lower bound; for a real layout, the user can export the Cutting Plan from BenchUp's Tools menu.

# One More Night: greybox brief for the Blender session

You are building the greybox of the manor in Blender, through the Blender MCP. This level is spoiler-free on purpose: it is designed before the mystery exists. Don't invent story content or suggest story-driven changes.

## Inputs (in `design/level/`)

- `level_spec_v2.md`: the level spec, exported from the design doc. It is the source of truth.
- `level_spec.json`: the same tables as data: 40 rooms with their boxes, 53 doors, 14 features, devices, metrics, anchor rules, and walk budget. Read this instead of parsing the Markdown. If the JSON and the Markdown disagree, follow the Markdown and report the difference.

## How to work

1. **Scripts, not one-off calls.** Keep the build as Python files in the repo, and use the MCP to run them and look at the results.
   - `tools/greybox/build.py` reads `level_spec.json` and rebuilds everything it generates from scratch. It deletes and recreates only objects whose custom property is `generated = True`, and never touches anything placed by hand.
   - `tools/greybox/export_level.py` does the headless export, writes the manifest and runs the lint. See "Blender conventions and the manifest" in the spec.
   - Run the scripts inside Blender through the MCP with `exec(open(path).read())`, or headless with `blender -b manor.blend -P <script>`.
2. **Build in this order.** After each step, take a screenshot of the level from above and one at the player's eye height (1.62 m), report what you built, and wait for a go-ahead.
   1. Floors, ceilings and walls for each level, from the room boxes. Put walls on box edges, with one wall where two rooms share an edge: 0.4 m on the exterior, 0.2 m inside. Make openings by splitting wall segments around them (no booleans), and add a lintel above each one. Outdoor rooms (`"outdoor": true`) get ground but no walls.
   2. One room volume per room, `room.<id>`, with all its boxes joined into one mesh and the custom properties from the spec.
   3. A door empty `door.<id>` for every door, and a simple leaf mesh `doorleaf.<id>` with its origin on the hinge. Archways, steps and plain openings get no leaf.
   4. Stairs:
      - the main U-shaped stair inside its footprint in the hall
      - the back stair, a dog-leg from the cellar to the attic
      - collision ramps for both, in `Nav`
      - the hall's south half (x 11–19, y 0–8) at double height, with no first-floor slab and a balustrade along y = 8 on the first floor
   5. Features F01–F14, each in its own `Features/F0N` collection, with a scene property that toggles each one for testing.
   6. Devices: the dumbwaiter shaft and its three stops, bell pulls, the bell board, both telephones, the spawns and the beds.
   7. Outdoors: the forecourt and yard ground, the terrace with steps down to the garden, the garden walls with the east gate, and the shed, garage and path.
   8. A first pass of anchors that meets each room's minimum. Block in furniture as simple boxes so `surface` anchors have something to sit on. Put seats along walls and by fireplaces. Pair anchors stand 1.2 m apart, facing each other. Tags go in custom properties. Gui will hand-tune them.
   9. The export script, the manifest and the lint. Add a walk-budget estimate: shortest paths on the anchor graph at 1.4 m/s, with stairs at ×0.75, PASS or FAIL for each route.
3. **Where the spec is silent,** for example window positions, fireplaces other than F06, the roof, the exact stair geometry, terrace and garden-wall heights or furniture layout, choose something plain and log each choice in `design/level/greybox_decisions.md`.
4. **Don't change** room bounds, ids, door positions or the feature list. If something doesn't fit, stop and report it.

## Hard rules

- Coordinates are in metres: +X east, +Y north, +Z up. The origin is the south-west corner of the main block at ground level. Floor heights: cellar −3.0, ground 0.0, first 3.8, attic 7.2.
- Object names and custom properties must match the spec's Blender conventions table exactly, because the exporter and the game read them.
- Every generated object carries `generated = True`. Never delete an object that doesn't have it.
- Save `manor.blend` after each step, and commit the scripts.

## Done when

Every box on the spec's greybox checklist is ticked except the walk test and the `level-v1` tag. Gui does those two himself.

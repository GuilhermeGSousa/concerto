# The manor greybox

The Blender source of the One More Night manor, the scripts that generate it,
and its export. Paths below are relative to this folder.

| Path | What it is |
|---|---|
| `manor.blend` | The greybox. Every object the scripts make carries `generated = True`. |
| `design/level/level_spec_v2.md` | The level spec, the source of truth. |
| `design/level/level_spec.json` | The spec's tables as data, read by the scripts. |
| `design/level/greybox_brief.md` | The build order and rules the greybox followed. |
| `design/level/greybox_decisions.md` | Every choice made where the spec is silent. |
| `tools/greybox/build.py` | Rebuilds every generated object from the spec and `layout.py`. |
| `tools/greybox/layout.py` | Furniture and anchors, one list of items per room. |
| `tools/greybox/views.py` | Viewport presets for review screenshots. |
| `tools/greybox/export_level.py` | Writes `export/`, runs the lint and estimates the walk budget. |
| `export/` | One `.glb` per level, `level_manifest.ron`, `travel_full.csv`, `level_lint.txt`. |

## Rebuild

`build.py` deletes and recreates only objects with `generated = True`. An
object moved by hand is rebuilt from the scripts on the next run unless its
`generated` property is removed first, or the change is made in `layout.py`.

In Blender's Python console, with `manor.blend` open:

```python
p = "<absolute path>/tools/greybox/build.py"
g = {"__file__": p, "__name__": "greybox_build"}
exec(compile(open(p).read(), p, "exec"), g)
report = g["main"]()
```

`report["ok"]` is false, and `report["errors"]` names the objects, when a door
has no wall, furniture collides, or a room misses its anchor minimum. Save the
file afterwards.

## Layout items

Each room in `layout.py` is a list of items. Anchors are numbered in list
order, so reordering a list renumbers that room's anchors.

| Item | Meaning |
|---|---|
| `W(kind, side, at)` | Furniture against the N, S, E or W wall of the room's box, centred at `at` along it. |
| `F(kind, x, y, face)` | Free-standing furniture centred at `(x, y)`, its front facing `face`. |
| `PAIR(x, y, axis)` | A standing conversation pair, 0.6 m either side of the point along the axis. |
| `PSEAT(x, y, axis)` | Two chairs facing each other 1.2 m apart, with a small table between. |
| `A(tags, x, y, facing)` | One anchor, `z` above the floor, facing N, S, E, W or degrees. |
| `PIC(side, at)` | A wall-hung prop anchor at 1.6 m. |
| `WIN(side, n)` | A standing spot at the n-th window of that wall. |
| `RUG(x, y, w, d)` | A rug with a `hide` anchor under it. |
| `OBST(x0, y0, x1, y1)` | Something built by an earlier step that furniture must avoid. |

`KINDS` gives each furniture kind its size and the anchors it brings; an item
overrides them with `surf=`, `drawer=`, `hide=`, `seats=` or `work=`.

## Review views

```python
p = "<absolute path>/tools/greybox/views.py"
v = {"__file__": p, "__name__": "views"}
exec(compile(open(p).read(), p, "exec"), v)
v["section"]("first")
v["eye"](15, 1.5, "ground", 0)
v["volumes"]()
v["swing"](True)
v["feature"]("F05", False)
v["reset"]()
```

`section` is a plan with the walls cut 1 m above the floor, `eye` a view at
1.62 m with the yaw in degrees counter-clockwise from north, `volumes` the room
volumes coloured by zone, `swing` opens every door leaf, `feature` sets a
feature flag, and `reset` puts everything back.

## Export

```sh
blender -b "$PWD/manor.blend" -P "$PWD/tools/greybox/export_level.py" -- "$PWD/export"
```

A Flatpak Blender is `flatpak run org.blender.Blender` with the same
arguments. It starts in its own directory, so every path is absolute. The exit
code is 1 when the lint fails.

In `level_manifest.ron`, coordinates are metres with +X east, +Y north and +Z
up, and `facing` is degrees counter-clockwise from north. `travel` holds one
representative anchor per room; every anchor pair is in `travel_full.csv`.
The `.glb` files are Y up, with each object's custom properties as glTF
extras.

After hand-tuning anchors, export again and commit `export/`. The manifest's
SHA-256 is the last line of `level_lint.txt`.

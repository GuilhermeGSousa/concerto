# The manor

The Blender source of the One More Night manor, the scripts that generate its
greybox and the art over it, and its export. Paths below are relative to this folder.

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
| `tools/art/build_art.py` | Rebuilds the art over the greybox: every object in the `Art` collection. |
| `tools/art/library.py` | Fits a downloaded model to its size and writes it to `library/`. |
| `tools/art/textures.py` | Makes the tinted and patterned textures in `textures/` from the fetched ones. |
| `tools/art/fetch_textures.py` | Downloads the surface textures into `textures/`. |
| `tools/art/preview.py` | Renders a lamplit view of the dressed rooms. |
| `library/` | One fitted `.glb` per furniture or fixture model. |
| `textures/` | The surface textures, fetched and made. |
| `export/` | One `.glb` per level and `manor_art.glb`, `level_manifest.ron`, `travel_full.csv`, `level_lint.txt`. |

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

## Art

The greybox is the collision and the art is the picture. Every art object is
named `art.<something>`, lives in the `Art` collection, carries
`generated_art = True` and exports to `export/manor_art.glb`. The game draws
it and never makes it solid. An art object named `art.<greybox object>`, such
as `art.furniture.hall.gong_01`, takes that object's place on screen: the game
keeps the greybox object solid and stops drawing it, and hides both when a
feature hides the object. A greybox object with no art is drawn as before, so
the house is dressed one room at a time. Today that is the hall and the
drawing room.

`build_art.py` deletes and recreates every `generated_art` object. Run it
after `build.py`, which rebuilds the boxes it reads:

```python
p = "<absolute path>/tools/art/build_art.py"
g = {"__file__": p, "__name__": "build_art"}
exec(compile(open(p).read(), p, "exec"), g)
report = g["main"](("hall", "drawing"))
```

It copies the faces of each wall, floor, ceiling and chimney breast of those
rooms and gives each face the material of the room it looks into, from
`STYLE`; adds skirting, dado rail, cornice, panelling, door architraves,
window glazing and curtains; replaces balustrades with turned balusters;
gives door leaves their panels in place, one material each; and puts a
library model over every furniture box, chosen by `FURNITURE`. `LAMPS` lists
where the game's `Level::lamps()` puts its ceiling lights, so a chandelier
hangs at each; change both together. `DRESS` holds the sconces and pictures.

A model is fitted once and then kept in `library/`:

```sh
blender -b -P "$PWD/tools/art/library.py" -- <downloads> "$PWD/library" [name ...]
```

`<downloads>` holds `<name>/<name>.glb` as Hyper3D's Rodin returned it, and
`<name>/<name>_emissive.png` for a model that glows. `MODELS` gives each
model its size in metres, the turn that brings its front to -Y, where its
origin sits, and the size of its textures.

`python3 tools/art/fetch_textures.py "$PWD/textures"` downloads the surface
textures at 1K; `build_art.py` makes the tinted and patterned ones beside
them when they are missing.

After a rebuild, export, import `export/manor_art.glb` and any level file that
changed through the editor on a scratch copy of the project, and copy
`content/manor/export/` back with the `.import.toml` files.

| Source | What | Licence |
|---|---|---|
| Hyper3D Rodin Gen-2.5, text prompts | Every model in `library/` | Generated for this project |
| Poly Haven: `herringbone_parquet`, `marble_01`, `dark_wood`, `painted_plaster_wall`, `wood_floor` | `textures/*.jpg` | CC0 |

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
`ways` and `paths` are the walk graph the times were measured on: every
standing anchor, door threshold, stair end and landing, and a grid of free
floor, joined where one can be walked to from the other. The game routes its
characters over it.

`-- <out> --manifest-only` rewrites the manifest, the lint and the travel table
and leaves the `.glb` files alone.
The `.glb` files are Y up, with each object's custom properties as glTF
extras.

After hand-tuning anchors, export again and commit `export/`. The manifest's
SHA-256 is the last line of `level_lint.txt`.

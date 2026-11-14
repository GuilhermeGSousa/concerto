# Sanatorium art

The art over the sanatorium graybox: an Alpine tuberculosis sanatorium in the
winter of 1928. Cold blue snow and moonlight against warm tungsten lamps; white
glazed tile under celadon plaster, dark larch, cast iron and brass.

The graybox is the collision and the art is the picture. `build_art.py` reads
the nine area files beside this folder (`../EXT_*.glb`, `../INT_*.glb`) and
writes one dressed level, `export/sanatorium_art.glb`. The game never draws the
graybox: the level scene, `content/sanatorium/level.gasset`, holds the dressed
art and the markers, and the recipe spawns the `COL` scene beside it for
collision.

| Path | What it is |
|---|---|
| `models.json` | Each model's Hyper3D Rodin generation id and the graybox pieces it stands for |
| `tools/library.py` | Fits a downloaded model to its size and triangle budget and writes it to `library/` |
| `tools/fetch_textures.py` | Downloads the surface textures into `textures/` |
| `tools/build_art.py` | Dresses the graybox and writes `export/sanatorium_art.glb` |
| `library/` | One fitted `.glb` per model |
| `textures/` | The surface textures, fetched, and the tinted copies `build_art.py` makes |
| `export/sanatorium_art.glb` | The dressed level: props, surfaces, lamp lights and the moon |

## What the art pass does

- Every prop, door and the funicular car is replaced by its library model,
  fitted to the graybox box in the box's own frame. `REPLACE` names the model
  and how it fits: `box` stretches it to the box, `footprint` fits its
  footprint and scales its height by the same amount, `height` scales it evenly
  to the box's height, `natural` keeps its size. Where a box runs the other way
  to its model, the model turns a quarter; where the turn is open, the model's
  back goes to the wall it stands against. Doors, trees, crutches and posts keep
  the yaw the graybox gave them. The boiler stands as a pair across its box.
- Every other piece keeps its geometry and takes a surface by its graybox
  material, `BY_MATERIAL`, or by `OVERRIDES` for a kind or an area: slate on
  the roofs, brick in the boiler basement, white frames on the windows, green
  shutters, wood floors in the director's wing. Each face gets UVs from its
  world position at the surface's tile size, so textures keep their scale
  across pieces. The backdrop ridge takes snow on faces flatter than
  `SNOW_ABOVE`.
- Pendants, lamp posts, the lantern, the desk lamp, the stove and the boilers
  get warm point lights, `LIGHTS`, each with a reach; the level gets one cold
  moon.

## Rebuild

```sh
python3 tools/fetch_textures.py "$PWD/textures"
flatpak run org.blender.Blender -b -P tools/library.py -- <downloads> library [name ...]
flatpak run org.blender.Blender -b -P tools/build_art.py
```

`<downloads>` holds `<name>/<name>.glb` as Rodin returned it, and
`<name>/<name>_emissive.png` for a model that glows. Downloads expire a week
after Rodin returns them; a model's generation id in `models.json` fetches it
again from Hyper3D. `door_leaf` and `door_oak_leaf` are cut from `door_room`
and `door_entrance` by `MODELS`' `cut`.

Then import `export/sanatorium_art.glb` through the editor on a scratch copy of
the project and copy `content/sanatorium/art/` back with the `.import.toml`.
The import keeps every asset id while the dressed level has the same objects,
so the level scene needs nothing. When objects are added or removed, open
`content/sanatorium/level.gasset`, delete its `Art` entity, place
`content/sanatorium/art/export/sanatorium_art/scene.gasset` again, name it
`Art` and save.

| Source | What | Licence |
|---|---|---|
| Hyper3D Rodin Gen-2.5, text prompts | Every model in `library/` | Generated for this project |
| Poly Haven, the assets named in `fetch_textures.py` | `textures/*.jpg` | CC0 |

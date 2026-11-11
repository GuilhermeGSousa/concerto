# Art

The look is a lamplit miniature: low-poly shapes with clean silhouettes, warm
lamps against a cold blue night, dark oak and brass. The drawing room has green
wallpaper, the study burgundy, and the service corridor plain plaster over
checker tile, so a player knows which side of the house they are on at a
glance.

Everything here is made by one script, `tools/build_art.py`, so nothing is
painted by hand and a change of palette is a change of numbers.

| Path | What it holds |
|---|---|
| `tools/build_art.py` | Generates the textures, builds the meshes, exports the glTF files and saves `art.blend` |
| `textures/` | The generated 512 px tileable textures: oak, mahogany, parquet, two wallpapers, plaster, ceiling, tiles, rug |
| `export/level_test.glb` | The test level's visible geometry: floors, walls, ceiling, skirting, furniture, rug, lamps |
| `export/kit_*.glb` | One model each for the door, bell pull, bell board, gong, bed, dumbwaiter and the five props |
| `art.blend` | The scene the script leaves behind, for looking at |

## Rebuilding

Run the script in Blender (5.2 was used). It clears the scene it runs in:

```sh
blender --background --python examples/one-more-night/assets/art/tools/build_art.py
```

Then import each file in `export/` through the editor, on a scratch copy of
the project, and copy `content/art/` back. The `.import.toml` beside each file
keeps the asset ids, so the game's `asset_id!` paths stay valid.

## How the game uses it

- A manifest point `(x, y, z)` is the same point in Blender; the export turns
  it into the engine's `(x, z, -y)`.
- `level_test` is spawned whole at startup. It has no colliders: the level
  scene, `content/level/test_level.gasset`, holds the collision boxes, lights,
  spawn points and fixtures, with no meshes of its own.
- A fixture gets its kit model as a child, picked by its kind; a prop gets the
  model of its prop id. Each kit model is centred on its own origin, at real
  size.

## Not here yet

Normal and roughness maps, the characters (still the UAL1 placeholder),
portraits, and anything for the manor.

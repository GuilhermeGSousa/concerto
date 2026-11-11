# Characters

The characters are painted wooden figurines in period dress, to sit in the
lamplit miniature rooms of `../art`. Each is built from faceted parts bound to
one bone each, so they move like jointed dolls, and each is told apart by
silhouette and colour before any detail.

| Model | Who wears it in the test case | Look |
|---|---|---|
| `lady` | Lady Ashby | A burgundy gown to the floor, long white gloves, grey hair in a bun, a brooch |
| `gentleman` | Mr Corwin | Black tails, a white shirt front and bow tie, a moustache |
| `footman` | Hobbs | Green livery with brass buttons, cream breeches, white stockings and gloves |

Every model is skinned to the skeleton of `UAL1.glb`, the Universal Animation
Library's mannequin, and exported with five of its clips: `Idle_Loop`,
`Walk_Loop`, `Sitting_Idle_Loop`, `Sitting_Enter` and `Sitting_Exit`. A new
character is a new entry in `LOOKS` in `tools/build_characters.py`.

| Path | What it holds |
|---|---|
| `tools/build_characters.py` | Builds the models on the rig, exports them, and renders the line-up and the portraits |
| `UAL1.glb` | The rig and its clips; the script reads it and the game does not |
| `export/*.glb` | One file per model, with the rig and the five clips |
| `portraits/portrait_NN.png` | A head-and-shoulders render of each model, shown in the casebook |
| `lineup.png` | All the models side by side, to look at |
| `characters.blend` | The scene the script leaves behind |

The game picks a model by the character's `portrait` id in the case:
`portrait_01` is the lady, `portrait_02` the gentleman, `portrait_03` the
footman.

## Rebuilding

Run the script in Blender (5.2 was used); it clears the scene it runs in.
Then import `export/*.glb` and `portraits/*.png` through the editor on a
scratch copy of the project, and copy `content/characters/` back.

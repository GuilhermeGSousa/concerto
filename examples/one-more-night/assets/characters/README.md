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
| `colonel` | Nobody yet | A khaki tweed jacket, brown trousers, white hair and moustache |
| `debutante` | Nobody yet | A sea-green gown, bare arms, fair hair, a brooch |
| `doctor` | Nobody yet | A grey suit, black hair, brass spectacles |
| `butler` | Nobody yet | Black tails over grey trousers, grey hair, white gloves |
| `cook` | Nobody yet | A white jacket with rolled sleeves, a blue-grey skirt, apron and cap, red hair |
| `housekeeper` | Nobody yet | A dark plum dress to the floor, a white collar, a bun, a brooch |
| `chauffeur` | Nobody yet | A blue-grey tunic with brass buttons, a peaked cap, black gauntlets and boots |
| `guest` | The player who is the Guest | A navy suit with a white shirt front, grey trousers, brown shoes |
| `maid` | The player who is the Staff | A black dress with a white apron, collar and cap |

Every model is skinned to the skeleton of `UAL1.glb`, the Universal Animation
Library's mannequin, and exported with five of its clips: `Idle_Loop`,
`Walk_Loop`, `Sitting_Idle_Loop`, `Sitting_Enter` and `Sitting_Exit`. A new
character is a new entry in `LOOKS` in `tools/build_characters.py`, a new arm
in `figure_of` in `src/world.rs`, and a new row in the two `PORTRAITS` tables.

| Path | What it holds |
|---|---|
| `tools/build_characters.py` | Builds the models on the rig, exports them, and renders the line-up and the portraits |
| `UAL1.glb` | The rig and its clips; the script reads it and the game does not |
| `export/*.glb` | One file per model, with the rig and the five clips |
| `portraits/portrait_NN.png` | A head-and-shoulders render of each model, shown in the casebook |
| `lineup.png` | All the models side by side, to look at |
| `characters.blend` | The scene the script leaves behind |

The game picks a model by the character's `portrait` id in the case:
`portrait_01` to `portrait_10` are the lady, the gentleman, the footman, the
colonel, the debutante, the doctor, the butler, the cook, the housekeeper and
the chauffeur. A look's `portrait` number in `LOOKS` sets it; the players'
models have none.

## Rebuilding

Run the script in Blender (5.2 was used); it clears the scene it runs in.
Then import `export/*.glb` and `portraits/*.png` through the editor on a
scratch copy of the project, and copy `content/characters/` back.

# The traveller

The player of `--retarget` and `--sanatorium --retarget`: a woman in her early
thirties who has come up to the sanatorium in the winter of 1928 to find her
brother. A thigh-length charcoal wool coat, an oxblood scarf, brown leather
gloves and laced winter boots. The coat stops above the knee so it bends with
the legs without cloth simulation.

| Path | What it is |
|---|---|
| `source/character_rodin.glb` | Hyper3D Rodin Gen-2.5 High, quad mesh, 2K PBR textures |
| `source/character_rodin_rigged.glb` | The same model rigged by Rodin's human template: 52 bones and their skin weights |
| `tools/fit_character.py` | Fits the rigged model onto UAL1's skeleton and writes `export/character.glb` |
| `export/character.glb` | The model skinned to UAL1's skeleton, with no clips |

## How it is fitted

The game plays UAL1's clips on this character and LAFAN1's motion through the
retarget node, and every UAL1 clip keys the translation of every bone, so the
character has to sit on UAL1's skeleton exactly: its joints, its bone frames
and its joint order. `fit_character.py`:

1. scales the model so its head joint is at UAL1's head height;
2. moves each Rodin bone onto the UAL1 bone it maps to, turning it to the
   same direction and stretching it along its length to the same length, the
   head and neck never stretched, and the hands and fingers turned by the palm;
3. deforms the mesh by Rodin's weights through those moves, which takes it
   from Rodin's A-pose to UAL1's T-pose and UAL1's proportions;
4. renames the weights to UAL1's bones, parents the mesh to UAL1's armature
   and exports it;
5. rewrites the exported skin in UAL1's joint order, because the importer
   numbers bones in that order and the retarget node compares the two lists.

```sh
flatpak run org.blender.Blender -b -P tools/fit_character.py -- \
  source/character_rodin_rigged.glb ../UAL1.glb export/character.glb
```

Then import `export/character.glb` through the editor on a scratch copy of the
project and copy `content/character/` back with the `.import.toml`.

| Source | Licence |
|---|---|
| Hyper3D Rodin, text prompt, generation `c78c8944-8463-47ea-9664-cad2ab3b8774`, rig `80fa9503-7ec7-46e2-ac4d-50192db31750` | Generated for this project |
| `../UAL1.glb`, the skeleton | As in `../UAL1.glb`'s source |

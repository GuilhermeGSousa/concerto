# 10. Crate map

Every directory under `crates/` is a package named `concerto-<dir>`. A game
depends on `concerto`, which re-exports most of them under short names
(`concerto::ecs`, `concerto::render`, ...). The others are tools, or crates a
game adds itself when it needs them.

| Area | Crate | What it holds | In `concerto` as |
|---|---|---|---|
| Core | `ecs` | entities, components, queries, resources, commands, events, signals, schedules, change detection | `ecs` |
| | `app` | `App`, plugins, schedules, the main and render sub-apps, edit time | `app` |
| | `foundation` | assets and the asset server, transforms, time, math, `asset_id!` | `foundation`, `asset_id!` |
| | `tasks` | task pools for background work | |
| | `color` | colors | `color` |
| | `reflect` | reflection: field access by path, the type registry | `reflect` |
| | `name` | `Name`, an interned string that compares as one integer and persists as its text, and `name!` | `name` |
| | `noise` | seeded hashes and 1D gradient and fractal noise, pure functions of their arguments | `noise` |
| Assets | `asset-format` | `AssetId` and the `.gasset` framing | |
| | `asset-import` | the `Importer` trait | |
| | `asset-build` | `track_assets` for build scripts, which also writes the registry a game embeds | |
| | `import` | the import library behind the editor's Import… | |
| | `gltf`, `obj` | the glTF and OBJ importers | |
| Rendering | `render` | the wgpu renderer, cameras, lights, materials, the render world | `render` |
| | `mesh` | mesh data, mesh primitives, bounds, ray tests, `MeshComponent`, skeleton assets and `JointPose` | `mesh` |
| | `skybox` | `Sun` and `Sky`: a physically based sky that follows the sun, lights the scene and colors its fog | `skybox` |
| | `world-grid` | the ground grid | `world_grid` |
| | `particles` | particle emitters, simulated on the CPU and drawn as billboards | `particles` |
| | `debug-gizmos` | immediate-mode debug lines and shapes, and `ComponentGizmo` with the engine's component gizmos for bounds, skeletons, lights and cameras | add `concerto-debug-gizmos` |
| | `motion-matching-gizmos` | `MotionMatchingGizmos`, which draws a running character's motion matching, and the drawing the database preview shares | add `concerto-motion-matching-gizmos` |
| | `navigation-gizmos` | `NavigationGizmosPlugin`, which draws the nav graph's points and links, in the game and the editor's scene view, and each agent's route and each loaded navmesh's polygons in the game; and `NavMeshGizmo`, the navmesh's gizmo in the editor | add `concerto-navigation-gizmos` |
| | `physics-gizmos` | `PhysicsGizmosPlugin`, which draws colliders by body activity, each ragdoll's parts and joint limits, and contacts; and the component gizmos of `BoxCollider`, `MeshCollider` and `Sensor` | add `concerto-physics-gizmos` |
| | `procedural-animation-gizmos` | `FootLockGizmos`, which draws each foot lock of a running player's `FootLockNode` | add `concerto-procedural-animation-gizmos` |
| | `terminal-renderer` | a renderer that draws into a terminal | |
| Simulation | `physics` | bodies, colliders, ray casts, character movement, over Jolt or Rapier | `physics` |
| | `jolt-ffi` | the bindings to Jolt, built from the submodule | |
| | `animation` | clips, skeletons, animation graphs, the animation player, retarget rigs, pose retargeting, and `RetargetNode`, which plays an inner graph made for another skeleton on the player's | `animation` |
| | `ragdoll` | the `Ragdoll` asset, a skeleton's physical body of parts and joints, and its fitting to a skeleton into a physics ragdoll; the `PhysicsAmount` and `MotorCommand` pose attributes and the `HoldNode` and `LimpNode` graph nodes that drive a character's body from its animation graph | `ragdoll` |
| | `procedural-animation` | procedural animation graph nodes: `FootPlacementNode`, which reads physics to put a character's feet on the ground, `FootLockNode`, which holds a planted foot where it was planted so it does not slide, `LookAtNode`, which turns its spine, head and eyes toward a point, `AimNode`, which turns its spine so a weapon's barrel points at a target, and `ProceduralMotionNode`, which adds seeded sway, tremor and breathing rotations to masked bones | `procedural_animation` |
| | `navigation` | hand-placed nav points and their links, the `NavGraph` built from them with nearest-point and shortest-path queries, the `NavMesh` baked from colliders with its path queries, and `NavAgent`s that route on a loaded navmesh or else the graph, steer toward a `NavDestination` and report `NavArrived` or `NavUnreachable` | `navigation` |
| | `scene` | scenes: spawning and capturing | `scene` |
| | `director` | the virtual-camera stack | `director` |
| | `gameplay` | camera rigs and player helpers, and the copy of nav steering into `DesiredMotion` | `gameplay` |
| | `audio` | `AudioClip` and its Ogg and WAV importer, `AudioPlayer`, the `AudioMixer`'s buses, over kira | `audio` |
| Interface | `window` | the window, input and their events as plain data, the clipboard | `window` |
| | `winit` | the winit host behind `concerto::run` | |
| | `ui` | the UI: layout, text, theme, widgets | `ui` |
| Tooling | `editor` | the editor, as a library | add `concerto-editor` in the `editor/` crate |
| | `animation-editor` | the editor's tab for an animation clip | list it in the game's `Cargo.toml` |
| | `navigation-editor` | the editor's navmesh gizmo, drawn while a navmesh's entity is selected | list it in the game's `Cargo.toml` |
| | `physics-editor` | the editor's gizmo kinds for colliders, sensors, contacts and ragdolls | list it in the game's `Cargo.toml` |
| | `ragdoll-editor` | the `Ragdoll` tab (a part tree, the selected part's form, and a preview that drops, pokes and plays the body), its tools `ragdoll_parts`, `set_ragdoll_part`, `test_ragdoll` and `preview_ragdoll`, and the `create_ragdoll` tool and Create ragdoll button that generate a humanoid `Ragdoll` from a retarget rig | list it in the game's `Cargo.toml` |
| | `project` | `cargo concerto` | install with `cargo install --path crates/project` |
| | `mcp` | an MCP server over any `App`, whose tools are systems; the editor's `--mcp` uses it | |

The `concerto` crate itself, in `src/`, adds `DefaultPlugins`,
`concerto::run` and `concerto::headless`.

`ecs`, `foundation`, `render` and `reflect` each have a `macros` sub-crate for
their derives; the derives are re-exported with the crate, so a game never
names the macro crates.

Back to the [table of contents](README.md).

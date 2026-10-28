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
| Assets | `asset-format` | `AssetId` and the `.gasset` framing | |
| | `asset-import` | the `Importer` trait | |
| | `asset-build` | `track_assets` for build scripts, which also writes the registry a game embeds | |
| | `import` | the import library behind the editor's Import… | |
| | `gltf`, `obj` | the glTF and OBJ importers | |
| Rendering | `render` | the wgpu renderer, cameras, lights, materials, the render world | `render` |
| | `mesh` | mesh data, bounds, ray tests, `MeshComponent` | `mesh` |
| | `skybox` | the sky | `skybox` |
| | `world-grid` | the ground grid | `world_grid` |
| | `debug-gizmos` | immediate-mode debug lines and shapes | add `concerto-debug-gizmos` |
| | `terminal-renderer` | a renderer that draws into a terminal | |
| Simulation | `physics` | bodies, colliders, ray casts, character movement, over Jolt or Rapier | `physics` |
| | `jolt-ffi` | the bindings to Jolt, built from the submodule | |
| | `animation` | clips, skeletons, animation graphs, the animation player | `animation` |
| | `scene` | scenes: spawning and capturing | `scene` |
| | `director` | the virtual-camera stack | `director` |
| | `gameplay` | camera rigs and player helpers | `gameplay` |
| Interface | `window` | the window, input and their events as plain data, the clipboard | `window` |
| | `winit` | the winit host behind `concerto::run` | |
| | `ui` | the UI: layout, text, theme, widgets | `ui` |
| Tooling | `editor` | the editor, as a library | add `concerto-editor` in the `editor/` crate |
| | `project` | `cargo concerto` | install with `cargo install --path crates/project` |
| | `mcp` | an MCP server over any `App`, whose tools are systems; the editor's `--mcp` uses it | |

The `concerto` crate itself, in `src/`, adds `DefaultPlugins`,
`concerto::run` and `concerto::headless`.

`ecs`, `foundation`, `render` and `reflect` each have a `macros` sub-crate for
their derives; the derives are re-exported with the crate, so a game never
names the macro crates.

Back to the [table of contents](README.md).

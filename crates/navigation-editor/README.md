# concerto-navigation-editor

The navmesh gizmo in the editor's viewport, as an extension of
`concerto-editor`. Only editor crates depend on it. A game lists it in its own
`Cargo.toml`, and `cargo concerto editor <dir> --update` writes the call into
the game's generated `editor/`:

```toml
[package.metadata.concerto.editor]
plugins = [{ package = "concerto-navigation-editor", install = "concerto_navigation_editor::install" }]
```

`install` calls `add_scene_tools_plugin`, so every edit and play `App` the
editor hosts registers `NavMeshGizmo` from `concerto-navigation-gizmos` with
`register_component_gizmo`. The viewport then draws a `NavMeshComponent`'s
loaded navmesh, its polygon edges in amber, while its entity or one of its
ancestors is selected, or always with Navmesh set to Always. Nothing is drawn
for a navmesh that has not loaded.

The game's own `NavigationGizmos::mesh` layer is separate: it draws every
navmesh at game time, which includes Play in the editor, and never while
editing. The nav graph is drawn from the `NavGraph` resource rather than a
component, so it stays with `NavigationGizmosPlugin`, at edit time too.

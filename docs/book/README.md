# The Concerto Book

This book introduces Concerto one idea at a time, with small examples you can
paste into a game. It assumes you know Rust; it does not assume you have used
an entity component system before.

Concerto is a work in progress. When the book and the code disagree, the code
wins: please open an issue.

## Chapters

1. [Getting started](01-getting-started.md): install, run an example, make a
   game with `cargo concerto`, and draw your first frame.
2. [Apps and plugins](02-apps-and-plugins.md): the recipe, `DefaultPlugins`,
   schedules, and what happens in a frame.
3. [Entities, components and systems](03-ecs.md): spawning, queries,
   resources, commands and change detection.
4. [Events and signals](04-events-and-signals.md): telling other systems that
   something happened, and reacting to it.
5. [Assets and scenes](05-assets-and-scenes.md): importing, `asset_id!`,
   handles, and scenes made in the editor.
6. [Rendering](06-rendering.md): cameras, lights, meshes and materials.
7. [Physics, animation and gameplay](07-physics-animation-gameplay.md): rigid
   bodies, colliders, characters, animation graphs and cameras that follow.
8. [User interface](08-ui.md): nodes, the theme, widgets and listeners.
9. [The editor](09-editor.md): how your game and the editor share one recipe.
10. [Crate map](10-crate-map.md): where everything lives.

## Conventions

Code imports items through the `concerto` crate, which re-exports most engine
crates under a short name: `concerto::ecs`, `concerto::render`,
`concerto::ui`, and so on. Examples also use `glam` for vectors and
quaternions; add it to your `Cargo.toml` when you need it.

Deeper references live next to the book:
[scheduling](../scheduling.md), [asset importing](../asset-importing.md),
[profiling](../profiling.md) and the
[editor's internals](../../crates/editor/README.md).

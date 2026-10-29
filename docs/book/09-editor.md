# 9. The editor

## Your game, inside the editor

The editor is not a separate program that loads your game. It is a library
compiled together with your game's library, by a small crate in your project,
`editor/`:

```rust
fn main() {
    concerto_editor::run(my_game::game, concerto_editor::editor_crate!());
}
```

`cargo concerto new` writes it, and `cargo concerto editor <dir>` adds it to
an existing game. Because the editor holds your recipe, every scene it opens is
built from your recipe: your components appear in the inspector, and your
edit-time systems run in the viewport. A game crate never depends on the
editor; the editor depends on the game.

```sh
cargo run -p my-game-editor
cargo run -p my-game-editor -- --project <another project directory>
```

`--project` opens another project's `content/` tree; `--decorated` uses the
window manager's title bar; `--mcp` runs the editor with no window and serves
it to an AI agent (below).

## What you can do

- **Import** glTF, OBJ and image files with **Import…** in the asset
  browser, the panel titled **Asset Browser**. Each becomes `.gasset` files in
  `content/` (chapter 5).
- **Open** a scene by double-clicking it in the asset browser.
- **Arrange** the scene: the hierarchy, titled **Warren**, adds, duplicates,
  renames (F2) and deletes entities; the viewport selects by clicking and
  moves with the gizmo; the inspector, titled **Looking Glass**, adds and
  edits components.
- **Save** with Ctrl+S. The saved scene belongs to the editor from then on:
  re-importing its source leaves it alone.
- **Play** with the button on the Scene tab, or Ctrl+P. The game is built from
  your recipe and starts in the open scene. Stop returns to the scene exactly
  as it was; changes made while playing are not kept.

## Three kinds of App

While the editor runs, three kinds of `App` exist:

- **The editor's own `App`** draws the window, the panels and the documents.
- **An edit `App`** holds each open scene. It is built from your recipe with
  `App::for_editing()`, and draws the viewport.
- **A play `App`** exists while you play. It is built from your recipe like a
  normal game and starts in the open scene.

`app.is_editing()` tells a recipe or a plugin which kind it is in.

## Systems at edit time

In an edit `App`, only main-world systems marked to run at edit time run; the
rest keep their place and do nothing. Gameplay should not happen while you
arrange a level, so that is the default. Mark the systems the scene needs in
order to look right, such as something that builds a mesh from a component:

```rust
use concerto::app::edit_time::AtEditTime;

app.add_system(Update, build_fence_mesh.at_edit_time());
app.add_listener(on_fence_added.at_edit_time());
```

In an edit `App`, `App::add_listener` drops an unmarked listener. Render-world
systems always run. The edit `App` has no physics simulation, so an edit-time
system must not need it. [docs/scheduling.md](../scheduling.md#edit-time) has
the exact rules.

## Components the editor can edit

A component appears in the inspector, and is saved in scenes, once it derives
`Reflect` and is registered, as in chapter 5:

```rust
#[derive(Component, Reflect, Default)]
#[reflect(Component, Default, SceneComponent)]
pub struct Door {
    open: bool,
    speed: f32,
}

app.register_type::<Door>();
```

`bool`, `f32`, `f64`, `Vec3`, `Quat` and asset handles get editors out of the
box. A field whose type has no editor shows its own fields when it is a
reflected struct, and otherwise shows as unsupported until a property editor
is registered for it. A type that needs its own widget gets a `PropertyEditor`;
see
[Custom property editors](../../crates/editor/README.md#custom-property-editors)
in the editor's README, and `cargo run -p concerto-editor --example custom_property`.

## Agents

`--mcp` runs the whole editor with no window and serves it over
[MCP](https://modelcontextprotocol.io) on stdin and stdout, so an AI agent can
open scenes, look at them, edit them, save, and play the game. The repository's
`.mcp.json` registers `tech-demo-editor -- --mcp` with Claude Code; build the
editor first, since a client gives up if starting it takes too long:

```sh
cargo build -p my-game-editor
cargo run -p my-game-editor -- --mcp --project <a copy of the project>
```

The tools, their arguments and their errors are listed under
[Agents (MCP)](../../crates/editor/README.md#agents-mcp) in the editor's README.

## More

[crates/editor/README.md](../../crates/editor/README.md) documents every panel,
key, and the editor's internals.

Next: [Crate map](10-crate-map.md).

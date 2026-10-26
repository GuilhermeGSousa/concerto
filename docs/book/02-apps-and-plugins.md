# 2. Apps and plugins

## The recipe

A Concerto game is a function:

```rust
pub fn game(app: &mut App) {
    app.register_plugin(DefaultPlugins)
        .add_system(Startup, setup)
        .add_system(Update, orbit);
}
```

The recipe registers everything the game is made of: plugins, systems,
resources, events, component types, and the scene it starts in. It never runs
anything itself. Whoever calls it, the **host**, decides when frames happen:

- `concerto::run(game)` opens a window and runs the game.
- The editor calls `game` for every scene it opens, and again when you press
  Play. The scenes you edit therefore run your own components and systems.
- `concerto::headless(game)` builds an `App` with a GPU and no window, which is
  handy in tests. It exists only on native targets and returns `None` when
  there is no GPU.

Keep the recipe the only place that sets the game up. Code that runs before
`concerto::run`, in `main`, is invisible to the editor.

## Plugins

A plugin is a reusable piece of recipe:

```rust
use concerto::app::{App, Plugin, schedule_groups::Update};

pub struct HealthPlugin;

impl Plugin for HealthPlugin {
    fn build(&self, app: &mut App) {
        app.add_system(Update, regenerate);
    }
}
```

Register it with `app.register_plugin(HealthPlugin)`. A plugin may also
implement `ready`, for set-up that finishes asynchronously, and `finish`,
which runs once every plugin is ready.

`DefaultPlugins` registers the engine's standard plugins in a fixed order:
schedules, assets, time, window, transforms, the camera director, rendering,
the skybox, shadows, the standard material, physics, animation, scenes, the
world grid, gameplay helpers and the UI. Most games start with it. Plugin order
is also the order systems were registered in, which decides how systems with
no explicit ordering run.

## Schedules

Systems are added to a schedule. The ones a game uses:

| Schedule | When |
|---|---|
| `Startup` | Once, after every plugin is ready |
| `First` | At the start of each frame; the engine swaps event buffers here |
| `FixedUpdate`, `LateFixedUpdate` | Zero or more times a frame, at a fixed rate; physics steps in `LateFixedUpdate` |
| `Update` | Once a frame; most game logic goes here |
| `LateUpdate` | After `Update`; transform propagation and UI layout run here |

They live in `concerto::app::schedule_groups`. Within a schedule, systems run
in the order you constrain them to, and otherwise in registration order:

```rust
use concerto::ecs::IntoSystemConfig;

app.add_system(Update, (read_input, move_player, follow_camera).chain());
app.add_system(Update, play_footsteps.after(move_player));
```

[docs/scheduling.md](../scheduling.md) covers ordering, sets and tags in full.

## Two worlds

One `App` holds two worlds. The **main world** is where the game lives: your
entities, components, resources and systems. The **render world** holds what
the renderer needs to draw a frame. Each frame:

1. The main world runs `First`, the fixed steps, `Update` and `LateUpdate`.
2. **Extract** copies what changed from the main world into the render world.
   Only entities with `SyncWithRenderWorld` get a mirror there. The engine adds
   it to the live camera, to UI nodes, and to the mesh and light nodes of an
   imported scene; add it yourself to lights, meshes and grids you spawn in
   code.
3. The render world runs `Render` and `LateRender`, and draws.

Game code almost never touches the render world. If you write a renderer
feature, add its systems with `app.add_render_system(...)`: `add_system` with
`Render` would put them in the main world, where that schedule never runs.

## Resources

A resource is a single value of a type, owned by the world rather than by an
entity: the clock, the input state, a game's score.

```rust
use concerto::ecs::Resource;

#[derive(Resource, Default)]
struct Score(u32);

app.insert_resource(Score::default());
```

Systems read it with `Res<Score>` and change it with `ResMut<Score>`; see the
next chapter.

## The startup scene

A game that starts in a scene made in the editor says so in its recipe:

```rust
app.set_startup_scene(asset_id!("content/forest/scene.gasset"));
```

The scene plugin spawns it when the game starts. The editor does not spawn it
in the scenes it edits. Chapter 5 explains `asset_id!`.

Next: [Entities, components and systems](03-ecs.md).

# 1. Getting started

## Install

Follow the prerequisites in the [README](../../README.md#prerequisites): Rust,
this repository with its Jolt submodule, the Linux system packages, and, for
the browser, the wasm target and Trunk. Then check that an example runs:

```sh
cargo run -p tech-demo
```

The first build compiles Jolt, a C++ physics library, and takes a few minutes.

## Make a game

Install the project tool from the checkout and create a game next to it:

```sh
cargo install --path crates/project
cargo concerto new ../hello --engine .
cd ../hello
cargo run
```

A window opens with nothing in it. `cargo concerto new` wrote:

```
hello/
  Cargo.toml          the game package, a library plus a binary
  build.rs            tracks content/ and copies it next to the binary
  src/lib.rs          the recipe: pub fn game(app: &mut App)
  src/main.rs         concerto::run(hello::game)
  content/            imported assets; empty for now
  index.html          for `trunk serve`
  Trunk.toml
  editor/             the hello-editor crate: the editor, built with your game
```

The heart of it is `src/lib.rs`:

```rust
use concerto::{DefaultPlugins, app::App};

pub fn game(app: &mut App) {
    app.register_plugin(DefaultPlugins);
}
```

`game` is the **recipe**: one function that says what the game is made of.
`src/main.rs` hands it to `concerto::run`, which opens a window and runs it;
the editor calls the same function to build the worlds it edits. Chapter 2
explains why that matters.

## Draw something

Add `glam` to `Cargo.toml`:

```toml
[dependencies]
glam = "0.30"
```

Then give the game a camera, a light and a grid, and make the camera circle
the origin:

```rust
use concerto::{
    DefaultPlugins,
    app::{
        App,
        schedule_groups::{Startup, Update},
    },
    color::Color,
    director::VirtualCamera,
    ecs::{CommandQueue, Component, Query, Res, With},
    foundation::{time::Time, transform::Transform},
    render::components::{
        light::{Light, LightType},
        render_entity::SyncWithRenderWorld,
    },
    world_grid::WorldGrid,
};
use glam::{Quat, Vec3};

#[derive(Component)]
struct Orbit {
    angle: f32,
}

pub fn game(app: &mut App) {
    app.register_plugin(DefaultPlugins)
        .add_system(Startup, setup)
        .add_system(Update, orbit);
}

fn setup(mut cmd: CommandQueue) {
    cmd.spawn((
        VirtualCamera::default(),
        Transform::from_translation(Vec3::new(0.0, 4.0, 12.0)),
        Orbit { angle: 0.0 },
    ));
    cmd.spawn((
        Light {
            color: Color::rgba(1.0, 1.0, 1.0, 1.0),
            intensity: 100.0,
            light_type: LightType::Point,
            range: 0.0,
            shadowmaps_enabled: false,
        },
        Transform::from_translation(Vec3::new(0.0, 8.0, 4.0)),
        SyncWithRenderWorld,
    ));
    cmd.spawn((WorldGrid::default(), SyncWithRenderWorld));
}

fn orbit(time: Res<Time>, cameras: Query<(&mut Orbit, &mut Transform), With<VirtualCamera>>) {
    for (mut orbit, mut transform) in cameras.iter() {
        orbit.angle += 0.3 * time.delta().as_secs_f32();
        let position = Vec3::new(orbit.angle.sin() * 12.0, 4.0, orbit.angle.cos() * 12.0);
        let look = Quat::from_rotation_y(orbit.angle) * Quat::from_rotation_x(-0.3);
        transform.translation = position;
        transform.rotation = look;
    }
}
```

`cargo run` again: the grid turns slowly under the camera.

What happened:

- `setup` runs once, in the `Startup` schedule. It **spawns** three entities
  through a `CommandQueue`, each with a tuple of **components**.
- `orbit` runs every frame, in `Update`. Its `Query` finds every entity that
  has both `Orbit` and `Transform` and also a `VirtualCamera`, and changes
  them in place. `Res<Time>` reads the engine's clock.
- `VirtualCamera` is a camera the camera director can make live; with only one,
  it is the one you see. The light and the grid carry `SyncWithRenderWorld`,
  which tells the engine to copy them to the renderer. Chapter 6 covers
  cameras, lights and that marker.

## Run it in the browser

```sh
trunk serve
```

Then open the address Trunk prints. The web build uses WebGL2 and the Rapier
physics backend instead of Jolt; the same code runs on both.

## Open the editor

```sh
cargo run -p hello-editor
```

The editor opens the project's `content/` tree, which is empty. Importing a
glTF or OBJ file with **Import…** produces a scene you can open, edit and
play; see [chapter 5](05-assets-and-scenes.md) and [chapter 9](09-editor.md).

Next: [Apps and plugins](02-apps-and-plugins.md).

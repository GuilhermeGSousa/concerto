# Concerto

A Rust game engine for character-driven, cinematic games. It has its own
entity component system, a wgpu renderer, an offline asset pipeline, physics
and animation, an engine-native UI, and an editor built on that same UI.

> **Status: work in progress.** APIs change from one week to the next and
> nothing is stable yet. `concerto` 0.1.0 on crates.io predates most of what
> this README describes: build against this repository instead.

## Features

- **ECS.** Components, resources, queries with change detection, commands,
  events, and signals with global and per-entity listeners. Systems are plain
  functions, ordered by explicit constraints and sets.
- **Rendering.** wgpu, with a physically based `StandardMaterial`, shadow
  maps, a skybox, skinned meshes, a world grid and debug gizmos. Runs natively
  and in the browser on WebGL2.
- **Assets.** glTF, OBJ and images are imported ahead of time into `.gasset`
  files. Code names an asset by its path with `asset_id!`, which resolves it to
  a stable id at compile time.
- **Scenes.** Reflection-based scenes that the editor saves and the game
  spawns, with asset and entity references.
- **Physics.** One API over two backends: Jolt natively, Rapier on the web.
  Rigid bodies, colliders, ray casts, and a character controller with ground
  detection.
- **Animation.** Skeletal animation driven by animation graphs: clips, blends,
  2D blend spaces and state machines.
- **Cameras.** A virtual-camera stack, with follow and pivot helpers.
- **UI.** A flexbox UI (on Taffy) with a theme and builders for buttons, text
  fields, checkboxes, sliders, lists, trees, popups and scroll areas.
- **Editor.** A scene editor compiled with your game's code: hierarchy,
  inspector, viewport with picking and gizmos, asset import, and Play inside
  the editor.

## Getting started

### Prerequisites

- Rust, stable, from <https://rustup.rs>, and a C++ compiler: Jolt, the
  native physics library, is built from source.
- [Git LFS](https://git-lfs.com), before you clone. The examples' `.gasset`
  files are stored in it, and a game built from LFS pointers fails with
  `missing the GRDY magic prefix`. In a clone made without it, run `git lfs pull`.
- This repository with its Jolt submodule. Without the submodule every native
  build fails:

  ```sh
  git lfs install
  git clone https://github.com/GuilhermeGSousa/concerto.git
  cd concerto
  git submodule update --init
  ```

- On Linux, these system packages:

  ```sh
  sudo apt install libasound2-dev libudev-dev libwayland-dev
  ```

- For the browser, the wasm target and [Trunk](https://trunkrs.dev):

  ```sh
  rustup target add wasm32-unknown-unknown
  cargo install trunk
  ```

### Run an example

```sh
cargo run -p tech-demo
```

| Example | What it shows |
|---|---|
| `tech-demo` | A forest with a playable character, a follow camera and animation; the one example with an editor |
| `ui-showcase` | Every UI element on one screen |

The first build compiles Jolt and takes a while. In the browser:

```sh
cd examples/tech-demo
trunk serve
```

The editor, over a copy of the tech demo, so nothing you save lands in the
repository:

```sh
cp -r examples/tech-demo /tmp/td
cargo run -p tech-demo-editor -- --project /tmp/td
```

### Start your own game

The project tool creates a game crate and its editor:

```sh
cargo install --path crates/project
cargo concerto new ../my-game --engine .
cd ../my-game
cargo run
cargo run -p my-game-editor
```

`--engine` points the new project at this checkout. Without it the project
depends on the crates.io release, which is older than this repository. A
project created outside the checkout is its own workspace; one created inside
it is added to the engine's workspace.
[crates/project/README.md](crates/project/README.md) has the details.

## Documentation

- **[The Concerto Book](docs/book/README.md)**: how the engine works, chapter
  by chapter, with small examples. Start here.
- [docs/scheduling.md](docs/scheduling.md): ordering systems, sets and tags.
- [docs/asset-importing.md](docs/asset-importing.md): the import pipeline and
  `.gasset` files in depth.
- [docs/profiling.md](docs/profiling.md): measuring performance.
- [crates/editor/README.md](crates/editor/README.md): the editor's internals.

## Repository layout

```
crates/      the engine, one crate per directory, each named concerto-<dir>
examples/    standalone games
src/         the `concerto` crate: re-exports, DefaultPlugins and concerto::run
docs/        user documentation and the book
```

## License

[PolyForm Noncommercial 1.0.0](LICENSE). You may use, change and share
Concerto for any noncommercial purpose. Commercial use needs a separate license;
open an issue or contact the author to ask for one.

Versions released before the license change, including `concerto` 0.1.0 on
crates.io, remain available under the MIT license they were published with.

The motion capture data in `examples/tech-demo/assets/lafan1` is Ubisoft's and
has [its own license](examples/tech-demo/assets/lafan1/README.md).

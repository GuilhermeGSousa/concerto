# 5. Assets and scenes

## Import first, load later

The running game never opens a `.glb`, `.obj` or `.png`. Source files are
**imported** once, ahead of time, into `.gasset` files under the project's
`content/` directory, and the game loads those.

```
assets/hero.glb  ──import──▶  content/hero/scene.gasset
                              content/hero/mesh_0.gasset
                              content/hero/material_0.gasset
                              content/hero/skeleton_0.gasset
                              content/hero/animation_0.gasset
                              content/.registry.toml
assets/hero.glb.import.toml
```

Import from the directory that owns `content/`:

```sh
cargo run -p concerto-import -- assets/hero.glb
```

or with **Import…** in the editor. A glTF yields meshes, materials, textures,
skeletons, animations and one scene; an OBJ yields meshes, a material and a
scene; an image yields a texture. Commit `content/` and every
`<source>.import.toml`: the import file pins each sub-asset's id, so importing
again after the artist changes the source keeps every reference intact.

## Naming an asset in code

Code names an asset by its id, which `asset_id!` reads from the file's header
while compiling:

```rust
use concerto::{asset_id, foundation::assets::AssetId};

const HERO: AssetId = asset_id!("content/hero/scene.gasset");
```

A missing or broken file is a compile error at that line. The path is relative
to the package. For the macro to see content changes, the game's `build.rs`
calls `concerto_asset_build::track_assets("content")`; projects made by
`cargo concerto new` already do.

## Loading

The `AssetServer` resource loads by id and returns a handle at once; the bytes
load in the background:

```rust
use concerto::{
    ecs::{CommandQueue, Res},
    foundation::assets::{asset_server::AssetServer, handle::AssetHandle},
    scene::{scene::Scene, spawner::SceneSpawnerComponent},
};

fn spawn_hero(mut cmd: CommandQueue, assets: Res<AssetServer>) {
    let scene: AssetHandle<Scene> = assets.load::<Scene>(HERO);
    cmd.spawn((SceneSpawnerComponent(scene), Transform::default()));
}
```

`SceneSpawnerComponent` spawns the scene's entities under its entity once the
scene has loaded. Other handles go in their own components: a mesh in
`MeshComponent`, a material in `MaterialComponent`, an animation graph in
`AnimationHandleComponent`.

Assets made in code are added rather than loaded:

```rust
let material = assets.add(StandardMaterial::new(None, None));
```

## Scenes

A scene is a tree of entities saved to a `.gasset`. Importing a model makes
one; the editor opens it, lets you add entities and components, and saves it.

A game starts in one by naming it in the recipe:

```rust
app.set_startup_scene(asset_id!("content/forest/scene.gasset"));
```

### Your components in scenes

A scene stores components by name, through reflection. For one of your
components to be placed in a scene, edited in the inspector and saved, derive
`Reflect` and `Default` and register it:

```rust
use concerto::{
    ecs::{
        Component,
        reflect::{ReflectComponent, ReflectSceneComponent},
    },
    reflect::{Reflect, ReflectDefault},
};

#[derive(Component, Reflect, Default)]
#[reflect(Component, Default, SceneComponent)]
pub struct PlayerSpawner {
    should_spawn: bool,
}

app.register_type::<PlayerSpawner>();
```

The tech demo works this way: its forest scene contains an entity with a
`PlayerSpawner`, placed in the editor, and a system spawns the character where
it stands. Every type a scene component contains must implement `Default`.
Asset handles are saved as their ids, and entity references as positions in
the scene's tree.

## More

[docs/asset-importing.md](../asset-importing.md) covers the registry, the
`.gasset` format, import flags, how the runtime finds files, and writing an
importer of your own.

Next: [Rendering](06-rendering.md).

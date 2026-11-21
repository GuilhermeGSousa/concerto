# 5. Assets and scenes

## Import first, load later

The running game never opens a `.glb`, `.obj` or `.png`. Source files are
**imported** once, ahead of time, into `.gasset` files under the project's
`content/` directory, and the game loads those.

```
assets/hero.glb  ──import──▶  content/hero/scene.gasset
                                the scene, with the meshes, materials
                                and skeletons embedded in it
                              content/hero/texture_0.gasset
                              content/hero/animation_Idle_Loop.gasset
assets/hero.glb.import.toml
```

Import with **Import…** in the editor. There is no import command line.

A glTF yields meshes, materials, textures, skeletons, animations and one
scene (each animation is named after its glTF clip, a BVH one after its file); an OBJ yields meshes, a material and a scene; an image yields a
texture. A skeleton asset holds bone names, parent indices (parents first), inverse bind poses, the rest pose and the bone ids. A glTF skin whose joints are not ordered parents-first is rejected. Commit `content/` and every
`<source>.import.toml`: the import file pins each sub-asset's id, so importing
again after the artist changes the source keeps every reference intact.

A source that yields a scene writes one `scene.gasset`, and its meshes,
materials and skeletons are stored inside that file. Each is still an asset
with its own id, named by the file, `#`, and its name in the source:
`content/hero/scene.gasset#mesh/0`. Textures, animations and the outputs of a
source with no scene are a file each. The scene file belongs to the import and
is read-only; to build on it, place it in a scene of your own. A project
imported before this, with a file per mesh, is converted by
`cargo run -p concerto-import --bin pack-imports -- <project directory>`.

## Naming an asset in code

Code names an asset by its id, which `asset_id!` reads from the file's header
while compiling:

```rust
use concerto::{asset_id, foundation::assets::AssetId};

const HERO: AssetId = asset_id!("content/hero/scene.gasset");
const HERO_SKELETON: AssetId = asset_id!("content/hero/scene.gasset#skeleton/0");
```

A missing file gives a nil id and a warning at that line; a broken file, or a
name after `#` that the file does not hold, is a compile error there. The path is relative
to the package. For the macro to see content changes, the game's `build.rs`
calls `concerto_asset_build::track_assets("content")`; projects made by
`cargo concerto new` already do.

The same call scans `content/` and writes the asset registry, which maps each
id to its file. `concerto::content_registry!()` embeds it in the game with
the project's directory, and `main` passes both to `concerto::run`. No registry file is committed, so moving
or adding a `.gasset` in a file explorer or through git is picked up at the
next build.

## Loading

The `AssetServer` resource loads by id and returns a handle at once; the bytes
load in the background:

```rust
use concerto::{
    ecs::{CommandQueue, Res},
    foundation::assets::{asset_server::AssetServer, handle::AssetHandle},
    scene::{link::SceneLink, scene::Scene},
};

fn spawn_hero(mut cmd: CommandQueue, assets: Res<AssetServer>) {
    let scene: AssetHandle<Scene> = assets.load::<Scene>(HERO);
    cmd.spawn(SceneLink::new(scene));
}
```

`SceneLink` spawns the scene's entities under its entity once the scene has
loaded, tags each with `LinkedNode`, and spawns them again whenever the scene
loads again; `SceneLinkState` says whether it is `Waiting`, `Spawned` or
`Refused`, and `SceneLinkSpawned` is written each time it spawns. A scene node
that is itself a link spawns as a `SceneLink` of its own. Other handles go in
their own components: a mesh in
`MeshComponent`, a material in `MaterialComponent`, an animation graph in
`AnimationHandleComponent`.
A mesh with no material, or with a `MaterialComponent` whose handle
is nil, draws plain grey.

Assets made in code are added rather than loaded:

```rust
let material = assets.add(StandardMaterial::new(None, None));
```

When a load or an `assets.add` lands an asset in its `AssetStore<A>`, an
`AssetLoaded<A>` event carries its id, and `Res<AssetStore<A>>::is_changed()`
reads true. The store reads as changed again only when an asset of that type
lands, is freed after its last handle drops, or is edited through
`ResMut<AssetStore<A>>`.

```rust
use concerto::{
    ecs::events::event_reader::EventReader,
    foundation::assets::handle::AssetLoaded,
    render::assets::mesh::Mesh,
};
use log::info;

fn on_mesh_loaded(mut loaded: EventReader<AssetLoaded<Mesh>>) {
    for event in loaded.read() {
        info!("mesh {:?} is ready", event.id);
    }
}
```

A load that fails, because the id is not in the registry, the file is missing
or corrupt, or `on_load` returns an error, sends `AssetLoadFailed` with the id
that was asked for. The event is the same for every asset type, and the reason
is in the log. An `assets.add` whose `on_load` fails only logs.

## Scenes

A scene is a tree of entities saved to a `.gasset`. Importing a model makes
one; the editor opens it, lets you add entities and components, and saves it.

A game starts in one by naming it in the recipe:

```rust
app.set_startup_scene(asset_id!("content/sanatorium/level.gasset"));
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

A game uses it this way: a scene holds an entity with a `PlayerSpawner`,
placed in the editor at the scene's root, and a system spawns the character
and its capsule where it stands. The system reads the spawner's
local `Transform`, so a spawner under a scaled parent would put the character
somewhere else. The component itself must implement `Default`: spawning builds
it with `Default::default()` and then sets the saved fields. So must anything the
reflection has to build from nothing: a type held in an `Option` or a `Vec`,
and the fields of an enum's variants. Asset handles are saved as their ids,
and entity references as positions in the scene's tree.

### Making an asset placeable

An asset kind that can stand in a scene as an entity says how, by implementing
`PlaceableAsset`. `place` is handed an entity that already exists, named,
positioned and parented, and adds to it what shows or uses the asset:
components on the entity, children under it.

```rust
use concerto::{
    ecs::{CommandQueue, Entity},
    foundation::assets::{handle::AssetHandle, PlaceableAsset},
};

impl PlaceableAsset for Prop {
    fn place(asset: &Self, handle: AssetHandle<Self>, entity: Entity, cmd: &mut CommandQueue) {
        cmd.insert(PropComponent { handle }, entity);
    }
}

app.register_placeable_asset::<Prop>();
```

`register_placeable_asset` registers the asset as `register_asset` does and
records the kind in the `PlaceableAssets` resource. A record is for code that
knows the kind only by its name: `request_load` starts the load and keeps the
asset alive, `is_loaded` says when it has landed, and `fill` runs `place` on an
entity and lets the asset go, so it lives on only through the handles `place`
stored. A load that fails lets it go too, and `release` lets it go when the
caller gives up on a load it asked for. Register the kind in the game's
recipe, with its other assets, so the editor has it.

The engine places one kind itself: `ScenePlugin` registers `Scene`, whose
root nodes become children of the entity. A `Mesh` is not placeable for now;
it reaches a scene inside a `Scene` asset or through a `MeshComponent` added
to an entity.

## More

[docs/asset-importing.md](../asset-importing.md) covers the registry, the
`.gasset` format, `content.toml`, how the runtime finds files, and writing an
importer of your own.

Next: [Rendering](06-rendering.md).

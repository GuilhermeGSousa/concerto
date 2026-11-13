# 6. Rendering

The renderer draws, each frame, what the main world says is there: a camera,
lights, and entities with a mesh and a material. You describe the scene with
components; you do not call draw functions.

## Getting to the renderer

The renderer reads a copy of the scene kept in the render world (chapter 2).
An entity gets a copy only if it has `SyncWithRenderWorld`. Cameras, lights,
meshes, materials, UI nodes and the world grid require it, so spawning one adds
it and you never write it by hand:

```rust
cmd.spawn((light, transform));
```

An entity with none of these components has no copy. See
[required components](../required-components.md).

Removing a component such as `Light` from a living entity does not remove its
copy, so the light keeps shining. Despawn the entity instead. Meshes are the
exception: removing `MeshComponent` stops the mesh drawing.

## Cameras

Spawn a `VirtualCamera` with a `Transform`. The **camera director** keeps a
stack of virtual cameras and drives the main camera from the live one, the
highest priority that is enabled:

```rust
use concerto::director::VirtualCamera;

cmd.spawn((
    VirtualCamera::default(),
    Transform::from_translation_rotation(Vec3::new(0.0, 5.0, 14.0), Quat::from_rotation_x(-0.25)),
));
cmd.spawn((VirtualCamera::new(10), Transform::default()));
```

`VirtualCamera::new(priority)` sets its priority; with several, the director
switches to the highest. A camera looks down its local −Z axis.

A game that moves its own cameras inserts a director built without its main
camera, `app.insert_resource(CameraDirector::default().with_main_camera(false))`,
from its plugin's `build`. `CameraDirectorPlugin` keeps a director that is already
there, so this works before or after `DefaultPlugins`.

### Split screen

A `Camera` draws to the whole of its target unless it has a `viewport`: a rect
given as `min` and `max` in fractions of the target's size, from its top-left
corner. It follows a window resize with no game code. Two players side by side:

```rust
use concerto::director::CameraDirector;
use concerto::render::components::camera::{Camera, Viewport};

app.insert_resource(CameraDirector::default().with_main_camera(false));

cmd.spawn((
    Camera::default().with_viewport(Viewport::new(Vec2::ZERO, Vec2::new(0.5, 1.0))),
    Transform::from_translation(Vec3::new(-2.0, 2.0, 8.0)),
));
cmd.spawn((
    Camera::default().with_viewport(Viewport::new(Vec2::new(0.5, 0.0), Vec2::ONE)),
    Transform::from_translation(Vec3::new(2.0, 2.0, 8.0)),
));
```

- A window camera's aspect follows its viewport, so each half stays
  undistorted.
- Each camera clears only its own rect, to its `clear_color`. The window
  outside every viewport is black.
- Window cameras must not overlap: where two do, which one shows is undefined.
  A camera without a viewport covers the whole window and overlaps every other
  one, so split screen either builds the director without its main camera or
  gives that camera a viewport too. The renderer warns once when it sees both kinds.
- A camera that renders to a texture keeps the `aspect` you give it, with a
  viewport or without one; setting it is yours.
- Culling is per camera, and shadow maps are per light and shared by every
  camera.
- Without its main camera, the director drives no camera: virtual cameras still
  stack, but nothing follows them. Move your cameras' `Transform`s yourself,
  from your own controllers.

### Layers

A camera draws a mesh when the two share a layer. `RenderLayers` goes on
either: `RenderLayers::layer(1)` is layer 1 alone, and `with(n)` and
`without(n)` add and drop one. There are 32, numbered from 0, and an entity or
camera without the component is on layer 0. It is not inherited: a mesh under
a parent with layers is still on layer 0 unless it carries its own.

```rust
use concerto::render::components::visibility::RenderLayers;

cmd.spawn((body_mesh, body_material, RenderLayers::layer(1)));
cmd.spawn((Camera::default(), RenderLayers::DEFAULT));
cmd.spawn((Camera::default(), RenderLayers::DEFAULT.with(1)));
```

The first camera never draws the body and the second does, which is how a
first-person player's own body is hidden from their view and shown to
everyone else. Layers decide only what a camera draws: a mesh still casts
shadows whatever its layers.

## Lights

```rust
use concerto::render::components::light::{Light, LightType};

cmd.spawn((
    Light {
        color: Color::rgba(1.0, 0.95, 0.9, 1.0),
        intensity: 1.0,
        light_type: LightType::Directional,
        range: 0.0,
        shadowmaps_enabled: true,
    },
    Transform::from_translation_rotation(Vec3::new(0.0, 8.0, 4.0), Quat::from_rotation_x(-0.8)),
));
```

`LightType` is `Point`, `Spot { cone_angle }` or `Directional`. A directional
light shines along its transform's −Z. `shadowmaps_enabled` makes it cast
shadows; it and `light_type` can be changed at any time and take effect on the
next frame. Up to 128 spot and directional lights cast shadows at once.

A point light casts into a cube map that reaches as far as its `range`, or 20
metres when the range is 0. Up to 42 point lights cast shadows at once, six
layers each of the 256 texture array layers the renderer asks the GPU for; the
others, the later ones in entity order, light without shadows. A browser
without cube map arrays casts from one point light.

## Meshes and materials

An imported model brings its meshes and materials with it. To build one in
code, fill a `Mesh` with vertices and triangle indices and add it to the asset
server:

```rust
use concerto::{
    mesh::MeshComponent,
    render::{
        MaterialComponent,
        assets::{material::StandardMaterial, mesh::Mesh, vertex::Vertex},
    },
};

fn spawn_triangle(mut cmd: CommandQueue, assets: Res<AssetServer>) {
    let normal = [0.0, 0.0, 1.0];
    let mut mesh = Mesh {
        vertices: vec![
            Vertex { pos_coords: [-1.0, 0.0, 0.0], normal, ..Vertex::default() },
            Vertex { pos_coords: [1.0, 0.0, 0.0], normal, ..Vertex::default() },
            Vertex { pos_coords: [0.0, 1.5, 0.0], normal, ..Vertex::default() },
        ],
        indices: vec![0, 1, 2],
    };
    mesh.compute_tangents();

    let material = StandardMaterial::new(None, None)
        .with_base_color_factor(Color::rgba(0.9, 0.3, 0.2, 1.0));

    cmd.spawn((
        MeshComponent { handle: assets.add(mesh) },
        MaterialComponent { handle: assets.add(material) },
        Transform::default(),
    ));
}
```

`StandardMaterial` is a physically based material. `new` takes an optional base
color texture and normal map; the `with_*` methods add a color factor,
metallic-roughness, emissive and occlusion textures, an alpha cutoff and a UV
scale. Share one mesh or material handle between many entities by cloning the
handle.

`concerto::mesh::primitives` builds the common shapes, with normals, tangents
and texture coordinates: `plane` on the XZ plane facing up, `cuboid`,
`uv_sphere`, and `capsule` along Y. Each is centred on the origin, and sizes
must be positive. A capsule's `half_height` is half the length of its cylinder,
so its full height is `2.0 * (half_height + radius)`, and 0 or less gives a
sphere. That is the same measure as
`Collider::capsule`, but the arguments are the other way round: the mesh takes
`radius` first. `cuboid` takes the full size, where `Collider::cuboid` takes
half extents.

```rust
use concerto::mesh::primitives::capsule;

let mesh = assets.add(capsule(0.3, 0.6, 8, 16));
```

## Batching and culling

Entities that share a mesh and a material are drawn together, in one draw call
per camera. Each camera draws only the meshes whose bounds, their `Aabb`, lie
at least partly inside its view, and each shadow-casting light draws into its
shadow map, or each face of its cube map, only the meshes inside that view. A mesh
gets its `Aabb` from its vertices a frame or so after the mesh loads, and is
drawn everywhere until then.

Two components from `concerto::render::components` opt a mesh out:

- `NoFrustumCulling` draws it in every view, whatever its bounds;
- `NotShadowCaster` keeps it out of every shadow map.

Skinned meshes are never culled, and each one takes its own draw call. A mesh
whose shader ignores its vertices or its model matrix, such as a skybox built
from `SkyboxCube`, needs both components, or it is culled whenever its
vertices' box is out of view. The world grid requires both already.

## The world grid and the sky

`WorldGrid::default()`, from `concerto::world_grid`, draws an infinite ground
grid that fades with distance; its fields set the cell size, colors and fade.
The skybox is drawn by `SkyboxPlugin`, part of `DefaultPlugins`.

## Debug drawing

`concerto-debug-gizmos` draws lines and shapes for one frame. It is not
re-exported by `concerto`, so add it to your `Cargo.toml`, with the path to
your Concerto checkout:

```toml
concerto-debug-gizmos = { path = "<checkout>/crates/debug-gizmos" }
```

Then register `DebugGizmosPlugin` and take a `DebugGizmos` parameter:

```rust
use concerto_debug_gizmos::{DebugGizmos, DebugGizmosPlugin};

app.register_plugin(DebugGizmosPlugin);

fn draw_targets(mut gizmos: DebugGizmos, targets: Query<&GlobalTransform, With<Enemy>>) {
    for transform in targets.iter() {
        gizmos.sphere(transform.translation(), 0.5, Color::rgba(1.0, 0.2, 0.2, 1.0));
    }
}
```

Gizmos are immediate mode: draw them every frame you want them seen.

`SkeletonGizmo`, from `concerto::mesh`, draws a skeleton without a mesh. Put it
on an entity with a `SkeletonComponent`, or add it through the editor's
inspector, and every frame it draws a line from each bone to its parent bone and
a sphere at each joint. Its fields set the colour, the joint size, and whether
each bone's axes are drawn. Every game built on `DefaultPlugins` keeps it in a
scene. It is drawn wherever `DebugGizmosPlugin` runs: in the editor's edit
viewport, and not while playing unless the game adds the plugin.

`MotionMatchingGizmos`, from the `concerto-motion-matching-gizmos` crate, draws
what motion matching compares on a running character: the simulation bone, the
last search's query, the frame it matched, and the path behind it. Register
`MotionMatchingGizmosPlugin` after `DefaultPlugins` and put the component on the
entity with `CharacterLink`. Its
[README](../../crates/motion-matching-gizmos/README.md) says what each toggle
draws.

`FootLockGizmos`, from the `concerto-procedural-animation-gizmos` crate, draws
each foot lock of a `FootLockNode`: a green ring and cross where a planted foot
is held, an amber ring and a line back to the animated foot while a lock
releases, and a grey cross at a free foot. Register
`ProceduralAnimationGizmosPlugin` after `DefaultPlugins` and put the component
on the entity with the `AnimationPlayer`. Its
[README](../../crates/procedural-animation-gizmos/README.md) says more.

`NavigationGizmosPlugin`, from the `concerto-navigation-gizmos` crate, draws
the nav graph, the route each agent is following, and each loaded navmesh; see
[Navigation](07-physics-animation-gameplay.md#drawing-the-graph).

## Custom materials

A material is an asset whose fields become shader bindings. Derive `Asset` and
`AsBindGroup`, name the shaders in `#[material(...)]`, then register a
`MaterialPlugin` for it:

```rust
use concerto::{
    foundation::assets::{Asset, LoadableAsset, handle::AssetHandle},
    render::{AsBindGroup, MaterialPlugin, assets::texture::Texture},
};

#[derive(Asset, AsBindGroup, serde::Serialize, serde::Deserialize)]
#[material(
    vertex_shader = include_str!("glow.wgsl"),
    fragment_shader = include_str!("glow.wgsl"),
    blend = "alpha",
)]
struct Glow {
    #[uniform(0)]
    color: [f32; 4],
    #[texture(1)]
    #[sampler(2)]
    texture: Option<AssetHandle<Texture>>,
}

impl LoadableAsset for Glow {}

app.register_plugin(MaterialPlugin::<Glow>::new());
```

Entities then use `MaterialComponent::<Glow>`. The material's own bindings
are in bind group 0; the camera, lighting and skeleton groups (1, 2 and 3) are
added when the material asks for them. The comments at the top of
`crates/render/src/material_plugin.rs` describe the convention.

Next: [Physics, animation and gameplay](07-physics-animation-gameplay.md).

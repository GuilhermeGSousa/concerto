# 6. Rendering

The renderer draws, each frame, what the main world says is there: a camera,
lights, and entities with a mesh and a material. You describe the scene with
components; you do not call draw functions.

## Getting to the renderer

The renderer reads a copy of the scene kept in the render world (chapter 2).
An entity gets a copy only if it has `SyncWithRenderWorld`, from
`concerto::render::components::render_entity`. UI nodes, the live camera,
and the mesh and light nodes of an imported scene already have it. When you spawn a light, a
mesh or a grid in code, add it:

```rust
cmd.spawn((light, transform, SyncWithRenderWorld));
```

A scene that renders black, or a grid that does not appear, is almost always
missing it.

Removing a component such as `Light` from a living entity does not remove its
copy, so the light keeps shining. Despawn the entity instead.

## Cameras

Spawn a `VirtualCamera` with a `Transform`. The **camera director** keeps a
stack of virtual cameras and drives the one real camera from the live one, the
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

## Lights

```rust
use concerto::render::components::light::{Light, LightType};

cmd.spawn((
    Light {
        color: Color::rgba(1.0, 0.95, 0.9, 1.0),
        intensity: 100.0,
        light_type: LightType::Point,
        range: 0.0,
        shadowmaps_enabled: true,
    },
    Transform::from_translation(Vec3::new(0.0, 8.0, 4.0)),
    SyncWithRenderWorld,
));
```

`LightType` is `Point`, `Spot { cone_angle }` or `Directional`. A directional
light shines along its transform's −Z. `shadowmaps_enabled` makes it cast
shadows.

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
        SyncWithRenderWorld,
    ));
}
```

`StandardMaterial` is a physically based material. `new` takes an optional base
color texture and normal map; the `with_*` methods add a color factor,
metallic-roughness, emissive and occlusion textures, an alpha cutoff and a UV
scale. Share one mesh or material handle between many entities by cloning the
handle.

`examples/physics-test/src/main.rs` has functions that build a plane, a box
and a sphere this way, which you can copy.

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

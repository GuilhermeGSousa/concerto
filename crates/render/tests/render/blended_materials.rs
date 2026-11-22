use std::f32::consts::FRAC_PI_2;

use concerto_app::{
    main_schedule::MainSchedulePlugin,
    plugins::{AssetManagerPlugin, TimePlugin, TransformPlugin},
    App,
};
use concerto_color::Color;
use concerto_ecs::Entity;
use concerto_foundation::{
    assets::{
        asset_server::AssetServer, asset_store::AssetStore, handle::AssetHandle, Asset, AssetId,
        LoadableAsset,
    },
    transform::Transform,
};
use concerto_mesh::{primitives::plane, MeshComponent};
use concerto_render::{
    assets::{
        material::{AlphaMode, StandardMaterial},
        mesh::Mesh,
        texture::Texture,
    },
    batching::{RenderDrawStats, ViewDrawStats},
    components::{
        camera::{Camera, RenderCamera, RenderTarget},
        light::{Light, ShadowCascades},
        material::MaterialComponent,
        render_entity::RenderEntity,
    },
    draw_target::DrawTarget,
    gpu::Gpu,
    material_plugin::MaterialPlugin,
    plugin::RenderPlugin,
    readback::read_texture,
    shadow_pipeline::ShadowPipelinePlugin,
    AsBindGroup, Tonemapping,
};
use concerto_window::plugin::{Window, WindowPlugin};
use glam::{Quat, Vec2, Vec3};

const SIZE: u32 = 32;
const TOLERANCE: u8 = 2;

const TINT_SHADER: &str = "
struct Tint {
    color: vec4<f32>,
};

struct CameraUniform {
    view_pos: vec3<f32>,
    view_proj: mat4x4<f32>,
};

@group(0) @binding(0)
var<uniform> tint: Tint;

@group(1) @binding(0)
var<uniform> camera: CameraUniform;

@vertex
fn vs_main(
    @location(0) position: vec3<f32>,
    @location(7) model_0: vec4<f32>,
    @location(8) model_1: vec4<f32>,
    @location(9) model_2: vec4<f32>,
    @location(10) model_3: vec4<f32>,
) -> @builtin(position) vec4<f32> {
    let model = mat4x4<f32>(model_0, model_1, model_2, model_3);
    return camera.view_proj * model * vec4<f32>(position, 1.0);
}

@fragment
fn fs_main() -> @location(0) vec4<f32> {
    return tint.color;
}
";

#[derive(Asset, AsBindGroup, serde::Serialize, serde::Deserialize)]
#[material(vertex_shader = TINT_SHADER, fragment_shader = TINT_SHADER)]
struct Tint {
    #[uniform(0)]
    color: [f32; 4],
    #[alpha_mode]
    alpha_mode: AlphaMode,
}

impl LoadableAsset for Tint {}

fn app(gpu: &Gpu) -> App {
    let mut app = App::new();
    app.insert_resource(gpu.clone())
        .insert_resource(DrawTarget::None)
        .insert_resource(Window::new((64, 48), 1.0));
    app.register_plugin(MainSchedulePlugin)
        .register_plugin(AssetManagerPlugin)
        .register_plugin(TimePlugin)
        .register_plugin(WindowPlugin)
        .register_plugin(TransformPlugin)
        .register_plugin(RenderPlugin)
        .register_plugin(ShadowPipelinePlugin)
        .register_plugin(MaterialPlugin::<StandardMaterial>::default())
        .register_plugin(MaterialPlugin::<Tint>::default());
    app.main_mut().world_mut().register_component::<Transform>();
    app
}

struct Surface {
    emissive: Vec3,
    alpha: f32,
    mode: AlphaMode,
}

fn material(app: &App, surface: &Surface) -> AssetHandle<StandardMaterial> {
    let mut material = StandardMaterial::default()
        .with_base_color_factor(Color::rgba(0.0, 0.0, 0.0, surface.alpha))
        .with_alpha_mode(surface.mode);
    material.set_emissive_factor(surface.emissive);
    material.set_metallic_factor(1.0);
    app.get_resource::<AssetServer>().unwrap().add(material)
}

fn spawn(
    app: &mut App,
    mesh: &AssetHandle<Mesh>,
    material: AssetHandle<StandardMaterial>,
    height: f32,
) -> Entity {
    app.main_mut().world_mut().spawn((
        MeshComponent {
            handle: mesh.clone(),
        },
        MaterialComponent::<StandardMaterial> { handle: material },
        Transform::from_translation(Vec3::new(0.0, height, 0.0)),
    ))
}

fn centre_pixel(app: &App, gpu: &Gpu, camera: Entity) -> [u8; 3] {
    let render_entity = **app
        .main()
        .world()
        .get_component_for_entity::<RenderEntity>(camera)
        .unwrap();
    let render_camera = app
        .render()
        .world()
        .get_component_for_entity::<RenderCamera>(render_entity)
        .unwrap();
    let image = read_texture(gpu, render_camera.render_target.as_ref().unwrap().texture());
    let [r, g, b, _] = image.get_pixel(SIZE / 2, SIZE / 2).0;
    [r, g, b]
}

fn shadow_layers(app: &App) -> Vec<ViewDrawStats> {
    app.render()
        .get_resource::<RenderDrawStats>()
        .unwrap()
        .shadow_layers
        .clone()
}

fn shown(light: Vec3) -> Vec3 {
    Tonemapping::Aces.apply(light)
}

fn over(surface: &Surface, behind: Vec3) -> Vec3 {
    surface.emissive * surface.alpha + behind * (1.0 - surface.alpha)
}

fn assert_pixel(actual: [u8; 3], expected: Vec3, what: &str) {
    let expected = (expected * 255.0)
        .round()
        .to_array()
        .map(|channel| channel as u8);
    assert!(
        actual
            .iter()
            .zip(expected)
            .all(|(&actual, expected)| actual.abs_diff(expected) <= TOLERANCE),
        "{what}: read {actual:?}, expected {expected:?}"
    );
}

#[test]
fn blended_surfaces_draw_back_to_front_over_the_opaque_scene_without_hiding_it() {
    let Some(gpu) = Gpu::request_for_test() else {
        return;
    };
    let mut app = app(&gpu);

    let floor = Surface {
        emissive: Vec3::new(0.2, 0.2, 1.0),
        alpha: 1.0,
        mode: AlphaMode::Opaque,
    };
    let far = Surface {
        emissive: Vec3::new(1.0, 0.0, 0.0),
        alpha: 0.5,
        mode: AlphaMode::Blend,
    };
    let near = Surface {
        emissive: Vec3::new(0.0, 1.0, 0.0),
        alpha: 0.5,
        mode: AlphaMode::Blend,
    };
    let near_opaque = Surface {
        mode: AlphaMode::Opaque,
        ..near
    };

    let server = app.get_resource::<AssetServer>().unwrap().clone();
    let floor_mesh = server.add(plane(Vec2::splat(8.0)));
    let sheet_mesh = server.add(plane(Vec2::splat(4.0)));
    let (floor_material, far_material, near_material, near_opaque_material) = (
        material(&app, &floor),
        material(&app, &far),
        material(&app, &near),
        material(&app, &near_opaque),
    );
    let near_entity = spawn(&mut app, &sheet_mesh, near_material, 2.0);
    spawn(&mut app, &sheet_mesh, far_material, 1.0);
    spawn(&mut app, &floor_mesh, floor_material, 0.0);

    app.main_mut().world_mut().spawn((
        Light::directional_light().with_shadows(),
        ShadowCascades::new().with_count(1),
        Transform::from_rotation(Quat::from_rotation_x(-FRAC_PI_2)),
    ));

    let target = AssetId::from_path("textures/blended_materials");
    app.main_mut()
        .world_mut()
        .get_resource_mut::<AssetStore<Texture>>()
        .unwrap()
        .insert(target, Texture::render_target(SIZE, SIZE));
    let camera = app.main_mut().world_mut().spawn((
        Camera {
            render_target: RenderTarget::Texture(AssetHandle::weak(target)),
            ..Camera::default()
        },
        Transform::from_translation_rotation(
            Vec3::new(0.0, 10.0, 0.0),
            Quat::from_rotation_x(-FRAC_PI_2),
        ),
    ));
    app.finish_plugin_build();
    for _ in 0..10 {
        app.update();
    }

    assert_pixel(
        centre_pixel(&app, &gpu, camera),
        shown(over(&near, over(&far, floor.emissive))),
        "the near sheet over the far sheet over the floor",
    );
    assert_eq!(
        shadow_layers(&app),
        [ViewDrawStats {
            draws: 1,
            culled: 0
        }],
        "only the opaque floor casts a shadow"
    );

    app.main_mut()
        .world_mut()
        .get_component_for_entity_mut::<MaterialComponent<StandardMaterial>>(near_entity)
        .unwrap()
        .handle = near_opaque_material;
    for _ in 0..3 {
        app.update();
    }

    assert_pixel(
        centre_pixel(&app, &gpu, camera),
        shown(near_opaque.emissive),
        "an opaque near sheet hides what is behind it",
    );
    assert_eq!(
        shadow_layers(&app),
        [ViewDrawStats {
            draws: 2,
            culled: 0
        }],
        "an opaque near sheet casts a shadow again"
    );
}

fn target_camera(app: &mut App, name: &str) -> Entity {
    let target = AssetId::from_path(name);
    app.main_mut()
        .world_mut()
        .get_resource_mut::<AssetStore<Texture>>()
        .unwrap()
        .insert(target, Texture::render_target(SIZE, SIZE));
    app.main_mut().world_mut().spawn((
        Camera {
            render_target: RenderTarget::Texture(AssetHandle::weak(target)),
            ..Camera::default()
        },
        Tonemapping::None,
        Transform::from_translation_rotation(
            Vec3::new(0.0, 10.0, 0.0),
            Quat::from_rotation_x(-FRAC_PI_2),
        ),
    ))
}

#[test]
fn blended_surfaces_of_two_material_types_are_sorted_together_by_distance() {
    let Some(gpu) = Gpu::request_for_test() else {
        return;
    };
    let mut app = app(&gpu);
    let floor = Surface {
        emissive: Vec3::new(0.2, 0.2, 0.2),
        alpha: 1.0,
        mode: AlphaMode::Opaque,
    };
    let far = Surface {
        emissive: Vec3::new(1.0, 0.0, 0.0),
        alpha: 0.5,
        mode: AlphaMode::Blend,
    };
    let near = Surface {
        emissive: Vec3::new(0.0, 0.0, 1.0),
        alpha: 0.5,
        mode: AlphaMode::Blend,
    };
    let middle = Vec3::new(0.0, 1.0, 0.0);

    let server = app.get_resource::<AssetServer>().unwrap().clone();
    let floor_mesh = server.add(plane(Vec2::splat(8.0)));
    let sheet_mesh = server.add(plane(Vec2::splat(4.0)));
    let (floor_material, far_material, near_material) = (
        material(&app, &floor),
        material(&app, &far),
        material(&app, &near),
    );
    let tint = server.add(Tint {
        color: [middle.x, middle.y, middle.z, 0.5],
        alpha_mode: AlphaMode::Blend,
    });
    spawn(&mut app, &sheet_mesh, near_material, 3.0);
    spawn(&mut app, &sheet_mesh, far_material, 1.0);
    spawn(&mut app, &floor_mesh, floor_material, 0.0);
    app.main_mut().world_mut().spawn((
        MeshComponent {
            handle: sheet_mesh.clone(),
        },
        MaterialComponent::<Tint> { handle: tint },
        Transform::from_translation(Vec3::new(0.0, 2.0, 0.0)),
    ));
    let camera = target_camera(&mut app, "textures/blended_two_types");
    app.finish_plugin_build();
    for _ in 0..10 {
        app.update();
    }

    let behind_tint = over(&far, floor.emissive);
    let through_tint = middle * 0.5 + behind_tint * 0.5;
    assert_pixel(
        centre_pixel(&app, &gpu, camera),
        over(&near, through_tint).min(Vec3::ONE),
        "a sheet of another material type between two standard ones is drawn between them",
    );
}

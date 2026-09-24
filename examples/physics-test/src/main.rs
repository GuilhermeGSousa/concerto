//! A minimal *rendered* Jolt collision test.
//!
//! Opens a window with a camera and a light, lays down a static floor, and drops
//! a handful of dynamic spheres. The spheres fall under gravity and visibly
//! collide with the floor (and each other), driven by the engine's physics
//! plugin. All meshes are generated in code, so no asset files are needed.
//!
//! Run it with `cargo run -p physics-test` (requires a display/GPU).

use concerto::{
    app::{
        schedule_groups::{Startup, Update},
        App,
    },
    core::{
        assets::asset_server::AssetServer,
        transform::{GlobalTransform, Transform},
    },
    director::VirtualCamera,
    ecs::{command::CommandQueue, component::Component, query::Query, resource::Res},
    mesh::MeshComponent,
    physics::{
        collider::Collider, physics_state::PhysicsState, rigid_body::RigidBody, shape::MeshCollider,
    },
    render::{
        assets::{material::StandardMaterial, mesh::Mesh, vertex::Vertex},
        components::{
            camera::Camera,
            light::{Light, LightType},
        },
        resources::RenderContext,
        MaterialComponent,
    },
    window::input::{Input, MouseButton},
    world_grid::WorldGrid,
    DefaultPlugins,
};
use concerto_color::Color;
use glam::{Quat, Vec3};

const SPHERE_RADIUS: f32 = 1.0;

/// Tags a ball with the color of its material so click-raycasts can report it.
#[derive(Component)]
struct BallColor(Color);

fn main() {
    env_logger::init();

    let mut app = App::new();
    app.register_plugin(DefaultPlugins::default())
        .add_system(Startup, spawn_scene)
        .add_system(Update, click_ball);
    app.run();
}

fn spawn_scene(mut cmd: CommandQueue, asset_server: Res<AssetServer>) {
    // Camera: pulled back and up, pitched slightly down to frame the floor.
    cmd.spawn((
        VirtualCamera::default(),
        Transform::from_translation_rotation(
            Vec3::new(0.0, 5.0, 14.0),
            Quat::from_rotation_x(-0.25),
        ),
    ));

    // Light above the scene.
    cmd.spawn((
        Light {
            color: Color::rgba(1.0, 1.0, 1.0, 1.0),
            intensity: 100.0,
            light_type: LightType::Point,
            shadowmaps_enabled: false,
        },
        Transform::from_translation_rotation(Vec3::new(0.0, 8.0, 4.0), Quat::IDENTITY),
    ));

    // Static floor: collider top surface sits at y = 0, with a matching plane mesh.
    let floor_transform =
        Transform::from_translation_rotation(Vec3::new(0.0, -0.5, 0.0), Quat::IDENTITY);

    let floor_mesh = asset_server.add(make_plane(40.0, 40.0));
    let floor_material = asset_server.add(
        StandardMaterial::new(None, None).with_base_color_factor(Color::rgba(0.4, 0.4, 0.45, 1.0)),
    );
    cmd.spawn((
        // Collider::cuboid(20.0, 0.5, 20.0),
        MeshCollider,
        MeshComponent { handle: floor_mesh },
        MaterialComponent {
            handle: floor_material,
        },
        floor_transform,
    ));

    // Low walls fence the balls into a 16 x 16 arena: rolling spheres never
    // stop on their own, so without walls the collision impulses would send
    // them drifting off the floor.
    let wall_material = asset_server.add(
        StandardMaterial::new(None, None).with_base_color_factor(Color::rgba(0.3, 0.3, 0.35, 1.0)),
    );
    for (pos, half) in [
        (Vec3::new(0.0, 0.75, -8.5), Vec3::new(9.0, 1.25, 0.5)),
        (Vec3::new(0.0, 0.75, 8.5), Vec3::new(9.0, 1.25, 0.5)),
        (Vec3::new(-8.5, 0.75, 0.0), Vec3::new(0.5, 1.25, 9.0)),
        (Vec3::new(8.5, 0.75, 0.0), Vec3::new(0.5, 1.25, 9.0)),
    ] {
        let wall_transform = Transform::from_translation_rotation(pos, Quat::IDENTITY);
        cmd.spawn((
            Collider::cuboid(half.x, half.y, half.z),
            MeshComponent {
                handle: asset_server.add(make_box(half)),
            },
            MaterialComponent {
                handle: wall_material.clone(),
            },
            wall_transform,
        ));
    }

    // Dynamic spheres dropped from increasing heights, placed on a tight
    // spiral. The spiral gives x and z independent offsets: if all spawn
    // points shared one vertical plane (and started at rest), every collision
    // impulse would stay in that plane forever and the balls would never
    // scatter into a 3D pile. The tight packing makes them land on each other
    // and collide.
    let sphere_mesh = asset_server.add(make_uv_sphere(SPHERE_RADIUS, 16, 32));
    let colors = [
        Color::random_color(),
        Color::random_color(),
        Color::random_color(),
        Color::random_color(),
        Color::random_color(),
        Color::random_color(),
        Color::random_color(),
        Color::random_color(),
        Color::random_color(),
        Color::random_color(),
        Color::random_color(),
        Color::random_color(),
        Color::random_color(),
    ];

    for (i, color) in colors.into_iter().enumerate() {
        let height = 2.5 + i as f32 * 0.8;
        // Golden-angle-ish steps with a slowly growing radius (sunflower
        // pattern) cluster the landing spots within a couple of ball diameters
        // of each other.
        let angle = i as f32 * 2.4;
        let radius = ((i + 1) as f32).sqrt() * 0.9;
        let transform = Transform::from_translation_rotation(
            Vec3::new(angle.cos() * radius, height, angle.sin() * radius),
            Quat::IDENTITY,
        );

        let material =
            asset_server.add(StandardMaterial::new(None, None).with_base_color_factor(color));
        cmd.spawn((
            RigidBody::default(),
            Collider::sphere(SPHERE_RADIUS),
            BallColor(color),
            MeshComponent {
                handle: sphere_mesh.clone(),
            },
            MaterialComponent { handle: material },
            transform,
        ));
    }

    cmd.spawn(WorldGrid::default());
}

/// On left click, casts a ray through the clicked pixel and prints the color
/// of the ball it hits, if any.
fn click_ball(
    cameras: Query<(&Camera, &GlobalTransform)>,
    balls: Query<(&BallColor,)>,
    input: Res<Input>,
    context: Res<RenderContext>,
    physics: Res<PhysicsState>,
) {
    if !input.is_mouse_button_just_pressed(MouseButton::Left) {
        return;
    }
    let Some((camera, camera_transform)) = cameras.iter().next() else {
        return;
    };

    let cursor = input.mouse_position();
    let ndc_x = 2.0 * cursor.x / context.surface_config.width as f32 - 1.0;
    let ndc_y = 1.0 - 2.0 * cursor.y / context.surface_config.height as f32;

    let tan_half_fovy = (camera.fovy * 0.5).tan();
    let view_dir = Vec3::new(
        ndc_x * tan_half_fovy * camera.aspect,
        ndc_y * tan_half_fovy,
        -1.0,
    );
    let direction = camera_transform
        .matrix()
        .transform_vector3(view_dir)
        .normalize();

    let origin = camera_transform.translation();
    let Some(hit) = physics.cast_ray(origin, direction * 100.0) else {
        println!("click: nothing hit");
        return;
    };

    let ball_color = hit
        .entity
        .and_then(|entity| balls.get_entity(entity))
        .map(|(color,)| color.0);
    match ball_color {
        Some(color) => println!("click: hit ball with color {color:?}"),
        None => println!("click: hit static geometry at {}", hit.point),
    }
}

/// Builds a flat `width` x `length` plane in the XZ plane (centered at the origin)
/// with upward-facing normals.
fn make_plane(width: f32, length: f32) -> Mesh {
    let hw = width / 2.0;
    let hl = length / 2.0;
    let normal = [0.0, 1.0, 0.0];
    let vertices = vec![
        Vertex {
            pos_coords: [-hw, 0.0, -hl],
            normal,
            ..Vertex::default()
        },
        Vertex {
            pos_coords: [hw, 0.0, -hl],
            normal,
            ..Vertex::default()
        },
        Vertex {
            pos_coords: [hw, 0.0, hl],
            normal,
            ..Vertex::default()
        },
        Vertex {
            pos_coords: [-hw, 0.0, hl],
            normal,
            ..Vertex::default()
        },
    ];
    // Counter-clockwise seen from above, so the face points +Y. Both the
    // renderer (FrontFace::Ccw + back-face culling) and Jolt's mesh shapes
    // treat CCW as the front, and mesh triangles are single sided, so the
    // opposite winding is a floor you fall through as well as cannot see.
    let indices = vec![0, 2, 1, 0, 3, 2];

    let mut mesh = Mesh { vertices, indices };
    mesh.compute_tangents();
    mesh
}

/// Builds an axis-aligned box mesh with the given half-extents, matching the
/// dimensions used for the wall colliders.
fn make_box(half: Vec3) -> Mesh {
    let positions: [[f32; 3]; 8] = [
        [-half.x, -half.y, -half.z],
        [half.x, -half.y, -half.z],
        [half.x, half.y, -half.z],
        [-half.x, half.y, -half.z],
        [-half.x, -half.y, half.z],
        [half.x, -half.y, half.z],
        [half.x, half.y, half.z],
        [-half.x, half.y, half.z],
    ];
    let faces: [([f32; 3], [usize; 4]); 6] = [
        ([0.0, 0.0, -1.0], [0, 3, 2, 1]),
        ([0.0, 0.0, 1.0], [4, 5, 6, 7]),
        ([-1.0, 0.0, 0.0], [0, 4, 7, 3]),
        ([1.0, 0.0, 0.0], [1, 2, 6, 5]),
        ([0.0, -1.0, 0.0], [0, 1, 5, 4]),
        ([0.0, 1.0, 0.0], [3, 7, 6, 2]),
    ];

    let mut vertices = Vec::with_capacity(24);
    let mut indices = Vec::with_capacity(36);
    for (normal, corners) in faces {
        let base = vertices.len() as u32;
        for &corner in &corners {
            vertices.push(Vertex {
                pos_coords: positions[corner],
                normal,
                ..Vertex::default()
            });
        }
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    let mut mesh = Mesh { vertices, indices };
    mesh.compute_tangents();
    mesh
}

/// Builds a UV sphere of the given radius with analytic normals.
fn make_uv_sphere(radius: f32, rings: u32, segments: u32) -> Mesh {
    let mut vertices = Vec::with_capacity(((rings + 1) * (segments + 1)) as usize);
    let mut indices = Vec::with_capacity((rings * segments * 6) as usize);

    for ring in 0..=rings {
        let phi = std::f32::consts::PI * ring as f32 / rings as f32;
        let (sin_phi, cos_phi) = phi.sin_cos();
        for segment in 0..=segments {
            let theta = std::f32::consts::TAU * segment as f32 / segments as f32;
            let (sin_theta, cos_theta) = theta.sin_cos();
            let normal = [sin_phi * cos_theta, cos_phi, sin_phi * sin_theta];
            vertices.push(Vertex {
                pos_coords: [normal[0] * radius, normal[1] * radius, normal[2] * radius],
                normal,
                ..Vertex::default()
            });
        }
    }

    let stride = segments + 1;
    for ring in 0..rings {
        for segment in 0..segments {
            let a = ring * stride + segment;
            let b = a + stride;
            indices.extend_from_slice(&[a, b, a + 1, a + 1, b, b + 1]);
        }
    }

    let mut mesh = Mesh { vertices, indices };
    mesh.compute_tangents();
    mesh
}

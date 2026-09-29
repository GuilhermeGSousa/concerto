use concerto::{asset_id, foundation::assets::AssetId};
use concerto::{
    scene::{scene::Scene, spawner::SceneSpawnerComponent},
    DefaultPlugins,
};
use concerto_app::{
    schedule_groups::{Startup, Update},
    App,
};
use concerto_color::Color;
use concerto_ecs::{command::CommandQueue, query::Query, resource::Res, Component, With};
use concerto_foundation::{assets::asset_server::AssetServer, time::Time, transform::Transform};
use concerto_render::components::light::{Light, LightType};
use glam::{Quat, Vec3};

#[cfg(feature = "terminal")]
use concerto_app::schedule_groups::LateRender;
#[cfg(feature = "terminal")]
use concerto_ecs::{resource::ResMut, IntoSystemConfig};
#[cfg(feature = "terminal")]
use concerto_render::{
    assets::texture::Texture,
    components::camera::{Camera, RenderTarget},
    wgpu,
};
#[cfg(feature = "terminal")]
use concerto_terminal_renderer::{
    frame::TerminalFrame, terminal::TerminalContext, TerminalInput, TerminalOutput,
    TerminalRendererPlugin,
};
#[cfg(feature = "terminal")]
use ratatui::{
    crossterm::event::KeyCode,
    layout::{Constraint, Layout},
    style::Stylize,
    text::{Line as TextLine, Span},
};

#[cfg(not(feature = "terminal"))]
use concerto::{
    mesh::MeshComponent,
    physics::{
        body::BodyId, collider::Collider, physics_state::PhysicsState, rigid_body::RigidBody,
    },
    window::input::{Input, MouseButton},
};
#[cfg(not(feature = "terminal"))]
use concerto_debug_gizmos::{DebugGizmos, DebugGizmosPlugin};
#[cfg(not(feature = "terminal"))]
use concerto_ecs::{resource::ResMut, Entity};
#[cfg(not(feature = "terminal"))]
use concerto_foundation::transform::GlobalTransform;
#[cfg(not(feature = "terminal"))]
use concerto_gameplay::{movement::first_person_player_fly, player::spawn_first_person_player};
#[cfg(not(feature = "terminal"))]
use concerto_render::{
    assets::{material::StandardMaterial, mesh::Mesh, vertex::Vertex},
    components::camera::Camera,
    MaterialComponent,
};

const SPONZA_SCENE: AssetId = asset_id!("content/Sponza/scene.gasset");

#[cfg(not(feature = "terminal"))]
const SPHERE_RADIUS: f32 = 0.35;
#[cfg(not(feature = "terminal"))]
const MUZZLE_SPEED: f32 = 25.0;

#[derive(Component)]
struct Cube;

/// A sphere that has been fired but whose body does not exist yet, carrying
/// the launch velocity until one does.
#[cfg(not(feature = "terminal"))]
#[derive(Component)]
struct Projectile(Vec3);

fn main() {
    cfg_if::cfg_if! {
        if #[cfg(target_arch = "wasm32")] {
            std::panic::set_hook(Box::new(console_error_panic_hook::hook));
            console_log::init_with_level(log::Level::Debug).expect("Couldn't initialize logger");
        } else {
            env_logger::init();
        }
    }

    let mut app = App::new();

    #[cfg(feature = "terminal")]
    {
        app.register_plugin(DefaultPlugins::headless())
            .register_plugin(TerminalRendererPlugin)
            .add_system(Startup, spawn_camera_terminal)
            .add_system(Startup, spawn_scene)
            .add_system(Update, rotate_cube)
            .add_system(Update, move_camera);
        app.add_system(LateRender, draw_terminal);
    }

    #[cfg(not(feature = "terminal"))]
    {
        use concerto::ui::frame_stats_overlay::FrameStatsOverlayPlugin;

        app.register_plugin(DefaultPlugins::default())
            .register_plugin(FrameStatsOverlayPlugin)
            .add_system(Startup, spawn_camera_windowed)
            .add_system(Startup, spawn_scene)
            .add_system(Update, rotate_cube)
            .add_system(Update, shoot_sphere)
            .add_system(Update, launch_projectiles)
            .add_system(Update, first_person_player_fly)
            .add_system(Update, draw_gizmos);
        app.register_plugin(DebugGizmosPlugin);
    }

    app.run();
}

#[cfg(feature = "terminal")]
fn spawn_camera_terminal(
    mut cmd: CommandQueue,
    asset_server: Res<AssetServer>,
    terminal: Res<TerminalContext>,
) {
    let terminal_size = terminal.size().unwrap();
    let rtt = asset_server.add(Texture::render_target(
        terminal_size.width as u32,
        terminal_size.height as u32,
    ));

    // Account for terminal cells being ~2x taller than wide
    let aspect = (terminal_size.width as f32 * 0.5) / terminal_size.height as f32;
    let camera = Camera {
        aspect,
        render_target: RenderTarget::texture(rtt),
        clear_color: Color::BLACK,
        ..Camera::default()
    };
    cmd.spawn((
        camera,
        Light {
            color: Color::WHITE,
            intensity: 100.0,
            light_type: LightType::Point,
            range: 0.0,
        },
        TerminalOutput,
        Transform::from_translation_rotation(Vec3::ZERO, Quat::IDENTITY),
    ));
}

#[cfg(not(feature = "terminal"))]
fn spawn_camera_windowed(mut cmd: CommandQueue) {
    spawn_first_person_player(
        &mut cmd,
        Vec3::new(0.0, 2.0, 0.0),
        Light {
            color: Color::WHITE,
            intensity: 10.0,
            light_type: LightType::Point,
            range: 0.0,
            shadowmaps_enabled: false,
        },
    );
}

fn spawn_scene(mut cmd: CommandQueue, asset_server: Res<AssetServer>) {
    // TODO(asset-import-pipeline): Sponza mesh colliders are gone — the old
    // runtime glTF spawner's physics-shapes option has no Scene equivalent yet.
    cmd.spawn(SceneSpawnerComponent(
        asset_server.load::<Scene>(SPONZA_SCENE),
    ));
    // cmd.spawn(WorldGrid::default());
}

/// Fires a sphere along the camera's view direction on left click.
///
// TODO(asset-import-pipeline): the sphere used to collide with Sponza's
// generated mesh colliders; with the Scene spawner those colliders are gone,
// so fired spheres now fall straight through the level.
#[cfg(not(feature = "terminal"))]
fn shoot_sphere(
    cameras: Query<(&Camera, &GlobalTransform)>,
    input: Res<Input>,
    mut cmd: CommandQueue,
    asset_server: Res<AssetServer>,
) {
    if !input.is_mouse_button_just_pressed(MouseButton::Left) {
        return;
    }
    let Some((_, camera_transform)) = cameras.iter().next() else {
        return;
    };

    let forward = camera_transform
        .matrix()
        .transform_vector3(Vec3::NEG_Z)
        .normalize();
    // Clear of the camera, so the sphere is not spawned inside the player.
    let origin = camera_transform.translation() + forward * (SPHERE_RADIUS * 4.0);

    let material = asset_server
        .add(StandardMaterial::new(None, None).with_base_color_factor(Color::random_color()));
    cmd.spawn((
        Projectile(forward * MUZZLE_SPEED),
        RigidBody::default(),
        Collider::sphere(SPHERE_RADIUS),
        MeshComponent {
            handle: asset_server.add(make_uv_sphere(SPHERE_RADIUS, 12, 24)),
        },
        MaterialComponent { handle: material },
        Transform::from_translation_rotation(origin, Quat::IDENTITY),
    ));
}

/// `RigidBody` carries no initial velocity, and the body itself is not created
/// until `register_colliders` runs, so a freshly fired sphere has nothing to
/// push until the frame after it spawns. Launch it once its `BodyId` appears.
#[cfg(not(feature = "terminal"))]
fn launch_projectiles(
    projectiles: Query<(Entity, &Projectile, &BodyId)>,
    mut physics: ResMut<PhysicsState>,
    mut cmd: CommandQueue,
) {
    for (entity, projectile, body) in projectiles.iter() {
        physics.set_linear_velocity(*body, projectile.0);
        cmd.remove::<Projectile>(entity);
    }
}

#[cfg(not(feature = "terminal"))]
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

fn rotate_cube(cubes: Query<&mut Transform, With<Cube>>, time: Res<Time>) {
    let delta = time.delta().as_secs_f32();
    for mut transform in cubes.iter() {
        transform.rotation *= Quat::from_rotation_y(delta);
    }
}

#[cfg(feature = "terminal")]
fn move_camera(
    cameras: Query<&mut Transform, With<Camera>>,
    input: Res<TerminalInput>,
    time: Res<Time>,
) {
    let speed = 5.0 * time.delta().as_secs_f32();
    let rot_speed = 2.0 * time.delta().as_secs_f32();

    for mut transform in cameras.iter() {
        if input.is_key_active(KeyCode::Char('z')) {
            let fwd = transform.forward();
            transform.translation += fwd * speed;
        }
        if input.is_key_active(KeyCode::Char('s')) {
            let bwd = transform.backward();
            transform.translation += bwd * speed;
        }
        if input.is_key_active(KeyCode::Char('q')) {
            let left = transform.left();
            transform.translation += left * speed;
        }
        if input.is_key_active(KeyCode::Char('d')) {
            let right = transform.right();
            transform.translation += right * speed;
        }

        if input.is_key_active(KeyCode::Left) {
            transform.rotation = Quat::from_rotation_y(rot_speed) * transform.rotation;
        }
        if input.is_key_active(KeyCode::Right) {
            transform.rotation = Quat::from_rotation_y(-rot_speed) * transform.rotation;
        }
        if input.is_key_active(KeyCode::Up) {
            transform.rotation = transform.rotation * Quat::from_rotation_x(-rot_speed);
        }
        if input.is_key_active(KeyCode::Down) {
            transform.rotation = transform.rotation * Quat::from_rotation_x(rot_speed);
        }
    }
}

#[cfg(feature = "terminal")]
fn draw_terminal(mut terminal: ResMut<TerminalContext>, terminal_frame: Res<TerminalFrame>) {
    terminal
        .draw(|frame| {
            if let Some(data) = terminal_frame.current_frame() {
                let vertical =
                    Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]).spacing(1);
                let horizontal = Layout::horizontal([Constraint::Percentage(100)]).spacing(1);
                let [top, main] = frame.area().layout(&vertical);
                let [area] = main.layout(&horizontal);

                let title = TextLine::from_iter([
                    Span::from("This is a Widget!").bold(),
                    Span::from(" (Press 'ESC' to quit)"),
                ]);

                frame.render_widget(title.centered(), top);
                frame.render_widget(data, area);
            }
        })
        .unwrap();
}

/// Immediate-mode gizmo demo.
///
/// Everything here is re-issued every frame: the static reference shapes are
/// redrawn, and a wireframe sphere is drawn around every live physics sphere by
/// querying its current transform — the classic immediate-mode use case for
/// visualising simulation state.
#[cfg(not(feature = "terminal"))]
fn draw_gizmos(mut gizmos: DebugGizmos, bodies: Query<&GlobalTransform, With<BodyId>>) {
    gizmos.axes(&Transform::from_translation(Vec3::ZERO), 1.0);

    let box_transform = Transform::from_translation_rotation_scale(
        Vec3::new(2.0, 1.0, 0.0),
        Quat::IDENTITY,
        Vec3::splat(1.0),
    );
    gizmos.cuboid(&box_transform, Color::rgba(1.0, 0.6, 0.1, 1.0));

    for transform in bodies.iter() {
        let center = transform.translation();
        gizmos.sphere(center, SPHERE_RADIUS, Color::GREEN);
        gizmos.arrow(center, center + Vec3::Y * 0.75, Color::RED);
    }
}

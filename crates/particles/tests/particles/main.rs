//! Tests of particle emission, simulation and drawing through the particles plugin.

use concerto_app::{
    main_schedule::MainSchedulePlugin,
    plugins::{AssetManagerPlugin, TimePlugin, TransformPlugin},
    App,
};
use concerto_color::Color;
use concerto_foundation::transform::Transform;
use concerto_particles::{ParticleEmitter, Particles, ParticlesPlugin, PARTICLE_SHADER};
use concerto_render::{
    capabilities::RenderCapabilities,
    components::camera::Camera,
    draw_target::{DrawTarget, SharedTexture},
    gpu::Gpu,
    plugin::RenderPlugin,
    shader::compose_shader,
    wgpu, Tonemapping,
};
use concerto_window::plugin::{Window, WindowPlugin};
use glam::{Mat4, Vec3};
use naga::{
    back::glsl,
    proc::BoundsCheckPolicies,
    valid::{Capabilities, ValidationFlags, Validator},
    ShaderStage,
};

fn emitter() -> ParticleEmitter {
    ParticleEmitter::default()
        .with_rate(10.0)
        .with_capacity(4)
        .with_lifetime(1.0)
        .with_velocity(Vec3::new(0.0, 2.0, 0.0), 0.0)
        .with_acceleration(Vec3::new(0.0, -1.0, 0.0))
}

#[test]
fn an_emitter_fills_to_its_capacity_moves_its_particles_and_lets_them_die() {
    let emitter = emitter();
    let origin = Mat4::from_translation(Vec3::new(5.0, 0.0, 0.0));
    let mut particles = Particles::default();

    particles.advance(&emitter, origin, 0.25);
    assert_eq!(particles.alive().len(), 2, "ten a second for a quarter");
    assert!(particles
        .alive()
        .iter()
        .all(|particle| particle.position == Vec3::new(5.0, 0.0, 0.0)
            && particle.velocity == Vec3::new(0.0, 2.0, 0.0)));

    particles.advance(&emitter, origin, 0.5);
    assert_eq!(particles.alive().len(), 4, "emission waits at capacity");
    let oldest = particles.alive()[0];
    assert_eq!(oldest.age, 0.5);
    assert_eq!(
        oldest.velocity,
        Vec3::new(0.0, 1.5, 0.0),
        "gravity slows it"
    );
    assert_eq!(oldest.position, Vec3::new(5.0, 0.75, 0.0));

    particles.advance(
        &emitter,
        Mat4::from_translation(Vec3::new(9.0, 0.0, 0.0)),
        0.3,
    );
    assert_eq!(particles.alive().len(), 4);
    assert_eq!(
        particles.alive()[0].position.x,
        5.0,
        "a particle stays where the emitter left it"
    );

    particles.advance(&emitter, origin, 0.25);
    assert!(
        particles
            .alive()
            .iter()
            .all(|particle| particle.age < emitter.lifetime),
        "the first two have lived their second and are gone"
    );
    assert_eq!(
        particles.alive().len(),
        4,
        "and the emitter has filled their places"
    );

    let stopped = emitter.clone().with_rate(0.0);
    particles.advance(&stopped, origin, 0.5);
    particles.advance(&stopped, origin, 0.6);
    assert!(
        particles.alive().is_empty(),
        "a rate of zero lets them die out"
    );

    let spread = ParticleEmitter::default()
        .with_rate(1000.0)
        .with_capacity(64)
        .with_extent(Vec3::new(1.0, 2.0, 3.0))
        .with_velocity(Vec3::ZERO, 0.5);
    let mut scattered = Particles::default();
    scattered.advance(&spread, Mat4::IDENTITY, 0.1);
    assert_eq!(scattered.alive().len(), 64);
    assert!(scattered.alive().iter().all(|particle| {
        let place = particle.position.abs();
        place.x <= 1.0
            && place.y <= 2.0
            && place.z <= 3.0
            && particle.velocity.abs().max_element() <= 0.5
    }));
    assert!(
        scattered.alive()[0].position != scattered.alive()[1].position,
        "particles appear at different places in the box"
    );
}

#[test]
fn particles_are_drawn_in_front_of_the_camera_that_sees_them() {
    let Some(gpu) = Gpu::request_for_test() else {
        return;
    };
    let shared = SharedTexture::new(wgpu::TextureFormat::Rgba8Unorm);
    let mut app = App::new();
    app.insert_resource(gpu.clone())
        .insert_resource(DrawTarget::Texture(shared.clone()))
        .insert_resource(Window::new((64, 64), 1.0));
    app.register_plugin(MainSchedulePlugin)
        .register_plugin(AssetManagerPlugin)
        .register_plugin(TimePlugin)
        .register_plugin(WindowPlugin)
        .register_plugin(TransformPlugin)
        .register_plugin(RenderPlugin)
        .register_plugin(ParticlesPlugin);
    let world = app.main_mut().world_mut();
    world.register_component::<Transform>();
    world.spawn((
        Camera {
            clear_color: Color::rgba(0.0, 0.0, 0.0, 1.0),
            ..Camera::default()
        },
        Tonemapping::None,
        Transform::IDENTITY,
    ));
    let red = Color::rgba(1.0, 0.0, 0.0, 1.0);
    let emitter = world.spawn((
        ParticleEmitter::default()
            .with_rate(1.0e6)
            .with_capacity(8)
            .with_lifetime(1.0e6)
            .with_velocity(Vec3::ZERO, 0.0)
            .with_size(1.0, 1.0)
            .with_color(red, red)
            .with_lit(false),
        Transform::from_translation(Vec3::new(0.0, 0.0, -3.0)),
    ));
    app.finish_plugin_build();

    let alive = |app: &App| {
        app.main()
            .world()
            .get_component_for_entity::<Particles>(emitter)
            .unwrap()
            .alive()
            .len()
    };
    for _ in 0..200 {
        app.update();
        if alive(&app) == 8 {
            break;
        }
    }
    assert_eq!(alive(&app), 8);
    app.update();
    app.update();

    let image = gpu.read_texture(shared.get().unwrap().texture());
    let centre = image.get_pixel(32, 32).0;
    let corner = image.get_pixel(2, 2).0;
    assert!(
        centre[0] > 200 && centre[1] == 0,
        "an unlit particle shows its colour where it is: {centre:?}"
    );
    assert_eq!(corner, [0, 0, 0, 255], "and nothing where it is not");
}

#[test]
fn the_particle_shader_translates_to_glsl_es_300() {
    let capabilities = RenderCapabilities {
        cube_array_textures: false,
        depth_bias_clamp: false,
        single_layer_texture_arrays: false,
        bc_textures: false,
        float_render_targets: false,
    };
    let module = compose_shader(
        "particles.wgsl",
        PARTICLE_SHADER,
        &capabilities.shader_defs(),
    )
    .unwrap();
    let info = Validator::new(ValidationFlags::all(), Capabilities::empty())
        .validate(&module)
        .unwrap();
    for stage in [ShaderStage::Vertex, ShaderStage::Fragment] {
        let entry_point = match stage {
            ShaderStage::Vertex => "vs_main",
            _ => "fs_main",
        };
        let options = glsl::Options {
            version: glsl::Version::Embedded {
                version: 300,
                is_webgl: true,
            },
            ..Default::default()
        };
        let pipeline_options = glsl::PipelineOptions {
            shader_stage: stage,
            entry_point: entry_point.to_string(),
            multiview: None,
        };
        let mut output = String::new();
        glsl::Writer::new(
            &mut output,
            &module,
            &info,
            &options,
            &pipeline_options,
            BoundsCheckPolicies::default(),
        )
        .and_then(|mut writer| writer.write())
        .unwrap_or_else(|err| panic!("{entry_point} doesn't translate: {err}"));
    }
}

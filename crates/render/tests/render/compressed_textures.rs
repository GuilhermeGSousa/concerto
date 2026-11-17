use std::f32::consts::FRAC_PI_2;
use std::sync::{Mutex, Once};
use std::time::{Duration, Instant};

use concerto_app::{
    main_schedule::MainSchedulePlugin,
    plugins::{AssetManagerPlugin, TimePlugin, TransformPlugin},
    App,
};
use concerto_color::Color;
use concerto_foundation::{
    assets::{
        asset_server::AssetServer, asset_store::AssetStore, content::save_content_asset_into,
        derived::DerivedDataCache, handle::AssetHandle, registry::AssetRegistry, AssetId,
        ContentAssetRoot,
    },
    transform::Transform,
};
use concerto_mesh::{primitives::plane, MeshComponent};
use concerto_render::{
    assets::{
        material::StandardMaterial,
        texture::{
            Compression, SourceEncoding, Texture, TextureRole, TextureSettings, TextureSource,
        },
    },
    capabilities::RenderCapabilities,
    components::{
        camera::{Camera, RenderCamera, RenderTarget},
        light::Light,
        material::MaterialComponent,
        render_entity::RenderEntity,
    },
    draw_target::DrawTarget,
    gpu::Gpu,
    material_plugin::MaterialPlugin,
    plugin::RenderPlugin,
    readback::read_texture,
};
use concerto_window::plugin::{Window, WindowPlugin};
use glam::{Quat, Vec2, Vec3};
use image::{DynamicImage, ImageBuffer, ImageFormat, Rgb, Rgba, RgbaImage};

use crate::texture_derive::TempProject;

const TARGET: u32 = 128;

static WARNINGS: Mutex<Vec<String>> = Mutex::new(Vec::new());

struct Capture;

impl log::Log for Capture {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::Level::Warn
    }

    fn log(&self, record: &log::Record) {
        if self.enabled(record.metadata()) {
            WARNINGS.lock().unwrap().push(record.args().to_string());
        }
    }

    fn flush(&self) {}
}

fn capture_warnings() {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        log::set_logger(&Capture).unwrap();
        log::set_max_level(log::LevelFilter::Warn);
    });
}

fn png(image: DynamicImage) -> TextureSource {
    let mut bytes = Vec::new();
    image
        .write_to(&mut std::io::Cursor::new(&mut bytes), ImageFormat::Png)
        .unwrap();
    TextureSource::new(bytes, SourceEncoding::Png).unwrap()
}

fn solid_blocks() -> DynamicImage {
    let tiles = [
        [66, 195, 255],
        [255, 65, 132],
        [132, 255, 66],
        [198, 130, 198],
    ];
    DynamicImage::ImageRgba8(ImageBuffer::from_fn(64, 64, |x, y| {
        let [r, g, b] = tiles[(x / 32 + 2 * (y / 32)) as usize];
        Rgba([r, g, b, 255])
    }))
}

struct Content {
    project: TempProject,
    registry: AssetRegistry,
}

impl Content {
    fn save(
        &mut self,
        name: &str,
        role: TextureRole,
        compression: Compression,
        source: TextureSource,
    ) -> AssetId {
        let texture = Texture::imported(
            role,
            TextureSettings::default().with_compression(compression),
            &source,
        );
        save_content_asset_into(
            &texture,
            Some(&source),
            &self.project.root,
            &format!("content/{name}.gasset"),
            &mut self.registry,
        )
        .unwrap()
    }
}

#[derive(Clone, Copy)]
enum Surface {
    Emissive(AssetId),
    Lit {
        base: Option<AssetId>,
        normal: Option<AssetId>,
    },
}

fn material(server: &AssetServer, surface: Surface) -> StandardMaterial {
    match surface {
        Surface::Emissive(id) => {
            let mut material = StandardMaterial::default()
                .with_base_color_factor(Color::BLACK)
                .with_emissive_texture(server.load(id));
            material.set_emissive_factor(Vec3::ONE);
            material
        }
        Surface::Lit { base, normal } => {
            let mut material = StandardMaterial::default();
            if let Some(id) = base {
                material.set_base_color_texture(server.load(id));
            }
            if let Some(id) = normal {
                material.set_normal_texture(server.load(id));
            }
            material
        }
    }
}

fn render(gpu: &Gpu, content: &Content, bc_textures: bool, surface: Surface) -> RgbaImage {
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
        .register_plugin(MaterialPlugin::<StandardMaterial>::default());
    app.main_mut().world_mut().register_component::<Transform>();
    let target = AssetId::new();
    app.main_mut()
        .world_mut()
        .get_resource_mut::<AssetStore<Texture>>()
        .unwrap()
        .insert(target, Texture::render_target(TARGET, TARGET));
    let camera = app.main_mut().world_mut().spawn((
        Camera {
            render_target: RenderTarget::Texture(AssetHandle::weak(target)),
            ..Camera::default()
        },
        Transform::IDENTITY,
    ));
    app.finish_plugin_build();
    if !bc_textures {
        let capabilities = *app.render().get_resource::<RenderCapabilities>().unwrap();
        app.render_mut().insert_resource(RenderCapabilities {
            bc_textures: false,
            ..capabilities
        });
    }
    app.update();

    let server = app.get_resource::<AssetServer>().unwrap().clone();
    server.set_derived_cache(DerivedDataCache::none());
    server.publish_content_source(
        ContentAssetRoot::Directory(content.project.root.clone()),
        content.registry.clone(),
    );
    if let Surface::Lit { .. } = surface {
        app.main_mut().world_mut().spawn((
            Light::point_light().with_intensity(2.0),
            Transform::from_translation(Vec3::new(0.3, 0.2, 0.0)),
        ));
    }
    for (center, size) in [
        (Vec3::new(-0.2, 0.0, -1.0), 0.42),
        (Vec3::new(0.25, 0.0, -1.0), 0.065),
    ] {
        let mesh = server.add(plane(Vec2::splat(size)));
        let material = server.add(material(&server, surface));
        app.main_mut().world_mut().spawn((
            MeshComponent { handle: mesh },
            MaterialComponent::<StandardMaterial> { handle: material },
            Transform::from_translation_rotation(center, Quat::from_rotation_x(FRAC_PI_2)),
        ));
    }

    let started = Instant::now();
    while server.is_loading() {
        assert!(
            started.elapsed() < Duration::from_secs(30),
            "load timed out"
        );
        std::thread::sleep(Duration::from_millis(1));
        app.update();
    }
    for _ in 0..10 {
        app.update();
    }

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
    read_texture(gpu, render_camera.render_target.as_ref().unwrap().texture())
}

fn max_difference(a: &RgbaImage, b: &RgbaImage, channels: usize) -> u8 {
    a.pixels()
        .zip(b.pixels())
        .flat_map(|(a, b)| (0..channels).map(move |c| a.0[c].abs_diff(b.0[c])))
        .max()
        .unwrap()
}

struct Case {
    compression: Compression,
    channels: usize,
}

#[test]
fn bc_formats_sample_like_their_uncompressed_source() {
    let Some(gpu) = Gpu::request_for_test() else {
        return;
    };
    capture_warnings();
    let mut content = Content {
        project: TempProject::new("compressed-textures"),
        registry: AssetRegistry::new(),
    };
    let uncompressed = content.save(
        "uncompressed",
        TextureRole::Data,
        Compression::Uncompressed,
        png(solid_blocks()),
    );
    let reference = render(&gpu, &content, true, Surface::Emissive(uncompressed));

    let cases = [
        Case {
            compression: Compression::Bc1,
            channels: 3,
        },
        Case {
            compression: Compression::Bc4,
            channels: 1,
        },
        Case {
            compression: Compression::Bc5,
            channels: 2,
        },
        Case {
            compression: Compression::Bc7,
            channels: 3,
        },
    ];
    for case in &cases {
        let name = format!("{:?}", case.compression).to_lowercase();
        let id = content.save(
            &name,
            TextureRole::Data,
            case.compression,
            png(solid_blocks()),
        );
        let image = render(&gpu, &content, true, Surface::Emissive(id));
        let difference = max_difference(&image, &reference, case.channels);
        assert!(
            difference <= 4,
            "{name} differs from uncompressed by {difference}/255"
        );
    }

    let plain = render(
        &gpu,
        &content,
        true,
        Surface::Lit {
            base: None,
            normal: None,
        },
    );
    let color = content.save(
        "refused_color",
        TextureRole::Color,
        Compression::Bc1,
        png(solid_blocks()),
    );
    let refused = render(
        &gpu,
        &content,
        false,
        Surface::Lit {
            base: Some(color),
            normal: None,
        },
    );
    let difference = max_difference(&refused, &plain, 4);
    assert!(
        difference <= 2,
        "a refused colour texture shades {difference}/255 away from no base colour texture"
    );
    let refusals = WARNINGS
        .lock()
        .unwrap()
        .iter()
        .filter(|message| {
            message.contains("content/refused_color.gasset")
                && message.contains("no BC texture compression")
        })
        .count();
    assert_eq!(refusals, 1, "the refusal names the texture, once");

    let flat = content.save(
        "flat_normal",
        TextureRole::Normal,
        Compression::Auto,
        png(DynamicImage::ImageRgb8(ImageBuffer::from_pixel(
            64,
            64,
            Rgb([128, 128, 255]),
        ))),
    );
    let mapped = render(
        &gpu,
        &content,
        true,
        Surface::Lit {
            base: None,
            normal: Some(flat),
        },
    );
    let difference = max_difference(&mapped, &plain, 3);
    assert!(
        difference <= 2,
        "a flat BC5 normal map shades {difference}/255 away from no normal map"
    );
    let refused = render(
        &gpu,
        &content,
        false,
        Surface::Lit {
            base: None,
            normal: Some(flat),
        },
    );
    let difference = max_difference(&refused, &plain, 3);
    assert!(
        difference <= 2,
        "a refused BC5 normal map shades {difference}/255 away from no normal map"
    );
}

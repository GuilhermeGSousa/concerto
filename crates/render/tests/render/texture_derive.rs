use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use concerto_app::{
    main_schedule::MainSchedulePlugin,
    plugins::{AssetManagerPlugin, TimePlugin},
    App,
};
use concerto_foundation::assets::{
    asset_server::AssetServer, asset_store::AssetStore, content::save_content_asset_into,
    derived::DerivedDataCache, registry::AssetRegistry, AssetId, ContentAssetRoot,
};
use concerto_render::assets::texture::{
    derive_levels, ColorSpace, Compression, MaxSize, SourceEncoding, Texture, TextureFormat,
    TextureRole, TextureSettings, TextureSource,
};
use image::{DynamicImage, ImageBuffer, ImageFormat, Luma, Rgb, Rgba};

fn encoded(image: DynamicImage, format: ImageFormat) -> Vec<u8> {
    let mut bytes = Vec::new();
    image
        .write_to(&mut std::io::Cursor::new(&mut bytes), format)
        .unwrap();
    bytes
}

fn png(image: DynamicImage) -> TextureSource {
    TextureSource::new(encoded(image, ImageFormat::Png), SourceEncoding::Png).unwrap()
}

fn rgb(width: u32, height: u32) -> DynamicImage {
    DynamicImage::ImageRgb8(ImageBuffer::from_fn(width, height, |x, y| {
        Rgb([(x * 7) as u8, (y * 11) as u8, 90])
    }))
}

fn rgba_with_alpha(width: u32, height: u32) -> DynamicImage {
    DynamicImage::ImageRgba8(ImageBuffer::from_fn(width, height, |x, y| {
        Rgba([
            200,
            (x * 9) as u8,
            (y * 5) as u8,
            if x == 0 { 10 } else { 255 },
        ])
    }))
}

fn grey16(width: u32, height: u32) -> DynamicImage {
    DynamicImage::ImageLuma16(ImageBuffer::from_fn(width, height, |x, y| {
        Luma([(x * 4000 + y * 300) as u16])
    }))
}

fn jpeg(width: u32, height: u32) -> TextureSource {
    TextureSource::new(
        encoded(rgb(width, height), ImageFormat::Jpeg),
        SourceEncoding::Jpeg,
    )
    .unwrap()
}

#[derive(Debug, PartialEq)]
struct Levels {
    format: TextureFormat,
    width: u32,
    height: u32,
    byte_lens: Vec<usize>,
}

fn expected(format: TextureFormat, width: u32, height: u32, count: u32) -> Levels {
    let byte_lens = (0..count)
        .map(|level| {
            let w = (width >> level).max(1) as usize;
            let h = (height >> level).max(1) as usize;
            match format {
                TextureFormat::Rgba8Unorm | TextureFormat::Rgba8UnormSrgb => w * h * 4,
                TextureFormat::Bc1RgbaUnormSrgb | TextureFormat::Bc4RUnorm => {
                    w.div_ceil(4) * h.div_ceil(4) * 8
                }
                _ => w.div_ceil(4) * h.div_ceil(4) * 16,
            }
        })
        .collect();
    Levels {
        format,
        width,
        height,
        byte_lens,
    }
}

struct Row {
    name: &'static str,
    source: TextureSource,
    role: TextureRole,
    settings: TextureSettings,
    outcome: Result<Levels, &'static str>,
}

#[test]
fn levels_follow_role_settings_and_size() {
    let defaults = TextureSettings::default();
    let rows = [
        Row {
            name: "colour with alpha",
            source: png(rgba_with_alpha(8, 8)),
            role: TextureRole::Color,
            settings: defaults,
            outcome: Ok(expected(TextureFormat::Bc7RgbaUnormSrgb, 8, 8, 4)),
        },
        Row {
            name: "opaque colour",
            source: png(rgb(8, 8)),
            role: TextureRole::Color,
            settings: defaults,
            outcome: Ok(expected(TextureFormat::Bc1RgbaUnormSrgb, 8, 8, 4)),
        },
        Row {
            name: "jpeg normal",
            source: jpeg(16, 8),
            role: TextureRole::Normal,
            settings: defaults,
            outcome: Ok(expected(TextureFormat::Bc5RgUnorm, 16, 8, 5)),
        },
        Row {
            name: "16-bit greyscale data",
            source: png(grey16(8, 4)),
            role: TextureRole::Data,
            settings: defaults,
            outcome: Ok(expected(TextureFormat::Bc7RgbaUnorm, 8, 4, 4)),
        },
        Row {
            name: "wide colour capped at 2048",
            source: png(rgb(3000, 1000)),
            role: TextureRole::Color,
            settings: defaults.with_max_size(MaxSize::S2048),
            outcome: Ok(expected(TextureFormat::Bc1RgbaUnormSrgb, 2048, 684, 12)),
        },
        Row {
            name: "6x6 rounds to blocks",
            source: png(rgb(6, 6)),
            role: TextureRole::Color,
            settings: defaults,
            outcome: Ok(expected(TextureFormat::Bc1RgbaUnormSrgb, 8, 8, 4)),
        },
        Row {
            name: "6x6 uncompressed keeps its size",
            source: png(rgb(6, 6)),
            role: TextureRole::Color,
            settings: defaults.with_compression(Compression::Uncompressed),
            outcome: Ok(expected(TextureFormat::Rgba8UnormSrgb, 6, 6, 3)),
        },
        Row {
            name: "no mips",
            source: png(rgb(8, 8)),
            role: TextureRole::Color,
            settings: defaults.with_mips(false),
            outcome: Ok(expected(TextureFormat::Bc1RgbaUnormSrgb, 8, 8, 1)),
        },
        Row {
            name: "srgb bc4",
            source: png(rgb(8, 8)),
            role: TextureRole::Data,
            settings: defaults
                .with_compression(Compression::Bc4)
                .with_color_space(Some(ColorSpace::Srgb)),
            outcome: Err("Linear"),
        },
        Row {
            name: "srgb bc5",
            source: png(rgb(8, 8)),
            role: TextureRole::Color,
            settings: defaults.with_compression(Compression::Bc5),
            outcome: Err("Linear"),
        },
    ];
    for row in rows {
        let derived = derive_levels(&row.source, row.role, &row.settings);
        match (derived, row.outcome) {
            (Ok(levels), Ok(expected)) => assert_eq!(
                Levels {
                    format: levels.format,
                    width: levels.width,
                    height: levels.height,
                    byte_lens: levels.levels.iter().map(Vec::len).collect(),
                },
                expected,
                "{}",
                row.name
            ),
            (Err(error), Err(names)) => {
                let message = format!("{error:#}");
                assert!(message.contains(names), "{}: {message}", row.name);
            }
            (derived, outcome) => panic!(
                "{}: derived {:?}, expected {:?}",
                row.name,
                derived.map(|levels| levels.format),
                outcome.map(|levels| levels.format)
            ),
        }
    }
}

pub(crate) struct TextureRuntime {
    app: App,
    server: AssetServer,
}

impl TextureRuntime {
    pub(crate) fn new(root: &Path, registry: &AssetRegistry, cache: DerivedDataCache) -> Self {
        let mut app = App::new();
        app.register_plugin(MainSchedulePlugin)
            .register_plugin(TimePlugin)
            .register_plugin(AssetManagerPlugin)
            .register_derived_asset::<Texture>();
        app.finish_plugin_build();
        app.update();
        let server = app.get_resource::<AssetServer>().unwrap().clone();
        server.set_derived_cache(cache);
        server.publish_content_source(
            ContentAssetRoot::Directory(root.to_path_buf()),
            registry.clone(),
        );
        Self { app, server }
    }

    pub(crate) fn load<R>(&mut self, id: AssetId, read: impl FnOnce(&Texture) -> R) -> Option<R> {
        let handle = self.server.load::<Texture>(id);
        let started = Instant::now();
        while self.server.is_loading() {
            assert!(
                started.elapsed() < Duration::from_secs(20),
                "load timed out"
            );
            std::thread::sleep(Duration::from_millis(1));
            self.app.update();
        }
        self.app.update();
        self.app
            .main()
            .world()
            .get_resource::<AssetStore<Texture>>()
            .unwrap()
            .get(&handle)
            .map(read)
    }
}

pub(crate) struct TempProject {
    pub(crate) root: PathBuf,
}

impl TempProject {
    pub(crate) fn new(tag: &str) -> Self {
        let root = std::env::temp_dir().join(format!("{tag}-{}", AssetId::new().simple_hex()));
        std::fs::create_dir_all(root.join("content")).unwrap();
        Self { root }
    }
}

impl Drop for TempProject {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[derive(Debug, PartialEq)]
pub(crate) struct Loaded {
    pub(crate) format: Option<TextureFormat>,
    pub(crate) level_count: usize,
    pub(crate) keeps_source: bool,
}

pub(crate) fn loaded(texture: &Texture) -> Loaded {
    Loaded {
        format: texture.levels().map(|levels| levels.format),
        level_count: texture.levels().map_or(0, |levels| levels.levels.len()),
        keeps_source: texture.source.is_some(),
    }
}

#[test]
fn a_cooked_texture_loads_its_levels_without_deriving() {
    let project = TempProject::new("cooked-texture");
    let mut registry = AssetRegistry::new();
    let texture = Texture::imported(
        TextureRole::Color,
        TextureSettings::default().with_compression(Compression::Bc7),
        png(rgb(8, 8)),
    );
    let id = save_content_asset_into(
        &texture,
        &project.root,
        "content/swatch.gasset",
        &mut registry,
    )
    .unwrap();
    let out = project.root.join("cooked");
    let cooking = TextureRuntime::new(&project.root, &registry, DerivedDataCache::none());

    let report = cooking.server.cook(&out).unwrap();

    assert_eq!(report.cooked, ["content/swatch.gasset"]);
    assert!(!project.root.join("derived-cache").exists());
    let cache = project.root.join("runtime-cache");
    let mut runtime = TextureRuntime::new(&out, &registry, DerivedDataCache::at(&cache));
    assert_eq!(
        runtime.load(id, loaded),
        Some(Loaded {
            format: Some(TextureFormat::Bc7RgbaUnormSrgb),
            level_count: 4,
            keeps_source: false,
        })
    );
    assert!(!cache.exists(), "a cooked load derives nothing");
}

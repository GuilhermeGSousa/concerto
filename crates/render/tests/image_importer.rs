//! Covers ImageImporter emitting a serialized Texture from a raw image file.
use std::path::Path;

use concerto_asset_import::{ImportContext, Importer};
use concerto_foundation::assets::Asset;
use concerto_render::assets::texture::{Texture, TextureFormat, TextureKind};
use concerto_render::importers::image_importer::ImageImporter;

fn write_test_png(path: &Path) {
    let img = image::RgbaImage::from_pixel(2, 2, image::Rgba([255, 0, 0, 255]));
    img.save(path).expect("failed to write test fixture PNG");
}

#[test]
fn import_produces_one_main_sub_asset_with_correct_pixels() {
    let temp_dir = std::env::temp_dir().join(format!("image-importer-test-{}", std::process::id()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let source_path = temp_dir.join("swatch.png");
    write_test_png(&source_path);

    let mut ctx = ImportContext::new(std::path::PathBuf::from("swatch.png"));
    ImageImporter
        .import(&source_path, &mut ctx)
        .expect("importing a valid PNG should succeed");
    let outputs = ctx.into_parts();

    assert_eq!(outputs.sub_assets.len(), 1);
    assert_eq!(outputs.sub_assets[0].name, "main");
    assert_eq!(outputs.sub_assets[0].type_name, Texture::name());

    let texture: Texture = bincode::deserialize(&outputs.sub_assets[0].bytes).unwrap();
    assert_eq!(texture.width, 2);
    assert_eq!(texture.height, 2);
    assert_eq!(texture.format, TextureFormat::Rgba8UnormSrgb);
    assert_eq!(texture.kind, TextureKind::Sampled);
    assert_eq!(texture.data.len(), 2 * 2 * 4);
    assert_eq!(&texture.data[0..4], &[255, 0, 0, 255]);

    std::fs::remove_dir_all(&temp_dir).ok();
}

#[test]
fn texture_round_trips_through_bincode() {
    let texture = Texture {
        width: 2,
        height: 1,
        format: TextureFormat::Rgba8UnormSrgb,
        kind: TextureKind::Sampled,
        data: vec![10, 20, 30, 255, 40, 50, 60, 255],
    };
    let bytes = bincode::serialize(&texture).expect("Texture should serialize");
    let decoded: Texture = bincode::deserialize(&bytes).expect("Texture should deserialize");

    assert_eq!(decoded.width, 2, "width must survive the round-trip");
    assert_eq!(decoded.height, 1, "height must survive the round-trip");
    assert_eq!(
        decoded.format,
        TextureFormat::Rgba8UnormSrgb,
        "format must survive the round-trip"
    );
    assert_eq!(
        decoded.kind,
        TextureKind::Sampled,
        "kind must survive the round-trip"
    );
    assert_eq!(
        decoded.data,
        vec![10, 20, 30, 255, 40, 50, 60, 255],
        "pixels must survive the round-trip unchanged"
    );
}

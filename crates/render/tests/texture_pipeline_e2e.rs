//! End-to-end proof that an imported texture round-trips through the
//! runtime byte-loading path — the same envelope `AssetServer` reads.
use concerto_asset_import::{ImportContext, Importer};
use concerto_core::assets::content::{
    write_content_asset, ContentAssetHeader, CONTENT_FORMAT_VERSION,
};
use concerto_core::assets::utils::load_content_asset_bytes;
use concerto_core::assets::{Asset, ContentAssetRoot};
use concerto_render::assets::texture::{Texture, TextureFormat, TextureKind};
use concerto_render::importers::image_importer::ImageImporter;

#[test]
fn an_imported_texture_is_reachable_at_its_conventional_address() {
    let temp_dir = std::env::temp_dir().join(format!("texture-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();

    let source_path = temp_dir.join("swatch.png");
    let img = image::RgbaImage::from_pixel(1, 1, image::Rgba([9, 9, 9, 255]));
    img.save(&source_path)
        .expect("failed to save fixture texture");

    let mut ctx = ImportContext::new(std::path::PathBuf::from("swatch.png"));
    ImageImporter
        .import(&source_path, &mut ctx)
        .expect("importing the fixture texture must succeed");
    let outputs = ctx.into_parts();
    assert_eq!(
        outputs.sub_assets.len(),
        1,
        "one image yields one sub-asset"
    );
    let sub_asset = &outputs.sub_assets[0];

    let address = "content/swatch/main.gasset";
    let header = ContentAssetHeader {
        format_version: CONTENT_FORMAT_VERSION,
        asset_id: sub_asset.asset_id,
        references: sub_asset.references.clone(),
        kind: sub_asset.type_name.to_string(),
        provenance: None,
    };
    std::fs::create_dir_all(temp_dir.join("content/swatch")).unwrap();
    std::fs::write(
        temp_dir.join(address),
        write_content_asset(&header, &sub_asset.bytes).unwrap(),
    )
    .unwrap();

    let bytes = pollster::block_on(load_content_asset_bytes(
        &ContentAssetRoot::Directory(temp_dir.clone()),
        address,
        Texture::name(),
    ))
    .expect("the imported texture must load back through the runtime byte path");

    let texture: Texture = bincode::deserialize(&bytes).expect("failed to deserialize texture");
    assert_eq!(texture.width, 1);
    assert_eq!(texture.height, 1);
    assert_eq!(texture.format, TextureFormat::Rgba8UnormSrgb);
    assert_eq!(texture.kind, TextureKind::Sampled);
    assert_eq!(texture.data, vec![9, 9, 9, 255]);

    let _ = std::fs::remove_dir_all(&temp_dir);
}

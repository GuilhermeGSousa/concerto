//! End-to-end proof that an imported texture round-trips through the
//! runtime byte-loading path — the same envelope `AssetServer` reads.
use concerto_asset_import::{ImportContext, Importer};
use concerto_foundation::assets::content::{write_content_asset, ContentAssetHeader};
use concerto_foundation::assets::derived::DerivedDataCache;
use concerto_foundation::assets::registry::{AssetEntry, AssetRegistry};
use concerto_foundation::assets::Asset;
use concerto_render::assets::texture::{Texture, TextureFormat};
use concerto_render::importers::image_importer::ImageImporter;

use crate::texture_derive::{loaded, Loaded, TempProject, TextureRuntime};

#[test]
fn an_imported_image_is_one_srgb_texture_that_loads_back_from_its_content_address_with_its_mips() {
    let project = TempProject::new("texture-e2e");
    let source_path = project.root.join("swatch.png");
    let img = image::RgbaImage::from_pixel(4, 4, image::Rgba([9, 9, 9, 255]));
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
    assert_eq!(sub_asset.name, "main");
    assert_eq!(sub_asset.type_name, Texture::name());

    let address = "content/swatch/main.gasset";
    let header = ContentAssetHeader {
        asset_id: sub_asset.asset_id,
        references: sub_asset.references.clone(),
        kind: sub_asset.type_name.to_string(),
        kind_version: sub_asset.kind_version,
        provenance: None,
        derived: None,
        embedded: Vec::new(),
    };
    std::fs::create_dir_all(project.root.join("content/swatch")).unwrap();
    std::fs::write(
        project.root.join(address),
        write_content_asset(&header, &sub_asset.bytes).unwrap(),
    )
    .unwrap();
    let mut registry = AssetRegistry::new();
    registry.insert(AssetEntry {
        id: sub_asset.asset_id,
        address: address.to_string(),
        kind: sub_asset.type_name.to_string(),
        kind_version: Some(sub_asset.kind_version),
        provenance: None,
        embedded_in: None,
    });

    let mut runtime = TextureRuntime::new(&project.root, &registry, DerivedDataCache::none());
    assert_eq!(
        runtime.load(sub_asset.asset_id, loaded),
        Some(Loaded {
            format: Some(TextureFormat::Bc1RgbaUnormSrgb),
            level_count: 3,
            keeps_source: false,
        }),
        "a 4x4 opaque colour image derives BC1 with mips down to 1x1"
    );
}

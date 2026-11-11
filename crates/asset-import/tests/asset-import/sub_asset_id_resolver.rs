//! ImportContext's sub-asset-id resolver lets a caller (the `import` tool)
//! redirect cross-references from `<source>#<sub>` to content-tree paths,
//! without the importers knowing which pipeline they run under.
use std::path::PathBuf;

use concerto_asset_import::{ImportContext, SubAssetIdResolver};
use concerto_foundation::assets::{Asset, AssetId};

#[derive(serde::Serialize, serde::Deserialize)]
struct Thing;

impl Asset for Thing {
    fn name() -> &'static str {
        "Thing"
    }
}

#[test]
fn a_resolver_decides_every_id_and_receives_the_rename_hint() {
    let resolver: SubAssetIdResolver = Box::new(|sub_name, old_name| {
        AssetId::from_path(&format!(
            "content/hero/{}|{}.gasset",
            sub_name.replace('/', "_"),
            old_name.unwrap_or("-")
        ))
    });
    let mut ctx =
        ImportContext::with_sub_asset_id_resolver(PathBuf::from("raw/hero.gltf"), resolver);
    ctx.renamed_from("animation/walk", "animation/0");
    ctx.renamed_from("animation/idle", "animation/idle");

    assert_eq!(
        ctx.sub_asset_id("mesh/0"),
        AssetId::from_path("content/hero/mesh_0|-.gasset"),
        "the resolver, not the source path, decides the id"
    );
    assert_eq!(
        ctx.sub_asset_id("animation/walk"),
        AssetId::from_path("content/hero/animation_walk|animation/0.gasset")
    );
    assert_eq!(
        ctx.sub_asset_id("animation/idle"),
        AssetId::from_path("content/hero/animation_idle|-.gasset"),
        "a self-hint is not recorded"
    );

    ctx.emit("animation/walk", &Thing).expect("emit");
    let outputs = ctx.into_parts();
    assert_eq!(
        outputs.sub_assets[0].asset_id,
        AssetId::from_path("content/hero/animation_walk|animation/0.gasset"),
        "emit() records the resolved id, so cross-references bake correctly"
    );
    assert_eq!(
        outputs.renames,
        vec![("animation/walk".to_string(), "animation/0".to_string())]
    );

    let mut plain = ImportContext::new(PathBuf::from("raw/hero.gltf"));
    plain.renamed_from("animation/walk", "animation/0");
    assert_eq!(
        plain.sub_asset_id("animation/walk"),
        AssetId::from_path("raw/hero.gltf#animation/walk"),
        "default addressing is against the source and ignores the hint"
    );
}

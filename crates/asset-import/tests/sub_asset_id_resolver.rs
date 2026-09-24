//! ImportContext's sub-asset-id resolver lets a caller (the `import` tool)
//! redirect cross-references from `<source>#<sub>` to content-tree paths,
//! without the importers knowing which pipeline they run under.
use std::path::PathBuf;

use concerto_asset_import::{ImportContext, SubAssetIdResolver};
use concerto_core::assets::{Asset, AssetId};

#[derive(serde::Serialize, serde::Deserialize)]
struct Thing;

impl Asset for Thing {
    fn name() -> &'static str {
        "Thing"
    }
}

#[test]
fn default_resolver_addresses_sub_assets_against_the_source() {
    let ctx = ImportContext::new(PathBuf::from("raw/hero.gltf"));
    assert_eq!(
        ctx.sub_asset_id("mesh/0"),
        AssetId::from_path("raw/hero.gltf#mesh/0")
    );
}

#[test]
fn custom_resolver_replaces_the_addressing_scheme() {
    let resolver: SubAssetIdResolver = Box::new(|sub_name| {
        AssetId::from_path(&format!(
            "content/hero/{}.gasset",
            sub_name.replace('/', "_")
        ))
    });
    let ctx = ImportContext::with_sub_asset_id_resolver(PathBuf::from("raw/hero.gltf"), resolver);

    assert_eq!(
        ctx.sub_asset_id("mesh/0"),
        AssetId::from_path("content/hero/mesh_0.gasset"),
        "the resolver, not the source path, decides the id"
    );
}

#[test]
fn emitted_sub_assets_carry_the_resolved_id() {
    let resolver: SubAssetIdResolver =
        Box::new(|sub_name| AssetId::from_path(&format!("content/x/{sub_name}.gasset")));
    let mut ctx =
        ImportContext::with_sub_asset_id_resolver(PathBuf::from("raw/hero.gltf"), resolver);

    ctx.emit("thing", &Thing).expect("emit");
    let emitted = ctx.into_parts().sub_assets;

    assert_eq!(emitted.len(), 1);
    assert_eq!(
        emitted[0].asset_id,
        AssetId::from_path("content/x/thing.gasset"),
        "emit() records the resolved id, so cross-references bake correctly"
    );
}

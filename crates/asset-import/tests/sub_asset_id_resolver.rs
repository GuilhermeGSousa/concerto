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
fn default_resolver_addresses_sub_assets_against_the_source() {
    let ctx = ImportContext::new(PathBuf::from("raw/hero.gltf"));
    assert_eq!(
        ctx.sub_asset_id("mesh/0"),
        AssetId::from_path("raw/hero.gltf#mesh/0")
    );
}

#[test]
fn custom_resolver_replaces_the_addressing_scheme() {
    let resolver: SubAssetIdResolver = Box::new(|sub_name, _old_name| {
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
        Box::new(|sub_name, _old_name| AssetId::from_path(&format!("content/x/{sub_name}.gasset")));
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

#[test]
fn resolver_receives_the_hinted_old_name() {
    let resolver: SubAssetIdResolver = Box::new(|sub_name, old_name| {
        AssetId::from_path(&format!("{sub_name}|{}", old_name.unwrap_or("-")))
    });
    let mut ctx =
        ImportContext::with_sub_asset_id_resolver(PathBuf::from("raw/hero.gltf"), resolver);

    ctx.renamed_from("animation/walk", "animation/0");

    assert_eq!(
        ctx.sub_asset_id("animation/walk"),
        AssetId::from_path("animation/walk|animation/0")
    );
    assert_eq!(
        ctx.sub_asset_id("animation/run"),
        AssetId::from_path("animation/run|-"),
        "a name without a hint receives None"
    );

    ctx.emit("animation/walk", &Thing).expect("emit");
    let outputs = ctx.into_parts();
    assert_eq!(
        outputs.sub_assets[0].asset_id,
        AssetId::from_path("animation/walk|animation/0"),
        "emit() resolves with the hint too"
    );
    assert_eq!(
        outputs.renames,
        vec![("animation/walk".to_string(), "animation/0".to_string())]
    );
}

#[test]
fn self_hint_is_ignored_and_default_addressing_ignores_hints() {
    let resolver: SubAssetIdResolver = Box::new(|sub_name, old_name| {
        assert!(old_name.is_none(), "a self-hint must not be recorded");
        AssetId::from_path(sub_name)
    });
    let mut ctx =
        ImportContext::with_sub_asset_id_resolver(PathBuf::from("raw/hero.gltf"), resolver);
    ctx.renamed_from("animation/0", "animation/0");
    ctx.sub_asset_id("animation/0");
    assert!(ctx.into_parts().renames.is_empty());

    let mut plain = ImportContext::new(PathBuf::from("raw/hero.gltf"));
    plain.renamed_from("animation/walk", "animation/0");
    assert_eq!(
        plain.sub_asset_id("animation/walk"),
        AssetId::from_path("raw/hero.gltf#animation/walk"),
        "default addressing ignores the hint"
    );
}

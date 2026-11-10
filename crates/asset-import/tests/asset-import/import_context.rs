//! Covers ImportContext's sub-asset emission and same-file reference-ID
//! computation.
use concerto_asset_import::ImportContext;
use concerto_foundation::assets::{Asset, AssetId};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
struct FakeCookedThing {
    referenced: AssetId,
}

impl Asset for FakeCookedThing {
    fn name() -> &'static str {
        "FakeThing"
    }

    fn referenced_sub_assets(&self) -> Vec<AssetId> {
        vec![self.referenced]
    }
}

#[derive(Serialize, Deserialize)]
struct FakeVersionedThing;

impl Asset for FakeVersionedThing {
    const FORMAT_VERSION: u32 = 5;

    fn name() -> &'static str {
        "FakeVersionedThing"
    }
}

#[test]
fn emit_records_the_named_sub_asset_with_its_source_scoped_id_type_version_references_and_bytes() {
    let mut ctx = ImportContext::new(std::path::PathBuf::from("models/character.gltf"));
    let referenced_id = ctx.sub_asset_id("texture/albedo");
    assert_eq!(ctx.sub_asset_id("texture/albedo"), referenced_id);
    assert_eq!(
        referenced_id,
        AssetId::from_path("models/character.gltf#texture/albedo"),
        "an id matches what a runtime load of the fully-qualified path computes"
    );

    ctx.emit(
        "material/0",
        &FakeCookedThing {
            referenced: referenced_id,
        },
    )
    .expect("emit");
    ctx.emit("thing/0", &FakeVersionedThing).expect("emit");

    let out = ctx.into_parts();
    assert_eq!(out.sub_assets.len(), 2);
    let entry = &out.sub_assets[0];
    assert_eq!(entry.name, "material/0");
    assert_eq!(
        entry.asset_id,
        AssetId::from_path("models/character.gltf#material/0")
    );
    assert_eq!(entry.type_name, "FakeThing");
    assert_eq!(entry.references, vec![referenced_id]);
    let round_tripped: FakeCookedThing = bincode::deserialize(&entry.bytes).unwrap();
    assert_eq!(round_tripped.referenced, referenced_id);
    assert_eq!(out.sub_assets[1].kind_version, 5);
}

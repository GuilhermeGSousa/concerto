//! Covers AssetHandle's Strong/Weak split and its serialization contract:
//! any handle (regardless of variant) serializes to its bare AssetId, and
//! deserializing always produces a Weak handle (never a live Strong one,
//! since deserialization has no AssetServer to resolve against).
use concerto_foundation::assets::{handle::AssetHandle, Asset, AssetId};

#[derive(serde::Serialize, serde::Deserialize)]
struct FakeAsset;
impl Asset for FakeAsset {
    type EditorOnly = ();

    fn name() -> &'static str {
        "FakeAsset"
    }
}

#[test]
fn a_handle_encodes_as_its_bare_id_and_decodes_weak() {
    let id = AssetId::from_path("models/character.gltf#texture/albedo");
    let bytes = bincode::serialize(&AssetHandle::<FakeAsset>::weak(id)).unwrap();
    assert_eq!(bytes, bincode::serialize(&id).unwrap());

    let decoded: AssetHandle<FakeAsset> = bincode::deserialize(&bytes).unwrap();
    assert!(matches!(decoded, AssetHandle::Weak(..)));
    assert_eq!(decoded.id(), id);
}

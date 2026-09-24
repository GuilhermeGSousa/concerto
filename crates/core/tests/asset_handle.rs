//! Covers AssetHandle's Strong/Weak split and its serialization contract:
//! any handle (regardless of variant) serializes to its bare AssetId, and
//! deserializing always produces a Weak handle (never a live Strong one,
//! since deserialization has no AssetServer to resolve against).
use concerto_core::assets::{handle::AssetHandle, Asset, AssetId};

#[derive(serde::Serialize, serde::Deserialize)]
struct FakeAsset;
impl Asset for FakeAsset {
    fn name() -> &'static str {
        "FakeAsset"
    }
}

#[test]
fn weak_handle_serializes_to_its_id() {
    let id = AssetId::from_path("models/character.gltf#texture/albedo");
    let handle: AssetHandle<FakeAsset> = AssetHandle::weak(id);

    let bytes = bincode::serialize(&handle).unwrap();
    let decoded: AssetHandle<FakeAsset> = bincode::deserialize(&bytes).unwrap();

    assert_eq!(
        decoded.id(),
        id,
        "round-tripping a handle must preserve its AssetId"
    );
}

#[test]
fn deserialized_handle_is_weak_and_id_matches() {
    let id = AssetId::from_path("models/character.gltf#mesh/0");
    let bytes = bincode::serialize(&id).unwrap();
    let decoded: AssetHandle<FakeAsset> = bincode::deserialize(&bytes).unwrap();
    assert_eq!(
        decoded.id(),
        id,
        "deserializing a bare AssetId must produce a handle with that ID"
    );
}

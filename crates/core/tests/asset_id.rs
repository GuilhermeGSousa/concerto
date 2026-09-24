//! Legacy path hashing and the persistent UUID representation used by macros.
use concerto_core::assets::AssetId;

#[test]
fn uuid_bytes_can_be_embedded_as_a_constant() {
    const BYTES: [u8; 16] = [0x42; 16];
    const ID: AssetId = AssetId::from_bytes(BYTES);
    const RESTORED: [u8; 16] = ID.to_bytes();
    assert_eq!(RESTORED, BYTES);
    assert_eq!(ID.simple_hex(), "42424242424242424242424242424242");
}

#[test]
fn from_path_is_deterministic() {
    let a = AssetId::from_path("models/character.gltf#texture/albedo");
    let b = AssetId::from_path("models/character.gltf#texture/albedo");
    assert_eq!(
        a, b,
        "the same path string must hash to the same AssetId every time"
    );
}

#[test]
fn from_path_differs_for_different_inputs() {
    let a = AssetId::from_path("models/character.gltf#texture/albedo");
    let b = AssetId::from_path("models/character.gltf#texture/normal");
    assert_ne!(a, b, "distinct sub-asset names must hash to distinct IDs");
}

#[test]
fn round_trips_through_bincode() {
    let id = AssetId::from_path("models/character.gltf#scene");
    let bytes = bincode::serialize(&id).unwrap();
    let decoded: AssetId = bincode::deserialize(&bytes).unwrap();
    assert_eq!(decoded, id);
}

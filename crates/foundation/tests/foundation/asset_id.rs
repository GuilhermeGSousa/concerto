//! Legacy path hashing and the persistent UUID representation used by macros.
use concerto_foundation::assets::AssetId;

#[test]
fn uuid_bytes_can_be_embedded_as_a_constant() {
    const BYTES: [u8; 16] = [0x42; 16];
    const ID: AssetId = AssetId::from_bytes(BYTES);
    const RESTORED: [u8; 16] = ID.to_bytes();
    assert_eq!(RESTORED, BYTES);
    assert_eq!(ID.simple_hex(), "42424242424242424242424242424242");
}

#[test]
fn from_path_is_the_same_for_the_same_path_and_differs_for_another() {
    let albedo = "models/character.gltf#texture/albedo";
    assert_eq!(AssetId::from_path(albedo), AssetId::from_path(albedo));
    assert_ne!(
        AssetId::from_path(albedo),
        AssetId::from_path("models/character.gltf#texture/normal")
    );
}

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

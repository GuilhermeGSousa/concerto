//! Covers StandardMaterial round-tripping directly through bincode (no DTO)
//! and reporting its texture references for import-time validation.
use concerto_color::Color;
use concerto_foundation::assets::{handle::AssetHandle, Asset, AssetId};
use concerto_render::assets::material::StandardMaterial;
use concerto_render::assets::texture::Texture;

#[test]
fn round_trips_through_bincode_with_weak_texture_handles() {
    let albedo_id = AssetId::from_path("models/character.gltf#texture/albedo");
    let material = StandardMaterial::new(Some(AssetHandle::weak(albedo_id)), None)
        .with_base_color_factor(Color::rgba(0.25, 0.5, 0.75, 1.0));

    let bytes =
        bincode::serialize(&material).expect("StandardMaterial should serialize via bincode");
    let decoded: StandardMaterial = bincode::deserialize(&bytes)
        .expect("StandardMaterial should deserialize from bincode bytes");

    assert_eq!(
        decoded.base_color_texture().map(|h| h.id()),
        Some(albedo_id),
        "the deserialized material's texture field must carry the same AssetId, as a Weak handle"
    );
    assert_eq!(
        decoded.base_color_factor(),
        material.base_color_factor(),
        "uniform PBR parameters (base colour, factors, MaterialFlags) must survive the serialized round-trip"
    );
}

#[test]
fn referenced_sub_assets_lists_present_textures_only() {
    let albedo_id = AssetId::from_path("models/character.gltf#texture/albedo");
    let material = StandardMaterial::new(Some(AssetHandle::<Texture>::weak(albedo_id)), None);

    let refs = Asset::referenced_sub_assets(&material);
    assert_eq!(
        refs,
        vec![albedo_id],
        "only Some(..) texture fields should be reported as references"
    );
}

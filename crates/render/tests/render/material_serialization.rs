//! Covers StandardMaterial round-tripping directly through bincode (no DTO)
//! and reporting its texture references for import-time validation.
use std::path::Path;

use concerto_color::{Color, LinearRgba};
use concerto_foundation::assets::content::check_encoding;
use concerto_foundation::assets::{handle::AssetHandle, Asset, AssetId};
use concerto_render::assets::material::StandardMaterial;
use concerto_render::assets::texture::{Texture, TextureFormat, TextureKind};
use glam::Vec3;

fn encoding_dir() -> &'static Path {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/encoding"))
}

fn texture(name: &str) -> AssetHandle<Texture> {
    AssetHandle::weak(AssetId::from_path(name))
}

#[test]
fn a_material_decodes_with_weak_texture_handles_and_lists_only_its_present_textures() {
    let albedo_id = AssetId::from_path("models/character.gltf#texture/albedo");
    let material = StandardMaterial::new(Some(AssetHandle::weak(albedo_id)), None)
        .with_base_color_factor(Color::rgba(0.25, 0.5, 0.75, 1.0));

    let bytes = bincode::serialize(&material).unwrap();
    let decoded: StandardMaterial = bincode::deserialize(&bytes).unwrap();

    assert_eq!(
        decoded.base_color_texture().map(|h| h.id()),
        Some(albedo_id)
    );
    assert_eq!(decoded.base_color_factor(), material.base_color_factor());
    assert_eq!(Asset::referenced_sub_assets(&decoded), vec![albedo_id]);
}

#[test]
fn texture_encoding_matches_its_fixture() {
    let texture = Texture {
        width: 2,
        height: 1,
        format: TextureFormat::Rgba8Unorm,
        kind: TextureKind::RenderTarget,
        data: vec![1, 2, 3, 4, 5, 6, 7, 8],
        mips: Vec::new(),
    };

    check_encoding(&texture, encoding_dir());
}

#[test]
fn standard_material_encoding_matches_its_fixture() {
    let mut material = StandardMaterial::new(
        Some(texture("textures/albedo")),
        Some(texture("textures/normal")),
    )
    .with_metallic_roughness_texture(texture("textures/metallic_roughness"))
    .with_emissive_texture(texture("textures/emissive"))
    .with_occlusion_texture(texture("textures/occlusion"))
    .with_base_color_factor(Color::from(LinearRgba::new(0.25, 0.5, 0.75, 0.5)))
    .with_alpha_cutoff(0.25)
    .with_uv_scale([2.0, 3.0]);
    material.set_metallic_factor(0.75);
    material.set_roughness_factor(0.25);
    material.set_emissive_factor(Vec3::new(0.5, 0.25, 0.125));
    material.set_occlusion_strength(0.5);

    check_encoding(&material, encoding_dir());
}

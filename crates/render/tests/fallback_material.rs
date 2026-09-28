//! Covers the magenta fallback material and per-frame slot coverage.
use concerto_color::Color;
use concerto_foundation::assets::{handle::AssetHandle, AssetId};
use concerto_render::{
    assets::material::StandardMaterial,
    components::{
        fallback_material::{fallback_material_asset, SlotCoverage},
        material::SlotBinding,
    },
};

#[test]
fn the_fallback_material_is_opaque_magenta() {
    let material = fallback_material_asset();

    assert_eq!(
        material.base_color_factor(),
        Color::rgba(1.0, 0.0, 1.0, 1.0)
    );
}

#[test]
fn the_fallback_material_samples_no_textures() {
    let material = fallback_material_asset();

    assert!(material.base_color_texture().is_none());
    assert!(material.normal_texture().is_none());
}

fn handle() -> AssetHandle<StandardMaterial> {
    AssetHandle::weak(AssetId::new())
}

#[test]
fn an_all_binding_covers_every_slot() {
    let mut coverage = SlotCoverage::default();

    coverage.cover(&SlotBinding::All(handle()));

    assert!(coverage.is_covered(0));
    assert!(coverage.is_covered(41));
}

#[test]
fn a_per_slot_binding_covers_only_its_populated_slots() {
    let mut coverage = SlotCoverage::default();

    coverage.cover(&SlotBinding::PerSlot(vec![Some(handle()), None]));

    assert!(coverage.is_covered(0));
    assert!(!coverage.is_covered(1));
    assert!(!coverage.is_covered(2), "slots past the binding stay open");
}

#[test]
fn an_unresolved_instance_leaves_nothing_to_the_fallback_until_cleared() {
    let mut coverage = SlotCoverage::unresolved();

    assert!(coverage.is_covered(0));

    coverage.clear();
    assert!(!coverage.is_covered(0));
}

#[test]
fn clearing_uncovers_every_slot() {
    let mut coverage = SlotCoverage::default();
    coverage.cover(&SlotBinding::All(handle()));
    coverage.cover(&SlotBinding::PerSlot(vec![Some(handle())]));

    coverage.clear();

    assert!(!coverage.is_covered(0));
}

#[test]
fn disjoint_bindings_do_not_overlap() {
    let mut coverage = SlotCoverage::default();

    assert!(!coverage.cover(&SlotBinding::PerSlot(vec![Some(handle()), None])));
    assert!(!coverage.cover(&SlotBinding::PerSlot(vec![None, Some(handle())])));
}

#[test]
fn an_overlap_is_reported_once_across_frames() {
    let mut coverage = SlotCoverage::default();
    coverage.cover(&SlotBinding::PerSlot(vec![Some(handle())]));

    assert!(coverage.cover(&SlotBinding::All(handle())));

    coverage.clear();
    coverage.cover(&SlotBinding::PerSlot(vec![Some(handle())]));
    assert!(
        !coverage.cover(&SlotBinding::All(handle())),
        "the overlap was already reported"
    );
}

//! Covers the magenta stand-in applied to primitive slots no material covers,
//! and the per-frame claim bookkeeping that decides which slots those are.
use concerto_color::Color;
use concerto_ecs::World;
use concerto_render::components::fallback_material::{fallback_material_asset, ClaimedSlots};

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

#[test]
fn a_claimed_slot_is_not_left_for_the_fallback() {
    let mut world = World::new();
    let covered = world.spawn(());
    let uncovered = world.spawn(());
    let mut claimed = ClaimedSlots::default();

    assert!(!claimed.claim(covered), "a first claim is not a conflict");

    assert!(claimed.is_claimed(covered));
    assert!(!claimed.is_claimed(uncovered));
}

#[test]
fn a_second_claim_on_one_slot_is_reported_once() {
    let mut world = World::new();
    let entity = world.spawn(());
    let mut claimed = ClaimedSlots::default();

    claimed.claim(entity);

    assert!(claimed.claim(entity), "two material types claimed one slot");
    assert!(
        !claimed.claim(entity),
        "the conflict was already reported for this slot"
    );
}

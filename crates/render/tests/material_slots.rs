//! Covers per-primitive material slot binding.
use concerto_foundation::assets::{handle::AssetHandle, AssetId};
use concerto_render::assets::material::StandardMaterial;
use concerto_render::components::material::{MaterialComponent, SlotBinding};

#[test]
fn all_covers_every_slot_index() {
    let id = AssetId::new();
    let binding = SlotBinding::<StandardMaterial>::All(AssetHandle::weak(id));

    for index in [0, 1, 7, 1000] {
        assert_eq!(
            binding.slot(index).map(AssetHandle::id),
            Some(id),
            "All must answer for slot {index}"
        );
    }
}

#[test]
fn per_slot_covers_only_its_populated_entries() {
    let wood = AssetId::new();
    let rope = AssetId::new();
    let binding = SlotBinding::<StandardMaterial>::PerSlot(vec![
        Some(AssetHandle::weak(wood)),
        None,
        Some(AssetHandle::weak(rope)),
    ]);

    assert_eq!(binding.slot(0).map(AssetHandle::id), Some(wood));
    assert!(binding.slot(1).is_none(), "an empty slot is uncovered");
    assert_eq!(binding.slot(2).map(AssetHandle::id), Some(rope));
    assert!(binding.slot(3).is_none(), "past the end is uncovered");
}

#[test]
fn a_slot_binding_round_trips_through_json() {
    let component = MaterialComponent::<StandardMaterial>::per_slot(vec![
        Some(AssetHandle::weak(AssetId::new())),
        None,
    ]);

    let text = serde_json::to_string(&component).expect("serializes");
    let restored: MaterialComponent<StandardMaterial> =
        serde_json::from_str(&text).expect("deserializes");

    match restored.binding {
        SlotBinding::PerSlot(slots) => {
            assert_eq!(slots.len(), 2);
            assert!(slots[0].is_some());
            assert!(slots[1].is_none());
        }
        SlotBinding::All(_) => panic!("PerSlot must not deserialize as All"),
    }
}

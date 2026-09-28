//! Covers when the per-primitive render-entity fan-out must be rebuilt.
use concerto_ecs::World;
use concerto_foundation::assets::AssetId;
use concerto_render::components::mesh::{fanout_is_stale, RenderMeshFanout};

fn fanout(mesh_asset_id: AssetId, primitive_count: usize) -> RenderMeshFanout {
    let mut world = World::new();
    RenderMeshFanout {
        mesh_asset_id,
        primitives: (0..primitive_count).map(|_| world.spawn(())).collect(),
    }
}

#[test]
fn a_missing_fanout_is_stale() {
    assert!(fanout_is_stale(None, AssetId::new(), 3));
}

#[test]
fn a_fanout_for_a_different_mesh_is_stale() {
    let built = fanout(AssetId::new(), 2);

    assert!(fanout_is_stale(Some(&built), AssetId::new(), 2));
}

#[test]
fn a_fanout_with_the_wrong_primitive_count_is_stale() {
    let id = AssetId::new();
    let built = fanout(id, 1);

    assert!(fanout_is_stale(Some(&built), id, 3));
}

#[test]
fn a_matching_fanout_is_current() {
    let id = AssetId::new();
    let built = fanout(id, 2);

    assert!(!fanout_is_stale(Some(&built), id, 2));
}

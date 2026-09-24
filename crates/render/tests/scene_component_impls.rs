//! Covers that the engine's spawnable components implement `SceneComponent`:
//! a handle-bearing one (`MeshComponent`) preserves the `AssetId` the serialized
//! scene referenced when `apply` runs, and a plain data component
//! (`Transform`) is inserted with its fields intact.
//!
//! The mesh case runs with no `AssetServer` resource in the world: a bare
//! `AssetServer` has no lifetime sender registered for `Mesh`, so
//! `load::<Mesh>` panics inside a unit test. With no server present the
//! impl leaves the handle `Weak`, and the property under test — that the
//! referenced `AssetId` survives `apply` — still holds.
use concerto_ecs::component::scene::{SceneComponent, SceneSpawnContext};
use concerto_ecs::World;
use concerto_foundation::assets::{handle::AssetHandle, AssetId};
use concerto_mesh::mesh::{Mesh, MeshComponent};

#[test]
fn mesh_component_apply_inserts_the_component_preserving_its_asset_id() {
    let mut world = World::default();
    world.register_component_type::<MeshComponent>();
    let entity = world.spawn(());

    let id = AssetId::from_path("models/character.gltf#mesh/0");
    let component = MeshComponent {
        handle: AssetHandle::<Mesh>::weak(id),
    };

    {
        let nodes = [entity];
        let mut ctx = SceneSpawnContext::new(&mut world, &nodes);
        component.apply(entity, &mut ctx);
    }

    let inserted = world
        .get_component_for_entity::<MeshComponent>(entity)
        .expect("MeshComponent::apply must insert the component");
    assert_eq!(
        inserted.handle.id(),
        id,
        "resolving must preserve the AssetId the serialized scene referenced"
    );
}

#[test]
fn transform_apply_inserts_itself_unchanged() {
    use concerto_foundation::transform::Transform;
    use glam::Vec3;

    let mut world = World::default();
    world.register_component_type::<Transform>();
    let entity = world.spawn(());

    let mut transform = Transform::IDENTITY;
    transform.translation = Vec3::new(1.0, 2.0, 3.0);

    {
        let nodes = [entity];
        let mut ctx = SceneSpawnContext::new(&mut world, &nodes);
        transform.clone().apply(entity, &mut ctx);
    }

    let inserted = world
        .get_component_for_entity::<Transform>(entity)
        .expect("Transform::apply must insert the component");
    assert_eq!(
        inserted.translation,
        Vec3::new(1.0, 2.0, 3.0),
        "a plain component must be inserted with its data intact"
    );
}

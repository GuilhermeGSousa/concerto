use concerto_ecs::{Entity, IntoSystem, System, World};
use concerto_foundation::assets::{asset_store::AssetStore, handle::AssetHandle, AssetId};
use concerto_mesh::{update_mesh_bounds, Aabb, AabbSource, Mesh, MeshComponent, Vertex};
use glam::Vec3;

fn mesh(extent: f32) -> Mesh {
    let vertex = |x: f32| Vertex {
        pos_coords: [x, x, x],
        ..Default::default()
    };
    Mesh {
        vertices: vec![vertex(-extent), vertex(extent)],
        indices: vec![],
    }
}

fn world() -> World {
    let mut world = World::new();
    world.insert_resource(AssetStore::<Mesh>::default());
    world
}

fn load(world: &mut World, id: AssetId, mesh: Mesh) {
    world
        .get_resource_mut::<AssetStore<Mesh>>()
        .unwrap()
        .insert(id, mesh);
}

fn with_mesh(world: &mut World, id: AssetId) -> Entity {
    world.spawn(MeshComponent {
        handle: AssetHandle::weak(id),
    })
}

fn run(world: &mut World) {
    let mut system = update_mesh_bounds.into_system();
    system.initialize(world);
    system.run_and_apply((), world);
}

fn bounds(world: &World, entity: Entity) -> Option<Aabb> {
    world.get_component_for_entity::<Aabb>(entity).copied()
}

fn cube(extent: f32) -> Aabb {
    Aabb {
        min: Vec3::splat(-extent),
        max: Vec3::splat(extent),
    }
}

fn handle(world: &mut World, entity: Entity, id: AssetId) {
    world
        .get_component_for_entity_mut::<MeshComponent>(entity)
        .unwrap()
        .handle = AssetHandle::weak(id);
}

fn computed(world: &World, entity: Entity) -> bool {
    world
        .get_component_for_entity::<AabbSource>(entity)
        .is_some()
}

#[test]
fn a_mesh_entity_gets_the_box_of_its_loaded_mesh_unless_it_set_its_own() {
    let mut world = world();
    let (small, large, empty) = (AssetId::new(), AssetId::new(), AssetId::new());
    let entity = with_mesh(&mut world, small);
    run(&mut world);
    assert_eq!(bounds(&world, entity), None, "the mesh has not loaded");

    load(&mut world, small, mesh(2.0));
    load(&mut world, large, mesh(5.0));
    load(
        &mut world,
        empty,
        Mesh {
            vertices: vec![],
            indices: vec![],
        },
    );
    let hand_set = with_mesh(&mut world, small);
    world.insert(cube(9.0), hand_set);
    run(&mut world);
    assert_eq!(bounds(&world, entity), Some(cube(2.0)));
    assert!(computed(&world, entity));
    assert_eq!(bounds(&world, hand_set), Some(cube(9.0)));
    assert!(!computed(&world, hand_set));

    handle(&mut world, entity, large);
    run(&mut world);
    assert_eq!(bounds(&world, entity), Some(cube(5.0)));

    handle(&mut world, entity, empty);
    run(&mut world);
    assert_eq!(
        bounds(&world, entity),
        None,
        "an empty mesh drops the stale box"
    );

    handle(&mut world, entity, small);
    run(&mut world);
    world.remove_component::<MeshComponent>(entity);
    run(&mut world);
    assert_eq!(bounds(&world, entity), None);
    assert!(!computed(&world, entity));
}

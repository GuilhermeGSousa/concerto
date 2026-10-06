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

#[test]
fn the_box_appears_once_the_mesh_has_loaded_and_not_before() {
    let mut world = world();
    let id = AssetId::new();
    let entity = with_mesh(&mut world, id);
    run(&mut world);
    assert_eq!(bounds(&world, entity), None);

    load(&mut world, id, mesh(2.0));
    run(&mut world);
    assert_eq!(bounds(&world, entity), Some(cube(2.0)));
    assert!(world
        .get_component_for_entity::<AabbSource>(entity)
        .is_some());
}

#[test]
fn the_box_follows_a_changed_handle() {
    let mut world = world();
    let (small, large) = (AssetId::new(), AssetId::new());
    load(&mut world, small, mesh(1.0));
    load(&mut world, large, mesh(5.0));
    let entity = with_mesh(&mut world, small);
    run(&mut world);
    assert_eq!(bounds(&world, entity), Some(cube(1.0)));

    world
        .get_component_for_entity_mut::<MeshComponent>(entity)
        .unwrap()
        .handle = AssetHandle::weak(large);
    run(&mut world);
    assert_eq!(bounds(&world, entity), Some(cube(5.0)));
}

#[test]
fn a_hand_set_box_is_left_alone() {
    let mut world = world();
    let id = AssetId::new();
    load(&mut world, id, mesh(1.0));
    let entity = with_mesh(&mut world, id);
    world.insert(cube(9.0), entity);
    run(&mut world);
    assert_eq!(bounds(&world, entity), Some(cube(9.0)));
    assert!(world
        .get_component_for_entity::<AabbSource>(entity)
        .is_none());
}

#[test]
fn a_computed_box_is_removed_with_the_mesh() {
    let mut world = world();
    let id = AssetId::new();
    load(&mut world, id, mesh(1.0));
    let entity = with_mesh(&mut world, id);
    run(&mut world);
    world.remove_component::<MeshComponent>(entity);
    run(&mut world);
    assert_eq!(bounds(&world, entity), None);
    assert!(world
        .get_component_for_entity::<AabbSource>(entity)
        .is_none());
}

#[test]
fn an_empty_mesh_gets_no_box_and_drops_a_stale_one() {
    let mut world = world();
    let (solid, empty) = (AssetId::new(), AssetId::new());
    load(&mut world, solid, mesh(1.0));
    load(
        &mut world,
        empty,
        Mesh {
            vertices: vec![],
            indices: vec![],
        },
    );
    let entity = with_mesh(&mut world, solid);
    run(&mut world);
    world
        .get_component_for_entity_mut::<MeshComponent>(entity)
        .unwrap()
        .handle = AssetHandle::weak(empty);
    run(&mut world);
    assert_eq!(bounds(&world, entity), None);
}

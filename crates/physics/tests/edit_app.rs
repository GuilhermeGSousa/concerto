use concerto_app::App;
use concerto_physics::{
    collider::BoxCollider, physics_pipeline::PhysicsPipeline, physics_state::PhysicsState,
    plugin::PhysicsPlugin,
};
use concerto_scene::capture::capture_subtree;
use glam::Vec3;

#[test]
fn an_edit_app_has_the_physics_world_and_no_stepper() {
    let mut app = App::for_editing();
    app.register_plugin(PhysicsPlugin);
    assert!(app.get_resource::<PhysicsState>().is_some());
    assert!(app.get_resource::<PhysicsPipeline>().is_none());
}

#[test]
fn a_game_app_has_both() {
    let mut app = App::new();
    app.register_plugin(PhysicsPlugin);
    assert!(app.get_resource::<PhysicsState>().is_some());
    assert!(app.get_resource::<PhysicsPipeline>().is_some());
}

#[test]
fn an_edit_app_opens_and_saves_a_mesh_collider() {
    let mut app = App::for_editing();
    app.register_plugin(PhysicsPlugin);
    let world = app.main_mut().world_mut();
    let entity = world.spawn(());

    let stored = concerto_reflect::to_value(&concerto_physics::shape::MeshCollider).unwrap();
    assert!(world.apply_scene_component("MeshCollider", &stored, entity, &[entity]));

    let scene = capture_subtree(world, entity).unwrap();
    let names: Vec<&str> = scene.nodes[0]
        .components
        .iter()
        .map(|component| component.type_name.as_str())
        .collect();
    assert_eq!(names, ["concerto_physics::shape::MeshCollider"]);
}

#[test]
fn an_edit_app_opens_and_saves_a_box_collider() {
    let mut app = App::for_editing();
    app.register_plugin(PhysicsPlugin);
    let world = app.main_mut().world_mut();
    let entity = world.spawn(());

    let authored = BoxCollider {
        half_extents: Vec3::new(2.0, 0.5, 3.0),
    };
    let stored = concerto_reflect::to_value(&authored).unwrap();
    assert!(world.apply_scene_component("BoxCollider", &stored, entity, &[entity]));
    assert_eq!(
        world
            .get_component_for_entity::<BoxCollider>(entity)
            .unwrap()
            .half_extents,
        authored.half_extents
    );

    let scene = capture_subtree(world, entity).unwrap();
    let names: Vec<&str> = scene.nodes[0]
        .components
        .iter()
        .map(|component| component.type_name.as_str())
        .collect();
    assert_eq!(names, ["concerto_physics::collider::BoxCollider"]);
}

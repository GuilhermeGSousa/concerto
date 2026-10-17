use concerto_app::App;
use concerto_physics::{
    physics_pipeline::PhysicsPipeline, physics_state::PhysicsState, plugin::PhysicsPlugin,
};
use concerto_scene::capture::capture_subtree;

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

    assert!(world.apply_scene_component("MeshCollider", "{}", entity, &[entity]));

    let scene = capture_subtree(world, entity).unwrap();
    let names: Vec<&str> = scene.nodes[0]
        .components
        .iter()
        .map(|component| component.type_name.as_str())
        .collect();
    assert_eq!(names, ["concerto_physics::shape::MeshCollider"]);
}

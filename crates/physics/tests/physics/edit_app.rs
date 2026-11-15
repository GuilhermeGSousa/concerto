use concerto_app::App;
use concerto_physics::{
    collider::BoxCollider, physics_pipeline::PhysicsPipeline, physics_state::PhysicsState,
    plugin::PhysicsPlugin, sensor::Sensor, shape::MeshCollider,
};
use concerto_scene::capture::capture_subtree;
use glam::Vec3;

#[test]
fn an_edit_app_has_the_physics_world_and_only_a_game_app_steps_it() {
    let mut edit = App::for_editing();
    edit.register_plugin(PhysicsPlugin);
    assert!(edit.get_resource::<PhysicsState>().is_some());
    assert!(edit.get_resource::<PhysicsPipeline>().is_none());

    let mut game = App::new();
    game.register_plugin(PhysicsPlugin);
    assert!(game.get_resource::<PhysicsState>().is_some());
    assert!(game.get_resource::<PhysicsPipeline>().is_some());
}

struct Authored {
    name: &'static str,
    stored: concerto_reflect::ReflectedValue,
    type_name: &'static str,
}

#[test]
fn an_edit_app_opens_and_saves_each_collider_component() {
    let components = [
        Authored {
            name: "MeshCollider",
            stored: concerto_reflect::to_value(&MeshCollider).unwrap(),
            type_name: "concerto_physics::shape::MeshCollider",
        },
        Authored {
            name: "BoxCollider",
            stored: concerto_reflect::to_value(&BoxCollider {
                half_extents: Vec3::new(2.0, 0.5, 3.0),
            })
            .unwrap(),
            type_name: "concerto_physics::collider::BoxCollider",
        },
        Authored {
            name: "Sensor",
            stored: concerto_reflect::to_value(&Sensor).unwrap(),
            type_name: "concerto_physics::sensor::Sensor",
        },
    ];
    for authored in components {
        let mut app = App::for_editing();
        app.register_plugin(PhysicsPlugin);
        let world = app.main_mut().world_mut();
        let entity = world.spawn(());

        assert!(
            world.apply_scene_component(authored.name, &authored.stored, entity, &[entity]),
            "{}",
            authored.name
        );

        let scene = capture_subtree(world, entity).unwrap();
        assert_eq!(scene.nodes[0].components.len(), 1, "{}", authored.name);
        assert_eq!(scene.nodes[0].components[0].type_name, authored.type_name);
        assert_eq!(scene.nodes[0].components[0].value, authored.stored);
    }
}

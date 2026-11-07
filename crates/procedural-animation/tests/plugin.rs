use std::any::TypeId;

use concerto_animation::node::ReflectAnimationNode;
use concerto_app::App;
use concerto_ecs::reflect::AppTypeRegistry;
use concerto_procedural_animation::{
    AimBone, AimNode, FootPlacementLeg, FootPlacementNode, LookAtBone, LookAtEye, LookAtNode,
    Muzzle, ProceduralAnimationPlugin,
};

#[test]
fn the_plugin_registers_the_foot_placement_node_and_its_leg() {
    let mut app = App::new();
    app.register_plugin(ProceduralAnimationPlugin);

    let registry = app.get_resource::<AppTypeRegistry>().unwrap().read();
    assert!(
        registry
            .get_type_data::<ReflectAnimationNode>(TypeId::of::<FootPlacementNode>())
            .is_some()
    );
    assert!(registry.get(TypeId::of::<FootPlacementLeg>()).is_some());
}

#[test]
fn the_plugin_registers_the_look_at_node_and_its_bones_and_eyes() {
    let mut app = App::new();
    app.register_plugin(ProceduralAnimationPlugin);

    let registry = app.get_resource::<AppTypeRegistry>().unwrap().read();
    assert!(
        registry
            .get_type_data::<ReflectAnimationNode>(TypeId::of::<LookAtNode>())
            .is_some()
    );
    assert!(registry.get(TypeId::of::<LookAtBone>()).is_some());
    assert!(registry.get(TypeId::of::<LookAtEye>()).is_some());
}

#[test]
fn the_plugin_registers_the_aim_node_and_its_bones_and_muzzle() {
    let mut app = App::new();
    app.register_plugin(ProceduralAnimationPlugin);

    let registry = app.get_resource::<AppTypeRegistry>().unwrap().read();
    assert!(
        registry
            .get_type_data::<ReflectAnimationNode>(TypeId::of::<AimNode>())
            .is_some()
    );
    assert!(registry.get(TypeId::of::<AimBone>()).is_some());
    assert!(registry.get(TypeId::of::<Muzzle>()).is_some());
}

use std::any::TypeId;

use concerto_animation::node::ReflectAnimationNode;
use concerto_app::App;
use concerto_ecs::reflect::AppTypeRegistry;
use concerto_procedural_animation::{
    FootPlacementLeg, FootPlacementNode, ProceduralAnimationPlugin,
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

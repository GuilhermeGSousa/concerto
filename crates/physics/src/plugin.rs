use app::{
    plugins::Plugin,
    schedule_groups::{FixedUpdate, LateFixedUpdate, LateUpdate, Update},
};

use crate::{
    collider::register_colliders,
    ground::probe_ground,
    interpolation::interpolate_body_transforms,
    movement::apply_character_movement,
    physics_pipeline::PhysicsPipeline,
    physics_state::PhysicsState,
    shape::{
        clean_shapes_for_dropped_meshes, generate_mesh_shapes, MeshCollider, PhysicsMeshShapes,
    },
    simulation::step_simulation,
};

pub struct PhysicsPlugin;

impl Plugin for PhysicsPlugin {
    fn build(&self, app: &mut app::App) {
        app.register_scene_component::<MeshCollider>();
        app.insert_resource(PhysicsPipeline::new())
            .insert_resource(PhysicsState::new())
            .insert_resource(PhysicsMeshShapes::default())
            .add_system(FixedUpdate, apply_character_movement)
            .add_system(LateFixedUpdate, step_simulation)
            .add_system(LateFixedUpdate, probe_ground)
            .add_system(Update, generate_mesh_shapes)
            // LateUpdate, so `TransformPlugin` has already propagated global
            // transforms this frame: bodies are placed from world space, and
            // a collider on a nested entity inherits its parent's scale.
            .add_system(LateUpdate, register_colliders)
            .add_system(Update, interpolate_body_transforms)
            .add_system(Update, clean_shapes_for_dropped_meshes);
    }
}

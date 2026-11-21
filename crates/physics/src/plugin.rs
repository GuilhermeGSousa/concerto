use concerto_app::{
    plugins::Plugin,
    schedule_groups::{LateFixedUpdate, LateUpdate, Update},
};
use concerto_ecs::IntoSystemConfig;

use crate::{
    collider::{build_box_colliders, register_colliders, BoxCollider},
    contact::report_contacts,
    ground::probe_ground,
    interpolation::interpolate_body_transforms,
    layers::{apply_collision_layers, CollisionLayers},
    physics_pipeline::PhysicsPipeline,
    physics_state::PhysicsState,
    sensor::{report_sensor_contacts, Sensor},
    settings::PhysicsSettings,
    shape::{
        clean_shapes_for_dropped_meshes, generate_mesh_shapes, MeshCollider, PhysicsMeshShapes,
    },
    simulation::step_simulation,
};

pub struct PhysicsPlugin;

impl Plugin for PhysicsPlugin {
    fn build(&self, app: &mut concerto_app::App) {
        app.register_type::<MeshCollider>();
        app.register_type::<BoxCollider>();
        app.register_type::<Sensor>();
        app.register_type::<CollisionLayers>();
        if !app.is_editing() {
            app.insert_resource(PhysicsPipeline::new());
        }
        app.insert_resource(PhysicsState::new())
            .insert_resource(PhysicsSettings::default())
            .insert_resource(PhysicsMeshShapes::default())
            .add_system(LateFixedUpdate, step_simulation)
            .add_system(LateFixedUpdate, probe_ground)
            .add_system(
                LateFixedUpdate,
                report_sensor_contacts.after(step_simulation),
            )
            .add_system(LateFixedUpdate, report_contacts.after(step_simulation))
            .add_system(Update, generate_mesh_shapes)
            .add_system(Update, build_box_colliders)
            // LateUpdate, so `TransformPlugin` has already propagated global
            // transforms this frame: bodies are placed from world space, and
            // a collider on a nested entity inherits its parent's scale.
            .add_system(LateUpdate, register_colliders)
            .add_system(LateUpdate, apply_collision_layers.after(register_colliders))
            .add_system(Update, interpolate_body_transforms)
            .add_system(Update, clean_shapes_for_dropped_meshes);
    }
}

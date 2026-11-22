//! The physics gizmos in the editor's viewport, as an extension of `concerto-editor`.
use concerto_app::App;
use concerto_editor::{scene::SceneToolsAppExt, tools::component_gizmos::ComponentGizmosAppExt};
use concerto_physics_gizmos::{
    BodySenseGizmo, BoxColliderGizmo, ColliderGizmo, ContactsGizmo, MeshColliderGizmo,
    RagdollGizmo, SensorGizmo,
};

/// Draws each `Collider`, `RagdollId`, `Contacts`, `BoxCollider`, `MeshCollider` and `Sensor` in every edit and play `App` while its entity is selected.
pub fn install(app: &mut App) {
    app.add_scene_tools_plugin(register_gizmos);
}

fn register_gizmos(app: &mut App) {
    app.register_component_gizmo::<ColliderGizmo>()
        .register_component_gizmo::<RagdollGizmo>()
        .register_component_gizmo::<ContactsGizmo>()
        .register_component_gizmo::<BoxColliderGizmo>()
        .register_component_gizmo::<MeshColliderGizmo>()
        .register_component_gizmo::<SensorGizmo>()
        .register_component_gizmo::<BodySenseGizmo>();
}

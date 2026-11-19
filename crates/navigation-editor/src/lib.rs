//! The navmesh gizmo in the editor's viewport, as an extension of `concerto-editor`.
use concerto_app::App;
use concerto_editor::{scene::SceneToolsAppExt, tools::component_gizmos::ComponentGizmosAppExt};
use concerto_navigation_gizmos::NavMeshGizmo;

/// Draws each `NavMeshComponent`'s navmesh in every edit and play `App` while its entity is selected.
pub fn install(app: &mut App) {
    app.add_scene_tools_plugin(register_gizmos);
}

fn register_gizmos(app: &mut App) {
    app.register_component_gizmo::<NavMeshGizmo>();
}

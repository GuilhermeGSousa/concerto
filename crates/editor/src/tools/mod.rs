//! Everything that runs in the world that holds the scene being edited.
pub mod camera;
pub mod edits;
pub mod entity_ops;
pub mod gizmo;
pub mod picking;
pub mod scene;
pub mod selection;

use concerto_app::{App, Plugin};
use concerto_debug_gizmos::DebugGizmosPlugin;
use concerto_render::plugin::RenderPlugin;

/// The scene tools, for a recipe to host: editing always, the viewport when the recipe has a renderer.
pub struct EditToolsPlugin;

impl Plugin for EditToolsPlugin {
    fn build(&self, app: &mut App) {
        app.register_plugin(EditToolsCorePlugin);
        if app.is_plugin_added::<RenderPlugin>() {
            if !app.is_plugin_added::<DebugGizmosPlugin>() {
                app.register_plugin(DebugGizmosPlugin);
            }
            build_viewport(app);
        }
    }
}

/// The scene tools that select, edit, and capture entities, without the viewport; a play `App` gets only these.
pub struct EditToolsCorePlugin;

impl Plugin for EditToolsCorePlugin {
    fn build(&self, app: &mut App) {
        build_core(app);
    }
}

pub(crate) fn build_core(app: &mut App) {
    scene::build(app);
    selection::build(app);
    entity_ops::build(app);
    edits::build(app);
}

pub(crate) fn build_viewport(app: &mut App) {
    camera::build(app);
    picking::build(app);
    gizmo::build(app);
}

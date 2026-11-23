//! Everything that runs in the world that holds the scene being edited.
pub mod camera;
pub mod component_gizmos;
pub mod edits;
pub mod entity_ops;
pub mod environment;
pub mod gizmo;
pub(crate) mod links;
pub mod picking;
pub mod placement;
pub mod scene;
pub mod selection;

use concerto_app::{App, Plugin};
use concerto_ecs::{component::name::Name, entity::hierarchy::Children};
use concerto_mesh::MeshComponent;
use concerto_render::{
    components::{camera::Camera, light::Light},
    plugin::RenderPlugin,
};

/// The scene tools, for a recipe to host: editing always, the viewport when the recipe has a renderer.
pub struct EditToolsPlugin;

impl Plugin for EditToolsPlugin {
    fn build(&self, app: &mut App) {
        app.register_plugin(EditToolsCorePlugin);
        if app.is_plugin_added::<RenderPlugin>() {
            build_viewport(app);
        }
    }
}

/// The scene tools that select, edit, and capture entities, and the component gizmos, without the viewport; a play `App` gets only these.
pub struct EditToolsCorePlugin;

impl Plugin for EditToolsCorePlugin {
    fn build(&self, app: &mut App) {
        build_core(app);
    }
}

pub(crate) fn build_core(app: &mut App) {
    app.track_removals::<Children>()
        .track_removals::<Name>()
        .track_removals::<Camera>()
        .track_removals::<Light>()
        .track_removals::<MeshComponent>();
    scene::build(app);
    selection::build(app);
    entity_ops::build(app);
    edits::build(app);
    links::build_tools(app);
    placement::build(app);
    component_gizmos::build(app);
}

pub(crate) fn build_viewport(app: &mut App) {
    camera::build(app);
    app.register_plugin(environment::ViewportEnvironmentPlugin);
    picking::build(app);
    gizmo::build(app);
}

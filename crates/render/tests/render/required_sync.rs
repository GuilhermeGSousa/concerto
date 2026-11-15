use concerto_app::App;
use concerto_render::{
    components::render_entity::SyncWithRenderWorld, plugin::register_sync_requirements,
};

fn bare_app() -> App {
    let mut app = App::new();
    register_sync_requirements(&mut app);
    app
}

#[test]
fn a_bare_mesh_or_skeleton_gets_the_marker() {
    let mut app = bare_app();
    let world = app.main_mut().world_mut();
    let mesh = world.spawn(concerto_mesh::MeshComponent::default());
    let skeleton = world.spawn(concerto_mesh::SkeletonComponent::default());

    for entity in [mesh, skeleton] {
        assert!(world
            .get_component_for_entity::<SyncWithRenderWorld>(entity)
            .is_some());
    }
}

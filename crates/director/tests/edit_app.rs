use concerto_app::App;
use concerto_director::CameraDirectorPlugin;
use concerto_scene::capture::capture_subtree;

#[test]
fn an_edit_app_opens_and_saves_a_virtual_camera() {
    let mut app = App::for_editing();
    app.register_plugin(CameraDirectorPlugin);
    let world = app.main_mut().world_mut();
    let entity = world.spawn(());

    assert!(world.apply_scene_component("VirtualCamera", r#"{"priority":5}"#, entity, &[entity]));

    let scene = capture_subtree(world, entity).unwrap();
    let names: Vec<&str> = scene.nodes[0]
        .components
        .iter()
        .map(|component| component.type_name.as_str())
        .collect();
    assert_eq!(names, ["concerto_director::virtual_camera::VirtualCamera"]);
}

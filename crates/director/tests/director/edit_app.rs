use concerto_app::App;
use concerto_director::{CameraDirector, CameraDirectorPlugin, VirtualCamera};
use concerto_reflect::ReflectedValue;
use concerto_scene::scene::Scene;

#[test]
fn an_edit_app_opens_and_saves_a_virtual_camera_whose_missing_fields_keep_their_defaults() {
    let mut app = App::for_editing();
    app.register_plugin(CameraDirectorPlugin);
    app.finish_plugin_build();
    let world = app.main_mut().world_mut();
    let camera = world.spawn(());
    let shot = world.spawn(());

    let camera_fields = ReflectedValue::Struct(vec![(
        "priority".to_owned(),
        ReflectedValue::encode(&7_i32).unwrap(),
    )]);
    let shot_fields = ReflectedValue::Struct(vec![
        (
            "priority".to_owned(),
            ReflectedValue::encode(&9_i32).unwrap(),
        ),
        (
            "enabled".to_owned(),
            ReflectedValue::encode(&false).unwrap(),
        ),
    ]);
    assert!(world.apply_scene_component("VirtualCamera", &camera_fields, camera, &[camera]));
    assert!(world.apply_scene_component("VirtualCamera", &shot_fields, shot, &[shot]));

    let director = world.get_resource::<CameraDirector>().unwrap();
    assert_eq!(director.live(), Some(camera));
    assert!(!director.is_enabled(shot));
    assert_eq!(
        world
            .get_component_for_entity::<VirtualCamera>(camera)
            .unwrap()
            .priority(),
        7
    );

    let scene = Scene::capture_subtree(world, camera).unwrap();
    let names: Vec<&str> = scene.nodes[0]
        .components
        .iter()
        .map(|component| component.type_name.as_str())
        .collect();
    assert_eq!(names, ["concerto_director::virtual_camera::VirtualCamera"]);
}

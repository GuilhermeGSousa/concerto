//! Enter in the staging dialog queues the batch, with the settings typed into it, and hands the keys back.
use crate::example;

use concerto_app::{App, plugins::AssetManagerPlugin};
use concerto_ecs::{
    Entity, World,
    entity::hierarchy::{ChildOf, Children},
    events::event_channel::EventChannel,
};
use concerto_editor::{
    actions::{ConfirmImport, ImportDialogContext},
    guard::UnsavedGuard,
    import::{ImportPlugin, ImportQueue, ImportStaging, accept_picked, scan_content},
    inspector::{PropertyRow, ReflectRowsPlugin},
    project::{AssetsWritten, Project, ProjectState},
};
use concerto_foundation::assets::registry::AssetRegistry;
use concerto_reflect::ReflectRef;
use concerto_ui::{
    focus::FocusedWidget,
    items::UIItemSelected,
    text::UIText,
    text_input::{UITextInput, UITextInputChanged, UITextInputSubmitted},
};
use concerto_window::input::actions::{ActionFired, ActionMap};
use std::path::{Path, PathBuf};

const WALK: &str = "HIERARCHY
ROOT Hips
{
	OFFSET 0.0 90.0 0.0
	CHANNELS 6 Xposition Yposition Zposition Zrotation Yrotation Xrotation
	End Site
	{
		OFFSET 0.0 5.0 0.0
	}
}
MOTION
Frames: 2
Frame Time: 0.0333333
0 90 0 0 0 0
1 90 0 10 0 0
";

fn editor(project: &Path, sources: Vec<PathBuf>) -> App {
    let (mut app, _) = example::headless_editor(
        |_| {},
        |editor| {
            let mut state = ProjectState::default();
            state.project = Some(Project {
                root: project.to_path_buf(),
                content_root: "content".into(),
                default_scene: None,
            });
            editor
                .register_plugin(AssetManagerPlugin)
                .insert_resource(state)
                .insert_resource(UnsavedGuard::default())
                .insert_resource(AssetRegistry::default())
                .register_event::<UITextInputChanged>()
                .register_event::<AssetsWritten>()
                .register_plugin(ImportPlugin)
                .register_plugin(ReflectRowsPlugin);
            concerto_ui::list::install_listeners(editor.main_mut().world_mut());
            concerto_ui::items::install_listeners(editor.main_mut().world_mut());
        },
    );
    let world = app.main_mut().world_mut();
    let mut state = world.remove_resource::<ProjectState>().expect("state");
    let mut staging = ImportStaging::default();
    accept_picked(Some(sources), &mut staging, &mut state);
    world.insert_resource(state);
    world.insert_resource(staging);
    app.update();
    app
}

fn labelled(world: &mut World, label: &str) -> Entity {
    let text = world
        .query::<(Entity, &UIText), ()>()
        .iter(world)
        .find(|(_, text)| text.text == label)
        .map(|(entity, _)| entity)
        .expect("a label");
    world
        .get_component_for_entity::<ChildOf>(text)
        .expect("a parent")
        .parent()
}

fn text_field_of_row(world: &mut World, path: &str) -> Entity {
    let row = world
        .query::<(Entity, &PropertyRow), ()>()
        .iter(world)
        .find(|(_, row)| row.address.path.to_string() == path)
        .map(|(entity, _)| entity)
        .expect("a settings row");
    let mut pending = vec![row];
    while let Some(entity) = pending.pop() {
        if world
            .get_component_for_entity::<UITextInput>(entity)
            .is_some()
        {
            return entity;
        }
        if let Some(children) = world.get_component_for_entity::<Children>(entity) {
            pending.extend(children.iter().copied());
        }
    }
    panic!("the row has no text field");
}

fn drain(app: &mut App) {
    for _ in 0..500 {
        app.update();
        let queue = app.get_resource::<ImportQueue>().expect("queue");
        let staging = app.get_resource::<ImportStaging>().expect("staging");
        if !staging.visible && queue.remaining() == 0 && !queue.is_running() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("the queue never drained");
}

#[test]
fn the_confirm_key_queues_the_batch_once_and_releases_the_key_context() {
    let sources = tempfile::tempdir().expect("tempdir");
    let picked: Vec<PathBuf> = ["walk.bvh", "run.bvh"]
        .iter()
        .map(|name| {
            let path = sources.path().join(name);
            std::fs::write(&path, WALK).expect("write bvh");
            path
        })
        .collect();
    let project = tempfile::tempdir().expect("tempdir");
    let mut app = editor(project.path(), picked);
    assert!(
        app.get_resource::<ActionMap>()
            .expect("actions")
            .is_active(ImportDialogContext),
        "an open dialog owns Enter and Escape"
    );

    let world = app.main_mut().world_mut();
    let walk = labelled(world, "walk.bvh");
    world.trigger_on(walk, UIItemSelected);
    app.update();
    app.update();
    let world = app.main_mut().world_mut();
    let scale = text_field_of_row(world, "scale");
    **world.get_resource_mut::<FocusedWidget>().expect("focus") = Some(scale);
    world
        .get_component_for_entity_mut::<UITextInput>(scale)
        .expect("field")
        .value = "0.5".into();
    world
        .get_resource_mut::<EventChannel<UITextInputSubmitted>>()
        .expect("channel")
        .push_event(UITextInputSubmitted {
            entity: scale,
            value: "0.5".into(),
        });
    world
        .get_resource_mut::<EventChannel<ActionFired>>()
        .expect("channel")
        .push_event(ActionFired::new(ConfirmImport));
    drain(&mut app);

    assert!(
        !app.get_resource::<ImportStaging>()
            .expect("staging")
            .visible
    );
    assert!(
        !app.get_resource::<ActionMap>()
            .expect("actions")
            .is_active(ImportDialogContext)
    );
    let staged = scan_content(project.path())
        .expect("the project scans")
        .staged_settings(&project.path().join("assets/walk.bvh"), "assets/walk.bvh")
        .expect("the walk stages from its record");
    let ReflectRef::Struct(fields) = staged.settings.reflect_ref() else {
        panic!("BVH settings are a struct");
    };
    assert_eq!(
        fields
            .field("scale")
            .and_then(|scale| scale.downcast_ref::<f32>()),
        Some(&0.5)
    );
    assert!(project.path().join("assets/run.bvh").exists());
}

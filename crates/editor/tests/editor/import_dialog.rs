//! Enter in the staging dialog queues the batch and hands the keys back.
use concerto_app::{App, schedule_groups::LateUpdate};
use concerto_ecs::events::event_channel::EventChannel;
use concerto_editor::{
    actions::{ConfirmImport, ImportDialogContext},
    guard::UnsavedGuard,
    import::{ImportPlugin, ImportQueue, ImportStaging, stage_sources},
    project::{Project, ProjectState},
};
use concerto_ui::{focus::FocusedWidget, text_input::UITextInputChanged, theme::UITheme};
use concerto_window::input::actions::{ActionFired, ActionMap};

fn staged(sources: &[&str]) -> ImportStaging {
    ImportStaging {
        rows: stage_sources(sources.iter().map(std::path::PathBuf::from).collect()),
        visible: true,
    }
}

fn editor(project: &std::path::Path, sources: &[&str]) -> App {
    let mut app = App::new();
    app.insert_resource(UITheme::default());
    app.insert_resource(ActionMap::default());
    app.insert_resource(FocusedWidget::default());
    app.insert_resource(UnsavedGuard::default());
    app.register_event::<UITextInputChanged>();
    app.register_event::<ActionFired>();
    let mut state = ProjectState::default();
    state.project = Some(Project {
        root: project.to_path_buf(),
        content_root: "content".into(),
        default_scene: None,
    });
    app.insert_resource(state);
    concerto_ui::list::install_listeners(app.main_mut().world_mut());
    app.register_plugin(ImportPlugin);
    app.finish_plugin_build();
    *app.get_resource_mut::<ImportStaging>().expect("staging") = staged(sources);
    frame(&mut app);
    app
}

fn frame(app: &mut App) {
    app.main_mut().world_mut().run_schedule(LateUpdate);
}

#[test]
fn the_confirm_key_queues_the_batch_once_and_releases_the_key_context() {
    let project = tempfile::tempdir().expect("tempdir");
    let mut app = editor(project.path(), &["/tmp/hero.obj", "/tmp/tree.obj"]);
    assert!(
        app.get_resource::<ActionMap>()
            .expect("actions")
            .is_active(ImportDialogContext),
        "an open dialog owns Enter and Escape"
    );

    app.get_resource_mut::<EventChannel<ActionFired>>()
        .expect("channel")
        .push_event(ActionFired::new(ConfirmImport));
    frame(&mut app);
    frame(&mut app);

    assert_eq!(
        app.get_resource::<ImportQueue>()
            .expect("queue")
            .remaining(),
        2
    );
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
}

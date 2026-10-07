//! The staging dialog queues exactly the importable rows, and only when it may.
use concerto_app::{App, schedule_groups::LateUpdate};
use concerto_ecs::{Entity, events::event_channel::EventChannel};
use concerto_editor::{
    actions::{ConfirmImport, ImportDialogContext},
    import::{ImportPlugin, ImportQueue, ImportStaging, RowState, dialog, stage_sources},
    project::{Project, ProjectState},
};
use concerto_foundation::assets::content::AssetRegistry;
use concerto_ui::{
    focus::FocusedWidget,
    text_input::{UITextInput, UITextInputChanged},
    theme::UITheme,
};
use concerto_window::input::actions::{ActionFired, ActionLabel, ActionMap};

fn staged(sources: &[&str]) -> ImportStaging {
    ImportStaging {
        rows: stage_sources(sources.iter().map(std::path::PathBuf::from).collect()),
        visible: true,
    }
}

fn reject(staging: &mut ImportStaging, row: usize) {
    staging.rows[row].state = RowState::Rejected("no importer handles '.txt'".into());
}

#[test]
fn confirming_queues_only_the_importable_rows() {
    let mut staging = staged(&["/tmp/hero.obj", "/tmp/notes.txt"]);
    reject(&mut staging, 1);
    let mut queue = ImportQueue::default();

    assert!(dialog::can_confirm(&staging));
    dialog::confirm(&mut staging, &mut queue);

    assert_eq!(queue.remaining(), 1, "the rejected row is not queued");
    assert!(!staging.visible, "confirming closes the dialog");
    assert!(staging.rows.is_empty(), "confirming clears the staged rows");
}

#[test]
fn confirming_with_no_importable_rows_queues_nothing() {
    let mut staging = staged(&["/tmp/notes.txt"]);
    reject(&mut staging, 0);
    let mut queue = ImportQueue::default();

    assert!(!dialog::can_confirm(&staging));
    dialog::confirm(&mut staging, &mut queue);

    assert_eq!(queue.remaining(), 0);
    assert!(!queue.is_running());
    assert!(staging.visible, "an inert confirm leaves the dialog open");
    assert_eq!(staging.rows.len(), 1, "an inert confirm keeps the rows");
}

#[test]
fn confirming_while_two_rows_share_a_destination_queues_nothing() {
    let mut staging = staged(&["/one/hero.obj", "/two/hero.obj", "/tmp/tree.obj"]);
    let mut queue = ImportQueue::default();

    assert!(!dialog::can_confirm(&staging));
    dialog::confirm(&mut staging, &mut queue);

    assert_eq!(queue.remaining(), 0, "not even the good row is queued");
    assert!(
        staging.visible,
        "the dialog stays open until the clash is fixed"
    );
    assert_eq!(staging.rows.len(), 3);
}

#[test]
fn destinations_that_normalise_to_the_same_path_block_confirming() {
    let mut staging = staged(&["/one/hero.obj", "/two/tree.obj"]);
    staging.rows[1].destination = "assets//hero.obj".into();

    assert!(!dialog::can_confirm(&staging));
}

#[test]
fn fixing_a_clash_makes_the_batch_confirmable() {
    let mut staging = staged(&["/one/hero.obj", "/two/hero.obj"]);
    staging.rows[1].destination = "assets/props/hero.obj".into();
    let mut queue = ImportQueue::default();

    dialog::confirm(&mut staging, &mut queue);

    assert_eq!(queue.remaining(), 2);
    assert!(!staging.visible);
}

#[test]
fn cancelling_discards_the_rows_without_queueing_them() {
    let mut staging = staged(&["/tmp/hero.obj"]);
    let queue = ImportQueue::default();

    dialog::cancel(&mut staging);

    assert!(staging.rows.is_empty());
    assert!(!staging.visible);
    assert_eq!(queue.remaining(), 0);
}

fn editor(sources: &[&str]) -> (App, tempfile::TempDir) {
    let project = tempfile::tempdir().expect("tempdir");
    let mut app = App::new();
    app.insert_resource(UITheme::default());
    app.insert_resource(ActionMap::default());
    app.insert_resource(FocusedWidget::default());
    app.register_event::<UITextInputChanged>();
    app.register_event::<ActionFired>();
    let mut state = ProjectState::default();
    state.project = Some(Project {
        root: project.path().to_path_buf(),
        assets: Vec::new(),
        registry: AssetRegistry::default(),
    });
    app.insert_resource(state);
    app.register_plugin(ImportPlugin);
    app.finish_plugin_build();
    *app.get_resource_mut::<ImportStaging>().expect("staging") = staged(sources);
    frame(&mut app);
    (app, project)
}

fn frame(app: &mut App) {
    app.main_mut().world_mut().run_schedule(LateUpdate);
}

fn field_showing(app: &mut App, value: &str) -> Entity {
    let world = app.main_mut().world_mut();
    world
        .query::<(Entity, &UITextInput), ()>()
        .iter(world)
        .find(|(_, input)| input.value == value)
        .map(|(entity, _)| entity)
        .expect("a field shows the destination")
}

#[test]
fn typing_in_a_field_edits_its_row_without_the_dialog_overwriting_it() {
    let (mut app, _project) = editor(&["/tmp/hero.obj", "/tmp/tree.obj"]);
    let field = field_showing(&mut app, "assets/hero.obj");

    let world = app.main_mut().world_mut();
    let input = world
        .get_component_for_entity_mut::<UITextInput>(field)
        .expect("input");
    input.value = "assets/props/hero.obj".into();
    input.cursor = 13;
    world
        .get_resource_mut::<EventChannel<UITextInputChanged>>()
        .expect("channel")
        .push_event(UITextInputChanged {
            entity: field,
            value: "assets/props/hero.obj".into(),
        });
    frame(&mut app);

    let staging = app.get_resource::<ImportStaging>().expect("staging");
    assert_eq!(staging.rows[0].destination, "assets/props/hero.obj");
    assert_eq!(staging.rows[1].destination, "assets/tree.obj");
    let input = app
        .main_mut()
        .world_mut()
        .get_component_for_entity::<UITextInput>(field)
        .expect("input");
    assert_eq!(input.value, "assets/props/hero.obj");
    assert_eq!(input.cursor, 13, "the caret stays where the user is typing");
}

#[test]
fn the_confirm_key_queues_the_batch_once_and_releases_the_key_context() {
    let (mut app, _project) = editor(&["/tmp/hero.obj", "/tmp/tree.obj"]);
    assert!(
        app.get_resource::<ActionMap>()
            .expect("actions")
            .is_active(ImportDialogContext),
        "an open dialog owns Enter and Escape"
    );

    app.get_resource_mut::<EventChannel<ActionFired>>()
        .expect("channel")
        .push_event(ActionFired {
            action: ConfirmImport.intern(),
        });
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

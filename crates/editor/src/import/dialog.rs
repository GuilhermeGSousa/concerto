//! The modal that confirms where each picked file lands.
use concerto_app::{
    App, Plugin,
    schedule_groups::{LateUpdate, Startup},
};
use concerto_color::Color;
use concerto_ecs::{
    Component, Entity, IntoSystemConfig, Query, Res, ResMut,
    command::CommandQueue,
    entity::hierarchy::{ChildOf, Children},
    events::event_reader::EventReader,
    query::{change_detection::DetectChanges, filter::With},
    signal::On,
};
use concerto_ui::{
    elements::prelude::*,
    focus::FocusedWidget,
    interaction::{Interactable, UIClick, UIDisabled},
    list::UIList,
    node::{UILayout, UINode, UIRect},
    scroll::{SCROLLBAR_WIDTH, UIScrollArea},
    sets::UiSet,
    text::UIText,
    text_input::{UITextInput, UITextInputChanged},
    theme::UITheme,
    transform::UIValue,
};
use concerto_window::input::{
    MouseButton,
    actions::{ActionFired, ActionMap},
};
use std::path::PathBuf;

use crate::actions::{CancelImport, ConfirmImport, ImportDialogContext};
use crate::guard::UnsavedGuard;
use crate::import::{
    ImportQueue, ImportStaging, RowState, StagedImport, destinations_clash, validate_rows,
};
use crate::project::ProjectState;

const DIALOG_LAYER: i32 = 200;
const CARD_WIDTH: f32 = 720.0;
const NAME_WIDTH: f32 = 140.0;
const STATUS_WIDTH: f32 = 190.0;
const SCROLLBAR_GUTTER: f32 = SCROLLBAR_WIDTH + 4.0;
const ROW_HEIGHT: f32 = 30.0;
const VISIBLE_ROWS: usize = 10;

#[derive(Component)]
struct DialogRoot;

#[derive(Component)]
struct DialogView;

#[derive(Component)]
struct DialogRow(PathBuf);

#[derive(Component)]
struct DialogField(usize);

#[derive(Component)]
struct DialogStatus(usize);

#[derive(Component)]
struct ImportButton;

/// Builds the staging dialog and keeps it in step with [`ImportStaging`].
pub struct DialogPlugin;

impl Plugin for DialogPlugin {
    fn build(&self, app: &mut App) {
        app.add_system(Startup, build_dialog).add_system(
            LateUpdate,
            (edit_destinations, press_keys, sync_dialog, render_dialog)
                .chain()
                .after(UiSet::Widgets)
                .before(UiSet::Project),
        );
    }
}

/// Whether confirming would queue anything: a row is importable and no two rows share a destination.
pub fn can_confirm(staging: &ImportStaging) -> bool {
    staging.rows.iter().any(StagedImport::importable) && !destinations_clash(&staging.rows)
}

/// Queues every importable row and closes the dialog; does nothing while [`can_confirm`] is false.
pub fn confirm(staging: &mut ImportStaging, queue: &mut ImportQueue) {
    if !can_confirm(staging) {
        return;
    }
    queue.enqueue(std::mem::take(&mut staging.rows));
    staging.visible = false;
}

/// Discards the staged rows and closes the dialog.
pub fn cancel(staging: &mut ImportStaging) {
    staging.rows.clear();
    staging.visible = false;
}

fn confirm_checked(staging: &mut ImportStaging, queue: &mut ImportQueue, project: &ProjectState) {
    let root = project.project.as_ref().map(|project| &project.root);
    if let Some(root) = root {
        validate_rows(&mut staging.rows, root);
    }
    confirm(staging, queue);
    if let Some(root) = root {
        queue.bind(root);
    }
}

fn build_dialog(mut cmd: CommandQueue, theme: Res<UITheme>) {
    let raised = theme.surface_raised.to_srgba();
    cmd.spawn((
        theme
            .canvas()
            .fill(Color::srgba(0.0, 0.0, 0.0, 0.55))
            .size(UIValue::Percent(100.0), UIValue::Percent(100.0))
            .column()
            .align_items(taffy::AlignItems::Center)
            .justify(taffy::AlignContent::Center)
            .z_index(DIALOG_LAYER)
            .hidden(),
        Interactable,
        DialogRoot,
    ))
    .add_child_with(
        (
            theme
                .popup()
                .fill(Color::srgba(raised.r, raised.g, raised.b, 1.0))
                .width(UIValue::Px(CARD_WIDTH))
                .max_width(UIValue::Percent(94.0))
                .padding(theme.spacing_md)
                .gap(theme.spacing_md),
            Interactable,
        ),
        |mut card| {
            card.add_child(
                theme
                    .label("Import assets")
                    .single_line()
                    .height(UIValue::Px(theme.control_height)),
            )
            .add_child((
                UINode::default()
                    .with_max_height(UIValue::Px(VISIBLE_ROWS as f32 * ROW_HEIGHT))
                    .with_flex_shrink(0.0),
                UIList {
                    row_height: Some(ROW_HEIGHT),
                    selectable: false,
                },
                DialogView,
            ))
            .add_child_with(
                theme.row().justify(taffy::AlignContent::End),
                |mut footer| {
                    footer
                        .add_child(
                            theme
                                .button("Cancel")
                                .width(UIValue::Px(96.0))
                                .on_click(press_cancel),
                        )
                        .add_child((
                            theme
                                .button("Import")
                                .width(UIValue::Px(96.0))
                                .on_click(press_confirm),
                            ImportButton,
                        ));
                },
            );
        },
    );
}

fn press_confirm(
    on: On<UIClick>,
    mut staging: ResMut<ImportStaging>,
    mut queue: ResMut<ImportQueue>,
    project: Res<ProjectState>,
) {
    if on.signal().button == MouseButton::Left {
        confirm_checked(&mut staging, &mut queue, &project);
    }
}

fn press_cancel(on: On<UIClick>, mut staging: ResMut<ImportStaging>) {
    if on.signal().button == MouseButton::Left {
        cancel(&mut staging);
    }
}

fn edit_destinations(
    mut changes: EventReader<UITextInputChanged>,
    fields: Query<&DialogField>,
    mut staging: ResMut<ImportStaging>,
    project: Res<ProjectState>,
) {
    let edits: Vec<(usize, String)> = changes
        .read()
        .filter_map(|change| {
            let field = fields.get_entity(change.entity)?;
            Some((field.0, change.value.clone()))
        })
        .collect();
    let Some(project) = project.project.as_ref() else {
        return;
    };
    if edits.is_empty() {
        return;
    }
    for (index, destination) in edits {
        if let Some(row) = staging.rows.get_mut(index) {
            row.destination = destination;
        }
    }
    validate_rows(&mut staging.rows, &project.root);
}

fn press_keys(
    mut fired: EventReader<ActionFired>,
    mut staging: ResMut<ImportStaging>,
    mut queue: ResMut<ImportQueue>,
    project: Res<ProjectState>,
    guard: Res<UnsavedGuard>,
) {
    for action in fired.read() {
        if !staging.visible || guard.asking() {
            continue;
        }
        if action.is(CancelImport) {
            cancel(&mut staging);
        } else if action.is(ConfirmImport) {
            confirm_checked(&mut staging, &mut queue, &project);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn sync_dialog(
    staging: Res<ImportStaging>,
    theme: Res<UITheme>,
    roots: Query<(&DialogRoot, &mut UINode)>,
    views: Query<(Entity, &mut UIScrollArea, Option<&Children>), With<DialogView>>,
    rows: Query<&DialogRow>,
    fields: Query<(Entity, &DialogField, Option<&UILayout>)>,
    mut actions: ResMut<ActionMap>,
    mut focus: ResMut<FocusedWidget>,
    mut cmd: CommandQueue,
) {
    let open = staging.visible;
    let mut opened = false;
    for (_, mut node) in roots.iter() {
        if node.visible != open {
            node.visible = open;
            opened = open;
        }
    }
    if actions.is_active(ImportDialogContext) != open {
        if open {
            actions.push_context(ImportDialogContext);
        } else {
            actions.pop_context(ImportDialogContext);
        }
    }
    let focused_field = (**focus).and_then(|entity| fields.get_entity(entity));
    let scrolled_off = focused_field
        .and_then(|(_, _, layout)| layout)
        .is_some_and(|layout| !layout.is_shown());
    let mut respawned = false;
    let mut first_field = None;
    for (list, mut area, children) in views.iter() {
        if !open && area.offset != 0.0 {
            area.offset = 0.0;
        }
        let shown: Vec<Entity> = children
            .into_iter()
            .flat_map(|children| children.iter().copied())
            .filter(|child| rows.contains_entity(*child))
            .collect();
        let current = shown
            .iter()
            .filter_map(|row| rows.get_entity(*row).map(|row| &row.0));
        if !staging.is_changed() || current.eq(staging.rows.iter().map(|row| &row.source)) {
            continue;
        }
        respawned = true;
        for row in shown {
            cmd.despawn(row);
        }
        for (index, staged) in staging.rows.iter().enumerate() {
            let field = spawn_row(&mut cmd, &theme, list, index, staged);
            if index == 0 {
                first_field = Some(field);
            }
        }
    }
    if focused_field.is_some() && (!open || respawned || scrolled_off) {
        **focus = None;
    }
    if opened {
        **focus = first_field.or_else(|| {
            fields
                .iter()
                .find(|(_, field, _)| field.0 == 0)
                .map(|(entity, _, _)| entity)
        });
    }
}

fn spawn_row(
    cmd: &mut CommandQueue,
    theme: &UITheme,
    list: Entity,
    index: usize,
    staged: &StagedImport,
) -> Entity {
    let name = staged
        .source
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut row = cmd.spawn((
        theme.row().padding(UIRect {
            right: SCROLLBAR_GUTTER,
            ..Default::default()
        }),
        DialogRow(staged.source.clone()),
        ChildOf::new(list),
    ));
    row.add_child(
        theme
            .label(name)
            .muted()
            .single_line()
            .width(UIValue::Px(NAME_WIDTH))
            .fixed(),
    );
    let field = row
        .spawn_child((
            theme
                .text_field("assets/…")
                .single_line()
                .value(staged.destination.clone())
                .grow()
                .shrink(1.0)
                .min_width(UIValue::Px(0.0)),
            DialogField(index),
        ))
        .entity();
    let (status, color) = status_of(&staged.state, theme);
    row.add_child((
        theme
            .label(status)
            .color(color)
            .small()
            .single_line()
            .width(UIValue::Px(STATUS_WIDTH))
            .fixed(),
        DialogStatus(index),
    ));
    field
}

fn status_of<'a>(state: &'a RowState, theme: &UITheme) -> (&'a str, Color) {
    match state {
        RowState::Replaces => ("replaces existing", theme.warning),
        RowState::Rejected(reason) => (reason.as_str(), theme.error),
        RowState::New => ("", theme.text_muted),
    }
}

fn render_dialog(
    staging: Res<ImportStaging>,
    theme: Res<UITheme>,
    fields: Query<(&DialogField, &mut UITextInput)>,
    statuses: Query<(&DialogStatus, &mut UIText)>,
    buttons: Query<(Entity, &ImportButton, &mut UIText, Option<&UIDisabled>)>,
    mut cmd: CommandQueue,
) {
    for (field, mut input) in fields.iter() {
        let value = staging
            .rows
            .get(field.0)
            .map_or("", |row| row.destination.as_str());
        if input.value != value {
            input.value = value.to_owned();
            input.cursor = value.len();
            input.selection_anchor = None;
        }
    }
    for (status, mut label) in statuses.iter() {
        let (value, color) = staging
            .rows
            .get(status.0)
            .map_or(("", theme.text_muted), |row| status_of(&row.state, &theme));
        if label.text != value {
            label.text = value.to_owned();
        }
        if label.color != color {
            label.color = color;
        }
    }

    let blocked = !can_confirm(&staging);
    for (entity, _, mut label, disabled) in buttons.iter() {
        let color = if blocked {
            theme.text_muted
        } else {
            theme.text
        };
        if label.color != color {
            label.color = color;
        }
        if blocked && disabled.is_none() {
            cmd.insert(UIDisabled, entity);
        } else if !blocked && disabled.is_some() {
            cmd.remove::<UIDisabled>(entity);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::guard::{GuardedIntent, UnsavedGuard};
    use crate::import::stage_sources;
    use crate::project::Project;
    use concerto_ecs::{
        IntoSystem, System, World,
        entity::hierarchy::{ChildOf, Children},
        events::event_channel::EventChannel,
    };
    use concerto_ui::{
        items::UIItem,
        list::update_lists_system,
        node::{UIBox, UICulled},
    };
    use concerto_window::input::actions::ActionLabel;
    use glam::Vec2;
    use std::path::PathBuf;

    const FILES: usize = 18;

    fn run<M>(world: &mut World, system: impl IntoSystem<(), M>) {
        let mut system = system.into_system();
        system.initialize(world);
        system.run_and_apply((), world);
    }

    fn place(world: &mut World) {
        let mut system = update_lists_system();
        system.initialize(world);
        system.run_and_apply((), world);
    }

    fn picked(names: &[&str]) -> Vec<StagedImport> {
        stage_sources(
            names
                .iter()
                .map(|name| PathBuf::from(format!("/picked/{name}")))
                .collect(),
        )
    }

    struct Fixture {
        world: World,
        _project: tempfile::TempDir,
    }

    fn fixture(files: usize) -> Fixture {
        let project = tempfile::tempdir().expect("tempdir");
        let mut world = World::new();
        world.register_component::<ChildOf>();
        world.register_component::<Children>();
        world.insert_resource(UITheme::default());
        world.insert_resource(ActionMap::default());
        world.insert_resource(FocusedWidget::default());
        world.insert_resource(ImportQueue::default());
        world.insert_resource(UnsavedGuard::default());
        world.insert_resource(EventChannel::<UITextInputChanged>::default());
        world.insert_resource(EventChannel::<ActionFired>::default());
        let mut state = ProjectState::default();
        state.project = Some(Project {
            root: project.path().to_path_buf(),
            content_root: "content".into(),
        });
        world.insert_resource(state);
        world.insert_resource(ImportStaging {
            rows: stage_sources(
                (0..files)
                    .map(|index| PathBuf::from(format!("/picked/file{index}.obj")))
                    .collect(),
            ),
            visible: true,
        });
        concerto_ui::list::install_listeners(&mut world);
        concerto_ui::items::install_listeners(&mut world);
        run(&mut world, build_dialog);
        let mut fixture = Fixture {
            world,
            _project: project,
        };
        fixture.frame();
        fixture
    }

    impl Fixture {
        fn frame(&mut self) {
            run(&mut self.world, sync_dialog);
            place(&mut self.world);
            run(&mut self.world, render_dialog);
        }

        fn staging(&mut self) -> &mut ImportStaging {
            self.world.get_resource_mut::<ImportStaging>().unwrap()
        }

        fn queued(&self) -> usize {
            self.world
                .get_resource::<ImportQueue>()
                .unwrap()
                .remaining()
        }

        fn list(&mut self) -> Entity {
            self.world
                .query::<(Entity, &DialogView), ()>()
                .iter(&mut self.world)
                .next()
                .unwrap()
                .0
        }

        fn area(&mut self) -> &mut UIScrollArea {
            let list = self.list();
            self.world
                .get_component_for_entity_mut::<UIScrollArea>(list)
                .expect("the list is its own scroll area")
        }

        fn scroll_to(&mut self, first: usize) {
            self.area().offset = first as f32 * ROW_HEIGHT;
            self.frame();
        }

        fn lay_out(&mut self, entity: Entity, shown: bool) {
            let rect = UIBox {
                min: Vec2::ZERO,
                size: Vec2::new(400.0, ROW_HEIGHT),
            };
            let clip_rect = if shown { rect } else { UIBox::default() };
            self.world.insert(
                UILayout {
                    rect,
                    content_rect: rect,
                    clip_rect,
                    paint_order: 0,
                },
                entity,
            );
        }

        fn open(&mut self) -> bool {
            self.world
                .query::<(&DialogRoot, &UINode), ()>()
                .iter(&mut self.world)
                .next()
                .unwrap()
                .1
                .visible
        }

        fn rows(&mut self) -> Vec<Entity> {
            let list = self.list();
            let children = self
                .world
                .get_component_for_entity::<Children>(list)
                .map(|children| children.iter().copied().collect::<Vec<_>>())
                .unwrap_or_default();
            children
                .into_iter()
                .filter(|child| {
                    self.world
                        .get_component_for_entity::<DialogRow>(*child)
                        .is_some()
                })
                .collect()
        }

        fn shown_rows(&mut self) -> usize {
            self.rows().len()
        }

        fn fields(&mut self) -> usize {
            self.world
                .query::<&DialogField, ()>()
                .iter(&mut self.world)
                .count()
        }

        fn focus(&mut self) -> &mut FocusedWidget {
            self.world.get_resource_mut::<FocusedWidget>().unwrap()
        }

        fn field(&mut self, index: usize) -> Entity {
            self.world
                .query::<(Entity, &DialogField), ()>()
                .iter(&mut self.world)
                .find(|(_, field)| field.0 == index)
                .unwrap()
                .0
        }

        fn input(&mut self, index: usize) -> &mut UITextInput {
            let field = self.field(index);
            self.world
                .get_component_for_entity_mut::<UITextInput>(field)
                .unwrap()
        }

        fn cell(&mut self, index: usize, column: usize) -> String {
            let row = self.rows()[index];
            let cell = self
                .world
                .get_component_for_entity::<Children>(row)
                .unwrap()
                .iter()
                .copied()
                .nth(column)
                .unwrap();
            self.world
                .get_component_for_entity::<UIText>(cell)
                .unwrap()
                .text
                .clone()
        }

        fn name(&mut self, index: usize) -> String {
            self.cell(index, 0)
        }

        fn status(&mut self, index: usize) -> String {
            self.cell(index, 2)
        }

        fn button(&mut self, label: &str) -> Entity {
            self.world
                .query::<(Entity, &UIText, &Interactable), ()>()
                .iter(&mut self.world)
                .find(|(_, text, _)| text.text == label)
                .unwrap()
                .0
        }

        fn import_disabled(&mut self) -> bool {
            let button = self.button("Import");
            self.world
                .get_component_for_entity::<UIDisabled>(button)
                .is_some()
        }

        fn click(&mut self, entity: Entity, button: MouseButton) {
            self.world.trigger_on(
                entity,
                UIClick {
                    position: Vec2::ZERO,
                    button,
                },
            );
        }

        fn type_into(&mut self, entity: Entity, value: &str) {
            let channel = self
                .world
                .get_resource_mut::<EventChannel<UITextInputChanged>>()
                .unwrap();
            channel.update();
            channel.update();
            channel.push_event(UITextInputChanged {
                entity,
                value: value.into(),
            });
            run(&mut self.world, edit_destinations);
            self.frame();
        }

        fn fire(&mut self, action: impl ActionLabel) {
            let channel = self
                .world
                .get_resource_mut::<EventChannel<ActionFired>>()
                .unwrap();
            channel.update();
            channel.update();
            channel.push_event(ActionFired {
                action: action.intern(),
            });
            run(&mut self.world, press_keys);
            self.frame();
        }
    }

    #[test]
    fn each_staged_file_gets_one_row_with_its_name_and_destination() {
        let mut dialog = fixture(3);

        assert!(dialog.open());
        assert_eq!(dialog.shown_rows(), 3);
        assert_eq!(dialog.fields(), 3);
        assert_eq!(dialog.name(2), "file2.obj");
        assert_eq!(dialog.input(2).value, "assets/file2.obj");
        assert_eq!(dialog.status(2), "");
        assert!(!dialog.import_disabled());
    }

    #[test]
    fn a_replacing_row_is_labelled_and_a_rejected_row_shows_its_reason() {
        let mut dialog = fixture(2);
        dialog.staging().rows[0].state = RowState::Replaces;
        dialog.staging().rows[1].state = RowState::Rejected("needs a file name".into());
        dialog.frame();

        assert!(dialog.status(0).contains("replaces"));
        assert_eq!(dialog.status(1), "needs a file name");
        assert!(
            !dialog.import_disabled(),
            "one importable row is enough to confirm"
        );
    }

    #[test]
    fn a_long_selection_is_one_list_item_per_file() {
        let mut dialog = fixture(FILES);
        let list = dialog.list();

        let rows = dialog.rows();
        assert_eq!(rows.len(), FILES);
        for (index, row) in rows.into_iter().enumerate() {
            let item = dialog
                .world
                .get_component_for_entity::<UIItem>(row)
                .expect("every row is a list item");
            assert_eq!((item.owner(), item.index()), (list, index));
        }
        assert_eq!(dialog.area().content_extent, FILES as f32 * ROW_HEIGHT);
        assert_eq!(dialog.name(FILES - 1), format!("file{}.obj", FILES - 1));
        assert_eq!(dialog.input(7).value, "assets/file7.obj");
    }

    #[test]
    fn an_edit_lands_on_the_row_of_its_field() {
        let mut dialog = fixture(FILES);
        dialog.scroll_to(6);
        let field = dialog.field(7);

        dialog.type_into(field, "assets/props/renamed.obj");

        let rows = &dialog.staging().rows;
        assert_eq!(rows[7].destination, "assets/props/renamed.obj");
        assert_eq!(rows[1].destination, "assets/file1.obj");
    }

    #[test]
    fn a_change_in_some_other_text_input_is_ignored() {
        let mut dialog = fixture(2);
        let elsewhere = dialog.world.spawn(UITextInput::new(""));

        dialog.type_into(elsewhere, "assets/stolen.obj");

        let rows = &dialog.staging().rows;
        assert_eq!(rows[0].destination, "assets/file0.obj");
        assert_eq!(rows[1].destination, "assets/file1.obj");
    }

    #[test]
    fn an_edit_revalidates_the_rows_and_gates_the_import_button() {
        let mut dialog = fixture(2);
        let field = dialog.field(0);

        dialog.type_into(field, "assets/file1.obj");
        assert!(dialog.status(0).contains("twice"));
        assert!(dialog.status(1).contains("twice"));
        assert!(dialog.import_disabled());

        dialog.type_into(field, "assets/other.obj");
        assert_eq!(dialog.status(0), "");
        assert_eq!(dialog.status(1), "");
        assert!(!dialog.import_disabled());
    }

    #[test]
    fn rendering_leaves_a_field_that_already_matches_its_row_alone() {
        let mut dialog = fixture(1);
        let field = dialog.field(0);
        let input = dialog.input(0);
        input.value = "assets/file0x.obj".into();
        input.cursor = 13;

        dialog.type_into(field, "assets/file0x.obj");

        let input = dialog.input(0);
        assert_eq!(input.value, "assets/file0x.obj");
        assert_eq!(input.cursor, 13, "the caret stays where the user is typing");
    }

    #[test]
    fn rewriting_a_field_moves_its_cursor_inside_the_new_value() {
        let mut dialog = fixture(FILES);
        let input = dialog.input(0);
        input.value = "assets/a/much/longer/destination/than/the/next.obj".into();
        input.cursor = input.value.len();
        input.selection_anchor = Some(input.value.len() - 4);

        dialog.frame();

        let input = dialog.input(0);
        assert_eq!(input.value, "assets/file0.obj");
        assert_eq!(input.cursor, input.value.len());
        assert_eq!(input.selection_anchor, None);
    }

    #[test]
    fn rows_follow_staging() {
        let mut dialog = fixture(3);
        let before = dialog.rows();
        assert_eq!(before.len(), 3);

        let field = dialog.field(1);
        dialog.type_into(field, "assets/moved.obj");
        assert_eq!(dialog.rows(), before, "an edit keeps the rows it edits");

        dialog.staging().rows = picked(&["late.obj", "later.obj"]);
        dialog.frame();

        let after = dialog.rows();
        assert_eq!(after.len(), 2);
        assert!(after.iter().all(|row| !before.contains(row)));
        assert!(before.iter().all(|row| {
            dialog
                .world
                .get_component_for_entity::<UINode>(*row)
                .is_none()
        }));
        assert_eq!(dialog.name(0), "late.obj");
        assert_eq!(dialog.name(1), "later.obj");
        assert_eq!(dialog.input(1).value, "assets/later.obj");
    }

    #[test]
    fn a_new_row_shows_its_destination_and_status_from_the_start() {
        let mut dialog = fixture(0);
        let mut rows = picked(&["late.obj"]);
        rows[0].state = RowState::Rejected("needs a file name".into());
        dialog.staging().rows = rows;

        run(&mut dialog.world, sync_dialog);

        assert_eq!(dialog.input(0).value, "assets/late.obj");
        assert_eq!(dialog.status(0), "needs a file name");
    }

    #[test]
    fn the_buttons_confirm_and_cancel_on_a_left_click_only() {
        let mut dialog = fixture(2);
        let import = dialog.button("Import");
        dialog.click(import, MouseButton::Right);
        assert_eq!(dialog.queued(), 0);
        dialog.click(import, MouseButton::Left);
        dialog.frame();
        assert_eq!(dialog.queued(), 2);
        assert!(!dialog.open());

        let mut dialog = fixture(2);
        let cancel = dialog.button("Cancel");
        dialog.click(cancel, MouseButton::Left);
        dialog.frame();
        assert_eq!(dialog.queued(), 0);
        assert!(!dialog.open());
        assert_eq!(dialog.shown_rows(), 0);
        assert_eq!(
            dialog.fields(),
            0,
            "a closed dialog leaves nothing in the focus ring"
        );
    }

    #[test]
    fn the_keys_confirm_and_cancel_only_while_the_dialog_is_open() {
        let mut dialog = fixture(2);
        dialog.fire(ConfirmImport);
        assert_eq!(dialog.queued(), 2);
        assert!(!dialog.open());

        dialog.staging().rows = picked(&["late.obj"]);
        dialog.fire(ConfirmImport);
        assert_eq!(dialog.queued(), 2, "a closed dialog ignores the key");
        dialog.fire(CancelImport);
        assert_eq!(dialog.staging().rows.len(), 1);

        let mut dialog = fixture(2);
        dialog.fire(CancelImport);
        assert_eq!(dialog.queued(), 0);
        assert!(!dialog.open());
    }

    #[test]
    fn the_key_context_is_active_exactly_while_the_dialog_is_open() {
        let mut dialog = fixture(1);
        let active = |dialog: &Fixture| {
            dialog
                .world
                .get_resource::<ActionMap>()
                .unwrap()
                .is_active(ImportDialogContext)
        };
        assert!(active(&dialog));

        dialog.fire(CancelImport);
        assert!(!active(&dialog));
    }

    #[test]
    fn closing_takes_keyboard_focus_away_from_a_dialog_field() {
        let mut dialog = fixture(1);
        let field = dialog.field(0);
        **dialog.focus() = Some(field);

        dialog.frame();
        assert_eq!(
            **dialog.focus(),
            Some(field),
            "an open dialog leaves focus alone"
        );

        dialog.fire(ConfirmImport);
        assert_eq!(**dialog.focus(), None);
    }

    #[test]
    fn scrolling_a_focused_field_off_screen_drops_its_focus() {
        let mut dialog = fixture(FILES);
        let field = dialog.field(0);
        **dialog.focus() = Some(field);

        dialog.scroll_to(1);
        let row = dialog.rows()[0];
        assert!(
            dialog
                .world
                .get_component_for_entity::<UICulled>(row)
                .is_none(),
            "the row above the view is overscan, laid out but clipped"
        );
        dialog.lay_out(field, false);
        dialog.frame();

        assert_eq!(dialog.input(0).value, "assets/file0.obj");
        assert_eq!(
            **dialog.focus(),
            None,
            "typing must not carry on into a row the user cannot see"
        );
    }

    #[test]
    fn a_focused_field_still_on_screen_keeps_its_focus() {
        let mut dialog = fixture(FILES);
        let field = dialog.field(1);
        **dialog.focus() = Some(field);

        dialog.scroll_to(1);
        dialog.lay_out(field, true);
        dialog.frame();

        assert_eq!(**dialog.focus(), Some(field));
    }

    #[test]
    fn closing_leaves_focus_held_elsewhere_alone() {
        let mut dialog = fixture(1);
        let elsewhere = dialog.world.spawn(UITextInput::new(""));
        **dialog.focus() = Some(elsewhere);

        dialog.fire(CancelImport);

        assert_eq!(**dialog.focus(), Some(elsewhere));
    }

    #[test]
    fn opening_puts_the_caret_in_the_first_destination_once() {
        let mut dialog = fixture(2);
        let first = dialog.field(0);
        assert_eq!(**dialog.focus(), Some(first));

        **dialog.focus() = None;
        dialog.frame();
        assert_eq!(
            **dialog.focus(),
            None,
            "only the frame that opens the dialog takes the focus"
        );
    }

    #[test]
    fn the_keys_do_nothing_while_the_unsaved_changes_prompt_is_asking() {
        let mut dialog = fixture(2);
        let document = dialog.world.spawn(UINode::default());
        dialog
            .world
            .get_resource_mut::<UnsavedGuard>()
            .unwrap()
            .hold(GuardedIntent::Quit, vec![document]);

        dialog.fire(ConfirmImport);
        assert_eq!(dialog.queued(), 0, "Enter belongs to the prompt on top");
        assert!(dialog.open());

        dialog.fire(CancelImport);
        assert!(dialog.open());
        assert_eq!(dialog.staging().rows.len(), 2);
    }

    #[test]
    fn the_import_button_rechecks_the_rows_before_queueing_them() {
        let mut dialog = fixture(2);
        dialog.staging().rows[0].destination = "../outside.obj".into();
        let import = dialog.button("Import");

        dialog.click(import, MouseButton::Left);
        dialog.frame();

        assert_eq!(dialog.queued(), 1, "the row that went stale is left out");
    }

    #[test]
    fn the_confirm_key_rechecks_the_rows_before_queueing_them() {
        let mut dialog = fixture(1);
        dialog.staging().rows[0].destination = "../outside.obj".into();

        dialog.fire(ConfirmImport);

        assert_eq!(dialog.queued(), 0);
        assert!(dialog.open(), "nothing importable is left, so it stays");
        assert!(dialog.status(0).contains("project"));
    }

    #[test]
    fn the_list_grows_with_its_rows_up_to_ten() {
        for files in [2, FILES] {
            let mut dialog = fixture(files);
            let list = dialog.list();
            let node = dialog
                .world
                .get_component_for_entity::<UINode>(list)
                .unwrap()
                .clone();
            assert_eq!(node.height, UIValue::Auto, "the height is the rows' own");
            assert_eq!(node.max_height, UIValue::Px(10.0 * ROW_HEIGHT));
            let extent = dialog.area().content_extent;
            assert_eq!(extent, files as f32 * ROW_HEIGHT);
        }
    }

    #[test]
    fn a_replacing_row_says_so_in_two_words() {
        let mut dialog = fixture(1);
        dialog.staging().rows[0].state = RowState::Replaces;
        dialog.frame();

        assert_eq!(dialog.status(0), "replaces existing");
    }
}

//! The modal that confirms where each picked file lands.
use concerto_app::{
    App, Plugin,
    schedule_groups::{LateUpdate, Startup},
};
use concerto_color::Color;
use concerto_ecs::{
    Component, Entity, IntoSystemConfig, Query, Res, ResMut, command::CommandQueue,
    events::event_reader::EventReader, signal::On,
};
use concerto_ui::{
    elements::prelude::*,
    focus::FocusedWidget,
    interaction::{Interactable, UIClick, UIDisabled},
    node::{UINode, UIRect},
    scroll::{UIScrollArea, UIVirtualList},
    text::UIText,
    text_input::{UITextInput, UITextInputChanged},
    theme::UITheme,
    transform::UIValue,
};
use concerto_window::input::{
    MouseButton,
    actions::{ActionFired, ActionMap},
};
use taffy::FlexDirection;

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
const SCROLLBAR_GUTTER: f32 = 12.0;
const ROW_HEIGHT: f32 = 30.0;
const VISIBLE_ROWS: usize = 10;
const OVERSCAN: usize = 1;
const SLOTS: usize = VISIBLE_ROWS + 2 * OVERSCAN;

#[derive(Component)]
struct DialogRoot;

#[derive(Component, Default)]
struct DialogView {
    first: usize,
}

#[derive(Component)]
struct DialogRow(usize);

#[derive(Component)]
struct DialogName(usize);

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
            (edit_destinations, press_keys, sync_dialog, render_dialog).chain(),
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
        |card| {
            card.add_child(
                theme
                    .label("Import assets")
                    .single_line()
                    .height(UIValue::Px(theme.control_height)),
            )
            .add_child_with(
                (
                    UINode::default()
                        .with_height(UIValue::Px(0.0))
                        .with_flex_shrink(0.0)
                        .with_flex_direction(FlexDirection::Column)
                        .clipped(),
                    Interactable,
                    DialogView::default(),
                    UIVirtualList::new(0, ROW_HEIGHT).with_overscan(OVERSCAN),
                ),
                |mut view| {
                    let mut pool = view.spawn_child_queue(
                        UINode::default()
                            .with_flex_direction(FlexDirection::Column)
                            .with_flex_shrink(0.0),
                    );
                    let pool_entity = pool.entity();
                    for slot in 0..SLOTS {
                        pool = pool.add_child_with(
                            (
                                theme
                                    .row()
                                    .height(UIValue::Px(ROW_HEIGHT))
                                    .padding(UIRect {
                                        right: SCROLLBAR_GUTTER,
                                        ..Default::default()
                                    })
                                    .fixed(),
                                DialogRow(slot),
                            ),
                            |row| {
                                row.add_child((
                                    theme
                                        .label("")
                                        .muted()
                                        .single_line()
                                        .width(UIValue::Px(NAME_WIDTH))
                                        .fixed(),
                                    DialogName(slot),
                                ))
                                .add_child((
                                    theme
                                        .text_field("assets/…")
                                        .single_line()
                                        .grow()
                                        .shrink(1.0)
                                        .min_width(UIValue::Px(0.0)),
                                    DialogField(slot),
                                ))
                                .add_child((
                                    theme
                                        .label("")
                                        .small()
                                        .single_line()
                                        .width(UIValue::Px(STATUS_WIDTH))
                                        .fixed(),
                                    DialogStatus(slot),
                                ));
                            },
                        );
                    }
                    view.insert(UIScrollArea {
                        content: Some(pool_entity),
                        ..Default::default()
                    });
                },
            )
            .add_child_with(theme.row().justify(taffy::AlignContent::End), |footer| {
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
            });
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
    views: Query<&DialogView>,
    mut staging: ResMut<ImportStaging>,
    project: Res<ProjectState>,
) {
    let first = views.iter().next().map_or(0, |view| view.first);
    let edits: Vec<(usize, String)> = changes
        .read()
        .filter_map(|change| {
            let field = fields.get_entity(change.entity)?;
            Some((first + field.0, change.value.clone()))
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

fn sync_dialog(
    staging: Res<ImportStaging>,
    roots: Query<(&DialogRoot, &mut UINode)>,
    views: Query<(
        &mut DialogView,
        &mut UIScrollArea,
        &mut UIVirtualList,
        &mut UINode,
    )>,
    fields: Query<(Entity, &DialogField)>,
    mut actions: ResMut<ActionMap>,
    mut focus: ResMut<FocusedWidget>,
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
    let mut rebound = false;
    for (mut view, mut area, mut list, mut node) in views.iter() {
        list.item_count = staging.rows.len();
        area.content_extent = staging.rows.len() as f32 * ROW_HEIGHT;
        let height = UIValue::Px(staging.rows.len().min(VISIBLE_ROWS) as f32 * ROW_HEIGHT);
        if node.height != height {
            node.height = height;
        }
        if !open {
            area.offset = 0.0;
        }
        let first = list.visible_range().start;
        rebound |= view.first != first;
        view.first = first;
    }
    if (!open || rebound) && (**focus).is_some_and(|entity| fields.get_entity(entity).is_some()) {
        **focus = None;
    }
    if opened {
        **focus = fields
            .iter()
            .find(|(_, field)| field.0 == 0)
            .map(|(entity, _)| entity);
    }
}

#[allow(clippy::too_many_arguments)]
fn render_dialog(
    staging: Res<ImportStaging>,
    theme: Res<UITheme>,
    views: Query<&DialogView>,
    rows: Query<(&DialogRow, &mut UINode)>,
    names: Query<(&DialogName, &mut UIText)>,
    fields: Query<(&DialogField, &mut UITextInput, &mut UINode)>,
    statuses: Query<(&DialogStatus, &mut UIText)>,
    buttons: Query<(Entity, &ImportButton, &mut UIText, Option<&UIDisabled>)>,
    mut cmd: CommandQueue,
) {
    let first = views.iter().next().map_or(0, |view| view.first);
    let staged = |slot: usize| staging.rows.get(first + slot);

    for (row, mut node) in rows.iter() {
        let present = staged(row.0).is_some();
        if node.visible != present {
            node.visible = present;
        }
    }
    for (name, mut label) in names.iter() {
        let value = staged(name.0)
            .and_then(|row| row.source.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        if label.text != value {
            label.text = value;
        }
    }
    for (field, mut input, mut node) in fields.iter() {
        let row = staged(field.0);
        if node.visible != row.is_some() {
            node.visible = row.is_some();
        }
        let value = row.map_or("", |row| row.destination.as_str());
        if input.value != value {
            input.value = value.to_owned();
            input.cursor = value.len();
            input.selection_anchor = None;
        }
    }
    for (status, mut label) in statuses.iter() {
        let (value, color) = match staged(status.0).map(|row| &row.state) {
            Some(RowState::Replaces) => ("replaces existing", theme.warning),
            Some(RowState::Rejected(reason)) => (reason.as_str(), theme.error),
            Some(RowState::New) | None => ("", theme.text_muted),
        };
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
    use concerto_foundation::assets::content::AssetRegistry;
    use concerto_window::input::actions::ActionLabel;
    use glam::Vec2;
    use std::path::PathBuf;

    fn run<M>(world: &mut World, system: impl IntoSystem<(), M>) {
        let mut system = system.into_system();
        system.initialize(world);
        system.run_and_apply((), world);
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
            assets: Vec::new(),
            registry: AssetRegistry::default(),
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

        fn scroll_to(&mut self, first: usize) {
            for mut view in self
                .world
                .query::<&mut DialogView, ()>()
                .iter(&mut self.world)
            {
                view.first = first;
            }
            run(&mut self.world, render_dialog);
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

        fn list_height(&mut self) -> UIValue {
            self.world
                .query::<(&DialogView, &UINode), ()>()
                .iter(&mut self.world)
                .next()
                .unwrap()
                .1
                .height
        }

        fn shown_rows(&mut self) -> usize {
            self.world
                .query::<(&DialogRow, &UINode), ()>()
                .iter(&mut self.world)
                .filter(|(_, node)| node.visible)
                .count()
        }

        fn focusable_fields(&mut self) -> usize {
            self.world
                .query::<(&DialogField, &UINode), ()>()
                .iter(&mut self.world)
                .filter(|(_, node)| node.visible)
                .count()
        }

        fn focus(&mut self) -> &mut FocusedWidget {
            self.world.get_resource_mut::<FocusedWidget>().unwrap()
        }

        fn field(&mut self, slot: usize) -> Entity {
            self.world
                .query::<(Entity, &DialogField), ()>()
                .iter(&mut self.world)
                .find(|(_, field)| field.0 == slot)
                .unwrap()
                .0
        }

        fn input(&mut self, slot: usize) -> &mut UITextInput {
            let field = self.field(slot);
            self.world
                .get_component_for_entity_mut::<UITextInput>(field)
                .unwrap()
        }

        fn name(&mut self, slot: usize) -> String {
            self.world
                .query::<(&DialogName, &UIText), ()>()
                .iter(&mut self.world)
                .find(|(name, _)| name.0 == slot)
                .unwrap()
                .1
                .text
                .clone()
        }

        fn status(&mut self, slot: usize) -> String {
            self.world
                .query::<(&DialogStatus, &UIText), ()>()
                .iter(&mut self.world)
                .find(|(status, _)| status.0 == slot)
                .unwrap()
                .1
                .text
                .clone()
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
        assert_eq!(dialog.focusable_fields(), 3);
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
    fn a_selection_longer_than_the_pool_scrolls_through_it() {
        let mut dialog = fixture(SLOTS + 8);

        assert_eq!(dialog.shown_rows(), SLOTS);
        let (list, area) = dialog
            .world
            .query::<(&UIVirtualList, &UIScrollArea), ()>()
            .iter(&mut dialog.world)
            .next()
            .unwrap();
        assert_eq!(list.item_count, SLOTS + 8);
        assert_eq!(area.content_extent, (SLOTS + 8) as f32 * ROW_HEIGHT);

        dialog.scroll_to(6);
        assert_eq!(dialog.name(0), "file6.obj");
        assert_eq!(dialog.input(1).value, "assets/file7.obj");

        dialog.scroll_to(SLOTS + 5);
        assert_eq!(
            dialog.shown_rows(),
            3,
            "slots past the last file are hidden"
        );
    }

    #[test]
    fn an_edit_in_a_scrolled_slot_lands_on_the_row_it_shows() {
        let mut dialog = fixture(SLOTS + 8);
        dialog.scroll_to(6);
        let field = dialog.field(1);

        dialog.type_into(field, "assets/props/renamed.obj");

        let rows = &dialog.staging().rows;
        assert_eq!(rows[7].destination, "assets/props/renamed.obj");
        assert_eq!(
            rows[1].destination, "assets/file1.obj",
            "the row that would sit in the slot when unscrolled is untouched"
        );
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
        let mut dialog = fixture(SLOTS + 8);
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
            dialog.focusable_fields(),
            0,
            "a closed dialog leaves nothing in the focus ring"
        );
        assert_eq!(dialog.input(0).value, "");
    }

    #[test]
    fn the_keys_confirm_and_cancel_only_while_the_dialog_is_open() {
        let mut dialog = fixture(2);
        dialog.fire(ConfirmImport);
        assert_eq!(dialog.queued(), 2);
        assert!(!dialog.open());

        dialog.staging().rows = stage_sources(vec![PathBuf::from("/picked/late.obj")]);
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
    fn scrolling_a_focused_field_onto_another_row_drops_its_focus() {
        let mut dialog = fixture(SLOTS + 8);
        dialog.scroll_to(6);
        let field = dialog.field(1);
        **dialog.focus() = Some(field);

        dialog.frame();

        assert_eq!(dialog.input(1).value, "assets/file1.obj");
        assert_eq!(
            **dialog.focus(),
            None,
            "typing must not carry on into a different file's destination"
        );
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
    fn the_list_is_as_tall_as_its_rows_up_to_the_visible_ten() {
        let mut dialog = fixture(2);
        assert_eq!(dialog.list_height(), UIValue::Px(2.0 * ROW_HEIGHT));

        let mut dialog = fixture(SLOTS + 8);
        assert_eq!(
            dialog.list_height(),
            UIValue::Px(VISIBLE_ROWS as f32 * ROW_HEIGHT)
        );
    }

    #[test]
    fn a_replacing_row_says_so_in_two_words() {
        let mut dialog = fixture(1);
        dialog.staging().rows[0].state = RowState::Replaces;
        dialog.frame();

        assert_eq!(dialog.status(0), "replaces existing");
    }
}

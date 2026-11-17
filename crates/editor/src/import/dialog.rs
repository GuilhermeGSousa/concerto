//! The modal that confirms where each picked file lands.
use concerto_app::{
    App, Plugin,
    schedule_groups::{LateUpdate, Startup},
};
use concerto_color::Color;
use concerto_ecs::{
    Added, Changed, Component, Entity, IntoSystemConfig, Query, Res, ResMut,
    command::CommandQueue,
    entity::hierarchy::{ChildOf, Children},
    events::event_reader::EventReader,
    query::{change_detection::DetectChanges, filter::With},
    reflect::AppTypeRegistry,
    signal::{
        On,
        listener::{IntoListener, Listener},
    },
};
use concerto_import::importer_for;
use concerto_reflect::Reflect;
use concerto_ui::{
    elements::prelude::*,
    focus::FocusedWidget,
    interaction::{Interactable, UIClick, UIDisabled},
    items::UIItemSelected,
    list::UIList,
    node::{FlexDirection, UILayout, UINode, UIRect},
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
    ImportQueue, ImportStaging, RowState, StagedImport, destinations_clash, stage_row,
    validate_rows,
};
use crate::inspector::ReflectRows;
use crate::project::ProjectState;

const DIALOG_LAYER: i32 = concerto_ui::anchor::UI_PANEL_LAYER - 1;
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

#[derive(Component, Default)]
struct SettingsPanel {
    row: Option<usize>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SettingsKind {
    Import,
    Asset,
}

impl SettingsKind {
    fn title(self) -> &'static str {
        match self {
            SettingsKind::Import => "Import settings",
            SettingsKind::Asset => "Asset settings",
        }
    }
}

#[derive(Component)]
struct SettingsSection(SettingsKind);

#[derive(Component)]
struct SettingsRows {
    row: usize,
    kind: SettingsKind,
    destination: String,
}

/// Builds the staging dialog and keeps it in step with [`ImportStaging`].
pub struct DialogPlugin;

impl Plugin for DialogPlugin {
    fn build(&self, app: &mut App) {
        app.add_system(Startup, build_dialog).add_system(
            LateUpdate,
            (
                edit_destinations,
                write_settings,
                press_keys,
                sync_dialog,
                show_settings,
                render_dialog,
            )
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
                    selectable: true,
                },
                DialogView,
            ))
            .add_child_with(
                (
                    theme.column().gap(theme.spacing_sm).hidden(),
                    SettingsPanel::default(),
                ),
                |mut panel| {
                    for kind in [SettingsKind::Import, SettingsKind::Asset] {
                        panel.add_child_with(
                            (theme.column(), SettingsSection(kind)),
                            |mut section| {
                                section.add_child(
                                    theme
                                        .label(kind.title())
                                        .single_line()
                                        .height(UIValue::Px(theme.control_height)),
                                );
                            },
                        );
                    }
                },
            )
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

fn select_row(index: usize) -> Listener<UIItemSelected> {
    (move |_: On<UIItemSelected>, panels: Query<&mut SettingsPanel>| {
        for mut panel in panels.iter() {
            if panel.row != Some(index) {
                panel.row = Some(index);
            }
        }
    })
    .into_listener()
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
        if let Some(row) = staging.rows.get_mut(index)
            && row.destination != destination
        {
            row.destination = destination;
            stage_row(row, &project.root);
        }
    }
    validate_rows(&mut staging.rows, &project.root);
}

fn write_settings(
    edited: Query<(Entity, &SettingsRows, &ReflectRows), Changed<ReflectRows>>,
    shown: Query<Entity, Added<ReflectRows>>,
    mut staging: ResMut<ImportStaging>,
) {
    for (entity, rows, value) in edited.iter() {
        if shown.contains_entity(entity) {
            continue;
        }
        let Some(row) = staging.rows.get_mut(rows.row) else {
            continue;
        };
        let Some(importer) = row
            .source
            .extension()
            .and_then(|extension| importer_for(&extension.to_string_lossy()))
        else {
            continue;
        };
        let edit = Some(importer.clone_settings(value.value.as_ref()));
        match rows.kind {
            SettingsKind::Import => row.settings = edit,
            SettingsKind::Asset => row.asset_settings = edit,
        }
    }
}

fn shown_value(row: &StagedImport, kind: SettingsKind) -> Option<Box<dyn Reflect>> {
    let Some(Ok(staged)) = &row.staged else {
        return None;
    };
    let (edit, current) = match kind {
        SettingsKind::Import => (row.settings.as_deref(), Some(staged.settings.as_ref())),
        SettingsKind::Asset => (
            row.asset_settings.as_deref(),
            staged.asset_settings.as_deref(),
        ),
    };
    let value = edit.or(current)?;
    let importer = importer_for(&row.source.extension()?.to_string_lossy())?;
    Some(importer.clone_settings(value))
}

fn show_settings(
    staging: Res<ImportStaging>,
    types: Res<AppTypeRegistry>,
    panels: Query<(&mut SettingsPanel, &mut UINode)>,
    sections: Query<(Entity, &SettingsSection, &mut UINode)>,
    shown: Query<(Entity, &SettingsRows)>,
    mut cmd: CommandQueue,
) {
    for (mut panel, mut node) in panels.iter() {
        if !staging.visible && panel.row.is_some() {
            panel.row = None;
        }
        let selected = panel.row.filter(|&index| {
            staging
                .rows
                .get(index)
                .is_some_and(|row| matches!(row.staged, Some(Ok(_))))
        });
        let destination = selected.map(|index| staging.rows[index].destination.as_str());
        let mut showing = shown.iter().map(|(_, rows)| rows).peekable();
        let up_to_date = match selected {
            Some(index) => {
                showing.peek().is_some()
                    && showing.all(|rows| {
                        rows.row == index && Some(rows.destination.as_str()) == destination
                    })
            }
            None => showing.peek().is_none(),
        };
        if up_to_date {
            continue;
        }
        for (entity, _) in shown.iter() {
            cmd.despawn(entity);
        }
        let mut visible = false;
        for (section, kind, mut section_node) in sections.iter() {
            let value = selected.and_then(|index| shown_value(&staging.rows[index], kind.0));
            if section_node.visible != value.is_some() {
                section_node.visible = value.is_some();
            }
            let (Some(index), Some(value)) = (selected, value) else {
                continue;
            };
            visible = true;
            cmd.spawn((
                UINode::default()
                    .with_flex_direction(FlexDirection::Column)
                    .with_flex_shrink(0.0),
                ReflectRows {
                    value,
                    types: types.clone(),
                },
                SettingsRows {
                    row: index,
                    kind: kind.0,
                    destination: staging.rows[index].destination.clone(),
                },
                ChildOf::new(section),
            ));
        }
        if node.visible != visible {
            node.visible = visible;
        }
    }
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
        select_row(index),
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
    use crate::guard::UnsavedGuard;
    use crate::import::stage_sources;
    use crate::project::Project;
    use concerto_ecs::{
        IntoSystem, System, World,
        entity::hierarchy::{ChildOf, Children},
        events::event_channel::EventChannel,
    };
    use concerto_ui::list::update_lists_system;
    use glam::Vec2;
    use std::path::PathBuf;

    const FILES: usize = 18;

    fn run<M>(world: &mut World, system: impl IntoSystem<(), M>) {
        let mut system = system.into_system();
        system.initialize(world);
        system.run_and_apply((), world);
    }

    struct Fixture {
        world: World,
        _project: tempfile::TempDir,
        _sources: tempfile::TempDir,
    }

    fn fixture(files: usize) -> Fixture {
        let project = tempfile::tempdir().expect("tempdir");
        let sources = tempfile::tempdir().expect("tempdir");
        let picked: Vec<PathBuf> = (0..files)
            .map(|index| sources.path().join(format!("file{index}.obj")))
            .collect();
        for path in &picked {
            std::fs::write(path, "").expect("write source");
        }
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
            default_scene: None,
        });
        world.insert_resource(state);
        world.insert_resource(ImportStaging {
            rows: stage_sources(picked),
            visible: true,
        });
        concerto_ui::list::install_listeners(&mut world);
        concerto_ui::items::install_listeners(&mut world);
        run(&mut world, build_dialog);
        let mut fixture = Fixture {
            world,
            _project: project,
            _sources: sources,
        };
        fixture.frame();
        fixture
    }

    impl Fixture {
        fn frame(&mut self) {
            run(&mut self.world, sync_dialog);
            let mut place = update_lists_system();
            place.initialize(&mut self.world);
            place.run_and_apply((), &mut self.world);
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

        fn open(&mut self) -> bool {
            self.world
                .query::<(&DialogRoot, &UINode), ()>()
                .iter(&mut self.world)
                .next()
                .unwrap()
                .1
                .visible
        }

        fn scroll_to(&mut self, first: usize) {
            let list = self
                .world
                .query::<(Entity, &DialogView), ()>()
                .iter(&mut self.world)
                .next()
                .unwrap()
                .0;
            self.world
                .get_component_for_entity_mut::<UIScrollArea>(list)
                .expect("the list is its own scroll area")
                .offset = first as f32 * ROW_HEIGHT;
            self.frame();
        }

        fn field(&mut self, index: usize) -> Entity {
            self.world
                .query::<(Entity, &DialogField), ()>()
                .iter(&mut self.world)
                .find(|(_, field)| field.0 == index)
                .unwrap()
                .0
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

        fn click(&mut self, label: &str) {
            let button = self
                .world
                .query::<(Entity, &UIText, &Interactable), ()>()
                .iter(&mut self.world)
                .find(|(_, text, _)| text.text == label)
                .unwrap()
                .0;
            self.world.trigger_on(
                button,
                UIClick {
                    position: Vec2::ZERO,
                    button: MouseButton::Left,
                },
            );
            self.frame();
        }
    }

    #[test]
    fn typing_a_destination_edits_the_row_of_its_field_and_revalidates_every_row() {
        let mut dialog = fixture(FILES);
        dialog.scroll_to(6);
        let field = dialog.field(7);

        dialog.type_into(field, "assets/file8.obj");
        assert_eq!(dialog.staging().rows[7].destination, "assets/file8.obj");
        assert_eq!(dialog.staging().rows[1].destination, "assets/file1.obj");
        assert!(!dialog.staging().rows[7].importable());
        assert!(!dialog.staging().rows[8].importable());

        dialog.type_into(field, "assets/props/renamed.obj");
        assert_eq!(
            dialog.staging().rows[7].destination,
            "assets/props/renamed.obj"
        );
        assert!(dialog.staging().rows.iter().all(StagedImport::importable));
    }

    #[test]
    fn import_queues_the_rows_still_importable_and_cancel_queues_nothing() {
        let mut dialog = fixture(2);
        dialog.staging().rows[0].destination = "../outside.obj".into();

        dialog.click("Import");

        assert_eq!(dialog.queued(), 1);
        assert!(!dialog.open());

        let mut dialog = fixture(2);
        dialog.click("Cancel");
        assert_eq!(dialog.queued(), 0);
        assert!(!dialog.open());
        assert!(dialog.staging().rows.is_empty());
    }
}

//! The project's asset catalogue, as a docked panel.
use concerto_app::{
    App, Plugin,
    schedule_groups::{LateUpdate, Startup},
};
use concerto_color::{Color, LinearRgba};
use concerto_ecs::{
    Component, Entity, Event, IntoSystemConfig, Query, Res, ResMut, Resource, World,
    command::CommandQueue,
    component::removed::RemovedComponents,
    entity::hierarchy::ChildOf,
    events::{event_reader::EventReader, event_writer::EventWriter},
    query::{
        change_detection::DetectChanges,
        filter::{Added, Changed, Or, With},
    },
    signal::{
        On, Signal,
        lifecycle::Add,
        listener::{IntoListener, Listener},
    },
};
use concerto_ui::{
    anchor::{UIAnchorAlign, UIAnchorTarget, UIAnchoredPanel, toggle_owned_panels},
    elements::prelude::*,
    focus::{FocusedWidget, UIFocusLost},
    interaction::{Interactable, UIClick, UIDisabled},
    items::{
        UIHidden, UIItem, UIItemActivated, UIItemContextMenu, UIItemSelected, UIReveal, UISelected,
    },
    material::UIMaterial,
    node::{UINode, UIRect},
    scroll::UIScrollArea,
    sets::UiSet,
    text::UIText,
    text_input::{UITextInput, UITextInputCancelled, UITextInputChanged, UITextInputSubmitted},
    theme::UITheme,
    transform::UIValue,
    tree::{UITree, UITreeElement},
};
use concerto_window::input::MouseButton;
use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;
use taffy::FlexDirection;

use crate::asset_editor::{AssetEditorRegistry, MAX_NEW_NUMBER, new_asset_name, valid_asset_name};
use crate::dock::{DockedApp, PanelDescriptor, PanelRegistry, Region};
use crate::fonts::{AssetIcons, draw_row_icon, glyph, row_icon};
use crate::import_fold::{ImportFolds, is_import_face, shown_name};
use crate::project::{AssetsWritten, EditorCommand, EditorCommands, ProjectOpened, ProjectState};
use crate::tree_items::{OrderItems, despawn_outermost, order_items};
use concerto_foundation::assets::{
    AssetId,
    registry::{AssetEntry, AssetRegistry},
};
use concerto_mesh::mesh::Mesh;
use concerto_render::assets::{material::StandardMaterial, texture::Texture};
use concerto_scene::scene::Scene;

/// The id the Asset Browser panel is registered under in the [`PanelRegistry`].
pub const PANEL_ID: &str = "concerto.curiosities";

const ROW_HEIGHT: f32 = 28.0;
const SEARCH_HEIGHT: f32 = 30.0;
const MENU_WIDTH: f32 = 168.0;
const EXTENSION: &str = "gasset";

type KindFilter = fn(&AssetEntry) -> bool;
type NewMenus = Or<(With<NewMenu>, With<ContentMenu>)>;

const KINDS: [(&str, Option<KindFilter>); 5] = [
    ("all", None),
    ("scene", Some(AssetEntry::is::<Scene> as KindFilter)),
    ("mesh", Some(AssetEntry::is::<Mesh> as KindFilter)),
    ("tex", Some(AssetEntry::is::<Texture> as KindFilter)),
    (
        "mat",
        Some(AssetEntry::is::<StandardMaterial> as KindFilter),
    ),
];

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum ContentKey {
    Folder(String),
    Asset(AssetId),
    Pending { kind: &'static str, folder: String },
}

struct PendingAsset {
    kind: &'static str,
    folder: String,
    prefill: String,
}

impl PendingAsset {
    fn key(&self) -> ContentKey {
        ContentKey::Pending {
            kind: self.kind,
            folder: self.folder.clone(),
        }
    }
}

/// The content browser's selection, filters, the asset being named, and the item it keeps for each folder and asset.
#[derive(Resource, Default)]
pub struct ContentState {
    selected: Option<AssetId>,
    selected_folder: Option<String>,
    filter: String,
    kind: usize,
    items: HashMap<ContentKey, Entity>,
    pending: Option<PendingAsset>,
    pending_error: Option<String>,
    reveal: Option<ContentKey>,
}

impl ContentState {
    /// The asset selected in the catalogue, independent of the open editor's entity selection.
    pub fn selected(&self) -> Option<AssetId> {
        self.selected
    }
}

/// On a content browser item that shows an asset: which one.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct ContentAsset {
    /// The asset the item shows.
    pub id: AssetId,
}

#[derive(Component)]
struct ContentItem {
    key: ContentKey,
    icon: char,
    icon_node: Entity,
    label: Entity,
}

#[derive(Component)]
struct CountLabel;

#[derive(Component)]
struct Tag(usize);

#[derive(Component)]
struct Filter;

#[derive(Component)]
struct ContentView;

#[derive(Component)]
struct NewButton;

#[derive(Component)]
struct NewMenu;

#[derive(Component)]
struct NewRow;

#[derive(Component)]
struct PendingField;

#[derive(Component)]
struct ContentMenu {
    folder: String,
}

#[derive(Component)]
struct ContentMenuRow;

/// Creates an asset of a creatable `kind` named `name` in `folder`, as the Asset Browser's New does.
///
/// Without a `folder` the asset goes in the content root. With `open` set, the new asset opens in its editor.
pub struct CreateAsset {
    /// The asset kind, one the Asset Browser's New lists, ignoring case.
    pub kind: String,
    /// The folder, relative to the content root.
    pub folder: Option<String>,
    /// The new asset's name, which becomes its file's name.
    pub name: String,
    /// Whether the new asset opens in its editor.
    pub open: bool,
}
impl Signal for CreateAsset {}

/// What a `CreateAsset` came to: the new asset's id, or why nothing was written.
#[derive(Event, Clone, Debug, PartialEq)]
pub struct AssetCreation {
    /// The kind that was asked for.
    pub kind: String,
    /// The address the asset was, or would have been, written to.
    pub address: String,
    /// The new asset's id, or why nothing was written.
    pub result: Result<AssetId, String>,
}

/// Adds the Asset Browser panel and the [`CreateAsset`] signal.
pub struct ContentPlugin;

impl Plugin for ContentPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(ContentState::default());
        if app.get_resource::<AssetIcons>().is_none() {
            app.insert_resource(AssetIcons::default());
        }
        app.register_event::<AssetCreation>();
        app.track_removals::<UISelected>();
        install_listeners(app.main_mut().world_mut());
        app.add_panel(PanelDescriptor {
            id: PANEL_ID,
            title: "Asset Browser",
            region: Region::Bottom,
        });
        app.add_system(Startup, build_panel)
            .add_system(
                LateUpdate,
                update_filter.after(UiSet::Widgets).before(UiSet::Project),
            )
            .add_system(
                LateUpdate,
                commit_pending_asset
                    .after(UiSet::Widgets)
                    .before(reconcile_content_tree),
            )
            .add_system(
                LateUpdate,
                reconcile_content_tree
                    .after(update_filter)
                    .before(UiSet::Project),
            )
            .add_system(LateUpdate, rebuild_new_rows)
            .add_system(LateUpdate, sync_new_items.after(rebuild_new_rows))
            .add_system(
                LateUpdate,
                sync_content_items
                    .after(reconcile_content_tree)
                    .before(UiSet::Materials),
            )
            .add_system(LateUpdate, render_tags.before(UiSet::Materials));
    }
}

fn install_listeners(world: &mut World) {
    world.add_listener(focus_pending_field);
    world.add_listener(create_asset);
}

fn visible_assets<'a>(
    registry: &'a AssetRegistry,
    folds: &ImportFolds,
    state: &ContentState,
) -> Vec<&'a AssetEntry> {
    let kind = KINDS[state.kind].1;
    let query = state.filter.to_lowercase();
    registry
        .iter()
        .filter(|asset| kind.is_none_or(|is_kind| is_kind(asset)))
        .filter(|asset| folds.matches(asset, &query))
        .collect()
}

fn build_panel(
    mut cmd: CommandQueue,
    registry: Res<PanelRegistry>,
    editors: Res<AssetEditorRegistry>,
    theme: Res<UITheme>,
) {
    let Some(body) = registry.body(PANEL_ID) else {
        return;
    };
    let kinds: Vec<&'static str> = editors.creatable().map(|(kind, _)| kind).collect();
    spawn_panel(&mut cmd, body, &theme, &kinds);
}

fn spawn_panel(cmd: &mut CommandQueue, body: Entity, theme: &UITheme, creatable: &[&'static str]) {
    let panel = cmd
        .entity(body)
        .spawn_child(
            UINode::default()
                .with_flex_grow(1.0)
                .with_flex_direction(FlexDirection::Column)
                .with_padding(UIRect::all(10.0))
                .clipped(),
        )
        .entity();
    let bar = cmd
        .entity(panel)
        .spawn_child(theme.row().gap(6.0).fixed())
        .entity();
    cmd.entity(bar)
        .add_child_with(
            theme
                .card()
                .radius_sm()
                .height(UIValue::Px(SEARCH_HEIGHT))
                .grow()
                .shrink(1.0)
                .min_width(UIValue::Px(0.0))
                .row()
                .padding(UIRect::axes(0.0, 8.0))
                .gap(6.0),
            |mut search| {
                search
                    .add_child(theme.label("⌕").muted().no_wrap())
                    .add_child((
                        theme
                            .text_field("Find assets…")
                            .bare()
                            .height(UIValue::Auto)
                            .padding(0.0)
                            .single_line()
                            .grow()
                            .shrink(1.0)
                            .min_width(UIValue::Px(0.0)),
                        Filter,
                    ));
            },
        )
        .add_child(
            theme
                .button("Import…")
                .height(UIValue::Px(SEARCH_HEIGHT))
                .padding(UIRect::axes(6.0, 10.0))
                .on_click(request_import),
        );
    let new_button = cmd
        .entity(bar)
        .spawn_child((
            theme
                .button("New")
                .height(UIValue::Px(SEARCH_HEIGHT))
                .padding(UIRect::axes(6.0, 10.0))
                .on_click(toggle_owned_panels),
            NewButton,
        ))
        .entity();

    cmd.entity(panel)
        .add_child_with(
            UINode::default()
                .with_flex_shrink(0.0)
                .with_flex_direction(FlexDirection::Row)
                .with_gap(glam::Vec2::new(4.0, 0.0))
                .with_margin(UIRect::axes(7.0, 0.0)),
            |mut tags| {
                for (index, (name, _)) in KINDS.iter().enumerate() {
                    tags.add_child((theme.chip(*name).on_click(select_kind), Tag(index)));
                }
            },
        )
        .add_child((
            UINode::default()
                .with_height(UIValue::Px(0.0))
                .with_flex_grow(1.0),
            UITree {
                row_height: Some(ROW_HEIGHT),
                ..Default::default()
            },
            ContentView,
            Interactable,
            order_items.into_listener(),
            open_root_menu.into_listener(),
        ))
        .add_child((
            theme
                .label("")
                .min_height(UIValue::Px(26.0))
                .fixed()
                .padding(UIRect::axes(4.0, 10.0)),
            CountLabel,
        ));

    let menu = cmd
        .spawn((
            theme
                .dropdown(new_button)
                .anchor_align(UIAnchorAlign::End)
                .width(UIValue::Px(MENU_WIDTH))
                .padding(theme.spacing_xs),
            NewMenu,
        ))
        .entity();
    let context_menu = cmd
        .spawn((
            theme
                .context_menu()
                .width(UIValue::Px(MENU_WIDTH))
                .padding(theme.spacing_xs),
            ContentMenu {
                folder: String::new(),
            },
        ))
        .entity();
    spawn_new_rows(cmd, menu, context_menu, theme, creatable);
}

fn spawn_new_rows(
    cmd: &mut CommandQueue,
    menu: Entity,
    context_menu: Entity,
    theme: &UITheme,
    kinds: &[&'static str],
) {
    for &kind in kinds {
        cmd.entity(menu).add_child_with(
            (
                theme
                    .pressable()
                    .row()
                    .padding(UIRect::axes(6.0, 8.0))
                    .on_click(start_from_dropdown(kind)),
                NewRow,
            ),
            |mut entry| {
                entry.add_child(theme.label(format!("New {kind}")).font_size(12.5).no_wrap());
            },
        );
        cmd.entity(context_menu).add_child_with(
            (
                theme
                    .pressable()
                    .row()
                    .padding(UIRect::axes(6.0, 8.0))
                    .on_click(start_from_menu(kind)),
                ContentMenuRow,
            ),
            |mut entry| {
                entry.add_child(theme.label(format!("New {kind}")).font_size(12.5).no_wrap());
            },
        );
    }
}

fn update_filter(
    mut changes: EventReader<UITextInputChanged>,
    filters: Query<&Filter>,
    mut views: Query<&mut UIScrollArea, With<ContentView>>,
    mut state: ResMut<ContentState>,
) {
    let mut reset = false;
    for change in changes.read() {
        if filters.get_entity(change.entity).is_some() {
            state.filter = change.value.clone();
            reset = true;
        }
    }
    if reset && let Some(mut area) = views.iter_mut().next() {
        area.offset = 0.0;
    }
}

fn select_kind(
    on: On<UIClick>,
    tags: Query<&Tag>,
    project: Res<ProjectState>,
    mut state: ResMut<ContentState>,
    mut views: Query<&mut UIScrollArea, With<ContentView>>,
) {
    if on.signal().button != MouseButton::Left || project.busy() {
        return;
    }
    let Some(tag) = tags.get_entity(on.entity()) else {
        return;
    };
    state.kind = tag.0;
    if let Some(mut area) = views.iter_mut().next() {
        area.offset = 0.0;
    }
}

fn request_import(
    on: On<UIClick>,
    mut picker: ResMut<crate::import::ImportPicker>,
    mut project: ResMut<ProjectState>,
) {
    if on.signal().button != MouseButton::Left {
        return;
    }
    if project.project.is_some() {
        picker.request();
    } else if !project.busy() {
        project.status = "Open a project before importing.".into();
    }
}

fn select_item(
    on: On<UIItemSelected>,
    items: Query<&ContentItem>,
    selected: Query<&UISelected>,
    project: Res<ProjectState>,
    mut state: ResMut<ContentState>,
    mut cmd: CommandQueue,
) {
    let entity = on.entity();
    if project.busy() {
        cmd.remove::<UISelected>(entity);
        if state.selected_folder.is_some() {
            state.selected_folder = None;
        }
        if let Some(&item) = state
            .selected
            .and_then(|id| state.items.get(&ContentKey::Asset(id)))
        {
            cmd.insert(UISelected, item);
        }
        return;
    }
    match items.get_entity(entity).map(|item| &item.key) {
        Some(ContentKey::Asset(id)) => {
            state.selected = Some(*id);
            if state.selected_folder.is_some() {
                state.selected_folder = None;
            }
            if !selected.contains_entity(entity) {
                cmd.insert(UISelected, entity);
            }
        }
        Some(ContentKey::Folder(key)) => {
            let folder = project
                .project
                .as_ref()
                .map(|project| format!("{}/{key}", project.content_root));
            if state.selected_folder != folder {
                state.selected_folder = folder;
            }
        }
        Some(ContentKey::Pending { .. }) if state.selected_folder.is_some() => {
            state.selected_folder = None;
        }
        _ => {}
    }
}

fn start_from_dropdown(kind: &'static str) -> Listener<UIClick> {
    (move |on: On<UIClick>,
           mut project: ResMut<ProjectState>,
           registry: Res<AssetRegistry>,
           mut state: ResMut<ContentState>,
           mut filters: Query<&mut UITextInput, With<Filter>>,
           mut menus: Query<&mut UIAnchoredPanel, NewMenus>| {
        if on.signal().button != MouseButton::Left {
            return;
        }
        let folder = state.selected_folder.clone();
        start_new(
            kind,
            folder,
            &mut project,
            &registry,
            &mut state,
            &mut filters,
            &mut menus,
        );
    })
    .into_listener()
}

fn start_from_menu(kind: &'static str) -> Listener<UIClick> {
    (move |on: On<UIClick>,
           mut project: ResMut<ProjectState>,
           registry: Res<AssetRegistry>,
           mut state: ResMut<ContentState>,
           mut filters: Query<&mut UITextInput, With<Filter>>,
           mut menus: Query<&mut UIAnchoredPanel, NewMenus>,
           targets: Query<&ContentMenu>| {
        if on.signal().button != MouseButton::Left {
            return;
        }
        let folder = targets.iter().next().map(|menu| menu.folder.clone());
        start_new(
            kind,
            folder,
            &mut project,
            &registry,
            &mut state,
            &mut filters,
            &mut menus,
        );
    })
    .into_listener()
}

fn open_folder_menu(key: String) -> Listener<UIItemContextMenu> {
    (move |on: On<UIItemContextMenu>,
           project: Res<ProjectState>,
           mut menus: Query<(&mut ContentMenu, &mut UIAnchoredPanel)>| {
        if let Some(open) = project.project.as_ref() {
            let folder = format!("{}/{key}", open.content_root);
            open_content_menu(folder, on.signal().position, Some(on.entity()), &mut menus);
        }
    })
    .into_listener()
}

fn open_root_menu(
    on: On<UIClick>,
    project: Res<ProjectState>,
    mut menus: Query<(&mut ContentMenu, &mut UIAnchoredPanel)>,
) {
    let click = on.signal();
    if click.button != MouseButton::Right {
        return;
    }
    if let Some(open) = project.project.as_ref() {
        open_content_menu(open.content_root.clone(), click.position, None, &mut menus);
    }
}

fn open_content_menu(
    folder: String,
    position: glam::Vec2,
    owner: Option<Entity>,
    menus: &mut Query<(&mut ContentMenu, &mut UIAnchoredPanel)>,
) {
    for (mut menu, mut panel) in menus.iter_mut() {
        if menu.folder != folder {
            menu.folder = folder.clone();
        }
        panel.target = UIAnchorTarget::from_point(position);
        panel.owner = owner;
        panel.open = true;
    }
}

fn start_new(
    kind: &'static str,
    folder: Option<String>,
    project: &mut ResMut<ProjectState>,
    registry: &AssetRegistry,
    state: &mut ContentState,
    filters: &mut Query<&mut UITextInput, With<Filter>>,
    menus: &mut Query<&mut UIAnchoredPanel, NewMenus>,
) {
    for mut panel in menus.iter_mut() {
        if panel.open {
            panel.open = false;
        }
    }
    if project.busy() {
        return;
    }
    let Some(open) = project.project.as_ref() else {
        project.status = "Open a project before creating an asset.".into();
        return;
    };
    let folder = folder.unwrap_or_else(|| open.content_root.clone());
    let taken = taken_names(&folder, &open.root, registry);
    let prefill = (0..=MAX_NEW_NUMBER)
        .map(|number| new_asset_name(kind, number))
        .find(|name| !taken.contains(&file_key(name)))
        .unwrap_or_else(|| new_asset_name(kind, 0));
    state.pending = Some(PendingAsset {
        kind,
        folder,
        prefill,
    });
    state.pending_error = None;
    state.reveal = state.pending.as_ref().map(PendingAsset::key);
    state.filter.clear();
    state.kind = 0;
    for mut input in filters.iter_mut() {
        if !input.value.is_empty() {
            input.value.clear();
            input.cursor = 0;
            input.selection_anchor = None;
        }
    }
}

fn file_key(name: &str) -> String {
    format!("{name}.{EXTENSION}").to_lowercase()
}

fn taken_names(folder: &str, project_root: &Path, registry: &AssetRegistry) -> HashSet<String> {
    let on_disk = std::fs::read_dir(project_root.join(folder))
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().to_lowercase());
    let listed = registry
        .iter()
        .filter(|asset| asset.folder().eq_ignore_ascii_case(folder))
        .map(|asset| file_key(&asset.name()));
    on_disk.chain(listed).collect()
}

fn focus_pending_field(on: On<Add<PendingField>>, mut focused: ResMut<FocusedWidget>) {
    **focused = Some(on.signal().entity);
}

fn finish_pending(state: &mut ContentState, focused: &mut FocusedWidget, field: Option<Entity>) {
    state.pending = None;
    state.pending_error = None;
    if field.is_some() && **focused == field {
        **focused = None;
    }
}

fn commit_pending_asset(
    (mut changed, mut submitted, mut cancelled, mut lost, mut opened): (
        EventReader<UITextInputChanged>,
        EventReader<UITextInputSubmitted>,
        EventReader<UITextInputCancelled>,
        EventReader<UIFocusLost>,
        EventReader<ProjectOpened>,
    ),
    fields: Query<(Entity, &UITextInput), With<PendingField>>,
    mut state: ResMut<ContentState>,
    mut focused: ResMut<FocusedWidget>,
    highlighted: Query<(Entity, &ContentItem), With<UISelected>>,
    mut cmd: CommandQueue,
) {
    let field = fields.iter().next();
    let on_field = |entity: Entity| field.is_some_and(|(field, _)| field == entity);
    let typed = changed
        .read()
        .fold(false, |hit, event| hit | on_field(event.entity));
    let submit = submitted
        .read()
        .fold(false, |hit, event| hit | on_field(event.entity));
    let cancel = cancelled
        .read()
        .fold(false, |hit, event| hit | on_field(event.entity));
    let lost = lost
        .read()
        .fold(false, |hit, event| hit | on_field(event.0));
    if opened.read().count() > 0 {
        if state.pending.is_some() || state.pending_error.is_some() {
            finish_pending(&mut state, &mut focused, field.map(|(field, _)| field));
        }
        if state.selected_folder.is_some() {
            state.selected_folder = None;
        }
        for (item, content) in highlighted.iter() {
            if matches!(content.key, ContentKey::Folder(_)) {
                cmd.remove::<UISelected>(item);
            }
        }
        return;
    }
    let (Some((field, input)), Some(pending)) = (field, state.pending.as_ref()) else {
        return;
    };
    let (kind, folder) = (pending.kind, pending.folder.clone());
    if cancel {
        finish_pending(&mut state, &mut focused, Some(field));
        return;
    }
    if typed && state.pending_error.is_some() {
        state.pending_error = None;
    }
    if !submit && !lost {
        return;
    }
    cmd.trigger(CreateAsset {
        kind: kind.into(),
        folder: Some(folder),
        name: input.value.clone(),
        open: true,
    });
}

enum CreateRefusal {
    Request(String),
    Write(String),
}

impl CreateRefusal {
    fn message(&self) -> &str {
        match self {
            Self::Request(message) | Self::Write(message) => message,
        }
    }
}

fn valid_folder(folder: &str, content_root: &str) -> Result<(), String> {
    let inside = folder
        .strip_prefix(content_root)
        .filter(|rest| rest.is_empty() || rest.starts_with('/'))
        .ok_or_else(|| format!("{folder} is not inside {content_root}"))?;
    inside.split('/').skip(1).try_for_each(|segment| {
        valid_asset_name(segment).map_err(|error| format!("{folder}: {error}"))
    })
}

fn create_asset(
    on: On<CreateAsset>,
    editors: Res<AssetEditorRegistry>,
    fields: Query<Entity, With<PendingField>>,
    mut project: ResMut<ProjectState>,
    mut registry: ResMut<AssetRegistry>,
    mut state: ResMut<ContentState>,
    mut focused: ResMut<FocusedWidget>,
    mut commands: ResMut<EditorCommands>,
    mut written: EventWriter<AssetsWritten>,
    mut outcome: EventWriter<AssetCreation>,
) {
    let request = on.signal();
    let name = request.name.as_str();
    let folder = request.folder.clone().or_else(|| {
        project
            .project
            .as_ref()
            .map(|open| open.content_root.clone())
    });
    let named = state.pending.as_ref().is_some_and(|pending| {
        pending.kind == request.kind && Some(&pending.folder) == folder.as_ref()
    });
    let field = fields.iter().next().filter(|_| named);
    let keeps_focus = field.is_some() && **focused == field;
    let address = format!(
        "{}/{name}.{EXTENSION}",
        folder.as_deref().unwrap_or_default()
    );
    let checked = match (project.project.as_ref(), folder.as_deref()) {
        _ if project.busy() => Err("Wait for the project to finish".to_string()),
        (Some(open), Some(folder)) => valid_folder(folder, &open.content_root)
            .and_then(|()| valid_asset_name(name))
            .and_then(|()| {
                if taken_names(folder, &open.root, &registry).contains(&file_key(name)) {
                    Err(format!("{address} already exists"))
                } else {
                    Ok(open.root.clone())
                }
            }),
        _ => Err("Open a project before creating an asset.".to_string()),
    };
    let created = checked.map_err(CreateRefusal::Request).and_then(|root| {
        let created = match editors
            .creatable()
            .find(|(creatable, _)| *creatable == request.kind)
        {
            Some((_, creator)) => creator.create(&root, &address, &mut registry),
            None => Err(anyhow::anyhow!("{} cannot be created", request.kind)),
        };
        created
            .map_err(|error| CreateRefusal::Write(format!("Could not create {address}: {error:#}")))
    });
    let result = match created {
        Ok(id) => {
            if named {
                finish_pending(&mut state, &mut focused, field);
            }
            state.selected = Some(id);
            state.selected_folder = None;
            state.reveal = Some(ContentKey::Asset(id));
            project.status = format!("Created {address}");
            written.write(AssetsWritten { ids: vec![id] });
            if request.open {
                commands.0.push_back(EditorCommand::OpenAsset(id));
            }
            Ok(id)
        }
        Err(refusal) => {
            if keeps_focus {
                state.pending_error = Some(refusal.message().into());
            } else if named {
                finish_pending(&mut state, &mut focused, field);
                if let CreateRefusal::Write(message) = &refusal {
                    project.status = message.clone();
                }
            }
            Err(refusal.message().into())
        }
    };
    outcome.write(AssetCreation {
        kind: request.kind.clone(),
        address,
        result,
    });
}

fn rebuild_new_rows(
    editors: Res<AssetEditorRegistry>,
    theme: Res<UITheme>,
    menus: Query<Entity, With<NewMenu>>,
    context_menus: Query<Entity, With<ContentMenu>>,
    rows: Query<Entity, Or<(With<NewRow>, With<ContentMenuRow>)>>,
    mut cmd: CommandQueue,
) {
    if !editors.is_changed() {
        return;
    }
    let (Some(menu), Some(context_menu)) = (menus.iter().next(), context_menus.iter().next())
    else {
        return;
    };
    for row in rows.iter() {
        cmd.despawn(row);
    }
    let kinds: Vec<&'static str> = editors.creatable().map(|(kind, _)| kind).collect();
    spawn_new_rows(&mut cmd, menu, context_menu, &theme, &kinds);
}

fn sync_new_items(
    project: Res<ProjectState>,
    controls: Query<
        (Entity, Option<&UIDisabled>),
        Or<(With<NewButton>, With<NewRow>, With<ContentMenuRow>)>,
    >,
    mut cmd: CommandQueue,
) {
    let disabled = project.project.is_none() || project.busy();
    for (entity, state) in controls.iter() {
        if disabled && state.is_none() {
            cmd.insert(UIDisabled, entity);
        } else if !disabled && state.is_some() {
            cmd.remove::<UIDisabled>(entity);
        }
    }
}

fn open_item(
    on: On<UIItemActivated>,
    items: Query<(&ContentItem, Option<&UIItem>)>,
    mut elements: Query<&mut UITreeElement>,
    trees: Query<&UITree>,
    project: Res<ProjectState>,
    mut commands: ResMut<EditorCommands>,
) {
    if project.busy() {
        return;
    }
    let entity = on.entity();
    let Some((item, placed)) = items.get_entity(entity) else {
        return;
    };
    match item.key {
        ContentKey::Asset(id) => commands.0.push_back(EditorCommand::OpenAsset(id)),
        ContentKey::Pending { .. } => {}
        ContentKey::Folder(_) => {
            let filtering = placed
                .and_then(|placed| trees.get_entity(placed.owner()))
                .is_some_and(|tree| tree.filtering);
            if !filtering && let Some(mut element) = elements.get_entity_mut(entity) {
                element.expanded = !element.expanded;
            }
        }
    }
}

#[derive(Default)]
struct Folder<'a> {
    folders: BTreeMap<(String, &'a str), Folder<'a>>,
    assets: Vec<&'a AssetEntry>,
}

#[derive(Clone)]
struct Listed<'a> {
    key: ContentKey,
    parent: Option<ContentKey>,
    name: Cow<'a, str>,
    icon: char,
}

fn inside_root<'a>(folder: &'a str, content_root: &str) -> &'a str {
    match folder.strip_prefix(content_root) {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => rest,
        _ => folder,
    }
}

fn folder_above(folder: &str) -> Option<&str> {
    folder.rsplit_once('/').map(|(above, _)| above)
}

fn folder_tree<'a>(
    registry: &'a AssetRegistry,
    folds: &ImportFolds<'a>,
    content_root: &str,
) -> Folder<'a> {
    let rows: Vec<&AssetEntry> = registry
        .iter()
        .filter(|asset| folds.face_of(asset).is_none())
        .collect();
    let mut held: HashMap<&str, usize> = HashMap::new();
    let mut above: HashSet<&str> = HashSet::new();
    for asset in &rows {
        *held.entry(asset.folder()).or_default() += 1;
        let mut folder = asset.folder();
        while let Some(parent) = folder_above(folder) {
            above.insert(parent);
            folder = parent;
        }
    }
    let mut root = Folder::default();
    for asset in rows {
        let own = asset.folder();
        let alone = is_import_face(asset)
            && held[own] == 1
            && !above.contains(own)
            && !inside_root(own, content_root).is_empty();
        let shown_in = if alone {
            folder_above(own).unwrap_or("")
        } else {
            own
        };
        let mut folder = &mut root;
        for part in inside_root(shown_in, content_root)
            .split('/')
            .filter(|part| !part.is_empty())
        {
            folder = folder
                .folders
                .entry((part.to_lowercase(), part))
                .or_default();
        }
        folder.assets.push(asset);
    }
    root
}

fn by_shown_name(assets: &mut [&AssetEntry]) {
    assets.sort_by(|a, b| {
        let (a_name, b_name) = (shown_name(a), shown_name(b));
        (a_name.to_lowercase(), a_name, &a.kind, &a.address).cmp(&(
            b_name.to_lowercase(),
            b_name,
            &b.kind,
            &b.address,
        ))
    });
}

fn list_folder<'a>(
    folder: &Folder<'a>,
    path: Option<&str>,
    pending: Option<&Listed<'a>>,
    folds: &ImportFolds<'a>,
    icons: &AssetIcons,
    listed: &mut Vec<Listed<'a>>,
) {
    let parent = path.map(|path| ContentKey::Folder(path.to_string()));
    for (&(_, name), child) in &folder.folders {
        let child_path = match path {
            Some(path) => format!("{path}/{name}"),
            None => name.to_string(),
        };
        listed.push(Listed {
            key: ContentKey::Folder(child_path.clone()),
            parent: parent.clone(),
            name: Cow::Borrowed(name),
            icon: glyph::FOLDER,
        });
        list_folder(child, Some(&child_path), pending, folds, icons, listed);
    }
    if let Some(pending) = pending.filter(|pending| pending.parent == parent) {
        listed.push(pending.clone());
    }
    let mut assets = folder.assets.clone();
    by_shown_name(&mut assets);
    for asset in assets {
        listed.push(Listed {
            key: ContentKey::Asset(asset.id),
            parent: parent.clone(),
            name: shown_name(asset),
            icon: icons.glyph(&asset.kind),
        });
        let mut parts = folds.parts_of(asset.id).to_vec();
        by_shown_name(&mut parts);
        for part in parts {
            listed.push(Listed {
                key: ContentKey::Asset(part.id),
                parent: Some(ContentKey::Asset(asset.id)),
                name: part.name(),
                icon: icons.glyph(&part.kind),
            });
        }
    }
}

fn spawn_item(cmd: &mut CommandQueue, theme: &UITheme, entry: &Listed, parent: Entity) -> Entity {
    let icon_node = cmd.spawn(row_icon(theme, entry.icon)).entity();
    let label = cmd
        .spawn((
            UINode::default()
                .with_flex_grow(1.0)
                .with_flex_shrink(1.0)
                .with_padding(UIRect::axes(0.0, 8.0)),
            theme.text(&*entry.name).single_line(),
        ))
        .entity();
    let item = cmd
        .spawn((
            theme.canvas().fill(Color::TRANSPARENT).radius_sm(),
            UITreeElement::default(),
            ContentItem {
                key: entry.key.clone(),
                icon: entry.icon,
                icon_node,
                label,
            },
            select_item.into_listener(),
            open_item.into_listener(),
            ChildOf::new(parent),
        ))
        .entity();
    match &entry.key {
        ContentKey::Folder(key) => cmd.insert(open_folder_menu(key.clone()), item),
        ContentKey::Asset(id) => cmd.insert(ContentAsset { id: *id }, item),
        ContentKey::Pending { .. } => {}
    }
    for content in [icon_node, label] {
        cmd.add_child(item, content);
    }
    item
}

fn spawn_pending_item(
    cmd: &mut CommandQueue,
    theme: &UITheme,
    entry: &Listed,
    prefill: &str,
    parent: Entity,
) -> Entity {
    let icon_node = cmd.spawn(row_icon(theme, entry.icon)).entity();
    let field = cmd
        .spawn((
            theme
                .text_field("Name")
                .value(prefill)
                .single_line()
                .height(UIValue::Px(ROW_HEIGHT - 6.0))
                .padding(UIRect::axes(0.0, 6.0))
                .margin(UIRect::axes(0.0, 4.0))
                .grow()
                .shrink(1.0)
                .min_width(UIValue::Px(0.0)),
            PendingField,
        ))
        .entity();
    let item = cmd
        .spawn((
            theme.canvas().fill(Color::TRANSPARENT).radius_sm(),
            UITreeElement::default(),
            ContentItem {
                key: entry.key.clone(),
                icon: entry.icon,
                icon_node,
                label: field,
            },
            select_item.into_listener(),
            ChildOf::new(parent),
        ))
        .entity();
    for content in [icon_node, field] {
        cmd.add_child(item, content);
    }
    item
}

impl PendingAsset {
    fn pending_listing(&self, content_root: &str, icons: &AssetIcons) -> Listed<'static> {
        let inside = inside_root(&self.folder, content_root).trim_start_matches('/');
        Listed {
            key: self.key(),
            parent: (!inside.is_empty()).then(|| ContentKey::Folder(inside.to_string())),
            name: Cow::Borrowed(""),
            icon: icons.glyph(self.kind),
        }
    }
}

fn list_content<'a>(
    project: &ProjectState,
    registry: &'a AssetRegistry,
    folds: &ImportFolds<'a>,
    state: &ContentState,
    icons: &AssetIcons,
) -> Vec<Listed<'a>> {
    let mut listed = Vec::new();
    if let Some(project) = project.project.as_ref() {
        let pending = state
            .pending
            .as_ref()
            .map(|pending| pending.pending_listing(&project.content_root, icons));
        list_folder(
            &folder_tree(registry, folds, &project.content_root),
            None,
            pending.as_ref(),
            folds,
            icons,
            &mut listed,
        );
        if let Some(pending) = pending
            && !listed.iter().any(|entry| entry.key == pending.key)
        {
            listed.push(pending);
        }
    }
    listed
}

struct ItemPlace {
    item: Entity,
    hide: bool,
    error_border: LinearRgba,
}

fn refresh_item(
    entry: &Listed,
    place: &ItemPlace,
    items: &mut Query<(&mut ContentItem, Option<&UIHidden>, Option<&UIItem>)>,
    texts: &mut Query<&mut UIText>,
    materials: &mut Query<&mut UIMaterial>,
    cmd: &mut CommandQueue,
) {
    let ItemPlace {
        item,
        hide,
        error_border,
    } = *place;
    let Some((mut content, hidden, _)) = items.get_entity_mut(item) else {
        return;
    };
    if content.icon != entry.icon {
        content.icon = entry.icon;
    }
    if let ContentKey::Pending { .. } = entry.key {
        if let Some(mut material) = materials.get_entity_mut(content.label)
            && material.border_color != error_border
        {
            material.border_color = error_border;
        }
    } else if let Some(mut text) = texts.get_entity_mut(content.label)
        && text.text != entry.name
    {
        text.text = entry.name.to_string();
    }
    if hide && hidden.is_none() {
        cmd.insert(UIHidden, item);
    } else if !hide && hidden.is_some() {
        cmd.remove::<UIHidden>(item);
    }
}

fn spawn_listed_item(
    cmd: &mut CommandQueue,
    theme: &UITheme,
    state: &ContentState,
    entry: &Listed,
    parent: Entity,
    hide: bool,
) -> Entity {
    let item = match &state.pending {
        Some(pending) if pending.key() == entry.key => {
            spawn_pending_item(cmd, theme, entry, &pending.prefill, parent)
        }
        _ => spawn_item(cmd, theme, entry, parent),
    };
    if hide {
        cmd.insert(UIHidden, item);
    }
    if matches!(entry.key, ContentKey::Asset(id) if state.selected == Some(id)) {
        cmd.insert(UISelected, item);
    }
    item
}

fn reveal_requested(
    state: &mut ContentState,
    items: &mut Query<(&mut ContentItem, Option<&UIHidden>, Option<&UIItem>)>,
    cmd: &mut CommandQueue,
) {
    if let Some(key) = state.reveal.as_ref() {
        match state.items.get(key).copied() {
            None => state.reveal = None,
            Some(item)
                if items
                    .get_entity_mut(item)
                    .is_some_and(|(_, _, placed)| placed.is_some()) =>
            {
                cmd.entity(item).trigger(UIReveal);
                state.reveal = None;
            }
            Some(_) => {}
        }
    }
}

fn write_count(
    state: &ContentState,
    matching: usize,
    counts: &Query<Entity, With<CountLabel>>,
    texts: &mut Query<&mut UIText>,
) {
    let count = match &state.pending_error {
        Some(error) => error.clone(),
        None if matching == 0 => String::new(),
        None => format!("{matching} assets"),
    };
    for label in counts.iter() {
        if let Some(mut text) = texts.get_entity_mut(label)
            && text.text != count
        {
            text.text = count.clone();
        }
    }
}

fn reconcile_content_tree(
    project: Res<ProjectState>,
    registry: Res<AssetRegistry>,
    mut state: ResMut<ContentState>,
    mut views: Query<(Entity, &mut UITree), With<ContentView>>,
    mut items: Query<(&mut ContentItem, Option<&UIHidden>, Option<&UIItem>)>,
    parents: Query<&ChildOf>,
    mut texts: Query<&mut UIText>,
    mut materials: Query<&mut UIMaterial>,
    counts: Query<Entity, With<CountLabel>>,
    theme: Res<UITheme>,
    icons: Res<AssetIcons>,
    mut cmd: CommandQueue,
) {
    if !project.is_changed()
        && !registry.is_changed()
        && !icons.is_changed()
        && !state.is_changed()
        && state.reveal.is_none()
    {
        return;
    }
    let Some((view, mut tree)) = views.iter_mut().next() else {
        return;
    };
    let filtering = !state.filter.is_empty() || state.kind != 0;
    if tree.filtering != filtering {
        tree.filtering = filtering;
    }
    let folds = ImportFolds::new(&registry);
    let matching: HashSet<AssetId> = visible_assets(&registry, &folds, &state)
        .iter()
        .map(|asset| asset.id)
        .collect();
    let listed = list_content(&project, &registry, &folds, &state, &icons);
    let error_border = if state.pending_error.is_some() {
        theme.error
    } else {
        theme.border
    }
    .to_linear();

    let mut previous = std::mem::take(&mut state.items);
    let mut kept: HashMap<ContentKey, Entity> = HashMap::new();
    let mut order: Vec<(Entity, Vec<Entity>)> = Vec::new();
    for entry in &listed {
        let parent = entry
            .parent
            .as_ref()
            .and_then(|key| kept.get(key).copied())
            .unwrap_or(view);
        let hide = filtering
            && match &entry.key {
                ContentKey::Folder(_) => true,
                ContentKey::Asset(id) => !matching.contains(id),
                ContentKey::Pending { .. } => false,
            };
        let item = match previous.remove(&entry.key) {
            Some(item) => {
                if parents.get_entity(item).map(ChildOf::parent) != Some(parent) {
                    cmd.add_child(parent, item);
                }
                let place = ItemPlace {
                    item,
                    hide,
                    error_border,
                };
                refresh_item(
                    entry,
                    &place,
                    &mut items,
                    &mut texts,
                    &mut materials,
                    &mut cmd,
                );
                item
            }
            None => spawn_listed_item(&mut cmd, &theme, &state, entry, parent, hide),
        };
        kept.insert(entry.key.clone(), item);
        match order.iter_mut().find(|(owner, _)| *owner == parent) {
            Some((_, siblings)) => siblings.push(item),
            None => order.push((parent, vec![item])),
        }
    }
    let gone: HashSet<Entity> = previous.into_values().collect();
    despawn_outermost(&gone, &parents, &mut cmd);
    state.items = kept;
    cmd.entity(view).trigger(OrderItems(order));

    reveal_requested(&mut state, &mut items, &mut cmd);
    write_count(&state, matching.len(), &counts, &mut texts);
}

fn sync_content_items(
    items: Query<(&ContentItem, &UITreeElement, Option<&UISelected>)>,
    changed: Query<
        Entity,
        Or<(
            Changed<ContentItem>,
            Changed<UITreeElement>,
            Added<UISelected>,
        )>,
    >,
    mut deselected: RemovedComponents<UISelected>,
    mut texts: Query<&mut UIText>,
    theme: Res<UITheme>,
) {
    let mut rows: HashSet<Entity> = deselected.read().collect();
    let apply = |(item, element, selected): (&ContentItem, &UITreeElement, Option<&UISelected>)| {
        let glyph = match item.key {
            ContentKey::Folder(_) if element.expanded => glyph::FOLDER_OPEN,
            _ => item.icon,
        };
        if let Some(mut text) = texts.get_entity_mut(item.icon_node) {
            draw_row_icon(&mut text, &theme, glyph, selected.is_some());
        }
    };
    if theme.is_changed() {
        items.iter().for_each(apply);
        return;
    }
    rows.extend(changed.iter());
    rows.into_iter()
        .filter_map(|row| items.get_entity(row))
        .for_each(apply);
}

fn render_tags(
    state: Res<ContentState>,
    theme: Res<UITheme>,
    mut tags: Query<(&Tag, &mut UIMaterial, &mut UIText)>,
) {
    for (tag, mut material, mut text) in tags.iter_mut() {
        let colors = theme.chip_colors(tag.0 == state.kind);
        let fill = colors.fill.to_linear();
        if material.color != fill {
            material.color = fill;
        }
        let border = colors.border.to_linear();
        if material.border_color != border {
            material.border_color = border;
        }
        if text.color != colors.text {
            text.color = colors.text;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        asset_editor::{ActiveEditor, AssetEditorRegistry},
        hosted::testing::SceneFixture,
        project::{AssetsWritten, Project, ProjectOpened, discover_project},
        scene::SceneEditor,
    };
    use concerto_ecs::{
        Event, IntoSystem, System,
        entity::hierarchy::{ChildOf, Children},
        events::{event_channel::EventChannel, event_cursor::EventCursor},
        query::filter::With,
        system::BoxedSystem,
    };
    use concerto_foundation::assets::{Asset, content::read_content_asset};
    use concerto_ui::{
        focus::{FocusedWidget, UIFocusLost},
        text_input::{UITextInput, UITextInputCancelled, UITextInputSubmitted},
        tree::update_trees_system,
    };
    use glam::Vec2;
    use std::path::{Path, PathBuf};

    fn run<M>(world: &mut World, system: impl IntoSystem<(), M>) {
        let mut system = system.into_system();
        system.initialize(world);
        system.run_and_apply((), world);
    }

    fn build(mut cmd: CommandQueue, theme: Res<UITheme>, editors: Res<AssetEditorRegistry>) {
        let body = cmd.spawn(UINode::default()).entity();
        let kinds: Vec<&'static str> = editors.creatable().map(|(kind, _)| kind).collect();
        spawn_panel(&mut cmd, body, &theme, &kinds);
    }

    fn channel<E: Event + 'static>(world: &mut World) {
        if world.get_resource::<EventChannel<E>>().is_none() {
            world.insert_resource(EventChannel::<E>::new());
        }
    }

    fn browser(world: &mut World) -> Vec<BoxedSystem> {
        if world.get_resource::<ContentState>().is_none() {
            world.insert_resource(ContentState::default());
        }
        if world.get_resource::<EditorCommands>().is_none() {
            world.insert_resource(EditorCommands::default());
        }
        if world.get_resource::<AssetIcons>().is_none() {
            world.insert_resource(AssetIcons::default());
        }
        channel::<UITextInputChanged>(world);
        channel::<UITextInputSubmitted>(world);
        channel::<UITextInputCancelled>(world);
        channel::<UIFocusLost>(world);
        channel::<ProjectOpened>(world);
        channel::<AssetsWritten>(world);
        channel::<AssetCreation>(world);
        world.track_removals::<UISelected>();
        install_listeners(world);
        run(world, build);
        let mut systems = vec![
            commit_pending_asset.into_boxed_system(),
            update_filter.into_boxed_system(),
            reconcile_content_tree.into_boxed_system(),
            update_trees_system(),
            sync_content_items.into_boxed_system(),
            rebuild_new_rows.into_boxed_system(),
            sync_new_items.into_boxed_system(),
        ];
        for system in &mut systems {
            system.initialize(world);
        }
        systems
    }

    fn run_browser(world: &mut World, systems: &mut [BoxedSystem]) {
        for system in systems {
            system.run_and_apply((), world);
        }
        world.tick();
    }

    fn opened(world: &World) -> Vec<AssetId> {
        world
            .get_resource::<EditorCommands>()
            .unwrap()
            .0
            .iter()
            .filter_map(|command| match command {
                EditorCommand::OpenAsset(id) => Some(*id),
                _ => None,
            })
            .collect()
    }

    fn files_under(path: &Path) -> Vec<PathBuf> {
        let mut files = Vec::new();
        for entry in std::fs::read_dir(path).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                files.extend(files_under(&path));
            } else {
                files.push(path);
            }
        }
        files.sort();
        files
    }

    fn events<E: Event + Clone + 'static>(world: &World) -> Vec<E> {
        world
            .get_resource::<EventChannel<E>>()
            .unwrap()
            .read_from(&mut EventCursor::default())
            .cloned()
            .collect()
    }

    fn entry(address: &str, kind: &str) -> AssetEntry {
        AssetEntry {
            embedded_in: None,
            id: AssetId::new(),
            address: address.into(),
            kind: kind.into(),
            kind_version: None,
            provenance: None,
        }
    }

    struct Fixture {
        world: World,
        systems: Vec<BoxedSystem>,
        root: tempfile::TempDir,
    }

    fn level_and_forest() -> Fixture {
        let root = tempfile::tempdir().unwrap();
        let mut world = World::new();
        world.register_component::<ChildOf>();
        world.register_component::<Children>();
        world.insert_resource(UITheme::default());
        world.insert_resource(FocusedWidget::default());
        let mut editors = AssetEditorRegistry::default();
        editors.register_creatable::<Scene, SceneEditor>().unwrap();
        world.insert_resource(editors);
        let mut project = ProjectState::default();
        project.project = Some(Project {
            root: root.path().to_path_buf(),
            content_root: "content".into(),
            default_scene: None,
        });
        world.insert_resource(project);
        let mut registry = AssetRegistry::default();
        registry.insert(entry("content/level.gasset", "Scene"));
        registry.insert(entry("content/forest/a.gasset", "Scene"));
        world.insert_resource(registry);
        concerto_ui::items::install_listeners(&mut world);
        concerto_ui::tree::install_listeners(&mut world);
        let systems = browser(&mut world);
        let mut fixture = Fixture {
            world,
            systems,
            root,
        };
        fixture.refresh();
        fixture
    }

    impl Fixture {
        fn refresh(&mut self) {
            run_browser(&mut self.world, &mut self.systems);
        }

        fn click(&mut self, entity: Entity) {
            self.world.trigger_on(
                entity,
                UIClick {
                    position: Vec2::ZERO,
                    button: MouseButton::Left,
                },
            );
            self.refresh();
        }

        fn asset(&self, address: &str) -> AssetId {
            self.world
                .get_resource::<AssetRegistry>()
                .unwrap()
                .id_for_address(address)
                .unwrap()
        }

        fn item(&self, key: &ContentKey) -> Entity {
            *self
                .world
                .get_resource::<ContentState>()
                .unwrap()
                .items
                .get(key)
                .expect("the key has an item")
        }

        fn field(&mut self) -> Option<Entity> {
            self.world
                .query::<Entity, With<PendingField>>()
                .iter(&mut self.world)
                .next()
        }

        fn create(&mut self, folder: Option<&str>, name: &str) {
            self.world.trigger(CreateAsset {
                kind: Scene::name().into(),
                folder: folder.map(str::to_string),
                name: name.into(),
                open: false,
            });
            self.refresh();
        }
    }

    #[test]
    fn create_asset_writes_a_readable_asset_in_its_folder_and_refuses_what_it_cannot_write() {
        let mut f = level_and_forest();

        f.create(Some("content/rooms"), "Hall");
        f.create(None, "Yard");
        f.create(None, "level");
        f.create(Some("elsewhere"), "Hall");
        f.create(Some("content/../up"), "Hall");

        let content = f.root.path().join("content");
        assert_eq!(
            files_under(f.root.path()),
            [
                content.join("Yard.gasset"),
                content.join("rooms/Hall.gasset")
            ]
        );
        let hall = "content/rooms/Hall.gasset";
        let (header, _) =
            read_content_asset(&std::fs::read(f.root.path().join(hall)).unwrap()).unwrap();
        assert_eq!(header.kind, Scene::name());
        let (hall, yard) = (f.asset(hall), f.asset("content/Yard.gasset"));
        assert_eq!(header.asset_id, hall);
        let results: Vec<Result<AssetId, ()>> = events::<AssetCreation>(&f.world)
            .into_iter()
            .map(|creation| creation.result.map_err(|_| ()))
            .collect();
        assert_eq!(results, [Ok(hall), Ok(yard), Err(()), Err(()), Err(())]);
        assert_eq!(
            events::<AssetsWritten>(&f.world),
            [
                AssetsWritten { ids: vec![hall] },
                AssetsWritten { ids: vec![yard] }
            ]
        );
        assert!(opened(&f.world).is_empty());
        let (_, reopened) = discover_project(f.root.path()).unwrap();
        assert_eq!(
            reopened.id_for_address("content/rooms/Hall.gasset"),
            Some(hall)
        );
        assert_eq!(reopened.id_for_address("content/Yard.gasset"), Some(yard));
    }

    #[test]
    fn new_writes_the_named_asset_in_the_selected_folder_and_opens_it() {
        let mut f = level_and_forest();
        let forest = f.item(&ContentKey::Folder("forest".into()));
        f.click(forest);
        let row = f
            .world
            .query::<Entity, With<NewRow>>()
            .iter(&mut f.world)
            .next()
            .unwrap();
        f.click(row);
        let field = f.field().expect("New shows a pending row");

        f.world
            .get_component_for_entity_mut::<UITextInput>(field)
            .unwrap()
            .value = "Meadow".into();
        let channel = f
            .world
            .get_resource_mut::<EventChannel<UITextInputSubmitted>>()
            .unwrap();
        channel.update();
        channel.update();
        channel.push_event(UITextInputSubmitted {
            entity: field,
            value: "Meadow".into(),
        });
        f.refresh();

        let address = "content/forest/Meadow.gasset";
        assert!(f.root.path().join(address).is_file());
        assert_eq!(opened(&f.world), [f.asset(address)]);
        assert_eq!(f.field(), None);
    }

    #[test]
    fn activating_an_asset_opens_it() {
        let mut f = level_and_forest();
        let level = f.asset("content/level.gasset");
        let item = f.item(&ContentKey::Asset(level));

        f.world.trigger_on(item, UIItemActivated);

        assert_eq!(opened(&f.world), [level]);
    }

    fn root_children(f: &mut SceneFixture, document: Entity) -> usize {
        let root = f.scene_root(document);
        f.scene_world(document)
            .get_component_for_entity::<Children>(root)
            .map_or(0, |children| children.iter().len())
    }

    #[test]
    fn a_created_scene_opens_empty_and_survives_a_save_and_a_reopen() {
        let mut f = SceneFixture::new();
        let mut systems = browser(&mut f.world);
        run_browser(&mut f.world, &mut systems);

        f.world.trigger(CreateAsset {
            kind: Scene::name().into(),
            folder: None,
            name: "Meadow".into(),
            open: true,
        });
        run_browser(&mut f.world, &mut systems);
        let [id] = opened(&f.world)[..] else {
            panic!("the created scene is opened once");
        };
        let created = f
            .world
            .get_resource::<AssetRegistry>()
            .unwrap()
            .get(id)
            .unwrap()
            .clone();
        assert_eq!(created.address, "content/Meadow.gasset");
        f.request(&created);
        f.frames_until_idle();

        let document = f.world.get_resource::<ActiveEditor>().unwrap().0.unwrap();
        assert_eq!(
            f.document(document).current.as_ref().map(|a| a.id),
            Some(id)
        );
        assert_eq!(root_children(&mut f, document), 0);

        f.world.insert(crate::asset_editor::SaveRequested, document);
        f.frame();
        assert!(
            f.status(document).starts_with("Saved"),
            "{}",
            f.status(document)
        );

        let scene = f.scene.clone();
        f.request(&scene);
        f.frames_until_idle();
        let (_, reopened) = discover_project(&f.project_root).unwrap();
        let entry = reopened.get(id).unwrap().clone();
        assert_eq!(entry.address, "content/Meadow.gasset");
        assert_eq!(entry.kind, Scene::name());
        f.add_to_project(&entry);
        f.request(&entry);
        f.frames_until_idle();

        assert_eq!(
            f.document(document).current.as_ref().map(|a| a.id),
            Some(id)
        );
        assert_eq!(root_children(&mut f, document), 0);
    }
}

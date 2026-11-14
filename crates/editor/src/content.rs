//! The project's asset catalogue, as a docked panel.
use concerto_app::{
    App, Plugin,
    schedule_groups::{LateUpdate, Startup},
};
use concerto_color::Color;
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

/// Creates an asset of a creatable `kind` named `name` in `folder`, or in the content root without one, as the Asset Browser's New does; `open` opens it in its editor.
pub struct CreateAsset {
    pub kind: String,
    pub folder: Option<String>,
    pub name: String,
    pub open: bool,
}
impl Signal for CreateAsset {}

/// What a `CreateAsset` came to: the new asset's id, or why nothing was written.
#[derive(Event, Clone, Debug, PartialEq)]
pub struct AssetCreation {
    pub kind: String,
    pub address: String,
    pub result: Result<AssetId, String>,
}

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
    views: Query<&mut UIScrollArea, With<ContentView>>,
    mut state: ResMut<ContentState>,
) {
    let mut reset = false;
    for change in changes.read() {
        if filters.get_entity(change.entity).is_some() {
            state.filter = change.value.clone();
            reset = true;
        }
    }
    if reset && let Some(mut area) = views.iter().next() {
        area.offset = 0.0;
    }
}

fn select_kind(
    on: On<UIClick>,
    tags: Query<&Tag>,
    project: Res<ProjectState>,
    mut state: ResMut<ContentState>,
    views: Query<&mut UIScrollArea, With<ContentView>>,
) {
    if on.signal().button != MouseButton::Left || project.busy() {
        return;
    }
    let Some(tag) = tags.get_entity(on.entity()) else {
        return;
    };
    state.kind = tag.0;
    if let Some(mut area) = views.iter().next() {
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
           filters: Query<&mut UITextInput, With<Filter>>,
           menus: Query<&mut UIAnchoredPanel, NewMenus>| {
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
            &filters,
            &menus,
        );
    })
    .into_listener()
}

fn start_from_menu(kind: &'static str) -> Listener<UIClick> {
    (move |on: On<UIClick>,
           mut project: ResMut<ProjectState>,
           registry: Res<AssetRegistry>,
           mut state: ResMut<ContentState>,
           filters: Query<&mut UITextInput, With<Filter>>,
           menus: Query<&mut UIAnchoredPanel, NewMenus>,
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
            &filters,
            &menus,
        );
    })
    .into_listener()
}

fn open_folder_menu(key: String) -> Listener<UIItemContextMenu> {
    (move |on: On<UIItemContextMenu>,
           project: Res<ProjectState>,
           menus: Query<(&mut ContentMenu, &mut UIAnchoredPanel)>| {
        if let Some(open) = project.project.as_ref() {
            let folder = format!("{}/{key}", open.content_root);
            open_content_menu(folder, on.signal().position, Some(on.entity()), &menus);
        }
    })
    .into_listener()
}

fn open_root_menu(
    on: On<UIClick>,
    project: Res<ProjectState>,
    menus: Query<(&mut ContentMenu, &mut UIAnchoredPanel)>,
) {
    let click = on.signal();
    if click.button != MouseButton::Right {
        return;
    }
    if let Some(open) = project.project.as_ref() {
        open_content_menu(open.content_root.clone(), click.position, None, &menus);
    }
}

fn open_content_menu(
    folder: String,
    position: glam::Vec2,
    owner: Option<Entity>,
    menus: &Query<(&mut ContentMenu, &mut UIAnchoredPanel)>,
) {
    for (mut menu, mut panel) in menus.iter() {
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
    filters: &Query<&mut UITextInput, With<Filter>>,
    menus: &Query<&mut UIAnchoredPanel, NewMenus>,
) {
    for mut panel in menus.iter() {
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
    for mut input in filters.iter() {
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
    elements: Query<&mut UITreeElement>,
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
            if !filtering && let Some(mut element) = elements.get_entity(entity) {
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

fn pending_listing(
    pending: &PendingAsset,
    content_root: &str,
    icons: &AssetIcons,
) -> Listed<'static> {
    let inside = inside_root(&pending.folder, content_root).trim_start_matches('/');
    Listed {
        key: pending.key(),
        parent: (!inside.is_empty()).then(|| ContentKey::Folder(inside.to_string())),
        name: Cow::Borrowed(""),
        icon: icons.glyph(pending.kind),
    }
}

fn reconcile_content_tree(
    project: Res<ProjectState>,
    registry: Res<AssetRegistry>,
    mut state: ResMut<ContentState>,
    views: Query<(Entity, &mut UITree), With<ContentView>>,
    items: Query<(&mut ContentItem, Option<&UIHidden>, Option<&UIItem>)>,
    parents: Query<&ChildOf>,
    texts: Query<&mut UIText>,
    materials: Query<&mut UIMaterial>,
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
    let Some((view, mut tree)) = views.iter().next() else {
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
    let mut listed = Vec::new();
    if let Some(project) = project.project.as_ref() {
        let pending = state
            .pending
            .as_ref()
            .map(|pending| pending_listing(pending, &project.content_root, &icons));
        list_folder(
            &folder_tree(&registry, &folds, &project.content_root),
            None,
            pending.as_ref(),
            &folds,
            &icons,
            &mut listed,
        );
        if let Some(pending) = pending
            && !listed.iter().any(|entry| entry.key == pending.key)
        {
            listed.push(pending);
        }
    }
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
                if let Some((mut content, hidden, _)) = items.get_entity(item) {
                    if content.icon != entry.icon {
                        content.icon = entry.icon;
                    }
                    if let ContentKey::Pending { .. } = entry.key {
                        if let Some(mut material) = materials.get_entity(content.label)
                            && material.border_color != error_border
                        {
                            material.border_color = error_border;
                        }
                    } else if let Some(mut text) = texts.get_entity(content.label)
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
                item
            }
            None => {
                let item = match &state.pending {
                    Some(pending) if pending.key() == entry.key => {
                        spawn_pending_item(&mut cmd, &theme, entry, &pending.prefill, parent)
                    }
                    _ => spawn_item(&mut cmd, &theme, entry, parent),
                };
                if hide {
                    cmd.insert(UIHidden, item);
                }
                if matches!(entry.key, ContentKey::Asset(id) if state.selected == Some(id)) {
                    cmd.insert(UISelected, item);
                }
                item
            }
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

    if let Some(key) = state.reveal.as_ref() {
        match state.items.get(key).copied() {
            None => state.reveal = None,
            Some(item)
                if items
                    .get_entity(item)
                    .is_some_and(|(_, _, placed)| placed.is_some()) =>
            {
                cmd.entity(item).trigger(UIReveal);
                state.reveal = None;
            }
            Some(_) => {}
        }
    }

    let count = match &state.pending_error {
        Some(error) => error.clone(),
        None if matching.is_empty() => String::new(),
        None => format!("{} assets", matching.len()),
    };
    for label in counts.iter() {
        if let Some(mut text) = texts.get_entity(label)
            && text.text != count
        {
            text.text = count.clone();
        }
    }
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
    texts: Query<&mut UIText>,
    theme: Res<UITheme>,
) {
    let mut rows: HashSet<Entity> = deselected.read().collect();
    let apply = |(item, element, selected): (&ContentItem, &UITreeElement, Option<&UISelected>)| {
        let glyph = match item.key {
            ContentKey::Folder(_) if element.expanded => glyph::FOLDER_OPEN,
            _ => item.icon,
        };
        if let Some(mut text) = texts.get_entity(item.icon_node) {
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
    tags: Query<(&Tag, &mut UIMaterial, &mut UIText)>,
) {
    for (tag, mut material, mut text) in tags.iter() {
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
        events::event_channel::EventChannel,
        query::filter::{Changed, With},
        system::BoxedSystem,
    };
    use concerto_foundation::assets::{
        Asset,
        content::{ImportProvenance, read_content_asset},
    };
    use concerto_render::assets::material::StandardMaterial;
    use concerto_ui::{
        focus::{FocusedWidget, UIFocusLost},
        interaction::{UIDisabled, UIInteractionStyle},
        items::UIItem,
        node::{UIBox, UILayout},
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

    fn send<E: Event + 'static>(world: &mut World, event: E) {
        let channel = world.get_resource_mut::<EventChannel<E>>().unwrap();
        channel.update();
        channel.update();
        channel.push_event(event);
    }

    fn left_click(world: &mut World, entity: Entity) {
        world.trigger_on(
            entity,
            UIClick {
                position: Vec2::ZERO,
                button: MouseButton::Left,
            },
        );
    }

    fn new_rows(world: &mut World) -> Vec<Entity> {
        world.query::<Entity, With<NewRow>>().iter(world).collect()
    }

    fn menu_rows(world: &mut World) -> Vec<Entity> {
        world
            .query::<Entity, With<ContentMenuRow>>()
            .iter(world)
            .collect()
    }

    fn pending_field(world: &mut World) -> Option<Entity> {
        world
            .query::<Entity, With<PendingField>>()
            .iter(world)
            .next()
    }

    fn set_name(world: &mut World, field: Entity, name: &str) {
        world
            .get_component_for_entity_mut::<UITextInput>(field)
            .unwrap()
            .value = name.into();
    }

    fn take_opened(world: &mut World) -> Vec<AssetId> {
        world
            .get_resource_mut::<EditorCommands>()
            .unwrap()
            .0
            .drain(..)
            .filter_map(|command| match command {
                EditorCommand::OpenAsset(id) => Some(id),
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

    fn fixture(assets: &[(&str, &str)]) -> Fixture {
        fixture_in("content", assets)
    }

    fn fixture_in(content_root: &str, assets: &[(&str, &str)]) -> Fixture {
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
            content_root: content_root.into(),
            default_scene: None,
        });
        world.insert_resource(project);
        let mut registry = AssetRegistry::default();
        for (address, kind) in assets {
            registry.insert(entry(address, kind));
        }
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

    fn nested() -> Fixture {
        fixture(&[
            ("content/forest/a.gasset", "Scene"),
            ("content/characters/hero/mesh.gasset", "Mesh"),
        ])
    }

    fn folder(path: &str) -> ContentKey {
        ContentKey::Folder(path.into())
    }

    fn pending(folder: &str) -> ContentKey {
        ContentKey::Pending {
            kind: Scene::name(),
            folder: folder.into(),
        }
    }

    impl Fixture {
        fn update(&mut self) {
            for system in &mut self.systems {
                system.run_and_apply((), &mut self.world);
            }
        }

        fn new_scene(&mut self) -> Entity {
            let row = new_rows(&mut self.world)[0];
            self.click(row);
            self.refresh();
            self.field().expect("New shows a pending row")
        }

        fn field(&mut self) -> Option<Entity> {
            pending_field(&mut self.world)
        }

        fn name_value(&self, field: Entity) -> String {
            self.world
                .get_component_for_entity::<UITextInput>(field)
                .unwrap()
                .value
                .clone()
        }

        fn submit(&mut self, name: &str) {
            let field = self.field().unwrap();
            set_name(&mut self.world, field, name);
            send(
                &mut self.world,
                UITextInputSubmitted {
                    entity: field,
                    value: name.into(),
                },
            );
            self.refresh();
        }

        fn lose_focus(&mut self, name: &str) {
            let field = self.field().unwrap();
            set_name(&mut self.world, field, name);
            self.focus_on(None);
            send(&mut self.world, UIFocusLost(field));
            self.refresh();
        }

        fn focused(&self) -> Option<Entity> {
            **self.world.get_resource::<FocusedWidget>().unwrap()
        }

        fn focus_on(&mut self, entity: Option<Entity>) {
            **self.world.get_resource_mut::<FocusedWidget>().unwrap() = entity;
        }

        fn footer(&mut self) -> String {
            self.world
                .query::<&UIText, With<CountLabel>>()
                .iter(&mut self.world)
                .next()
                .unwrap()
                .text
                .clone()
        }

        fn border(&self, field: Entity) -> concerto_color::LinearRgba {
            self.world
                .get_component_for_entity::<UIMaterial>(field)
                .unwrap()
                .border_color
        }

        fn files(&self) -> Vec<PathBuf> {
            files_under(self.root.path())
        }

        fn status(&self) -> String {
            self.world
                .get_resource::<ProjectState>()
                .unwrap()
                .status
                .clone()
        }

        fn disabled(&self, entity: Entity) -> bool {
            self.world
                .get_component_for_entity::<UIDisabled>(entity)
                .is_some()
        }

        fn content_menu(&mut self) -> Entity {
            self.world
                .query::<Entity, With<ContentMenu>>()
                .iter(&mut self.world)
                .next()
                .unwrap()
        }

        fn menu_panel(&self, menu: Entity) -> &UIAnchoredPanel {
            self.world
                .get_component_for_entity::<UIAnchoredPanel>(menu)
                .unwrap()
        }

        fn menu_folder(&self, menu: Entity) -> String {
            self.world
                .get_component_for_entity::<ContentMenu>(menu)
                .unwrap()
                .folder
                .clone()
        }

        fn row_label(&self, row: Entity) -> String {
            let label = *self
                .world
                .get_component_for_entity::<Children>(row)
                .unwrap()
                .iter()
                .next()
                .unwrap();
            self.world
                .get_component_for_entity::<UIText>(label)
                .unwrap()
                .text
                .clone()
        }

        fn press(&mut self, entity: Entity, button: MouseButton, position: Vec2) {
            self.world.trigger_on(entity, UIClick { position, button });
            self.refresh();
        }

        fn new_button(&mut self) -> Entity {
            self.world
                .query::<Entity, With<NewButton>>()
                .iter(&mut self.world)
                .next()
                .unwrap()
        }

        fn refresh(&mut self) {
            self.update();
            self.world.tick();
        }

        fn state(&self) -> &ContentState {
            self.world.get_resource::<ContentState>().unwrap()
        }

        fn asset(&self, address: &str) -> AssetId {
            self.world
                .get_resource::<AssetRegistry>()
                .unwrap()
                .id_for_address(address)
                .unwrap()
        }

        fn item(&self, key: &ContentKey) -> Entity {
            *self.state().items.get(key).expect("the key has an item")
        }

        fn asset_item(&self, address: &str) -> Entity {
            self.item(&ContentKey::Asset(self.asset(address)))
        }

        fn area(&self, view: Entity) -> &UIScrollArea {
            self.world
                .get_component_for_entity::<UIScrollArea>(view)
                .unwrap()
        }

        fn lay_out_view(&mut self, height: f32) {
            let view = self.view();
            let frame = UIBox {
                min: Vec2::ZERO,
                size: Vec2::new(300.0, height),
            };
            self.world.insert(
                UILayout {
                    rect: frame,
                    content_rect: frame,
                    clip_rect: frame,
                    paint_order: 0,
                },
                view,
            );
            self.refresh();
        }

        fn scroll_view_to_end(&mut self) {
            let view = self.view();
            let height = self
                .world
                .get_component_for_entity::<UILayout>(view)
                .unwrap()
                .content_rect
                .size
                .y;
            let end = self.area(view).content_extent - height;
            assert!(end > 0.0);
            self.world
                .get_component_for_entity_mut::<UIScrollArea>(view)
                .unwrap()
                .offset = end;
            self.refresh();
            assert_eq!(self.area(view).offset, end);
        }

        fn in_view(&mut self, item: Entity) -> bool {
            let view = self.view();
            let height = self
                .world
                .get_component_for_entity::<UILayout>(view)
                .unwrap()
                .content_rect
                .size
                .y;
            let placed = *self.world.get_component_for_entity::<UIItem>(item).unwrap();
            let top = placed.index() as f32 * ROW_HEIGHT;
            let offset = self.area(view).offset;
            placed.is_shown() && offset <= top && top + ROW_HEIGHT <= offset + height
        }

        fn view(&mut self) -> Entity {
            self.world
                .query::<Entity, With<ContentView>>()
                .iter(&mut self.world)
                .next()
                .unwrap()
        }

        fn keys_under(&self, parent: Entity) -> Vec<ContentKey> {
            self.world
                .get_component_for_entity::<Children>(parent)
                .map(|children| {
                    children
                        .iter()
                        .filter_map(|child| {
                            self.world
                                .get_component_for_entity::<ContentItem>(*child)
                                .map(|item| item.key.clone())
                        })
                        .collect()
                })
                .unwrap_or_default()
        }

        fn labels_under(&self, parent: Entity) -> Vec<String> {
            self.keys_under(parent)
                .iter()
                .map(|key| self.label(self.item(key)))
                .collect()
        }

        fn label(&self, item: Entity) -> String {
            let label = self
                .world
                .get_component_for_entity::<ContentItem>(item)
                .unwrap()
                .label;
            self.world
                .get_component_for_entity::<UIText>(label)
                .unwrap()
                .text
                .clone()
        }

        fn expanded(&self, item: Entity) -> bool {
            self.world
                .get_component_for_entity::<UITreeElement>(item)
                .unwrap()
                .expanded
        }

        fn expand(&mut self, item: Entity) {
            self.world
                .get_component_for_entity_mut::<UITreeElement>(item)
                .unwrap()
                .expanded = true;
            self.refresh();
        }

        fn shown(&self, item: Entity) -> bool {
            self.world
                .get_component_for_entity::<UIItem>(item)
                .is_some_and(UIItem::is_shown)
        }

        fn hidden(&self, item: Entity) -> bool {
            self.world
                .get_component_for_entity::<UIHidden>(item)
                .is_some()
        }

        fn ui_selected(&self, item: Entity) -> bool {
            self.world
                .get_component_for_entity::<UISelected>(item)
                .is_some()
        }

        fn filtering(&mut self) -> bool {
            let view = self.view();
            self.world
                .get_component_for_entity::<UITree>(view)
                .unwrap()
                .filtering
        }

        fn search(&mut self, value: &str) {
            let filter = self
                .world
                .query::<Entity, With<Filter>>()
                .iter(&mut self.world)
                .next()
                .unwrap();
            let channel = self
                .world
                .get_resource_mut::<EventChannel<UITextInputChanged>>()
                .unwrap();
            channel.update();
            channel.update();
            channel.push_event(UITextInputChanged {
                entity: filter,
                value: value.into(),
            });
            self.refresh();
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

        fn chip(&mut self, index: usize) -> Entity {
            self.world
                .query::<(Entity, &Tag), ()>()
                .iter(&mut self.world)
                .find(|(_, tag)| tag.0 == index)
                .unwrap()
                .0
        }

        fn busy(&mut self) -> std::sync::mpsc::Sender<()> {
            let (release, wait) = std::sync::mpsc::channel();
            self.world
                .get_resource_mut::<ProjectState>()
                .unwrap()
                .begin_job_for_test(wait);
            release
        }

        fn opened(&self) -> Vec<AssetId> {
            self.world
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
    }

    #[test]
    fn folders_nest_as_on_disk() {
        let mut f = nested();
        let view = f.view();

        assert_eq!(f.labels_under(view), ["characters", "forest"]);
        assert_eq!(f.keys_under(view), [folder("characters"), folder("forest")]);
        let characters = f.item(&folder("characters"));
        assert_eq!(f.keys_under(characters), [folder("characters/hero")]);
        assert_eq!(f.labels_under(characters), ["hero"]);
        let hero = f.item(&folder("characters/hero"));
        let mesh = f.asset("content/characters/hero/mesh.gasset");
        assert_eq!(f.keys_under(hero), [ContentKey::Asset(mesh)]);
        assert_eq!(f.labels_under(hero), ["mesh"]);
        let tile = f.item(&ContentKey::Asset(mesh));
        assert_eq!(
            f.world.get_component_for_entity::<ContentAsset>(tile),
            Some(&ContentAsset { id: mesh })
        );
        assert!(
            f.world
                .get_component_for_entity::<ContentAsset>(hero)
                .is_none()
        );
        let forest = f.item(&folder("forest"));
        assert_eq!(f.labels_under(forest), ["a"]);
        assert_eq!(f.state().items.len(), 5);
    }

    #[test]
    fn same_names_in_different_folders_stay_apart() {
        let f = fixture(&[
            ("content/a/props/rock.gasset", "Mesh"),
            ("content/b/props/rock.gasset", "Mesh"),
        ]);

        let a = f.item(&folder("a/props"));
        let b = f.item(&folder("b/props"));
        assert_ne!(a, b);
        assert_eq!(f.keys_under(f.item(&folder("a"))), [folder("a/props")]);
        assert_eq!(f.keys_under(f.item(&folder("b"))), [folder("b/props")]);
        let first = f.asset("content/a/props/rock.gasset");
        let second = f.asset("content/b/props/rock.gasset");
        assert_eq!(f.keys_under(a), [ContentKey::Asset(first)]);
        assert_eq!(f.keys_under(b), [ContentKey::Asset(second)]);
    }

    #[test]
    fn folders_sort_before_assets_alphabetically() {
        let mut f = fixture(&[
            ("content/zeta.gasset", "Scene"),
            ("content/moss/one.gasset", "Mesh"),
            ("content/alpha.gasset", "Texture"),
            ("content/birch/two.gasset", "Mesh"),
        ]);
        let view = f.view();

        assert_eq!(f.labels_under(view), ["birch", "moss", "alpha", "zeta"]);
    }

    #[test]
    fn the_material_chip_shows_standard_materials() {
        let mut f = fixture(&[
            ("content/stone.gasset", StandardMaterial::name()),
            ("content/rock.gasset", "Mesh"),
        ]);

        f.world.get_resource_mut::<ContentState>().unwrap().kind = 4;
        f.refresh();

        assert!(f.shown(f.asset_item("content/stone.gasset")));
        assert!(!f.shown(f.asset_item("content/rock.gasset")));
    }

    #[test]
    fn folders_start_collapsed() {
        let f = nested();

        let characters = f.item(&folder("characters"));
        assert!(f.shown(characters));
        assert!(!f.expanded(characters));
        assert!(!f.shown(f.item(&folder("characters/hero"))));
        assert!(!f.shown(f.asset_item("content/forest/a.gasset")));
    }

    #[test]
    fn an_import_keeps_expanded_folders_expanded() {
        let mut f = nested();
        let forest = f.item(&folder("forest"));
        f.expand(forest);

        let catalogue = f.world.get_resource_mut::<AssetRegistry>().unwrap();
        catalogue.insert(entry("content/forest/b.gasset", "Mesh"));
        catalogue.insert(entry("content/forest/_first.gasset", "Mesh"));
        catalogue.insert(entry("content/birch/bark.gasset", "Texture"));
        f.refresh();

        assert_eq!(f.item(&folder("forest")), forest);
        assert!(f.expanded(forest));
        assert_eq!(f.labels_under(forest), ["_first", "a", "b"]);
        assert!(f.shown(f.asset_item("content/forest/b.gasset")));
        let view = f.view();
        assert_eq!(f.labels_under(view), ["birch", "characters", "forest"]);
    }

    #[test]
    fn a_search_hides_non_matches_and_keeps_the_path() {
        let mut f = nested();

        f.search("mesh");

        assert!(f.filtering());
        let mesh = f.asset_item("content/characters/hero/mesh.gasset");
        let hero = f.item(&folder("characters/hero"));
        let characters = f.item(&folder("characters"));
        let forest = f.item(&folder("forest"));
        let a = f.asset_item("content/forest/a.gasset");
        assert!(!f.hidden(mesh));
        for item in [hero, characters, forest, a] {
            assert!(f.hidden(item));
        }
        for item in [characters, hero, mesh] {
            assert!(f.shown(item));
        }
        assert!(!f.shown(forest) && !f.shown(a));
        assert!(!f.expanded(characters));

        f.search("");

        assert!(!f.filtering());
        for item in [mesh, hero, characters, forest, a] {
            assert!(!f.hidden(item));
        }
        assert!(f.shown(characters) && f.shown(forest));
        assert!(!f.shown(hero));
    }

    #[test]
    fn a_kind_chip_filters_like_a_search() {
        let mut f = nested();

        let mesh_chip = f.chip(2);
        f.click(mesh_chip);

        assert!(f.filtering());
        let mesh = f.asset_item("content/characters/hero/mesh.gasset");
        let a = f.asset_item("content/forest/a.gasset");
        assert!(!f.hidden(mesh) && f.shown(mesh));
        assert!(f.hidden(a) && !f.shown(a));
        assert!(!f.shown(f.item(&folder("forest"))));

        let all_chip = f.chip(0);
        f.click(all_chip);

        assert!(!f.filtering());
        assert!(!f.hidden(a));
    }

    #[test]
    fn a_chip_write_applied_after_the_reconcile_is_reconciled_next_run() {
        let mut f = nested();
        let mut reconcile = reconcile_content_tree.into_system();
        reconcile.initialize(&mut f.world);
        reconcile.run_and_apply((), &mut f.world);
        assert!(!f.filtering());

        let mesh_chip = f.chip(2);
        f.world.trigger_on(
            mesh_chip,
            UIClick {
                position: Vec2::ZERO,
                button: MouseButton::Left,
            },
        );
        f.world.tick();
        reconcile.run_and_apply((), &mut f.world);

        assert!(f.filtering());
        let a = f.asset_item("content/forest/a.gasset");
        assert!(f.hidden(a));
    }

    #[test]
    fn selecting_an_item_selects_the_asset() {
        let mut f = fixture(&[
            ("content/level.gasset", "Scene"),
            ("content/rock.gasset", "Mesh"),
        ]);
        let level = f.asset("content/level.gasset");
        let item = f.asset_item("content/level.gasset");

        f.world.trigger_on(item, UIItemSelected);
        f.refresh();

        assert_eq!(f.state().selected(), Some(level));
        assert!(f.ui_selected(item));
        assert!(!f.ui_selected(f.asset_item("content/rock.gasset")));
        assert!(f.opened().is_empty());
    }

    #[test]
    fn deselecting_an_asset_restores_its_mark() {
        let mut f = fixture(&[
            ("content/level.gasset", "Scene"),
            ("content/rock.gasset", "Mesh"),
        ]);
        let level = f.asset_item("content/level.gasset");
        let rock = f.asset_item("content/rock.gasset");
        f.world.trigger_on(level, UIItemSelected);
        f.refresh();

        f.world.trigger_on(rock, UIItemSelected);
        f.refresh();

        assert!(!f.ui_selected(level));
    }

    #[test]
    fn a_click_while_busy_is_ignored_and_the_highlight_snaps_back() {
        let mut f = fixture(&[("content/level.gasset", "Scene")]);
        let item = f.asset_item("content/level.gasset");
        let release = f.busy();

        f.click(item);

        assert_eq!(f.state().selected(), None);
        assert!(!f.ui_selected(item));
        drop(release);
    }

    fn level_and_forest() -> Fixture {
        fixture(&[
            ("content/level.gasset", "Scene"),
            ("content/forest/a.gasset", "Scene"),
        ])
    }

    #[test]
    fn a_folder_click_keeps_the_folder_selected() {
        let mut f = level_and_forest();
        let level = f.asset("content/level.gasset");
        let level_item = f.asset_item("content/level.gasset");
        let forest = f.item(&folder("forest"));
        f.click(level_item);

        f.click(forest);
        f.refresh();

        assert!(f.ui_selected(forest));
        assert!(!f.ui_selected(level_item));
        assert_eq!(f.state().selected(), Some(level));
    }

    #[test]
    fn activating_a_folder_toggles_it() {
        let mut f = level_and_forest();
        let forest = f.item(&folder("forest"));

        f.world.trigger_on(forest, UIItemActivated);
        f.refresh();
        assert!(f.expanded(forest));
        assert!(f.shown(f.asset_item("content/forest/a.gasset")));

        f.world.trigger_on(forest, UIItemActivated);
        f.refresh();
        assert!(!f.expanded(forest));
        assert!(f.opened().is_empty());
    }

    #[test]
    fn a_busy_click_on_a_folder_snaps_back() {
        let mut f = level_and_forest();
        let level = f.asset("content/level.gasset");
        let level_item = f.asset_item("content/level.gasset");
        let forest = f.item(&folder("forest"));
        f.click(level_item);
        let release = f.busy();

        f.click(forest);
        f.refresh();

        assert!(!f.ui_selected(forest));
        assert!(f.ui_selected(level_item));
        assert_eq!(f.state().selected(), Some(level));
        drop(release);
    }

    #[test]
    fn names_sort_ignoring_case() {
        let mut f = fixture(&[
            ("content/Zeta.gasset", "Scene"),
            ("content/alpha.gasset", "Scene"),
            ("content/Birch/x.gasset", "Mesh"),
            ("content/apple/y.gasset", "Mesh"),
        ]);
        let view = f.view();

        assert_eq!(f.labels_under(view), ["apple", "Birch", "alpha", "Zeta"]);
    }

    #[test]
    fn folders_are_relative_to_the_configured_content_root() {
        let mut f = fixture_in(
            "game/content",
            &[
                ("game/content/forest/a.gasset", "Scene"),
                ("game/content/level.gasset", "Scene"),
            ],
        );
        let view = f.view();

        assert_eq!(
            f.keys_under(view),
            [
                folder("forest"),
                ContentKey::Asset(f.asset("game/content/level.gasset"))
            ]
        );
        assert_eq!(f.labels_under(view), ["forest", "level"]);
    }

    #[test]
    fn an_idle_frame_leaves_marks_unchanged() {
        let mut f = level_and_forest();
        let level_item = f.asset_item("content/level.gasset");
        f.click(level_item);
        f.refresh();

        f.update();

        let marks: HashSet<Entity> = f
            .world
            .query::<&ContentItem, ()>()
            .iter(&mut f.world)
            .map(|item| item.icon_node)
            .collect();
        let changed: Vec<Entity> = f
            .world
            .query::<Entity, Changed<UIText>>()
            .iter(&mut f.world)
            .filter(|entity| marks.contains(entity))
            .collect();
        assert!(changed.is_empty(), "{changed:?}");
    }

    #[test]
    fn activating_an_asset_opens_it() {
        let mut f = fixture(&[("content/level.gasset", "Scene")]);
        let level = f.asset("content/level.gasset");
        let item = f.asset_item("content/level.gasset");

        f.world.trigger_on(item, UIItemActivated);

        assert_eq!(f.opened(), [level]);
    }

    fn imported(address: &str, kind: &str, source: &str, sub_asset: &str) -> AssetEntry {
        AssetEntry {
            provenance: Some(ImportProvenance {
                source: source.into(),
                sub_asset: sub_asset.into(),
            }),
            ..entry(address, kind)
        }
    }

    #[test]
    fn an_import_with_a_scene_folds_under_it_and_one_without_stays_flat() {
        let mut f = fixture(&[("content/props/barrel.gasset", "Mesh")]);
        let registry = f.world.get_resource_mut::<AssetRegistry>().unwrap();
        for asset in [
            imported(
                "content/props/crate/scene.gasset",
                "Scene",
                "assets/props/crate.glb",
                "scene",
            ),
            imported(
                "content/props/crate/mesh_0.gasset",
                "Mesh",
                "assets/props/crate.glb",
                "mesh/0",
            ),
            imported(
                "content/props/crate/material_0.gasset",
                "StandardMaterial",
                "assets/props/crate.glb",
                "material/0",
            ),
            imported(
                "content/hero/scene.gasset",
                "Scene",
                "assets/hero.glb",
                "scene",
            ),
            imported(
                "content/hero/mesh_0.gasset",
                "Mesh",
                "assets/hero.glb",
                "mesh/0",
            ),
            entry("content/hero/rig.gasset", "RetargetRig"),
            imported(
                "content/town/scene.gasset",
                "Scene",
                "assets/town.glb",
                "scene",
            ),
            entry("content/town/signs/inn.gasset", "Texture"),
            imported(
                "content/moves/run/animation.gasset",
                "AnimationClip",
                "assets/moves/run.bvh",
                "animation/run",
            ),
            imported(
                "content/moves/run/skeleton.gasset",
                "Skeleton",
                "assets/moves/run.bvh",
                "skeleton/0",
            ),
        ] {
            registry.insert(asset);
        }
        f.refresh();
        let view = f.view();
        let scene = f.asset("content/props/crate/scene.gasset");
        let face = f.asset_item("content/props/crate/scene.gasset");
        let mesh = f.asset_item("content/props/crate/mesh_0.gasset");
        let material = f.asset_item("content/props/crate/material_0.gasset");
        let barrel = f.asset_item("content/props/barrel.gasset");
        let props = f.item(&folder("props"));

        assert_eq!(f.labels_under(view), ["hero", "moves", "props", "town"]);
        assert_eq!(f.labels_under(f.item(&folder("hero"))), ["hero", "rig"]);
        assert_eq!(f.labels_under(f.item(&folder("town"))), ["signs", "town"]);
        assert_eq!(f.labels_under(props), ["barrel", "crate"]);
        assert!(!f.state().items.contains_key(&folder("props/crate")));
        assert_eq!(
            f.world.get_component_for_entity::<ContentAsset>(face),
            Some(&ContentAsset { id: scene })
        );
        assert_eq!(f.labels_under(face), ["material_0", "mesh_0"]);
        assert!(!f.expanded(face));
        let run = f.item(&folder("moves/run"));
        assert_eq!(f.labels_under(run), ["animation", "skeleton"]);

        f.world.trigger_on(face, UIItemActivated);
        assert_eq!(f.opened(), [scene]);

        f.search("crate");
        assert!(f.shown(face));
        assert!(!f.shown(mesh) && !f.shown(material) && !f.shown(barrel));

        f.search("mesh_0");
        assert!(f.hidden(face) && f.shown(face));
        assert!(f.shown(mesh));
        assert!(!f.shown(material) && !f.shown(barrel));

        f.search("");
        let mesh_chip = f.chip(2);
        f.click(mesh_chip);
        assert!(f.shown(barrel) && f.shown(face) && f.shown(mesh));
        assert!(!f.shown(material));
    }

    fn parent(world: &World, entity: Entity) -> Entity {
        world
            .get_component_for_entity::<ChildOf>(entity)
            .unwrap()
            .parent()
    }

    fn import_buttons(world: &mut World) -> Vec<Entity> {
        world
            .query::<(Entity, &UIText), ()>()
            .iter(world)
            .filter(|(_, text)| text.text == "Import…")
            .map(|(entity, _)| entity)
            .collect()
    }

    #[test]
    fn import_is_one_button_beside_the_search_not_a_chip_among_the_filters() {
        let mut f = fixture(&[("content/level.gasset", "Scene")]);
        let world = &mut f.world;

        let buttons = import_buttons(world);
        assert_eq!(buttons.len(), 1);
        let button = buttons[0];
        let tag = world
            .query::<(Entity, &Tag), ()>()
            .iter(world)
            .next()
            .unwrap()
            .0;
        let filter = world
            .query::<(Entity, &Filter), ()>()
            .iter(world)
            .next()
            .unwrap()
            .0;
        let search_card = parent(world, filter);

        assert_ne!(parent(world, button), parent(world, tag));
        assert_eq!(parent(world, button), parent(world, search_card));
        let style = world
            .get_component_for_entity::<UIInteractionStyle>(button)
            .expect("a pressable, so it reacts to the pointer");
        assert_ne!(style.normal, style.hovered);
    }

    #[test]
    fn clicking_import_with_no_project_open_says_what_to_do_first() {
        let mut f = fixture(&[("content/level.gasset", "Scene")]);
        let world = &mut f.world;
        world.insert_resource(crate::import::ImportPicker::default());
        world.insert_resource(ProjectState::default());
        let button = import_buttons(world)[0];

        world.trigger_on(
            button,
            UIClick {
                position: Vec2::ZERO,
                button: MouseButton::Left,
            },
        );

        let status = &world.get_resource::<ProjectState>().unwrap().status;
        assert!(status.contains("project"), "got: {status}");
        assert!(status.contains("import"), "got: {status}");
    }

    #[test]
    fn new_is_a_dropdown_beside_import_with_a_row_per_creatable_kind() {
        let mut f = fixture(&[("content/level.gasset", "Scene")]);
        let button = f.new_button();
        let import = import_buttons(&mut f.world)[0];

        assert_eq!(parent(&f.world, button), parent(&f.world, import));
        let rows = new_rows(&mut f.world);
        assert_eq!(rows.len(), 1);
        let menu = parent(&f.world, rows[0]);
        let panel = f
            .world
            .get_component_for_entity::<UIAnchoredPanel>(menu)
            .expect("the rows sit in the dropdown");
        assert_eq!(panel.owner, Some(button));
        assert!(panel.toggled_by_owner && !panel.open);
        assert!(f.world.get_component_for_entity::<ChildOf>(menu).is_none());
        let label = *f
            .world
            .get_component_for_entity::<Children>(rows[0])
            .unwrap()
            .iter()
            .next()
            .unwrap();
        assert_eq!(
            f.world
                .get_component_for_entity::<UIText>(label)
                .unwrap()
                .text,
            "New Scene"
        );
    }

    #[test]
    fn new_from_the_dropdown_shows_a_focused_pending_row_in_the_selected_folder() {
        let mut f = level_and_forest();
        let forest = f.item(&folder("forest"));
        f.click(forest);
        assert_eq!(f.state().selected_folder.as_deref(), Some("content/forest"));
        let row = new_rows(&mut f.world)[0];
        let menu = parent(&f.world, row);
        f.world
            .get_component_for_entity_mut::<UIAnchoredPanel>(menu)
            .unwrap()
            .open = true;

        let field = f.new_scene();

        let a = f.asset("content/forest/a.gasset");
        assert_eq!(
            f.keys_under(forest),
            [pending("content/forest"), ContentKey::Asset(a)]
        );
        assert_eq!(f.focused(), Some(field));
        assert_eq!(f.name_value(field), "New Scene");
        assert!(f.shown(f.item(&pending("content/forest"))));
        assert!(
            !f.world
                .get_component_for_entity::<UIAnchoredPanel>(menu)
                .unwrap()
                .open
        );
        assert!(f.files().is_empty());
    }

    #[test]
    fn submitting_a_valid_name_writes_the_asset_in_the_selected_folder_and_opens_it() {
        let mut f = level_and_forest();
        let forest = f.item(&folder("forest"));
        f.click(forest);
        let field = f.new_scene();
        let before: Vec<AssetId> = f
            .world
            .get_resource::<AssetRegistry>()
            .unwrap()
            .iter()
            .map(|asset| asset.id)
            .collect();

        f.submit("Meadow");

        let address = "content/forest/Meadow.gasset";
        let bytes = std::fs::read(f.root.path().join(address)).unwrap();
        let (header, _) = read_content_asset(&bytes).unwrap();
        assert_eq!(header.kind, Scene::name());
        assert!(header.provenance.is_none());
        assert!(!before.contains(&header.asset_id));
        let id = f.asset(address);
        assert_eq!(id, header.asset_id);
        let created = f.asset_item(address);
        assert_eq!(parent(&f.world, created), forest);
        assert_eq!(f.state().selected(), Some(id));
        assert_eq!(f.state().selected_folder, None);
        assert!(f.ui_selected(created));
        assert_eq!(f.opened(), [id]);
        assert_eq!(f.status(), "Created content/forest/Meadow.gasset");
        assert!(f.state().pending.is_none());
        assert_eq!(f.field(), None);
        assert!(!f.state().items.contains_key(&pending("content/forest")));
        assert_ne!(f.focused(), Some(field));
        assert_eq!(f.focused(), None);

        let (_, reopened) = discover_project(f.root.path()).unwrap();
        assert_eq!(reopened.id_for_address(address), Some(id));
    }

    fn creations(f: &Fixture) -> Vec<AssetCreation> {
        f.world
            .get_resource::<EventChannel<AssetCreation>>()
            .unwrap()
            .read_from(&mut concerto_ecs::events::event_cursor::EventCursor::default())
            .cloned()
            .collect()
    }

    fn create(f: &mut Fixture, folder: Option<&str>, name: &str) {
        f.world.trigger(CreateAsset {
            kind: Scene::name().into(),
            folder: folder.map(str::to_string),
            name: name.into(),
            open: false,
        });
        f.refresh();
    }

    #[test]
    fn create_asset_writes_into_a_new_folder_without_a_pending_row_or_an_open() {
        let mut f = level_and_forest();

        create(&mut f, Some("content/rooms"), "Hall");

        let address = "content/rooms/Hall.gasset";
        assert!(f.root.path().join(address).is_file());
        let id = f.asset(address);
        assert_eq!(
            creations(&f),
            [AssetCreation {
                kind: Scene::name().into(),
                address: address.into(),
                result: Ok(id),
            }]
        );
        assert_eq!(f.state().selected(), Some(id));
        assert_eq!(f.status(), "Created content/rooms/Hall.gasset");
        assert!(f.opened().is_empty());
        assert_eq!(f.field(), None);
    }

    #[test]
    fn create_asset_without_a_folder_writes_in_the_content_root() {
        let mut f = level_and_forest();

        create(&mut f, None, "Hall");

        assert!(f.root.path().join("content/Hall.gasset").is_file());
    }

    #[test]
    fn create_asset_reports_a_refusal_and_writes_nothing() {
        let mut f = level_and_forest();
        let status = f.status();

        create(&mut f, None, "level");
        create(&mut f, Some("elsewhere"), "Hall");
        create(&mut f, Some("content/../up"), "Hall");

        let refusals: Vec<String> = creations(&f)
            .into_iter()
            .map(|creation| creation.result.unwrap_err())
            .collect();
        assert_eq!(
            refusals,
            [
                "content/level.gasset already exists",
                "elsewhere is not inside content",
                "content/../up: A name holds only letters, digits, spaces, - and _",
            ]
        );
        assert!(f.files().is_empty());
        assert_eq!(f.status(), status);
    }

    #[test]
    fn a_created_asset_is_sent_as_written() {
        let mut f = level_and_forest();
        f.new_scene();

        f.submit("Meadow");

        let id = f.asset("content/Meadow.gasset");
        let written: Vec<AssetsWritten> = f
            .world
            .get_resource::<EventChannel<AssetsWritten>>()
            .unwrap()
            .read_from(&mut concerto_ecs::events::event_cursor::EventCursor::default())
            .cloned()
            .collect();
        assert_eq!(written, [AssetsWritten { ids: vec![id] }]);
    }

    #[test]
    fn new_creates_in_the_root_when_nothing_or_an_asset_is_selected() {
        let mut f = level_and_forest();
        f.new_scene();
        let view = f.view();
        assert_eq!(
            f.keys_under(view)[..2],
            [folder("forest"), pending("content")]
        );
        f.submit("Clearing");
        assert!(f.root.path().join("content/Clearing.gasset").is_file());

        let forest = f.item(&folder("forest"));
        f.click(forest);
        let level = f.asset_item("content/level.gasset");
        f.click(level);
        assert_eq!(f.state().selected_folder, None);
        f.new_scene();
        f.submit("Glade");

        assert!(f.root.path().join("content/Glade.gasset").is_file());
        assert_eq!(f.files().len(), 2);
    }

    #[test]
    fn the_prefill_skips_names_taken_in_the_registry_or_on_disk() {
        let mut f = fixture(&[("content/new scene.gasset", "Scene")]);
        std::fs::create_dir_all(f.root.path().join("content")).unwrap();
        std::fs::write(f.root.path().join("content/New Scene 1.gasset"), b"x").unwrap();

        let field = f.new_scene();

        assert_eq!(f.name_value(field), "New Scene 2");
    }

    #[test]
    fn a_refused_name_keeps_the_row_says_why_in_the_footer_and_writes_nothing() {
        let mut f = fixture(&[("content/level.gasset", "Scene")]);
        let field = f.new_scene();
        let theme = UITheme::default();
        let count = f.footer();
        let long = "a".repeat(65);

        for name in [
            "LEVEL",
            "",
            long.as_str(),
            " lead",
            "con",
            "a/b",
            "a.b",
            "a*b",
        ] {
            f.submit(name);

            let error = f.state().pending_error.clone();
            assert!(error.is_some(), "{name:?} was accepted");
            assert_eq!(f.footer(), error.unwrap(), "{name:?}");
            assert_eq!(f.border(field), theme.error.to_linear(), "{name:?}");
            assert_eq!(f.field(), Some(field), "{name:?}");
            assert_eq!(f.focused(), Some(field), "{name:?}");
            assert!(f.files().is_empty(), "{name:?}");
            assert!(f.opened().is_empty(), "{name:?}");
        }

        send(
            &mut f.world,
            UITextInputChanged {
                entity: field,
                value: "a*bc".into(),
            },
        );
        f.refresh();

        assert_eq!(f.state().pending_error, None);
        assert_eq!(f.footer(), count);
        assert_eq!(f.border(field), theme.border.to_linear());
    }

    #[test]
    fn escape_cancels_the_pending_asset() {
        let mut f = fixture(&[("content/level.gasset", "Scene")]);
        let field = f.new_scene();
        f.submit("a/b");

        send(&mut f.world, UITextInputCancelled { entity: field });
        f.refresh();

        assert!(f.state().pending.is_none());
        assert_eq!(f.state().pending_error, None);
        assert_eq!(f.field(), None);
        assert_eq!(f.focused(), None);
        assert!(f.files().is_empty());
        assert!(f.opened().is_empty());
        assert_eq!(f.footer(), "1 assets");
    }

    #[test]
    fn losing_focus_with_a_valid_name_creates_the_asset() {
        let mut f = fixture(&[("content/level.gasset", "Scene")]);
        f.new_scene();

        f.lose_focus("Meadow");

        let id = f.asset("content/Meadow.gasset");
        assert_eq!(f.opened(), [id]);
        assert_eq!(f.field(), None);
    }

    #[test]
    fn losing_focus_with_an_invalid_name_cancels_quietly() {
        let mut f = fixture(&[("content/level.gasset", "Scene")]);
        f.new_scene();
        let status = f.status();

        f.lose_focus("level");

        assert!(f.state().pending.is_none());
        assert_eq!(f.state().pending_error, None);
        assert_eq!(f.field(), None);
        assert!(f.files().is_empty());
        assert!(f.opened().is_empty());
        assert_eq!(f.status(), status);
    }

    #[test]
    fn a_second_new_creates_the_first_then_shows_the_second() {
        let mut f = fixture(&[("content/level.gasset", "Scene")]);
        let first = f.new_scene();
        set_name(&mut f.world, first, "First");
        let button = f.new_button();
        f.focus_on(Some(button));
        send(&mut f.world, UIFocusLost(first));
        f.refresh();
        let id = f.asset("content/First.gasset");
        assert_eq!(f.opened(), [id]);

        let second = f.new_scene();

        assert_ne!(second, first);
        assert_eq!(f.focused(), Some(second));
        assert_eq!(f.name_value(second), "New Scene");
        assert_eq!(f.field(), Some(second));
    }

    #[test]
    fn new_reveals_a_pending_row_inside_collapsed_folders() {
        for (path, ancestors) in [
            ("forest", &["forest"][..]),
            ("characters/hero", &["characters", "characters/hero"][..]),
        ] {
            let mut f = nested();
            let target = f.item(&folder(path));
            f.world.trigger_on(target, UIItemSelected);
            f.refresh();
            for ancestor in ancestors {
                assert!(!f.expanded(f.item(&folder(ancestor))));
            }

            f.new_scene();

            for ancestor in ancestors {
                assert!(f.expanded(f.item(&folder(ancestor))), "{path}: {ancestor}");
            }
            let row = f.item(&pending(&format!("content/{path}")));
            assert!(f.shown(row), "{path}");
            assert_eq!(f.state().reveal, None, "{path}");
        }
    }

    #[test]
    fn new_keeps_its_row_in_view_in_a_long_folder_before_and_after_it_is_created() {
        let addresses: Vec<String> = (0..60)
            .map(|n| format!("content/forest/mesh_{n:02}.gasset"))
            .collect();
        let assets: Vec<(&str, &str)> = addresses
            .iter()
            .map(|address| (address.as_str(), "Mesh"))
            .collect();
        let mut f = fixture(&assets);
        f.lay_out_view(10.0 * ROW_HEIGHT);
        let forest = f.item(&folder("forest"));
        f.expand(forest);
        f.click(forest);
        f.scroll_view_to_end();

        f.new_scene();
        f.refresh();

        let row = f.item(&pending("content/forest"));
        assert!(f.in_view(row), "the pending row is out of view");

        f.submit("New Scene");
        f.refresh();

        let created = f.asset_item("content/forest/New Scene.gasset");
        assert!(f.ui_selected(created));
        assert!(f.in_view(created), "the created asset is out of view");
    }

    #[test]
    fn the_new_menu_on_a_folder_creates_in_that_folder() {
        let mut f = level_and_forest();
        let forest = f.item(&folder("forest"));
        let menu = f.content_menu();
        let rows = menu_rows(&mut f.world);
        assert_eq!(rows.len(), 1);
        assert_eq!(f.row_label(rows[0]), "New Scene");
        assert_eq!(parent(&f.world, rows[0]), menu);
        assert!(f.world.get_component_for_entity::<ChildOf>(menu).is_none());
        assert!(!f.menu_panel(menu).open);

        f.press(forest, MouseButton::Right, Vec2::new(40.0, 90.0));

        let panel = f.menu_panel(menu);
        assert!(panel.open);
        assert_eq!(
            panel.target,
            UIAnchorTarget::from_point(Vec2::new(40.0, 90.0))
        );
        assert_eq!(f.menu_folder(menu), "content/forest");

        f.click(rows[0]);
        f.refresh();

        let field = f.field().expect("the menu shows a pending row");
        let a = f.asset("content/forest/a.gasset");
        assert_eq!(
            f.keys_under(forest),
            [pending("content/forest"), ContentKey::Asset(a)]
        );
        assert_eq!(f.focused(), Some(field));
        assert_eq!(f.name_value(field), "New Scene");
        assert!(!f.menu_panel(menu).open);

        f.submit("Meadow");

        assert!(f.root.path().join("content/forest/Meadow.gasset").is_file());
        assert_eq!(f.files().len(), 1);
    }

    #[test]
    fn the_new_menu_on_the_empty_space_creates_in_the_root() {
        let mut f = level_and_forest();
        let forest = f.item(&folder("forest"));
        f.click(forest);
        let view = f.view();
        let menu = f.content_menu();

        f.press(view, MouseButton::Left, Vec2::new(60.0, 200.0));
        assert!(!f.menu_panel(menu).open);

        f.press(view, MouseButton::Right, Vec2::new(60.0, 200.0));

        let panel = f.menu_panel(menu);
        assert!(panel.open);
        assert_eq!(panel.owner, None);
        assert_eq!(
            panel.target,
            UIAnchorTarget::from_point(Vec2::new(60.0, 200.0))
        );
        assert_eq!(f.menu_folder(menu), "content");

        let row = menu_rows(&mut f.world)[0];
        f.click(row);
        f.refresh();

        assert!(f.keys_under(view).contains(&pending("content")));
        assert!(!f.menu_panel(menu).open);
        f.submit("Clearing");
        assert!(f.root.path().join("content/Clearing.gasset").is_file());
    }

    #[test]
    fn a_right_click_on_the_empty_space_with_no_project_opens_no_menu() {
        let mut f = level_and_forest();
        f.world.insert_resource(ProjectState::default());
        f.refresh();
        let view = f.view();
        let menu = f.content_menu();

        f.press(view, MouseButton::Right, Vec2::new(60.0, 200.0));

        assert!(!f.menu_panel(menu).open);
    }

    #[test]
    fn a_selected_folder_does_not_carry_over_to_another_project() {
        let mut f = level_and_forest();
        let forest = f.item(&folder("forest"));
        f.click(forest);
        assert_eq!(f.state().selected_folder.as_deref(), Some("content/forest"));
        let other = tempfile::tempdir().unwrap();
        let mut project = ProjectState::default();
        project.project = Some(Project {
            root: other.path().to_path_buf(),
            content_root: "game/content".into(),
            default_scene: None,
        });
        f.world.insert_resource(project);
        f.world.insert_resource(AssetRegistry::default());
        send(
            &mut f.world,
            ProjectOpened {
                root: other.path().to_path_buf(),
            },
        );
        f.refresh();

        assert_eq!(f.state().selected_folder, None);
        f.new_scene();
        f.submit("Meadow");

        assert!(other.path().join("game/content/Meadow.gasset").is_file());
        assert_eq!(files_under(other.path()).len(), 1);
        assert!(f.files().is_empty());
    }

    #[test]
    fn reopening_the_same_project_drops_the_folder_highlight_with_the_selected_folder() {
        let mut f = level_and_forest();
        let forest = f.item(&folder("forest"));
        f.click(forest);
        let root = f
            .world
            .get_resource::<ProjectState>()
            .unwrap()
            .project
            .as_ref()
            .unwrap()
            .root
            .clone();

        send(&mut f.world, ProjectOpened { root });
        f.refresh();

        assert!(!f.ui_selected(forest));
        assert_eq!(f.state().selected_folder, None);
    }

    #[test]
    fn the_selected_folder_goes_when_its_highlight_does() {
        let mut f = level_and_forest();
        let forest = f.item(&folder("forest"));
        let level = f.asset_item("content/level.gasset");
        f.click(forest);
        f.new_scene();
        let row = f.item(&pending("content/forest"));

        f.click(row);

        assert!(!f.ui_selected(forest));
        assert_eq!(f.state().selected_folder, None);

        f.click(forest);
        assert!(f.state().selected_folder.is_some());
        let release = f.busy();

        f.click(level);

        assert!(!f.ui_selected(forest));
        assert_eq!(f.state().selected_folder, None);
        drop(release);
    }

    #[test]
    fn new_clears_the_search_the_filter_field_and_the_kind_chip() {
        let mut f = level_and_forest();
        let filter = f
            .world
            .query::<Entity, With<Filter>>()
            .iter(&mut f.world)
            .next()
            .unwrap();
        set_name(&mut f.world, filter, "level");
        f.search("level");
        let scene_chip = f.chip(1);
        f.click(scene_chip);
        assert!(f.filtering());

        f.new_scene();

        assert_eq!(f.state().filter, "");
        assert_eq!(f.state().kind, 0);
        assert_eq!(f.name_value(filter), "");
        assert!(!f.filtering());
        assert!(!f.hidden(f.asset_item("content/forest/a.gasset")));
        assert!(f.shown(f.item(&pending("content"))));
    }

    #[test]
    fn a_right_click_on_an_asset_opens_no_new_menu() {
        let mut f = level_and_forest();
        let level = f.asset_item("content/level.gasset");
        let menu = f.content_menu();

        f.press(level, MouseButton::Right, Vec2::new(40.0, 90.0));

        assert!(!f.menu_panel(menu).open);
        assert!(f.state().pending.is_none());
    }

    fn menu_labels(world: &World, row: Entity) -> Vec<String> {
        world
            .get_component_for_entity::<Children>(parent(world, row))
            .unwrap()
            .iter()
            .map(|row| {
                let label = *world
                    .get_component_for_entity::<Children>(*row)
                    .unwrap()
                    .iter()
                    .next()
                    .unwrap();
                world
                    .get_component_for_entity::<UIText>(label)
                    .unwrap()
                    .text
                    .clone()
            })
            .collect()
    }

    #[test]
    fn both_new_menus_gain_a_row_when_a_kind_becomes_creatable() {
        let mut f = fixture(&[("content/level.gasset", "Scene")]);
        f.world
            .get_resource_mut::<AssetEditorRegistry>()
            .unwrap()
            .register_inspected(concerto_app::InspectedAsset {
                kind: "Dialogue",
                create: |_, _, _| anyhow::bail!("unused"),
                decode: |_| anyhow::bail!("unused"),
                save: |_, _, _, _| anyhow::bail!("unused"),
            });

        f.refresh();

        let rows = new_rows(&mut f.world);
        let menu = menu_rows(&mut f.world);
        assert_eq!(rows.len(), 2);
        assert_eq!(menu.len(), 2);
        assert_eq!(
            menu_labels(&f.world, rows[0]),
            ["New Dialogue", "New Scene"]
        );
        assert_eq!(
            menu_labels(&f.world, menu[0]),
            ["New Dialogue", "New Scene"]
        );
        assert!(rows.iter().chain(&menu).all(|row| !f.disabled(*row)));
    }

    #[test]
    fn new_is_disabled_and_does_nothing_without_a_project_or_while_busy() {
        let mut f = fixture(&[("content/level.gasset", "Scene")]);
        let button = f.new_button();
        let row = new_rows(&mut f.world)[0];
        let menu_row = menu_rows(&mut f.world)[0];
        assert!(!f.disabled(button) && !f.disabled(row) && !f.disabled(menu_row));

        let release = f.busy();
        f.refresh();
        assert!(f.disabled(button) && f.disabled(row) && f.disabled(menu_row));
        f.click(row);
        f.click(menu_row);
        assert!(f.state().pending.is_none());
        drop(release);

        f.world.insert_resource(ProjectState::default());
        f.refresh();
        assert!(f.disabled(button) && f.disabled(row) && f.disabled(menu_row));
        f.click(row);
        f.refresh();
        assert!(f.state().pending.is_none());
        assert_eq!(f.field(), None);
        assert!(f.status().contains("project"), "{}", f.status());
        f.click(menu_row);
        f.refresh();
        assert!(f.state().pending.is_none());
    }

    #[test]
    fn opening_a_project_drops_the_pending_asset() {
        let mut f = fixture(&[("content/level.gasset", "Scene")]);
        f.new_scene();
        f.submit("a/b");

        send(
            &mut f.world,
            ProjectOpened {
                root: f.root.path().to_path_buf(),
            },
        );
        f.refresh();

        assert!(f.state().pending.is_none());
        assert_eq!(f.state().pending_error, None);
        assert_eq!(f.field(), None);
        assert_eq!(f.focused(), None);
    }

    #[test]
    fn a_submit_while_busy_asks_to_wait_and_writes_nothing() {
        let mut f = fixture(&[("content/level.gasset", "Scene")]);
        let field = f.new_scene();
        let release = f.busy();

        f.submit("Meadow");

        assert_eq!(
            f.state().pending_error.as_deref(),
            Some("Wait for the project to finish")
        );
        assert_eq!(f.focused(), Some(field));
        assert!(f.files().is_empty());
        drop(release);
    }

    #[test]
    fn a_failed_write_shows_inline_on_submit_and_in_the_status_on_focus_loss() {
        let mut f = fixture(&[("content/level.gasset", "Scene")]);
        let blocker = f.root.path().join("blocker");
        std::fs::write(&blocker, b"x").unwrap();
        f.world
            .get_resource_mut::<ProjectState>()
            .unwrap()
            .project
            .as_mut()
            .unwrap()
            .root = blocker;
        let field = f.new_scene();

        f.submit("Meadow");

        let error = f.state().pending_error.clone().unwrap();
        assert!(
            error.starts_with("Could not create content/Meadow.gasset"),
            "{error}"
        );
        assert_eq!(f.focused(), Some(field));

        f.lose_focus("Meadow");

        assert!(f.state().pending.is_none());
        assert_eq!(f.state().pending_error, None);
        assert!(
            f.status()
                .starts_with("Could not create content/Meadow.gasset"),
            "{}",
            f.status()
        );
    }

    fn create_scene(world: &mut World, systems: &mut [BoxedSystem], name: &str) -> AssetId {
        run_browser(world, systems);
        let row = new_rows(world)[0];
        left_click(world, row);
        run_browser(world, systems);
        let field = pending_field(world).unwrap();
        set_name(world, field, name);
        send(
            world,
            UITextInputSubmitted {
                entity: field,
                value: name.into(),
            },
        );
        run_browser(world, systems);
        let opened = take_opened(world);
        assert_eq!(opened.len(), 1);
        opened[0]
    }

    fn entry_of(f: &SceneFixture, id: AssetId) -> AssetEntry {
        f.world
            .get_resource::<AssetRegistry>()
            .unwrap()
            .get(id)
            .unwrap()
            .clone()
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

        let id = create_scene(&mut f.world, &mut systems, "Meadow");
        let created = entry_of(&f, id);
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

    #[test]
    fn with_an_unsaved_scene_open_a_new_scene_is_written_and_the_guard_holds_its_open() {
        let mut f = SceneFixture::new();
        let document = f.open();
        f.rename(document, "first", "renamed");
        assert!(f.unsaved(document));
        let mut systems = browser(&mut f.world);

        let id = create_scene(&mut f.world, &mut systems, "Meadow");

        assert!(f.project_root.join("content/Meadow.gasset").is_file());
        let created = entry_of(&f, id);
        f.request(&created);
        assert!(
            f.world
                .get_resource::<crate::guard::UnsavedGuard>()
                .unwrap()
                .asking()
        );
        assert_eq!(
            f.document(document).current.as_ref().map(|a| a.id),
            Some(f.scene.id)
        );
    }
}

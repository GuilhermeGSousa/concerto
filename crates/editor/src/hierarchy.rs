//! The world tree: the entities of the scene being edited, walked through `Children` from its root, or while playing from every top-level entity of the game's world.
use crate::actions::{DeleteEntity, DuplicateEntity, EditorContextSet, RenameEntity, TreeContext};
use crate::asset_editor::{ActiveEditor, EditorDocument};
use crate::dock::{DockedApp, PanelDescriptor, PanelRegistry, Region};
use crate::fonts::{button_icon, draw_row_icon, glyph, icon, icon_button, row_icon};
use crate::hosted::{EditTarget, EditWorld, EditWorldSignals, PlayState};
use crate::inspector::FocusNameField;
use crate::scene::SceneState;
use crate::tools::{
    entity_ops::EntityEdit,
    scene::SceneRoot,
    selection::{Select, Selection},
};
use crate::tree_items::{OrderItems, despawn_outermost, order_items};
use concerto_app::{
    App, Plugin,
    schedule_groups::{LateUpdate, Startup},
};
use concerto_color::Color;
use concerto_ecs::{
    Component, Entity, IntoSystemConfig, Query, Ref, Res, ResMut, Resource, World,
    command::CommandQueue,
    component::{name::Name, removed::RemovedComponents},
    entity::{
        EntityHashMap,
        hierarchy::{ChildOf, Children},
    },
    events::event_reader::EventReader,
    query::{
        change_detection::DetectChanges,
        filter::{Added, Changed, Or, With, Without},
    },
    signal::{
        On,
        lifecycle::{Add, Remove},
        listener::{GlobalListener, IntoListener},
    },
};
use concerto_mesh::MeshComponent;
use concerto_render::components::{camera::Camera, light::Light};
use concerto_ui::{
    anchor::{UIAnchorTarget, UIAnchoredPanel},
    elements::prelude::*,
    focus::FocusedWidget,
    interaction::{Interactable, UIClick, UIDisabled},
    items::{UIHidden, UIItem, UIItemContextMenu, UIItemSelected, UIReveal, UISelected},
    node::{UINode, UIRect},
    scroll::UIScrollArea,
    sets::UiSet,
    text::UIText,
    text_input::UITextInputChanged,
    theme::UITheme,
    transform::UIValue,
    tree::{UITree, UITreeElement},
};
use concerto_window::input::MouseButton;
use concerto_window::input::actions::{ActionFired, ActionMap};
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};

pub const PANEL_ID: &str = "concerto.score";

const ROW_HEIGHT: f32 = 32.0;

#[derive(Resource, Default)]
pub struct HierarchyState {
    filter: String,
    items: HashMap<Entity, Entity>,
    pending_scene_reveal: Option<Entity>,
    top_level: Vec<Entity>,
}

impl HierarchyState {
    /// Expands the ancestors of `scene_entity`, an entity of the edit `App`'s world, and scrolls its row into view once its item is placed.
    pub fn reveal_entity(&mut self, scene_entity: Entity) {
        self.pending_scene_reveal = Some(scene_entity);
    }
}

/// How a row of the world tree differs from a plain one: a muted name, its own icon, or a marker after the name.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RowLook {
    pub muted: bool,
    pub icon: Option<char>,
    pub marker: Option<char>,
}

/// The rows that look other than plain, by their entity in the edit `App`'s world; the plugins that know why fill it.
#[derive(Resource, Default, Debug, PartialEq)]
pub struct RowLooks(pub EntityHashMap<RowLook>);

#[derive(Component)]
pub(crate) struct TreeView;
#[derive(Component)]
pub(crate) struct Filter;

#[derive(Component)]
enum Label {
    Title,
    Footer,
}

#[derive(Component)]
pub(crate) struct HierarchyItem {
    scene_entity: Entity,
    mark: Entity,
    label: Entity,
    marker: Entity,
    count: Entity,
}

#[derive(Component)]
struct AddEntityButton;

/// The shared row menu and the entity it was opened for.
#[derive(Component)]
struct RowMenu {
    scene_target: Option<Entity>,
}

#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
enum RowMenuItem {
    AddChild,
    Duplicate,
    Rename,
    Delete,
}

const MENU_WIDTH: f32 = 168.0;

pub struct HierarchyPlugin;

impl Plugin for HierarchyPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(HierarchyState::default());
        app.insert_resource(RowLooks::default());
        app.add_panel(PanelDescriptor {
            id: PANEL_ID,
            title: "Score",
            region: Region::Left,
        });
        install_listeners(app.main_mut().world_mut());
        app.add_system(Startup, build_panel);
        app.add_system(LateUpdate, filter_tree)
            .add_system(LateUpdate, reconcile_structure.before(UiSet::Widgets))
            .add_system(
                LateUpdate,
                filter_items
                    .after(reconcile_structure)
                    .before(UiSet::Project),
            )
            .add_system(
                LateUpdate,
                reconcile_content
                    .after(reconcile_structure)
                    .before(UiSet::Materials),
            )
            .add_system(LateUpdate, sync_tree_context.in_set(EditorContextSet))
            .add_system(LateUpdate, entity_shortcuts)
            .add_system(LateUpdate, sync_row_menu_items)
            .add_system(LateUpdate, show_header);
    }
}

pub(crate) fn install_listeners(world: &mut World) {
    world.add_listener(forget_scene);
    world.add_listener(enable_add_button);
    world.add_listener(disable_add_button);
}

fn forget_scene(
    _: On<Remove<EditTarget>>,
    mut state: ResMut<HierarchyState>,
    looks: Option<ResMut<RowLooks>>,
    mut menus: Query<&mut RowMenu>,
    mut views: Query<&mut UIScrollArea, With<TreeView>>,
    parents: Query<&ChildOf>,
    mut cmd: CommandQueue,
) {
    let items: HashSet<Entity> = state.items.drain().map(|(_, item)| item).collect();
    despawn_outermost(&items, &parents, &mut cmd);
    state.pending_scene_reveal = None;
    state.top_level.clear();
    if let Some(mut looks) = looks
        && !looks.0.is_empty()
    {
        looks.0.clear();
    }
    for mut menu in menus.iter_mut() {
        menu.scene_target = None;
    }
    for mut area in views.iter_mut() {
        if area.offset != 0.0 {
            area.offset = 0.0;
        }
    }
}

fn enable_add_button(
    _: On<Add<EditTarget>>,
    buttons: Query<(Entity, &AddEntityButton, Option<&UIDisabled>)>,
    mut cmd: CommandQueue,
) {
    for (entity, _, disabled) in buttons.iter() {
        if disabled.is_some() {
            cmd.remove::<UIDisabled>(entity);
        }
    }
}

fn disable_add_button(
    _: On<Remove<EditTarget>>,
    buttons: Query<(Entity, &AddEntityButton, Option<&UIDisabled>)>,
    mut cmd: CommandQueue,
) {
    for (entity, _, disabled) in buttons.iter() {
        if disabled.is_none() {
            cmd.insert(UIDisabled, entity);
        }
    }
}

fn line(height: f32) -> UINode {
    UINode::default()
        .with_height(UIValue::Px(height))
        .with_flex_shrink(0.0)
        .with_padding(UIRect::axes(6.0, 8.0))
}

fn build_panel(mut cmd: CommandQueue, registry: Res<PanelRegistry>, theme: Res<UITheme>) {
    if let Some(body) = registry.body(PANEL_ID) {
        spawn_panel(&mut cmd, body, &theme);
    }
}

pub fn spawn_panel(cmd: &mut CommandQueue, parent: Entity, theme: &UITheme) {
    cmd.entity(parent).add_child_with(
        (
            UINode::default()
                .with_width(UIValue::Percent(100.0))
                .with_flex_grow(1.0)
                .with_flex_direction(taffy::FlexDirection::Column)
                .clipped(),
            Interactable,
        ),
        |mut tree| {
            tree.add_child_with(
                theme.row().height(UIValue::Px(42.0)).fixed(),
                |mut header| {
                    header
                        .add_child((
                            line(42.0).with_flex_grow(1.0).with_flex_shrink(1.0),
                            theme.text("WORLD"),
                            Label::Title,
                        ))
                        .add_child((
                            icon_button(theme).on_click(add_entity),
                            button_icon(theme, glyph::PLUS),
                            AddEntityButton,
                            UIDisabled,
                        ));
                },
            )
            .add_child((
                theme
                    .text_field("Search entities…")
                    .height(UIValue::Px(38.0))
                    .padding(UIRect::axes(6.0, 8.0)),
                Filter,
            ));

            tree.add_child((
                UINode::default()
                    .with_height(UIValue::Px(0.0))
                    .with_flex_grow(1.0),
                UITree {
                    row_height: Some(ROW_HEIGHT),
                    ..Default::default()
                },
                TreeView,
                order_items.into_listener(),
            ));

            tree.add_child((line(46.0), theme.text(""), Label::Footer));
        },
    );

    let mut menu = cmd.spawn((
        theme
            .context_menu()
            .width(UIValue::Px(MENU_WIDTH))
            .padding(theme.spacing_xs),
        RowMenu { scene_target: None },
    ));
    for (label, item) in [
        ("Add child", RowMenuItem::AddChild),
        ("Duplicate", RowMenuItem::Duplicate),
        ("Rename", RowMenuItem::Rename),
        ("Delete", RowMenuItem::Delete),
    ] {
        let color = if item == RowMenuItem::Delete {
            theme.error
        } else {
            theme.text
        };
        menu.add_child_with(
            (
                theme
                    .pressable()
                    .row()
                    .padding(UIRect::axes(6.0, 8.0))
                    .on_click(press_row_menu_item),
                item,
            ),
            |mut entry| {
                entry.add_child(theme.label(label).font_size(12.5).color(color).no_wrap());
            },
        );
    }
}

fn spawn_item(
    cmd: &mut CommandQueue,
    theme: &UITheme,
    scene_entity: Entity,
    parent: Entity,
) -> Entity {
    let mark = cmd.spawn(row_icon(theme, glyph::CIRCLE)).entity();
    let label = cmd
        .spawn((
            UINode::default()
                .with_flex_grow(1.0)
                .with_flex_shrink(1.0)
                .with_padding(UIRect::axes(0.0, 8.0)),
            theme.text("").single_line(),
        ))
        .entity();
    let marker = cmd
        .spawn((
            UINode::default()
                .with_flex_shrink(0.0)
                .with_align_self(taffy::AlignItems::Center),
            icon(theme, ' ', 12.0).color(theme.accent),
        ))
        .entity();
    let count = cmd
        .spawn(
            theme
                .label("")
                .muted()
                .mono()
                .font_size(10.0)
                .no_wrap()
                .fixed()
                .margin(UIRect::axes(0.0, 8.0)),
        )
        .entity();
    let item = cmd
        .spawn((
            theme.canvas().fill(Color::TRANSPARENT).radius_sm(),
            UITreeElement::default(),
            HierarchyItem {
                scene_entity,
                mark,
                label,
                marker,
                count,
            },
            select_item.into_listener(),
            open_row_menu.into_listener(),
            ChildOf::new(parent),
        ))
        .entity();
    for content in [mark, label, marker, count] {
        cmd.add_child(item, content);
    }
    item
}

fn playing(active: &ActiveEditor, states: &Query<&PlayState>) -> bool {
    active
        .0
        .and_then(|document| states.get_entity(document))
        .is_some_and(|state| *state == PlayState::Playing)
}

fn add_entity(
    on: On<UIClick>,
    selection: EditWorld<Res<Selection>>,
    roots: EditWorld<Query<(Entity, &SceneRoot)>>,
    entities: EditWorld<Query<Entity>>,
    active: Res<ActiveEditor>,
    states: Query<&PlayState>,
    mut scene: EditWorldSignals,
) {
    if on.signal().button != MouseButton::Left {
        return;
    }
    if playing(&active, &states) {
        scene.trigger(EntityEdit::Create {
            parent: selection
                .entity()
                .filter(|entity| entities.get_entity(*entity).is_some()),
        });
        return;
    }
    let root = roots.iter().next().map(|(entity, _)| entity);
    if let Some(parent) = selection.entity().or(root) {
        scene.trigger(EntityEdit::Create {
            parent: Some(parent),
        });
    }
}

fn press_row_menu_item(
    on: On<UIClick>,
    items: Query<&RowMenuItem>,
    mut menus: Query<(&RowMenu, &mut UIAnchoredPanel)>,
    mut focus_name: ResMut<FocusNameField>,
    mut scene: EditWorldSignals,
) {
    if on.signal().button != MouseButton::Left {
        return;
    }
    let Some(item) = items.get_entity(on.entity()) else {
        return;
    };
    for (menu, mut panel) in menus.iter_mut() {
        if let Some(scene_target) = menu.scene_target.filter(|_| panel.open) {
            match *item {
                RowMenuItem::AddChild => scene.trigger(EntityEdit::Create {
                    parent: Some(scene_target),
                }),
                RowMenuItem::Duplicate => scene.trigger(EntityEdit::Duplicate(scene_target)),
                RowMenuItem::Rename => focus_name.0 = true,
                RowMenuItem::Delete => scene.trigger(EntityEdit::Delete(scene_target)),
            }
        }
        panel.open = false;
    }
}

fn entity_shortcuts(
    mut fired: EventReader<ActionFired>,
    selection: EditWorld<Res<Selection>>,
    mut focus_name: ResMut<FocusNameField>,
    mut scene: EditWorldSignals,
) {
    for action in fired.read() {
        let Some(entity) = selection.entity() else {
            continue;
        };
        if action.is(DeleteEntity) {
            scene.trigger(EntityEdit::Delete(entity));
        } else if action.is(DuplicateEntity) {
            scene.trigger(EntityEdit::Duplicate(entity));
        } else if action.is(RenameEntity) {
            focus_name.0 = true;
        }
    }
}

fn sync_row_menu_items(
    roots: EditWorld<Query<&SceneRoot>>,
    menus: Query<&RowMenu>,
    items: Query<(Entity, &RowMenuItem, Option<&UIDisabled>)>,
    mut cmd: CommandQueue,
) {
    let on_root = menus
        .iter()
        .next()
        .and_then(|menu| menu.scene_target)
        .is_some_and(|scene_target| roots.get_entity(scene_target).is_some());
    for (entity, item, disabled) in items.iter() {
        let wanted = on_root && *item != RowMenuItem::AddChild;
        if wanted && disabled.is_none() {
            cmd.insert(UIDisabled, entity);
        } else if !wanted && disabled.is_some() {
            cmd.remove::<UIDisabled>(entity);
        }
    }
}

fn select_item(on: On<UIItemSelected>, items: Query<&HierarchyItem>, mut scene: EditWorldSignals) {
    if let Some(item) = items.get_entity(on.entity()) {
        scene.trigger(Select(Some(item.scene_entity)));
    }
}

fn open_row_menu(
    on: On<UIItemContextMenu>,
    items: Query<&HierarchyItem>,
    mut menus: Query<(&mut RowMenu, &mut UIAnchoredPanel)>,
    mut scene: EditWorldSignals,
) {
    let Some(item) = items.get_entity(on.entity()) else {
        return;
    };
    scene.trigger(Select(Some(item.scene_entity)));
    for (mut menu, mut panel) in menus.iter_mut() {
        menu.scene_target = Some(item.scene_entity);
        panel.target = UIAnchorTarget::from_point(on.signal().position);
        panel.owner = Some(on.entity());
        panel.open = true;
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ListRoot {
    Include,
    Exclude,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct ListedEntity {
    pub(crate) entity: Entity,
    pub(crate) parent: Option<Entity>,
    pub(crate) depth: usize,
}

pub(crate) fn listed_entities(
    roots: &Query<(Entity, &SceneRoot)>,
    top_level: &Query<Entity, (Without<ChildOf>, Without<GlobalListener>)>,
    scene_children: &Query<&Children>,
    active: &ActiveEditor,
    states: &Query<&PlayState>,
    root: ListRoot,
) -> Vec<ListedEntity> {
    let mut tops: Vec<Entity> = if playing(active, states) {
        top_level.iter().collect()
    } else {
        roots.iter().map(|(entity, _)| entity).collect()
    };
    tops.sort_by_key(|entity| (entity.index(), entity.generation()));
    if root == ListRoot::Exclude {
        tops = tops
            .into_iter()
            .flat_map(|top| {
                if roots.contains_entity(top) {
                    scene_children
                        .get_entity(top)
                        .map(|children| children.iter().copied().collect())
                        .unwrap_or_default()
                } else {
                    vec![top]
                }
            })
            .collect();
    }
    let mut listed = Vec::new();
    let mut pending: Vec<ListedEntity> = tops
        .into_iter()
        .rev()
        .map(|entity| ListedEntity {
            entity,
            parent: None,
            depth: 0,
        })
        .collect();
    while let Some(next) = pending.pop() {
        listed.push(next);
        if let Some(children) = scene_children.get_entity(next.entity) {
            pending.extend(children.iter().rev().map(|child| ListedEntity {
                entity: *child,
                parent: Some(next.entity),
                depth: next.depth + 1,
            }));
        }
    }
    listed
}

pub(crate) fn reconcile_structure(
    mut scene: EditWorld<(
        Query<(Entity, &SceneRoot)>,
        Query<Entity, (Without<ChildOf>, Without<GlobalListener>)>,
        Query<&Children>,
        Query<Entity, Changed<Children>>,
        RemovedComponents<Children>,
    )>,
    active: Res<ActiveEditor>,
    states: Query<&PlayState>,
    (played, targeted): (
        Query<Entity, Or<(Added<PlayState>, Changed<PlayState>)>>,
        Query<Entity, Added<EditTarget>>,
    ),
    mut state: ResMut<HierarchyState>,
    views: Query<Entity, With<TreeView>>,
    parents: Query<&ChildOf>,
    placed: Query<&UIItem>,
    theme: Res<UITheme>,
    mut cmd: CommandQueue,
) {
    let (roots, top_level, scene_children, changed_children, removed_children) = &mut *scene;
    let removed = removed_children.read().count() > 0;
    let changed = changed_children.iter().next().is_some();
    let played = played.iter().next().is_some();
    let targeted = targeted.iter().next().is_some();
    let tops: Vec<Entity> = top_level.iter().collect();
    let structural =
        active.is_changed() || played || targeted || changed || removed || tops != state.top_level;
    let Some(view) = views.iter().next() else {
        return;
    };
    if structural {
        state.top_level = tops;
        reconcile_items(
            &listed_entities(
                roots,
                top_level,
                scene_children,
                &active,
                &states,
                ListRoot::Include,
            ),
            view,
            &mut state,
            &parents,
            &theme,
            &mut cmd,
        );
    }

    if let Some(scene_entity) = state.pending_scene_reveal {
        match state.items.get(&scene_entity) {
            None => state.pending_scene_reveal = None,
            Some(&item) if placed.contains_entity(item) => {
                cmd.entity(item).trigger(UIReveal);
                state.pending_scene_reveal = None;
            }
            Some(_) => {}
        }
    }
}

fn reconcile_items(
    listed: &[ListedEntity],
    view: Entity,
    state: &mut HierarchyState,
    parents: &Query<&ChildOf>,
    theme: &UITheme,
    cmd: &mut CommandQueue,
) {
    let wanted: HashSet<Entity> = listed.iter().map(|listed| listed.entity).collect();
    let mut order: HashMap<Entity, Vec<Entity>> = HashMap::new();
    for listed in listed {
        let scene_entity = listed.entity;
        let parent = listed
            .parent
            .and_then(|scene_parent| state.items.get(&scene_parent).copied())
            .unwrap_or(view);
        let item = match state.items.get(&scene_entity) {
            Some(&item) => {
                if parents.get_entity(item).map(ChildOf::parent) != Some(parent) {
                    cmd.add_child(parent, item);
                }
                item
            }
            None => {
                let item = spawn_item(cmd, theme, scene_entity, parent);
                state.items.insert(scene_entity, item);
                item
            }
        };
        order.entry(parent).or_default().push(item);
    }

    let gone: Vec<Entity> = state
        .items
        .keys()
        .copied()
        .filter(|scene_entity| !wanted.contains(scene_entity))
        .collect();
    let gone: HashSet<Entity> = gone
        .into_iter()
        .filter_map(|scene_entity| state.items.remove(&scene_entity))
        .collect();
    despawn_outermost(&gone, parents, cmd);
    cmd.entity(view)
        .trigger(OrderItems(order.into_iter().collect()));
}

pub(crate) fn reconcile_content(
    mut scene: EditWorld<(
        Res<Selection>,
        Query<Ref<Name>>,
        Query<&SceneRoot>,
        Query<Ref<Children>>,
        Query<(Option<&Camera>, Option<&Light>, Option<&MeshComponent>)>,
        Query<
            Entity,
            Or<(
                Changed<Name>,
                Changed<Children>,
                Added<SceneRoot>,
                Added<Camera>,
                Added<Light>,
                Added<MeshComponent>,
            )>,
        >,
        (
            RemovedComponents<Name>,
            RemovedComponents<Children>,
            RemovedComponents<Camera>,
            RemovedComponents<Light>,
            RemovedComponents<MeshComponent>,
        ),
    )>,
    active: Res<ActiveEditor>,
    documents: Query<Ref<EditorDocument>>,
    (state, looks): (Res<HierarchyState>, Res<RowLooks>),
    (items, added): (
        Query<(Entity, Ref<HierarchyItem>, Option<&UISelected>)>,
        Query<Entity, Added<HierarchyItem>>,
    ),
    mut texts: Query<&mut UIText>,
    theme: Res<UITheme>,
    mut cmd: CommandQueue,
) {
    let (selection, names, roots, scene_children, kinds, changed, removed) = &mut *scene;
    let (removed_names, removed_children, removed_cameras, removed_lights, removed_meshes) =
        removed;
    let mut dirty: HashSet<Entity> = changed.iter().collect();
    dirty.extend(removed_names.read());
    dirty.extend(removed_children.read());
    dirty.extend(removed_cameras.read());
    dirty.extend(removed_lights.read());
    dirty.extend(removed_meshes.read());
    let document = active.0.and_then(|document| documents.get_entity(document));
    let address = document
        .as_ref()
        .and_then(|document| document.current.as_ref())
        .map(|asset| asset.address.as_str())
        .unwrap_or_default();
    let every_row = selection.is_changed()
        || active.is_changed()
        || theme.is_changed()
        || looks.is_changed()
        || document
            .as_ref()
            .is_some_and(|document| document.is_changed());
    let scene = (&*names, &*roots, &*scene_children, &*kinds);
    let selected = selection.entity();
    let mut update = |row| {
        update_row(
            row, selected, address, scene, &looks, &mut texts, &theme, &mut cmd,
        )
    };
    if every_row {
        items.iter().for_each(&mut update);
        return;
    }
    let rows: HashSet<Entity> = added
        .iter()
        .chain(
            dirty
                .iter()
                .filter_map(|scene_entity| state.items.get(scene_entity).copied()),
        )
        .collect();
    rows.into_iter()
        .filter_map(|row| items.get_entity(row))
        .for_each(update);
}

fn update_row(
    (entity, item, selected): (Entity, Ref<HierarchyItem>, Option<&UISelected>),
    selection: Option<Entity>,
    address: &str,
    (names, roots, scene_children, kinds): (
        &Query<Ref<Name>>,
        &Query<&SceneRoot>,
        &Query<Ref<Children>>,
        &Query<(Option<&Camera>, Option<&Light>, Option<&MeshComponent>)>,
    ),
    looks: &RowLooks,
    texts: &mut Query<&mut UIText>,
    theme: &UITheme,
    cmd: &mut CommandQueue,
) {
    let scene_entity = item.scene_entity;
    let is_root = roots.get_entity(scene_entity).is_some();
    let look = looks.0.get(&scene_entity).copied().unwrap_or_default();
    let children = scene_children.get_entity(scene_entity);
    let count = children
        .as_ref()
        .map_or(0, |children| children.iter().len());
    let kind = kinds.get_entity(scene_entity);
    let mark = match (kind, look.icon) {
        _ if is_root => glyph::STACK,
        (_, Some(icon)) => icon,
        (Some((Some(_), _, _)), _) => glyph::CAMERA,
        (Some((_, Some(_), _)), _) => glyph::LIGHTBULB,
        (Some((_, _, Some(_))), _) => glyph::CUBE,
        _ => glyph::CIRCLE,
    };
    let name = names.get_entity(scene_entity);
    let same_name = !item.is_added() && name.as_ref().is_some_and(|name| !name.is_changed());
    let label: Option<Cow<str>> = match name.as_deref() {
        _ if is_root => Some(address.into()),
        _ if same_name => None,
        Some(name) if !name.as_str().is_empty() => Some(name.as_str().into()),
        _ => Some(format!("Entity {}", scene_entity.index()).into()),
    };
    let same_children = !item.is_added() && children.is_some_and(|children| !children.is_changed());
    let count: Option<Cow<str>> = match count {
        0 => Some("".into()),
        _ if same_children => None,
        count => Some(count.to_string().into()),
    };
    let marker: Cow<str> = look
        .marker
        .map_or("".into(), |marker| marker.to_string().into());
    for (text, value) in [
        (item.label, label),
        (item.count, count),
        (item.marker, Some(marker)),
    ] {
        if let Some(value) = value
            && let Some(mut text) = texts.get_entity_mut(text)
            && text.text != *value
        {
            text.text = value.into_owned();
        }
    }
    let label_color = if look.muted {
        theme.text_muted
    } else {
        theme.text
    };
    if let Some(mut text) = texts.get_entity_mut(item.label)
        && text.color != label_color
    {
        text.color = label_color;
    }
    let wanted = selection == Some(scene_entity);
    if let Some(mut text) = texts.get_entity_mut(item.mark) {
        draw_row_icon(&mut text, theme, mark, wanted);
    }
    if wanted && selected.is_none() {
        cmd.insert(UISelected, entity);
    } else if !wanted && selected.is_some() {
        cmd.remove::<UISelected>(entity);
    }
}

fn filter_tree(
    mut events: EventReader<UITextInputChanged>,
    filters: Query<&Filter>,
    mut state: ResMut<HierarchyState>,
    mut views: Query<&mut UIScrollArea, With<TreeView>>,
) {
    for event in events.read() {
        if filters.get_entity(event.entity).is_some() {
            state.filter = event.value.clone();
            for mut area in views.iter_mut() {
                area.offset = 0.0;
            }
        }
    }
}

fn filter_items(
    state: Res<HierarchyState>,
    names: EditWorld<Query<&Name>>,
    items: Query<(Entity, &HierarchyItem, Option<&UIHidden>)>,
    mut views: Query<&mut UITree, With<TreeView>>,
    mut cmd: CommandQueue,
) {
    let filter = state.filter.to_lowercase();
    let filtering = !filter.is_empty();
    for mut tree in views.iter_mut() {
        if tree.filtering != filtering {
            tree.filtering = filtering;
        }
    }
    for (entity, item, hidden) in items.iter() {
        let hide = filtering
            && !names
                .get_entity(item.scene_entity)
                .is_some_and(|name| name.as_str().to_lowercase().contains(&filter));
        if hide && hidden.is_none() {
            cmd.insert(UIHidden, entity);
        } else if !hide && hidden.is_some() {
            cmd.remove::<UIHidden>(entity);
        }
    }
}

fn sync_tree_context(
    focus: Res<FocusedWidget>,
    items: Query<&UIItem>,
    views: Query<&TreeView>,
    mut actions: ResMut<ActionMap>,
) {
    let focused = (**focus)
        .and_then(|entity| items.get_entity(entity))
        .is_some_and(|item| views.contains_entity(item.owner()));
    if focused {
        actions.push_context(TreeContext);
    } else {
        actions.pop_context(TreeContext);
    }
}

fn show_header(
    state: Res<HierarchyState>,
    scenes: Res<SceneState>,
    mut labels: Query<(&Label, &mut UIText)>,
) {
    for (label, mut text) in labels.iter_mut() {
        let value = match label {
            Label::Title => format!("WORLD · {} entities", state.items.len()),
            Label::Footer if scenes.loading() => "Loading…".to_string(),
            Label::Footer => String::new(),
        };
        if text.text != value {
            text.text = value;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        hosted::{HostedApp, testing::SceneFixture},
        inspector::InspectorData,
    };
    use concerto_ecs::{
        IntoSystem, System, events::event_channel::EventChannel, query::filter::With,
        system::BoxedSystem,
    };
    use concerto_foundation::transform::Transform;
    use concerto_ui::tree::update_trees_system;
    use glam::Vec2;

    struct Fixture {
        scene: SceneFixture,
        document: Entity,
        root: Entity,
        node: Entity,
        systems: Vec<BoxedSystem>,
    }

    fn fixture() -> Fixture {
        let mut scene = SceneFixture::new();
        scene.world.insert_resource(FocusNameField::default());
        scene
            .world
            .insert_resource(EventChannel::<ActionFired>::default());
        let document = scene.open();
        let root = scene.scene_root(document);
        let node = scene.scene_entity(document, "first");
        let second = scene.scene_entity(document, "second");
        scene.scene_world_mut(document).despawn(second);
        let mut systems = vec![
            reconcile_structure.into_boxed_system(),
            filter_items.into_boxed_system(),
            update_trees_system(),
            reconcile_content.into_boxed_system(),
            sync_row_menu_items.into_boxed_system(),
            show_header.into_boxed_system(),
        ];
        for system in &mut systems {
            system.initialize(&mut scene.world);
        }
        let mut fixture = Fixture {
            scene,
            document,
            root,
            node,
            systems,
        };
        fixture.refresh();
        fixture
    }

    impl Fixture {
        fn refresh(&mut self) {
            for system in &mut self.systems {
                system.run_and_apply((), &mut self.scene.world);
            }
        }
        fn state(&self) -> &HierarchyState {
            self.scene.world.get_resource::<HierarchyState>().unwrap()
        }
        fn item_of(&self, scene_entity: Entity) -> Entity {
            *self
                .state()
                .items
                .get(&scene_entity)
                .expect("the entity has an item")
        }
        fn expand(&mut self, scene_entity: Entity) {
            let item = self.item_of(scene_entity);
            self.scene
                .world
                .get_component_for_entity_mut::<UITreeElement>(item)
                .unwrap()
                .expanded = true;
            self.refresh();
        }
        fn expanded(&self, scene_entity: Entity) -> bool {
            self.scene
                .world
                .get_component_for_entity::<UITreeElement>(self.item_of(scene_entity))
                .unwrap()
                .expanded
        }
        fn ui_selected(&self, scene_entity: Entity) -> bool {
            self.scene
                .world
                .get_component_for_entity::<UISelected>(self.item_of(scene_entity))
                .is_some()
        }
        fn label_of(&self, scene_entity: Entity) -> String {
            let label = self
                .scene
                .world
                .get_component_for_entity::<HierarchyItem>(self.item_of(scene_entity))
                .unwrap()
                .label;
            self.scene
                .world
                .get_component_for_entity::<UIText>(label)
                .unwrap()
                .text
                .clone()
        }
        fn item_children(&self, parent: Entity) -> Vec<Entity> {
            self.scene
                .world
                .get_component_for_entity::<Children>(parent)
                .map(|children| {
                    children
                        .iter()
                        .filter_map(|child| {
                            self.scene
                                .world
                                .get_component_for_entity::<HierarchyItem>(*child)
                                .map(|item| item.scene_entity)
                        })
                        .collect()
                })
                .unwrap_or_default()
        }
        fn top_level(&mut self) -> Vec<Entity> {
            let tree = self
                .scene
                .world
                .query::<Entity, With<TreeView>>()
                .iter(&mut self.scene.world)
                .next()
                .unwrap();
            self.item_children(tree)
        }
        fn click(&mut self, entity: Entity, button: MouseButton) {
            self.scene.world.trigger_on(
                entity,
                UIClick {
                    position: Vec2::new(40.0, 60.0),
                    button,
                },
            );
            self.refresh();
        }
        fn select(&mut self, entity: Entity) {
            self.scene
                .scene_world_mut(self.document)
                .trigger(Select(Some(entity)));
        }
        fn menu_item(&mut self, scene_entity: Entity, wanted: RowMenuItem) {
            let row = self.item_of(scene_entity);
            self.click(row, MouseButton::Right);
            let item = self
                .scene
                .world
                .query::<(Entity, &RowMenuItem), ()>()
                .iter(&mut self.scene.world)
                .find(|(_, item)| **item == wanted)
                .unwrap()
                .0;
            self.click(item, MouseButton::Left);
        }
        fn selected(&self) -> Option<Entity> {
            self.scene.selected(self.document)
        }
        fn valid(&self, entity: Entity) -> bool {
            self.scene
                .scene_world(self.document)
                .entity_is_valid(entity)
        }
        fn children(&self, entity: Entity) -> Vec<Entity> {
            self.scene
                .scene_world(self.document)
                .get_component_for_entity::<Children>(entity)
                .map(|children| children.iter().copied().collect())
                .unwrap_or_default()
        }
        fn spawn_under(&mut self, parent: Entity, name: &str) -> Entity {
            let scene = self.scene.scene_world_mut(self.document);
            let entity = scene.spawn((Name::new(name), Transform::IDENTITY));
            scene.entity_mut(parent).add_child(entity);
            entity
        }
        fn fire(&mut self, action: impl concerto_window::input::actions::ActionLabel) {
            let channel = self
                .scene
                .world
                .get_resource_mut::<EventChannel<ActionFired>>()
                .unwrap();
            channel.update();
            channel.update();
            channel.push_event(ActionFired::new(action));
            let mut shortcuts = entity_shortcuts.into_system();
            shortcuts.initialize(&mut self.scene.world);
            shortcuts.run_and_apply((), &mut self.scene.world);
            self.refresh();
        }
        fn rename_requested(&mut self) -> bool {
            std::mem::take(
                &mut self
                    .scene
                    .world
                    .get_resource_mut::<FocusNameField>()
                    .unwrap()
                    .0,
            )
        }
    }

    #[test]
    fn the_tree_follows_spawns_renames_reorders_reparenting_and_despawns() {
        let mut f = fixture();
        let a = f.spawn_under(f.root, "a");
        let b = f.spawn_under(f.root, "b");
        let leaf = f.spawn_under(f.node, "leaf");
        f.refresh();
        f.expand(f.root);
        f.expand(f.node);
        let root_item = f.item_of(f.root);
        assert_eq!(f.item_children(root_item), [f.node, a, b]);

        f.scene
            .scene_world_mut(f.document)
            .insert(Name::new("renamed"), a);
        let order = [b, a, f.node];
        f.scene
            .scene_world_mut(f.document)
            .get_component_for_entity_mut::<Children>(f.root)
            .unwrap()
            .sort_by_key(|child| order.iter().position(|wanted| *wanted == child));
        f.refresh();
        assert_eq!(f.label_of(a), "renamed");
        assert_eq!(f.item_children(root_item), order);

        let node_item = f.item_of(f.node);
        f.scene
            .scene_world_mut(f.document)
            .entity_mut(b)
            .add_child(f.node);
        f.refresh();
        assert_eq!(f.item_of(f.node), node_item);
        assert_eq!(f.item_children(f.item_of(b)), [f.node]);
        assert_eq!(f.item_children(root_item), [b, a]);
        assert_eq!(f.item_children(node_item), [leaf]);
        assert!(f.expanded(f.node));

        let gone = f.item_of(a);
        f.scene.scene_world_mut(f.document).despawn(a);
        f.refresh();
        assert_eq!(f.item_children(root_item), [b]);
        assert!(!f.scene.world.entity_is_valid(gone));
    }

    #[test]
    fn selection_flows_both_ways() {
        let mut f = fixture();
        f.expand(f.root);
        let item = f.item_of(f.node);

        f.scene.world.trigger_on(item, UIItemSelected);
        assert_eq!(f.selected(), Some(f.node));

        f.select(f.root);
        f.refresh();
        assert!(f.ui_selected(f.root));
        assert!(!f.ui_selected(f.node));

        f.scene.scene_world_mut(f.document).trigger(Select(None));
        f.refresh();
        assert!(!f.ui_selected(f.root));
    }

    #[test]
    fn the_row_menu_the_add_button_and_the_shortcuts_edit_the_scene() {
        let mut f = fixture();
        f.expand(f.root);

        f.menu_item(f.node, RowMenuItem::AddChild);
        assert_eq!(f.children(f.node).len(), 1);

        f.menu_item(f.node, RowMenuItem::Duplicate);
        assert_eq!(f.children(f.root).len(), 2);
        let copy = f.selected().unwrap();
        assert_ne!(copy, f.node);

        f.menu_item(copy, RowMenuItem::Rename);
        assert!(f.rename_requested());

        f.menu_item(copy, RowMenuItem::Delete);
        assert!(!f.valid(copy));
        assert_eq!(f.children(f.root), [f.node]);

        let button = f
            .scene
            .world
            .query::<Entity, With<AddEntityButton>>()
            .iter(&mut f.scene.world)
            .next()
            .unwrap();
        f.select(f.root);
        f.click(button, MouseButton::Left);
        assert_eq!(f.children(f.root).len(), 2);

        f.select(f.node);
        f.fire(DeleteEntity);
        assert!(!f.valid(f.node));
        assert_eq!(f.children(f.root).len(), 1);
    }

    fn game(app: &mut App) {
        crate::hosted::testing::scene_recipe(app);
        app.add_system(Startup, spawn_character);
    }

    fn spawn_character(mut cmd: CommandQueue) {
        cmd.spawn((Name::new("character"), Transform::IDENTITY))
            .add_child((Name::new("camera"), Transform::IDENTITY));
    }

    #[test]
    fn play_and_stop_rebuild_the_rows_from_the_new_targets_world() {
        let mut f = fixture();
        let document = f.document;
        f.scene
            .world
            .get_resource_mut::<crate::scene::SceneRecipe>()
            .unwrap()
            .recipe = game;
        f.scene.world.trigger(crate::scene::Play(document));
        f.scene.frames_until_playing(document);
        f.scene
            .frames_until("the game never spawned the character", |scene| {
                scene
                    .scene_names(document)
                    .iter()
                    .any(|name| name == "character")
            });
        f.refresh();
        let character = f.scene.scene_entity(document, "character");
        assert!(f.top_level().contains(&character));

        f.scene.world.trigger(crate::hosted::StopPlay(document));
        f.scene.frames_until("the game never stopped", |scene| {
            scene.hosted_is_editing(document)
        });
        f.refresh();

        let root = f.scene.scene_root(document);
        let node = f.scene.scene_entity(document, "first");
        assert_eq!(f.top_level(), [root]);
        f.expand(root);
        assert_eq!(f.item_children(f.item_of(root)), [node]);
        assert_eq!(f.state().items.len(), 2);
    }

    #[test]
    fn replacing_the_scene_rebuilds_the_rows_and_closing_its_tab_empties_the_panels() {
        let mut f = SceneFixture::new();
        let document = f.open();
        f.frame();
        let stale: Vec<Entity> = f
            .world
            .get_resource::<HierarchyState>()
            .unwrap()
            .items
            .values()
            .copied()
            .collect();
        assert!(!stale.is_empty());

        f.request_other_scene(document);
        f.frames_until_idle();

        let state = f.world.get_resource::<HierarchyState>().unwrap();
        let hosted = f
            .world
            .get_component_for_entity::<HostedApp>(document)
            .unwrap();
        assert!(!state.items.is_empty());
        for scene_entity in state.items.keys() {
            assert!(hosted.world_for_test().entity_is_valid(*scene_entity));
        }
        assert!(stale.iter().all(|item| !f.world.entity_is_valid(*item)));

        f.close(document);
        f.frame();
        f.frame();

        assert!(
            f.world
                .get_resource::<HierarchyState>()
                .unwrap()
                .items
                .is_empty()
        );
        assert_eq!(
            *f.world.get_resource::<InspectorData>().unwrap(),
            InspectorData::default()
        );
    }
}

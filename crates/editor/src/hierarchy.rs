//! The world tree: the entities of the scene being edited, walked through `Children` from its root, or while playing from every top-level entity of the game's world.
use crate::actions::{DeleteEntity, DuplicateEntity, RenameEntity, TreeContext};
use crate::asset_editor::{ActiveEditor, EditorDocument};
use crate::dock::{DockedApp, PanelDescriptor, PanelRegistry, Region};
use crate::fonts::{glyph, icon};
use crate::hosted::{EditTarget, EditWorld, EditWorldSignals, PlayState};
use crate::inspector::FocusNameField;
use crate::marks::{self, Mark};
use crate::scene::SceneState;
use crate::tools::{
    entity_ops::EntityEdit,
    scene::SceneRoot,
    selection::{Select, Selection},
};
use crate::tree_items::{despawn_outermost, follow_order};
use concerto_app::{
    App, Plugin,
    schedule_groups::{LateUpdate, Startup},
};
use concerto_color::Color;
use concerto_ecs::{
    Component, Entity, IntoSystemConfig, Query, Res, ResMut, Resource, World,
    command::CommandQueue,
    component::name::Name,
    entity::hierarchy::{ChildOf, Children},
    events::event_reader::EventReader,
    query::filter::{With, Without},
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
    material::UIMaterial,
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
use std::collections::{HashMap, HashSet};

pub const PANEL_ID: &str = "concerto.warren";

const ROW_HEIGHT: f32 = 32.0;

#[derive(Resource, Default)]
pub struct HierarchyState {
    filter: String,
    items: HashMap<Entity, Entity>,
    pending_scene_reveal: Option<Entity>,
}

impl HierarchyState {
    /// Expands the ancestors of `scene_entity`, an entity of the edit `App`'s world, and scrolls its row into view once its item is placed.
    pub fn reveal_entity(&mut self, scene_entity: Entity) {
        self.pending_scene_reveal = Some(scene_entity);
    }

    #[cfg(test)]
    pub(crate) fn pending_scene_reveal(&self) -> Option<Entity> {
        self.pending_scene_reveal
    }

    #[cfg(test)]
    pub(crate) fn items(&self) -> &HashMap<Entity, Entity> {
        &self.items
    }
}

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

const MENU_FILL: Color = Color::srgba(0.090, 0.102, 0.157, 0.96);
const MENU_WIDTH: f32 = 168.0;

pub struct HierarchyPlugin;

impl Plugin for HierarchyPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(HierarchyState::default());
        app.add_panel(PanelDescriptor {
            id: PANEL_ID,
            title: "Warren",
            region: Region::Rail,
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
            .add_system(LateUpdate, sync_tree_context)
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
    menus: Query<&mut RowMenu>,
    views: Query<&mut UIScrollArea, With<TreeView>>,
    parents: Query<&ChildOf>,
    mut cmd: CommandQueue,
) {
    let items: HashSet<Entity> = state.items.drain().map(|(_, item)| item).collect();
    despawn_outermost(&items, &parents, &mut cmd);
    state.pending_scene_reveal = None;
    for mut menu in menus.iter() {
        menu.scene_target = None;
    }
    for mut area in views.iter() {
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
            theme
                .panel()
                .radius(0.0)
                .width(UIValue::Percent(100.0))
                .grow()
                .column()
                .padding(8.0)
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
                            theme
                                .pressable()
                                .size(UIValue::Px(24.0), UIValue::Px(24.0))
                                .padding(UIRect::axes(4.0, 5.0))
                                .on_click(add_entity),
                            icon(theme, glyph::PLUS, theme.font_size_md).muted(),
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
            ));

            tree.add_child((line(46.0), theme.text(""), Label::Footer));
        },
    );

    let mut menu = cmd.spawn((
        theme
            .context_menu()
            .fill(MENU_FILL)
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
    let mark = cmd.spawn(marks::node()).entity();
    let label = cmd
        .spawn((
            UINode::default()
                .with_flex_grow(1.0)
                .with_flex_shrink(1.0)
                .with_padding(UIRect::axes(0.0, 8.0)),
            theme.text("").single_line(),
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
                count,
            },
            select_item.into_listener(),
            open_row_menu.into_listener(),
            ChildOf::new(parent),
        ))
        .entity();
    for content in [mark, label, count] {
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
    menus: Query<(&RowMenu, &mut UIAnchoredPanel)>,
    mut focus_name: ResMut<FocusNameField>,
    mut scene: EditWorldSignals,
) {
    if on.signal().button != MouseButton::Left {
        return;
    }
    let Some(item) = items.get_entity(on.entity()) else {
        return;
    };
    for (menu, mut panel) in menus.iter() {
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
    menus: Query<(&mut RowMenu, &mut UIAnchoredPanel)>,
    mut scene: EditWorldSignals,
) {
    let Some(item) = items.get_entity(on.entity()) else {
        return;
    };
    scene.trigger(Select(Some(item.scene_entity)));
    for (mut menu, mut panel) in menus.iter() {
        menu.scene_target = Some(item.scene_entity);
        panel.target = UIAnchorTarget::from_point(on.signal().position);
        panel.owner = Some(on.entity());
        panel.open = true;
    }
}

fn listed_entities(
    tops: Vec<Entity>,
    scene_children: &Query<&Children>,
) -> Vec<(Entity, Option<Entity>)> {
    let mut listed = Vec::new();
    let mut pending: Vec<(Entity, Option<Entity>)> = tops
        .into_iter()
        .rev()
        .map(|entity| (entity, None))
        .collect();
    while let Some((scene_entity, scene_parent)) = pending.pop() {
        listed.push((scene_entity, scene_parent));
        if let Some(children) = scene_children.get_entity(scene_entity) {
            pending.extend(
                children
                    .iter()
                    .rev()
                    .map(|child| (*child, Some(scene_entity))),
            );
        }
    }
    listed
}

pub(crate) fn reconcile_structure(
    roots: EditWorld<Query<(Entity, &SceneRoot)>>,
    top_level: EditWorld<Query<Entity, (Without<ChildOf>, Without<GlobalListener>)>>,
    scene_children: EditWorld<Query<&Children>>,
    active: Res<ActiveEditor>,
    states: Query<&PlayState>,
    mut state: ResMut<HierarchyState>,
    views: Query<Entity, With<TreeView>>,
    parents: Query<&ChildOf>,
    children: Query<&mut Children>,
    placed: Query<&UIItem>,
    theme: Res<UITheme>,
    mut cmd: CommandQueue,
) {
    let Some(view) = views.iter().next() else {
        return;
    };
    let mut tops: Vec<Entity> = if playing(&active, &states) {
        top_level.iter().collect()
    } else {
        roots.iter().map(|(entity, _)| entity).collect()
    };
    tops.sort_by_key(|entity| (entity.index(), entity.generation()));
    let listed = listed_entities(tops, &scene_children);
    let wanted: HashSet<Entity> = listed.iter().map(|(entity, _)| *entity).collect();

    let mut order: HashMap<Entity, Vec<Entity>> = HashMap::new();
    for &(scene_entity, scene_parent) in &listed {
        let parent = scene_parent
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
                let item = spawn_item(&mut cmd, &theme, scene_entity, parent);
                state.items.insert(scene_entity, item);
                item
            }
        };
        order.entry(parent).or_default().push(item);
    }
    for (parent, items) in &order {
        if let Some(mut children) = children.get_entity(*parent) {
            follow_order(&mut children, items);
        }
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
    despawn_outermost(&gone, &parents, &mut cmd);

    if let Some(scene_entity) = state.pending_scene_reveal {
        if !wanted.contains(&scene_entity) {
            state.pending_scene_reveal = None;
        } else if let Some(&item) = state.items.get(&scene_entity)
            && placed.contains_entity(item)
        {
            cmd.entity(item).trigger(UIReveal);
            state.pending_scene_reveal = None;
        }
    }
}

pub(crate) fn reconcile_content(
    selection: EditWorld<Res<Selection>>,
    names: EditWorld<Query<&Name>>,
    roots: EditWorld<Query<&SceneRoot>>,
    scene_children: EditWorld<Query<&Children>>,
    kinds: EditWorld<Query<(Option<&Camera>, Option<&Light>, Option<&MeshComponent>)>>,
    active: Res<ActiveEditor>,
    documents: Query<&EditorDocument>,
    items: Query<(Entity, &HierarchyItem, Option<&UISelected>)>,
    texts: Query<&mut UIText>,
    marks: Query<(&mut UINode, &mut UIMaterial)>,
    theme: Res<UITheme>,
    mut cmd: CommandQueue,
) {
    let address = active
        .0
        .and_then(|document| documents.get_entity(document))
        .and_then(|document| document.current.as_ref())
        .map(|asset| asset.address.as_str())
        .unwrap_or_default();
    for (entity, item, selected) in items.iter() {
        let scene_entity = item.scene_entity;
        let is_root = roots.get_entity(scene_entity).is_some();
        let count = scene_children
            .get_entity(scene_entity)
            .map_or(0, |children| children.iter().len());
        let kind = kinds.get_entity(scene_entity);
        let mark = match kind {
            _ if is_root => Mark::Group,
            Some((Some(_), _, _)) => Mark::Camera,
            Some((_, Some(_), _)) => Mark::Light,
            Some((_, _, Some(_))) => Mark::Mesh,
            _ if count > 0 => Mark::Group,
            _ => Mark::Plain,
        };
        let label = if is_root {
            address.to_string()
        } else {
            names
                .get_entity(scene_entity)
                .filter(|name| !name.as_str().is_empty())
                .map(|name| name.as_str().to_string())
                .unwrap_or_else(|| format!("Entity {}", scene_entity.index()))
        };
        let count = if count > 0 {
            count.to_string()
        } else {
            String::new()
        };
        for (text, value) in [(item.label, label), (item.count, count)] {
            if let Some(mut text) = texts.get_entity(text)
                && text.text != value
            {
                text.text = value;
            }
        }
        let wanted = selection.entity() == Some(scene_entity);
        if let Some((mut node, mut material)) = marks.get_entity(item.mark) {
            marks::apply(mark, &theme, wanted, &mut node, &mut material);
        }
        if wanted && selected.is_none() {
            cmd.insert(UISelected, entity);
        } else if !wanted && selected.is_some() {
            cmd.remove::<UISelected>(entity);
        }
    }
}

fn filter_tree(
    mut events: EventReader<UITextInputChanged>,
    filters: Query<&Filter>,
    mut state: ResMut<HierarchyState>,
    views: Query<&mut UIScrollArea, With<TreeView>>,
) {
    for event in events.read() {
        if filters.get_entity(event.entity).is_some() {
            state.filter = event.value.clone();
            for mut area in views.iter() {
                area.offset = 0.0;
            }
        }
    }
}

fn filter_items(
    state: Res<HierarchyState>,
    names: EditWorld<Query<&Name>>,
    items: Query<(Entity, &HierarchyItem, Option<&UIHidden>)>,
    views: Query<&mut UITree, With<TreeView>>,
    mut cmd: CommandQueue,
) {
    let filter = state.filter.to_lowercase();
    let filtering = !filter.is_empty();
    for mut tree in views.iter() {
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
    labels: Query<(&Label, &mut UIText)>,
) {
    for (label, mut text) in labels.iter() {
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
        IntoSystem, System, World,
        events::event_channel::EventChannel,
        query::filter::{Changed, Or, With},
    };
    use concerto_foundation::transform::Transform;
    use concerto_ui::{
        items::UIItem,
        node::{UIBox, UILayout},
        tree::update_trees_system,
    };
    use glam::Vec2;

    struct Fixture {
        scene: SceneFixture,
        document: Entity,
        root: Entity,
        node: Entity,
    }

    static RECORDS: std::sync::Mutex<Vec<(std::thread::ThreadId, String)>> =
        std::sync::Mutex::new(Vec::new());

    struct Capture;

    impl log::Log for Capture {
        fn enabled(&self, metadata: &log::Metadata) -> bool {
            metadata.level() <= log::Level::Warn
        }

        fn log(&self, record: &log::Record) {
            if record.level() <= log::Level::Warn {
                RECORDS
                    .lock()
                    .unwrap()
                    .push((std::thread::current().id(), record.args().to_string()));
            }
        }

        fn flush(&self) {}
    }

    fn invalid_despawns() -> usize {
        static INSTALL: std::sync::Once = std::sync::Once::new();
        INSTALL.call_once(|| {
            log::set_logger(&Capture).unwrap();
            log::set_max_level(log::LevelFilter::Warn);
        });
        let me = std::thread::current().id();
        RECORDS
            .lock()
            .unwrap()
            .iter()
            .filter(|(thread, message)| {
                *thread == me && message.starts_with("Attempted to despawn invalid entity")
            })
            .count()
    }

    fn run<M>(world: &mut World, system: impl IntoSystem<(), M>) {
        let mut system = system.into_system();
        system.initialize(world);
        system.run_and_apply((), world);
    }

    fn place(world: &mut World) {
        let mut system = update_trees_system();
        system.initialize(world);
        system.run_and_apply((), world);
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
        let mut fixture = Fixture {
            scene,
            document,
            root,
            node,
        };
        fixture.refresh();
        fixture
    }

    impl Fixture {
        fn refresh(&mut self) {
            run(&mut self.scene.world, reconcile_structure);
            run(&mut self.scene.world, filter_items);
            place(&mut self.scene.world);
            run(&mut self.scene.world, reconcile_content);
            run(&mut self.scene.world, sync_row_menu_items);
            run(&mut self.scene.world, show_header);
        }
        fn state(&self) -> &HierarchyState {
            self.scene.world.get_resource::<HierarchyState>().unwrap()
        }
        fn state_mut(&mut self) -> &mut HierarchyState {
            self.scene
                .world
                .get_resource_mut::<HierarchyState>()
                .unwrap()
        }
        fn item_of(&self, scene_entity: Entity) -> Entity {
            *self
                .state()
                .items
                .get(&scene_entity)
                .expect("the entity has an item")
        }
        fn hierarchy_item(&self, scene_entity: Entity) -> &HierarchyItem {
            self.scene
                .world
                .get_component_for_entity::<HierarchyItem>(self.item_of(scene_entity))
                .unwrap()
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
        fn shown_depth(&self, scene_entity: Entity) -> Option<usize> {
            self.state()
                .items
                .get(&scene_entity)
                .and_then(|item| self.scene.world.get_component_for_entity::<UIItem>(*item))
                .filter(|item| item.is_shown())
                .map(UIItem::depth)
        }
        fn ui_selected(&self, scene_entity: Entity) -> bool {
            self.scene
                .world
                .get_component_for_entity::<UISelected>(self.item_of(scene_entity))
                .is_some()
        }
        fn hidden(&self, scene_entity: Entity) -> bool {
            self.scene
                .world
                .get_component_for_entity::<UIHidden>(self.item_of(scene_entity))
                .is_some()
        }
        fn text_of(&self, entity: Entity) -> String {
            self.scene
                .world
                .get_component_for_entity::<UIText>(entity)
                .unwrap()
                .text
                .clone()
        }
        fn label_of(&self, scene_entity: Entity) -> String {
            self.text_of(self.hierarchy_item(scene_entity).label)
        }
        fn count_of(&self, scene_entity: Entity) -> String {
            self.text_of(self.hierarchy_item(scene_entity).count)
        }
        fn mark_width(&self, scene_entity: Entity) -> UIValue {
            self.scene
                .world
                .get_component_for_entity::<UINode>(self.hierarchy_item(scene_entity).mark)
                .unwrap()
                .width
        }
        fn mark_border(&self, scene_entity: Entity) -> concerto_color::LinearRgba {
            self.scene
                .world
                .get_component_for_entity::<UIMaterial>(self.hierarchy_item(scene_entity).mark)
                .unwrap()
                .border_color
        }
        fn mark_fill(&self, scene_entity: Entity) -> concerto_color::LinearRgba {
            self.scene
                .world
                .get_component_for_entity::<UIMaterial>(self.hierarchy_item(scene_entity).mark)
                .unwrap()
                .color
        }
        fn parent_item(&self, item: Entity) -> Option<Entity> {
            self.scene
                .world
                .get_component_for_entity::<ChildOf>(item)
                .map(ChildOf::parent)
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
        fn tree(&mut self) -> Entity {
            self.scene
                .world
                .query::<Entity, With<TreeView>>()
                .iter(&mut self.scene.world)
                .next()
                .unwrap()
        }
        fn title(&mut self) -> String {
            let title = self
                .scene
                .world
                .query::<(Entity, &Label), ()>()
                .iter(&mut self.scene.world)
                .find(|(_, label)| matches!(label, Label::Title))
                .unwrap()
                .0;
            self.text_of(title)
        }
        fn item_entities(&mut self) -> usize {
            self.scene
                .world
                .query::<Entity, With<HierarchyItem>>()
                .iter(&mut self.scene.world)
                .count()
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
        fn item(&mut self, wanted: RowMenuItem) -> Entity {
            self.scene
                .world
                .query::<(Entity, &RowMenuItem), ()>()
                .iter(&mut self.scene.world)
                .find(|(_, item)| **item == wanted)
                .unwrap()
                .0
        }
        fn menu(&mut self) -> (Option<Entity>, bool, UIAnchorTarget) {
            let (menu, panel) = self
                .scene
                .world
                .query::<(&RowMenu, &UIAnchoredPanel), ()>()
                .iter(&mut self.scene.world)
                .next()
                .unwrap();
            (menu.scene_target, panel.open, panel.target)
        }
        fn add_button(&mut self) -> Entity {
            self.scene
                .world
                .query::<Entity, With<AddEntityButton>>()
                .iter(&mut self.scene.world)
                .next()
                .unwrap()
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
        fn disabled(&self, entity: Entity) -> bool {
            self.scene
                .world
                .get_component_for_entity::<UIDisabled>(entity)
                .is_some()
        }
        fn fire(&mut self, action: impl concerto_window::input::actions::ActionLabel) {
            let channel = self
                .scene
                .world
                .get_resource_mut::<EventChannel<ActionFired>>()
                .unwrap();
            channel.update();
            channel.update();
            channel.push_event(ActionFired {
                action: action.intern(),
            });
            run(&mut self.scene.world, entity_shortcuts);
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
    fn right_clicking_a_row_selects_it_and_opens_the_menu_at_the_pointer() {
        let mut f = fixture();
        f.expand(f.root);
        let item = f.item_of(f.node);
        f.click(item, MouseButton::Right);
        assert_eq!(f.selected(), Some(f.node));
        let (target, open, anchor) = f.menu();
        assert_eq!(target, Some(f.node));
        assert!(open);
        assert!(matches!(
            anchor,
            UIAnchorTarget::Point { position } if position == Vec2::new(40.0, 60.0)
        ));
        for item in [
            RowMenuItem::AddChild,
            RowMenuItem::Duplicate,
            RowMenuItem::Rename,
            RowMenuItem::Delete,
        ] {
            let entity = f.item(item);
            assert!(!f.disabled(entity), "{item:?}");
        }
    }

    #[test]
    fn menu_items_apply_their_edit_to_the_row_and_close_the_menu() {
        let mut f = fixture();
        f.expand(f.root);

        let item = f.item_of(f.node);
        f.click(item, MouseButton::Right);
        let add = f.item(RowMenuItem::AddChild);
        f.click(add, MouseButton::Left);
        assert_eq!(f.children(f.node).len(), 1);
        assert!(!f.menu().1);

        let item = f.item_of(f.node);
        f.click(item, MouseButton::Right);
        let duplicate = f.item(RowMenuItem::Duplicate);
        f.click(duplicate, MouseButton::Left);
        assert_eq!(f.children(f.root).len(), 2);
        let copy = f.selected().unwrap();
        assert_ne!(copy, f.node);

        let item = f.item_of(copy);
        f.click(item, MouseButton::Right);
        let rename = f.item(RowMenuItem::Rename);
        f.click(rename, MouseButton::Left);
        assert!(f.rename_requested());
        assert!(f.valid(copy));

        let item = f.item_of(copy);
        f.click(item, MouseButton::Right);
        let delete = f.item(RowMenuItem::Delete);
        f.click(delete, MouseButton::Left);
        assert!(!f.valid(copy));
        assert_eq!(f.children(f.root), [f.node]);
    }

    #[test]
    fn a_closed_menu_does_nothing_when_an_item_is_clicked() {
        let mut f = fixture();
        let delete = f.item(RowMenuItem::Delete);
        f.click(delete, MouseButton::Left);
        assert!(f.valid(f.node));
    }

    #[test]
    fn the_scene_root_only_offers_add_child() {
        let mut f = fixture();
        let item = f.item_of(f.root);
        f.click(item, MouseButton::Right);
        for (item, disabled) in [
            (RowMenuItem::AddChild, false),
            (RowMenuItem::Duplicate, true),
            (RowMenuItem::Rename, true),
            (RowMenuItem::Delete, true),
        ] {
            let entity = f.item(item);
            assert_eq!(f.disabled(entity), disabled, "{item:?}");
        }
    }

    #[test]
    fn the_add_button_targets_the_selection_or_the_root_and_needs_a_scene() {
        let mut f = fixture();
        let button = f.add_button();
        assert!(!f.disabled(button));
        f.select(f.root);
        f.click(button, MouseButton::Left);
        assert_eq!(f.children(f.root).len(), 2);

        f.select(f.node);
        f.click(button, MouseButton::Left);
        assert_eq!(f.children(f.node).len(), 1);

        f.scene
            .world
            .remove_component::<crate::asset_editor::Unsaved>(f.document);
        f.scene.close(f.document);
        f.scene.frame();
        f.scene.frame();
        assert!(f.disabled(button));
    }

    #[test]
    fn the_add_button_is_disabled_until_a_scene_is_open() {
        let mut scene = SceneFixture::new();
        let button = scene
            .world
            .query::<Entity, With<AddEntityButton>>()
            .iter(&mut scene.world)
            .next()
            .unwrap();
        let disabled = |scene: &SceneFixture| {
            scene
                .world
                .get_component_for_entity::<UIDisabled>(button)
                .is_some()
        };
        assert!(disabled(&scene));

        scene.open();

        assert!(!disabled(&scene));
    }

    #[test]
    fn with_nothing_selected_the_add_button_adds_to_the_scene_root() {
        let mut f = fixture();
        let button = f.add_button();
        f.scene.scene_world_mut(f.document).trigger(Select(None));

        f.click(button, MouseButton::Left);

        assert_eq!(f.children(f.root).len(), 2);
    }

    #[test]
    fn shortcuts_act_on_the_selection_and_do_nothing_without_one() {
        let mut f = fixture();
        f.scene.scene_world_mut(f.document).trigger(Select(None));
        f.fire(DeleteEntity);
        assert!(f.valid(f.node));

        f.select(f.node);
        f.fire(RenameEntity);
        assert!(f.rename_requested());

        f.fire(DuplicateEntity);
        assert_eq!(f.children(f.root).len(), 2);

        f.select(f.node);
        f.fire(DeleteEntity);
        assert!(!f.valid(f.node));
        assert_eq!(f.children(f.root).len(), 1);
    }

    #[test]
    fn focus_on_an_item_of_the_tree_pushes_the_tree_context() {
        let mut f = fixture();
        f.scene.world.insert_resource(ActionMap::default());
        let item = f.item_of(f.root);
        **f.scene.world.get_resource_mut::<FocusedWidget>().unwrap() = Some(item);
        run(&mut f.scene.world, sync_tree_context);
        assert!(
            f.scene
                .world
                .get_resource::<ActionMap>()
                .unwrap()
                .is_active(TreeContext)
        );

        let field = f
            .scene
            .world
            .query::<Entity, With<Filter>>()
            .iter(&mut f.scene.world)
            .next()
            .unwrap();
        **f.scene.world.get_resource_mut::<FocusedWidget>().unwrap() = Some(field);
        run(&mut f.scene.world, sync_tree_context);
        assert!(
            !f.scene
                .world
                .get_resource::<ActionMap>()
                .unwrap()
                .is_active(TreeContext)
        );
    }

    #[test]
    fn an_edit_that_selects_an_entity_expands_its_ancestors_and_reveals_its_row() {
        let mut f = fixture();
        assert_eq!(f.shown_depth(f.node), None);
        f.scene.signal(EntityEdit::Create {
            parent: Some(f.node),
        });
        let created = f.selected().unwrap();

        f.refresh();
        f.refresh();

        assert!(f.expanded(f.root));
        assert!(f.expanded(f.node));
        assert_eq!(f.shown_depth(created), Some(2));
        assert!(f.ui_selected(created));
        assert_eq!(f.state().pending_scene_reveal, None);
    }

    #[test]
    fn a_reveal_for_an_entity_created_this_frame_lands_once_its_item_exists() {
        let mut f = fixture();
        let created = f.spawn_under(f.node, "late");
        f.state_mut().reveal_entity(created);

        run(&mut f.scene.world, reconcile_structure);

        assert!(f.state().items.contains_key(&created));
        assert_eq!(
            f.state().pending_scene_reveal,
            Some(created),
            "the item is not placed yet, so the reveal waits"
        );
        assert!(!f.expanded(f.root));

        place(&mut f.scene.world);
        run(&mut f.scene.world, reconcile_structure);
        place(&mut f.scene.world);

        assert_eq!(f.state().pending_scene_reveal, None);
        assert!(f.expanded(f.root));
        assert!(f.expanded(f.node));
        assert_eq!(f.shown_depth(created), Some(2));
    }

    #[test]
    fn reparenting_moves_the_item_and_keeps_its_expansion() {
        let mut f = fixture();
        let second = f.spawn_under(f.root, "second");
        let leaf = f.spawn_under(f.node, "leaf");
        f.refresh();
        f.expand(f.root);
        f.expand(f.node);
        let item = f.item_of(f.node);

        f.scene
            .scene_world_mut(f.document)
            .entity_mut(second)
            .add_child(f.node);
        f.refresh();

        assert_eq!(f.item_of(f.node), item);
        assert_eq!(f.parent_item(item), Some(f.item_of(second)));
        assert!(f.expanded(f.node));
        assert_eq!(f.item_children(f.item_of(second)), [f.node]);
        assert_eq!(f.item_children(f.item_of(f.root)), [second]);
        assert_eq!(f.item_children(item), [leaf]);
    }

    #[test]
    fn the_tree_follows_spawns_despawns_renames_and_reorders() {
        let mut f = fixture();
        let a = f.spawn_under(f.root, "a");
        let b = f.spawn_under(f.root, "b");
        f.refresh();
        let root_item = f.item_of(f.root);
        assert_eq!(f.item_children(root_item), [f.node, a, b]);
        assert_eq!(f.count_of(f.root), "3");

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

        let gone = f.item_of(a);
        f.scene.scene_world_mut(f.document).despawn(a);
        f.refresh();
        assert_eq!(f.item_children(root_item), [b, f.node]);
        assert!(!f.scene.world.entity_is_valid(gone));
        assert!(!f.state().items.contains_key(&a));
        assert_eq!(f.count_of(f.root), "2");
    }

    #[test]
    fn losing_the_edit_target_clears_every_item() {
        let mut f = fixture();
        let child = f.spawn_under(f.node, "child");
        f.spawn_under(child, "grandchild");
        f.refresh();
        f.expand(f.root);
        let items: Vec<Entity> = f.state().items.values().copied().collect();
        assert_eq!(items.len(), 4);
        let before = invalid_despawns();

        f.scene.activate(None);

        assert_eq!(
            invalid_despawns(),
            before,
            "each item is despawned once, through its outermost item"
        );

        assert!(f.state().items.is_empty());
        assert!(
            items
                .iter()
                .all(|item| !f.scene.world.entity_is_valid(*item))
        );
        assert_eq!(f.item_entities(), 0);
    }

    #[test]
    fn deleting_an_entity_with_children_despawns_each_item_once() {
        let mut f = fixture();
        let child = f.spawn_under(f.node, "child");
        let grandchild = f.spawn_under(child, "grandchild");
        f.refresh();
        let items = [f.item_of(f.node), f.item_of(child), f.item_of(grandchild)];
        let before = invalid_despawns();

        f.scene.scene_world_mut(f.document).despawn(f.node);
        f.refresh();

        assert!(
            items
                .iter()
                .all(|item| !f.scene.world.entity_is_valid(*item))
        );
        for scene_entity in [f.node, child, grandchild] {
            assert!(!f.state().items.contains_key(&scene_entity));
        }
        assert_eq!(f.item_children(f.item_of(f.root)), []);
        assert_eq!(invalid_despawns(), before);
    }

    #[test]
    fn an_external_selection_draws_the_mark_selected_in_the_same_frame() {
        let mut f = fixture();
        let theme = UITheme::default();
        assert!(!f.ui_selected(f.node));
        f.select(f.node);

        run(&mut f.scene.world, reconcile_content);

        assert!(f.ui_selected(f.node));
        assert_eq!(
            f.mark_fill(f.node),
            Mark::Plain.shape(&theme, true).fill.to_linear()
        );
    }

    #[test]
    fn the_title_counts_every_listed_entity() {
        let mut f = fixture();
        let deep = f.spawn_under(f.node, "deep");
        f.spawn_under(f.root, "other");
        f.refresh();
        assert_eq!(f.shown_depth(deep), None, "the root is collapsed");
        assert_eq!(f.title(), "WORLD · 4 entities");

        f.state_mut().filter = "nothing matches this".into();
        f.refresh();
        assert!(f.hidden(deep));
        assert_eq!(f.title(), "WORLD · 4 entities");
    }

    #[test]
    fn the_filter_hides_what_does_not_match_and_clearing_it_restores_the_tree() {
        let mut f = fixture();
        let deep = f.spawn_under(f.node, "Deep");
        let other = f.spawn_under(f.root, "other");
        f.refresh();

        f.state_mut().filter = "deep".into();
        f.refresh();
        let tree = f.tree();
        assert!(
            f.scene
                .world
                .get_component_for_entity::<UITree>(tree)
                .unwrap()
                .filtering
        );
        assert!(!f.hidden(deep));
        assert!(f.hidden(other));
        assert!(f.hidden(f.node));
        assert_eq!(f.shown_depth(deep), Some(2));
        assert_eq!(f.shown_depth(other), None);

        f.state_mut().filter.clear();
        f.refresh();
        assert!(
            !f.scene
                .world
                .get_component_for_entity::<UITree>(tree)
                .unwrap()
                .filtering
        );
        assert!(!f.hidden(other) && !f.hidden(f.node) && !f.hidden(deep));
        assert_eq!(f.shown_depth(deep), None);
        assert!(!f.expanded(f.root));
    }

    #[test]
    fn an_idle_frame_leaves_marks_unchanged() {
        let mut f = fixture();
        f.expand(f.root);
        f.select(f.node);
        f.refresh();
        f.scene.world.tick();

        run(&mut f.scene.world, reconcile_content);

        let marks: HashSet<Entity> = f
            .scene
            .world
            .query::<&HierarchyItem, ()>()
            .iter(&mut f.scene.world)
            .map(|item| item.mark)
            .collect();
        assert_eq!(marks.len(), 2);
        let changed: Vec<Entity> = f
            .scene
            .world
            .query::<Entity, Or<(Changed<UINode>, Changed<UIMaterial>)>>()
            .iter(&mut f.scene.world)
            .filter(|entity| marks.contains(entity))
            .collect();
        assert!(changed.is_empty(), "{changed:?}");
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
        f.refresh();
        let theme = UITheme::default();
        assert_eq!(
            f.mark_border(f.root),
            Mark::Group.shape(&theme, true).border.to_linear()
        );

        f.scene.scene_world_mut(f.document).trigger(Select(None));
        f.refresh();
        assert!(!f.ui_selected(f.root));
        f.refresh();
        assert_eq!(
            f.mark_border(f.root),
            Mark::Group.shape(&theme, false).border.to_linear()
        );
    }

    #[test]
    fn rows_are_drawn_from_the_scene_and_emptied_when_its_tab_closes() {
        let mut f = fixture();
        f.expand(f.root);
        f.select(f.node);
        f.refresh();
        assert_eq!(f.title(), "WORLD · 2 entities");
        assert_eq!(
            f.label_of(f.root),
            "first.gasset",
            "the root is labelled with the address its document holds"
        );
        assert_eq!(f.count_of(f.root), "1");
        assert_eq!(f.label_of(f.node), "first");
        assert_eq!(f.count_of(f.node), "");
        assert!(f.ui_selected(f.node) && !f.ui_selected(f.root));
        let theme = UITheme::default();
        assert_eq!(
            f.mark_width(f.root),
            UIValue::Px(Mark::Group.shape(&theme, false).size)
        );
        assert_eq!(
            f.mark_width(f.node),
            UIValue::Px(Mark::Plain.shape(&theme, false).size)
        );

        f.scene.close(f.document);
        f.scene.frame();
        f.scene.frame();
        f.refresh();

        assert_eq!(f.title(), "WORLD · 0 entities");
        assert!(f.state().items.is_empty());
        assert_eq!(f.item_entities(), 0);
    }

    #[test]
    fn the_row_menu_forgets_its_entity_when_the_scene_is_replaced() {
        let mut f = fixture();
        f.expand(f.root);
        let item = f.item_of(f.node);
        f.click(item, MouseButton::Right);
        assert_eq!(f.menu().0, Some(f.node));

        f.scene.request_other_scene(f.document);
        f.scene.frames_until_idle();

        assert_eq!(f.menu().0, None);
    }

    #[test]
    fn a_tree_scrolled_down_one_scene_shows_the_next_from_its_top() {
        let mut f = fixture();
        for index in 0..6 {
            f.spawn_under(f.root, &format!("extra {index}"));
        }
        f.refresh();
        f.expand(f.root);
        let rect = UIBox {
            min: Vec2::ZERO,
            size: Vec2::new(200.0, 64.0),
        };
        let tree = f.tree();
        f.scene.world.insert(
            UILayout {
                rect,
                content_rect: rect,
                clip_rect: rect,
                paint_order: 0,
            },
            tree,
        );
        let offset = |f: &Fixture| {
            f.scene
                .world
                .get_component_for_entity::<UIScrollArea>(tree)
                .unwrap()
                .offset
        };
        f.scene
            .world
            .get_component_for_entity_mut::<UIScrollArea>(tree)
            .unwrap()
            .offset = 3.0 * ROW_HEIGHT;
        f.refresh();
        assert_eq!(offset(&f), 3.0 * ROW_HEIGHT);

        f.scene.request_other_scene(f.document);
        f.scene.frames_until_idle();

        assert_eq!(offset(&f), 0.0);
    }

    #[test]
    fn replacing_a_scene_clears_panel_state() {
        let mut f = SceneFixture::new();
        let document = f.open();
        f.frame();
        let state = f.world.get_resource::<HierarchyState>().unwrap();
        let stale = state.items().values().copied().collect::<Vec<_>>();
        assert!(!stale.is_empty());
        f.world
            .get_component_for_entity_mut::<UITreeElement>(stale[0])
            .unwrap()
            .expanded = true;

        f.request_other_scene(document);
        f.frames_until_idle();

        let state = f.world.get_resource::<HierarchyState>().unwrap();
        let hosted = f
            .world
            .get_component_for_entity::<HostedApp>(document)
            .unwrap();
        assert!(!state.items().is_empty());
        for (scene_entity, item) in state.items() {
            assert!(hosted.world_for_test().entity_is_valid(*scene_entity));
            assert!(
                !f.world
                    .get_component_for_entity::<UITreeElement>(*item)
                    .unwrap()
                    .expanded
            );
        }
        assert!(stale.iter().all(|item| !f.world.entity_is_valid(*item)));
    }

    #[test]
    fn closing_the_tab_empties_the_panels() {
        let mut f = SceneFixture::new();
        let document = f.open();
        f.frame();

        f.close(document);
        f.frame();
        f.frame();

        assert!(
            f.world
                .get_resource::<HierarchyState>()
                .unwrap()
                .items()
                .is_empty()
        );
        assert_eq!(
            *f.world.get_resource::<InspectorData>().unwrap(),
            InspectorData::default()
        );
    }

    fn game(app: &mut App) {
        crate::hosted::testing::scene_recipe(app);
        app.add_system(Startup, spawn_character)
            .add_system(Startup, spawn_data);
    }

    #[derive(Component)]
    struct Score;

    fn spawn_data(mut cmd: CommandQueue) {
        cmd.spawn(Score);
        cmd.spawn((Score, (|_: On<UIClick>| {}).into_listener()));
    }

    fn scores(f: &mut Fixture) -> Vec<Entity> {
        let world = f.scene.scene_world_mut(f.document);
        world.query::<Entity, With<Score>>().iter(world).collect()
    }

    fn spawn_character(mut cmd: CommandQueue) {
        cmd.spawn((Name::new("character"), Transform::IDENTITY))
            .add_child((Name::new("camera"), Transform::IDENTITY));
    }

    fn playing_fixture() -> Fixture {
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
            .frames_until("the game never spawned the scene", |scene| {
                let names = scene.scene_names(document);
                ["first", "character"]
                    .iter()
                    .all(|wanted| names.iter().any(|name| name == wanted))
            });
        f.node = f.scene.scene_entity(document, "first");
        f.root = f
            .scene
            .scene_world(document)
            .get_component_for_entity::<ChildOf>(f.node)
            .map(ChildOf::parent)
            .expect("the scene's nodes hang from its spawner");
        f.refresh();
        f
    }

    fn top_level(f: &mut Fixture) -> Vec<Entity> {
        let tree = f.tree();
        f.item_children(tree)
    }

    #[test]
    fn while_playing_the_tree_lists_every_top_level_entity_with_children_nested() {
        let mut f = playing_fixture();
        let character = f.scene.scene_entity(f.document, "character");
        let camera = f.scene.scene_entity(f.document, "camera");

        let mut expected = vec![f.root, character];
        let data = scores(&mut f);
        assert_eq!(data.len(), 2);
        expected.extend(data);
        let mut listed = top_level(&mut f);
        listed.sort_by_key(|entity| (entity.index(), entity.generation()));
        expected.sort_by_key(|entity| (entity.index(), entity.generation()));
        assert_eq!(listed, expected);
        assert_eq!(f.shown_depth(camera), None);

        f.expand(character);
        f.expand(f.root);

        assert_eq!(f.shown_depth(camera), Some(1));
        assert_eq!(f.shown_depth(f.node), Some(1));
    }

    #[test]
    fn top_level_entities_while_playing_sort_by_index() {
        let mut f = playing_fixture();
        let listed = top_level(&mut f);
        assert!(listed.len() >= 4);
        let mut sorted = listed.clone();
        sorted.sort_by_key(|entity| (entity.index(), entity.generation()));
        assert_eq!(listed, sorted);
    }

    #[test]
    fn clicking_a_row_while_playing_selects_it_in_the_play_world_and_the_inspector_shows_it() {
        let mut f = playing_fixture();
        let character = f.scene.scene_entity(f.document, "character");
        let item = f.item_of(character);
        let edit_selection = |f: &Fixture| {
            f.scene
                .parked_world(f.document)
                .get_resource::<Selection>()
                .unwrap()
                .entity()
        };
        let before = edit_selection(&f);

        f.click(item, MouseButton::Left);
        f.scene.frame();

        assert_eq!(f.selected(), Some(character));
        assert_eq!(edit_selection(&f), before);
        assert_eq!(
            f.scene
                .world
                .get_resource::<InspectorData>()
                .unwrap()
                .scene_entity,
            Some(character)
        );
    }

    #[test]
    fn while_editing_only_the_scene_root_is_listed_at_the_top() {
        let mut f = fixture();
        let stray = f
            .scene
            .scene_world_mut(f.document)
            .spawn((Name::new("stray"), Transform::IDENTITY));

        f.refresh();

        assert_eq!(top_level(&mut f), [f.root]);
        assert!(!f.state().items.contains_key(&stray));
    }

    #[test]
    fn while_playing_the_add_button_with_nothing_selected_adds_a_top_level_entity() {
        let mut f = playing_fixture();
        let button = f.add_button();
        assert!(!f.disabled(button));
        f.scene.scene_world_mut(f.document).trigger(Select(None));

        f.click(button, MouseButton::Left);

        let created = f.selected().expect("the new entity is selected");
        assert!(
            f.scene
                .scene_world(f.document)
                .get_component_for_entity::<ChildOf>(created)
                .is_none()
        );
        assert_eq!(f.shown_depth(created), Some(0));
        assert!(!f.scene.unsaved(f.document));
    }

    #[test]
    fn while_playing_a_top_level_row_offers_every_item_and_can_be_deleted() {
        let mut f = playing_fixture();
        let character = f.scene.scene_entity(f.document, "character");
        let item = f.item_of(character);
        f.click(item, MouseButton::Right);
        for item in [
            RowMenuItem::AddChild,
            RowMenuItem::Duplicate,
            RowMenuItem::Rename,
            RowMenuItem::Delete,
        ] {
            let entity = f.item(item);
            assert!(!f.disabled(entity), "{item:?}");
        }

        let delete = f.item(RowMenuItem::Delete);
        f.click(delete, MouseButton::Left);

        assert!(!f.valid(character));
        assert!(top_level(&mut f).contains(&f.root));
        assert!(!top_level(&mut f).contains(&character));
    }

    fn created(f: &mut Fixture) -> (Entity, Option<Entity>) {
        let created = f.selected().expect("the new entity is selected");
        let parent = f
            .scene
            .scene_world(f.document)
            .get_component_for_entity::<ChildOf>(created)
            .map(ChildOf::parent);
        (created, parent)
    }

    #[test]
    fn while_playing_the_add_button_adds_a_child_to_the_selection() {
        let mut f = playing_fixture();
        let character = f.scene.scene_entity(f.document, "character");
        f.select(character);
        let button = f.add_button();

        f.click(button, MouseButton::Left);

        let (created, parent) = created(&mut f);
        assert_ne!(created, character);
        assert_eq!(parent, Some(character));
    }

    #[test]
    fn while_playing_the_add_button_adds_at_the_top_level_when_the_selection_is_gone() {
        let mut f = playing_fixture();
        let character = f.scene.scene_entity(f.document, "character");
        f.select(character);
        f.scene.scene_world_mut(f.document).despawn(character);
        let button = f.add_button();

        f.click(button, MouseButton::Left);

        let (created, parent) = created(&mut f);
        assert_ne!(created, character);
        assert_eq!(parent, None);
        assert_eq!(f.shown_depth(created), Some(0));
        assert_eq!(f.scene.status(f.document), "");
    }

    #[test]
    fn while_the_game_starts_there_is_no_edit_target_and_the_tree_is_empty() {
        let mut f = fixture();
        f.scene
            .scene_world_mut(f.document)
            .spawn((Name::new("stray"), Transform::IDENTITY));
        f.scene.world.trigger(crate::scene::Play(f.document));
        assert_eq!(
            f.scene.play_state(f.document),
            Some(crate::hosted::PlayState::Starting)
        );

        f.refresh();

        assert!(top_level(&mut f).is_empty());
    }
}

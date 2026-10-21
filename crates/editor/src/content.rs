//! The project's asset catalogue, as a docked panel.
use concerto_app::{
    App, Plugin,
    schedule_groups::{LateUpdate, Startup},
};
use concerto_color::Color;
use concerto_ecs::{
    Component, Entity, IntoSystemConfig, Query, Res, ResMut, Resource, World,
    command::CommandQueue,
    entity::hierarchy::{ChildOf, Children},
    events::event_reader::EventReader,
    query::{change_detection::DetectChanges, filter::With},
    signal::{On, Signal, listener::IntoListener},
};
use concerto_ui::{
    elements::prelude::*,
    interaction::UIClick,
    items::{UIHidden, UIItem, UIItemActivated, UIItemSelected, UISelected},
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
use std::collections::{BTreeMap, HashMap, HashSet};
use taffy::FlexDirection;

use crate::dock::{DockedApp, PanelDescriptor, PanelRegistry, Region};
use crate::marks::{self, Mark};
use crate::project::{AssetEntry, EditorCommand, EditorCommands, ProjectState};
use crate::tree_items::{despawn_outermost, follow_order};
use concerto_foundation::assets::AssetId;

pub const PANEL_ID: &str = "concerto.curiosities";

const ROW_HEIGHT: f32 = 28.0;
const SEARCH_HEIGHT: f32 = 30.0;

const KINDS: [(&str, Option<&str>); 5] = [
    ("all", None),
    ("scene", Some("Scene")),
    ("mesh", Some("Mesh")),
    ("tex", Some("Texture")),
    ("mat", Some("Material")),
];

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum ContentKey {
    Folder(String),
    Asset(AssetId),
}

#[derive(Resource, Default)]
pub struct ContentState {
    /// Catalogue selection is independent of the open editor's entity selection.
    pub selected: Option<AssetId>,
    filter: String,
    kind: usize,
    items: HashMap<ContentKey, Entity>,
}

#[derive(Component)]
struct ContentItem {
    key: ContentKey,
    mark: Mark,
    mark_node: Entity,
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

struct OrderItems(Vec<(Entity, Vec<Entity>)>);

impl Signal for OrderItems {}

pub struct ContentPlugin;

impl Plugin for ContentPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(ContentState::default());
        app.add_panel(PanelDescriptor {
            id: PANEL_ID,
            title: "Curiosities",
            region: Region::Rail,
        });
        install_listeners(app.main_mut().world_mut());
        app.add_system(Startup, build_panel)
            .add_system(
                LateUpdate,
                update_filter.after(UiSet::Widgets).before(UiSet::Project),
            )
            .add_system(
                LateUpdate,
                reconcile_content_tree
                    .after(update_filter)
                    .before(UiSet::Project),
            )
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
    world.add_listener(order_items);
}

fn visible_assets<'a>(project: &'a ProjectState, state: &ContentState) -> Vec<&'a AssetEntry> {
    let kind = KINDS[state.kind].1;
    project
        .project
        .iter()
        .flat_map(|project| project.filtered_assets(&state.filter, None, kind))
        .collect()
}

fn build_panel(mut cmd: CommandQueue, registry: Res<PanelRegistry>, theme: Res<UITheme>) {
    let Some(body) = registry.body(PANEL_ID) else {
        return;
    };
    spawn_panel(&mut cmd, body, &theme);
}

fn spawn_panel(cmd: &mut CommandQueue, body: Entity, theme: &UITheme) {
    cmd.entity(body).add_child_with(
        UINode::default()
            .with_flex_grow(1.0)
            .with_flex_direction(FlexDirection::Column)
            .with_padding(UIRect::all(10.0))
            .clipped(),
        |mut panel| {
            panel.add_child_with(theme.row().gap(6.0).fixed(), |mut bar| {
                bar.add_child_with(
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
                                    .text_field("Find imported assets…")
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
            });

            panel.add_child_with(
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
            );

            panel.add_child((
                UINode::default()
                    .with_height(UIValue::Px(0.0))
                    .with_flex_grow(1.0),
                UITree {
                    row_height: Some(ROW_HEIGHT),
                    ..Default::default()
                },
                ContentView,
            ));

            panel.add_child((
                theme
                    .label("")
                    .height(UIValue::Px(26.0))
                    .fixed()
                    .padding(UIRect::axes(4.0, 10.0)),
                CountLabel,
            ));
        },
    );
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
        if let Some(&item) = state
            .selected
            .and_then(|id| state.items.get(&ContentKey::Asset(id)))
        {
            cmd.insert(UISelected, item);
        }
        return;
    }
    if let Some(ContentKey::Asset(id)) = items.get_entity(entity).map(|item| &item.key) {
        state.selected = Some(*id);
        if !selected.contains_entity(entity) {
            cmd.insert(UISelected, entity);
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

fn asset_mark(kind: &str) -> Mark {
    match kind {
        "Scene" => Mark::Group,
        "Mesh" => Mark::Mesh,
        "Texture" => Mark::Texture,
        "Material" => Mark::Material,
        _ => Mark::Plain,
    }
}

#[derive(Default)]
struct Folder<'a> {
    folders: BTreeMap<(String, &'a str), Folder<'a>>,
    assets: Vec<&'a AssetEntry>,
}

struct Listed<'a> {
    key: ContentKey,
    parent: Option<ContentKey>,
    name: &'a str,
    mark: Mark,
}

fn inside_root<'a>(folder: &'a str, content_root: &str) -> &'a str {
    match folder.strip_prefix(content_root) {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => rest,
        _ => folder,
    }
}

fn folder_tree<'a>(assets: &'a [AssetEntry], content_root: &str) -> Folder<'a> {
    let mut root = Folder::default();
    for asset in assets {
        let mut folder = &mut root;
        for part in inside_root(&asset.folder, content_root)
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

fn list_folder<'a>(folder: &Folder<'a>, path: Option<&str>, listed: &mut Vec<Listed<'a>>) {
    let parent = path.map(|path| ContentKey::Folder(path.to_string()));
    for (&(_, name), child) in &folder.folders {
        let child_path = match path {
            Some(path) => format!("{path}/{name}"),
            None => name.to_string(),
        };
        listed.push(Listed {
            key: ContentKey::Folder(child_path.clone()),
            parent: parent.clone(),
            name,
            mark: Mark::Folder,
        });
        list_folder(child, Some(&child_path), listed);
    }
    let mut assets = folder.assets.clone();
    assets.sort_by(|a, b| {
        (
            a.display_name.to_lowercase(),
            &a.display_name,
            &a.kind,
            &a.address,
        )
            .cmp(&(
                b.display_name.to_lowercase(),
                &b.display_name,
                &b.kind,
                &b.address,
            ))
    });
    for asset in assets {
        listed.push(Listed {
            key: ContentKey::Asset(asset.id),
            parent: parent.clone(),
            name: &asset.display_name,
            mark: asset_mark(&asset.kind),
        });
    }
}

fn spawn_item(cmd: &mut CommandQueue, theme: &UITheme, entry: &Listed, parent: Entity) -> Entity {
    let mark_node = cmd.spawn(marks::shaped(entry.mark, theme)).entity();
    let label = cmd
        .spawn((
            UINode::default()
                .with_flex_grow(1.0)
                .with_flex_shrink(1.0)
                .with_padding(UIRect::axes(0.0, 8.0)),
            theme.text(entry.name).single_line(),
        ))
        .entity();
    let item = cmd
        .spawn((
            theme.canvas().fill(Color::TRANSPARENT).radius_sm(),
            UITreeElement::default(),
            ContentItem {
                key: entry.key.clone(),
                mark: entry.mark,
                mark_node,
                label,
            },
            select_item.into_listener(),
            open_item.into_listener(),
            ChildOf::new(parent),
        ))
        .entity();
    for content in [mark_node, label] {
        cmd.add_child(item, content);
    }
    item
}

fn reconcile_content_tree(
    project: Res<ProjectState>,
    mut state: ResMut<ContentState>,
    views: Query<(Entity, &mut UITree), With<ContentView>>,
    items: Query<(&mut ContentItem, Option<&UIHidden>)>,
    parents: Query<&ChildOf>,
    texts: Query<&mut UIText>,
    counts: Query<Entity, With<CountLabel>>,
    theme: Res<UITheme>,
    mut cmd: CommandQueue,
) {
    if !project.has_changed() && !state.has_changed() {
        return;
    }
    let Some((view, mut tree)) = views.iter().next() else {
        return;
    };
    let filtering = !state.filter.is_empty() || state.kind != 0;
    if tree.filtering != filtering {
        tree.filtering = filtering;
    }
    let matching: HashSet<AssetId> = visible_assets(&project, &state)
        .iter()
        .map(|asset| asset.id)
        .collect();
    let mut listed = Vec::new();
    if let Some(project) = project.project.as_ref() {
        list_folder(
            &folder_tree(&project.assets, &project.content_root),
            None,
            &mut listed,
        );
    }

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
            };
        let item = match previous.remove(&entry.key) {
            Some(item) => {
                if parents.get_entity(item).map(ChildOf::parent) != Some(parent) {
                    cmd.add_child(parent, item);
                }
                if let Some((mut content, hidden)) = items.get_entity(item) {
                    if content.mark != entry.mark {
                        content.mark = entry.mark;
                    }
                    if let Some(mut text) = texts.get_entity(content.label)
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
                let item = spawn_item(&mut cmd, &theme, entry, parent);
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
    cmd.trigger(OrderItems(order));

    let count = if matching.is_empty() {
        String::new()
    } else {
        format!("{} assets", matching.len())
    };
    for label in counts.iter() {
        if let Some(mut text) = texts.get_entity(label)
            && text.text != count
        {
            text.text = count.clone();
        }
    }
}

fn order_items(on: On<OrderItems>, children: Query<&mut Children>) {
    for (parent, items) in &on.signal().0 {
        if let Some(mut children) = children.get_entity(*parent) {
            follow_order(&mut children, items);
        }
    }
}

fn sync_content_items(
    items: Query<(&ContentItem, Option<&UISelected>)>,
    marks: Query<(&mut UINode, &mut UIMaterial)>,
    theme: Res<UITheme>,
) {
    for (item, selected) in items.iter() {
        if let Some((mut node, mut material)) = marks.get_entity(item.mark_node) {
            marks::apply(
                item.mark,
                &theme,
                selected.is_some(),
                &mut node,
                &mut material,
            );
        }
    }
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
    use crate::project::Project;
    use concerto_ecs::{
        IntoSystem, System, World,
        entity::hierarchy::{ChildOf, Children},
        events::event_channel::EventChannel,
        query::filter::{Changed, Or, With},
    };
    use concerto_foundation::assets::content::AssetRegistry;
    use concerto_ui::{
        focus::FocusedWidget, interaction::UIInteractionStyle, items::UIItem,
        tree::update_trees_system,
    };
    use glam::Vec2;

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

    fn build(mut cmd: CommandQueue, theme: Res<UITheme>) {
        let body = cmd.spawn(UINode::default()).entity();
        spawn_panel(&mut cmd, body, &theme);
    }

    fn entry(address: &str, kind: &str) -> AssetEntry {
        AssetEntry::from_address(AssetId::new(), address, kind.into(), None)
    }

    struct Fixture {
        world: World,
    }

    fn fixture(assets: &[(&str, &str)]) -> Fixture {
        fixture_in("content", assets)
    }

    fn fixture_in(content_root: &str, assets: &[(&str, &str)]) -> Fixture {
        let mut world = World::new();
        world.register_component::<ChildOf>();
        world.register_component::<Children>();
        world.insert_resource(UITheme::default());
        world.insert_resource(FocusedWidget::default());
        world.insert_resource(EditorCommands::default());
        world.insert_resource(ContentState::default());
        world.insert_resource(EventChannel::<UITextInputChanged>::new());
        let mut project = ProjectState::default();
        project.project = Some(Project {
            root: Default::default(),
            assets: assets
                .iter()
                .map(|(address, kind)| entry(address, kind))
                .collect(),
            registry: AssetRegistry::default(),
            content_root: content_root.into(),
        });
        world.insert_resource(project);
        install_listeners(&mut world);
        concerto_ui::items::install_listeners(&mut world);
        concerto_ui::tree::install_listeners(&mut world);
        run(&mut world, build);
        let mut fixture = Fixture { world };
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

    impl Fixture {
        fn refresh(&mut self) {
            run(&mut self.world, update_filter);
            run(&mut self.world, reconcile_content_tree);
            place(&mut self.world);
            run(&mut self.world, sync_content_items);
            self.world.tick();
        }

        fn state(&self) -> &ContentState {
            self.world.get_resource::<ContentState>().unwrap()
        }

        fn asset(&self, address: &str) -> AssetId {
            self.world
                .get_resource::<ProjectState>()
                .unwrap()
                .project
                .as_ref()
                .unwrap()
                .assets
                .iter()
                .find(|asset| asset.address == address)
                .unwrap()
                .id
        }

        fn item(&self, key: &ContentKey) -> Entity {
            *self.state().items.get(key).expect("the key has an item")
        }

        fn asset_item(&self, address: &str) -> Entity {
            self.item(&ContentKey::Asset(self.asset(address)))
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

        fn mark_border(&self, item: Entity) -> concerto_color::LinearRgba {
            let mark = self
                .world
                .get_component_for_entity::<ContentItem>(item)
                .unwrap()
                .mark_node;
            self.world
                .get_component_for_entity::<UIMaterial>(mark)
                .unwrap()
                .border_color
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
        let theme = UITheme::default();
        let birch = f.item(&folder("birch"));
        let mark = f
            .world
            .get_component_for_entity::<ContentItem>(birch)
            .unwrap()
            .mark_node;
        assert_eq!(
            f.world
                .get_component_for_entity::<UIMaterial>(mark)
                .unwrap()
                .border_color,
            Mark::Folder.shape(&theme, false).border.to_linear()
        );
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

        let mut project = f.world.remove_resource::<ProjectState>().unwrap();
        let catalogue = project.project.as_mut().unwrap();
        catalogue.insert(entry("content/forest/b.gasset", "Mesh"));
        catalogue.insert(entry("content/forest/_first.gasset", "Mesh"));
        catalogue.insert(entry("content/birch/bark.gasset", "Texture"));
        f.world.insert_resource(project);
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
    fn selecting_an_item_selects_the_asset() {
        let mut f = fixture(&[
            ("content/level.gasset", "Scene"),
            ("content/rock.gasset", "Mesh"),
        ]);
        let level = f.asset("content/level.gasset");
        let item = f.asset_item("content/level.gasset");

        f.world.trigger_on(item, UIItemSelected);
        f.refresh();

        assert_eq!(f.state().selected, Some(level));
        assert!(f.ui_selected(item));
        assert!(!f.ui_selected(f.asset_item("content/rock.gasset")));
        let theme = UITheme::default();
        let mark = f
            .world
            .get_component_for_entity::<ContentItem>(item)
            .unwrap()
            .mark_node;
        assert_eq!(
            f.world
                .get_component_for_entity::<UINode>(mark)
                .unwrap()
                .width,
            UIValue::Px(Mark::Group.shape(&theme, true).size)
        );
        assert_eq!(
            f.world
                .get_component_for_entity::<UIMaterial>(mark)
                .unwrap()
                .border_color,
            Mark::Group.shape(&theme, true).border.to_linear()
        );
        assert!(f.opened().is_empty());
    }

    #[test]
    fn a_click_while_busy_is_ignored_and_the_highlight_snaps_back() {
        let mut f = fixture(&[("content/level.gasset", "Scene")]);
        let item = f.asset_item("content/level.gasset");
        let release = f.busy();

        f.click(item);

        assert_eq!(f.state().selected, None);
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
        assert_eq!(f.state().selected, Some(level));
        let theme = UITheme::default();
        assert_eq!(
            f.mark_border(forest),
            Mark::Folder.shape(&theme, true).border.to_linear()
        );
        assert_eq!(
            f.mark_border(level_item),
            Mark::Group.shape(&theme, false).border.to_linear()
        );
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
        assert_eq!(f.state().selected, Some(level));
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

        run(&mut f.world, reconcile_content_tree);
        place(&mut f.world);
        run(&mut f.world, sync_content_items);

        let marks: HashSet<Entity> = f
            .world
            .query::<&ContentItem, ()>()
            .iter(&mut f.world)
            .map(|item| item.mark_node)
            .collect();
        let changed: Vec<Entity> = f
            .world
            .query::<Entity, Or<(Changed<UINode>, Changed<UIMaterial>)>>()
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
}

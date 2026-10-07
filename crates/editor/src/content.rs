//! The project's asset catalogue, as a docked panel.
use concerto_app::{
    App, Plugin,
    schedule_groups::{LateUpdate, Startup},
};
use concerto_ecs::{
    Component, Entity, IntoSystemConfig, Query, Res, ResMut, Resource, command::CommandQueue,
    events::event_reader::EventReader, signal::On,
};
use concerto_ui::{
    elements::prelude::*,
    interaction::{Interactable, UIClick, UIDoubleClick, UIInteractionStyle},
    material::UIMaterial,
    node::{UINode, UIRect},
    scroll::{UIScrollArea, UIVirtualList},
    sets::UiSet,
    text::UIText,
    text_input::UITextInputChanged,
    theme::{ButtonVariant, UITheme},
    transform::UIValue,
};
use concerto_window::input::MouseButton;
use taffy::FlexDirection;

use crate::dock::{DockedApp, PanelDescriptor, PanelRegistry, Region};
use crate::marks::{self, Mark};
use crate::project::{AssetEntry, EditorCommand, EditorCommands, ProjectState};
use concerto_foundation::assets::AssetId;

pub const PANEL_ID: &str = "concerto.curiosities";

const ROW_HEIGHT: f32 = 28.0;
const ROWS: usize = 24;

const KINDS: [(&str, Option<&str>); 5] = [
    ("all", None),
    ("scene", Some("Scene")),
    ("mesh", Some("Mesh")),
    ("tex", Some("Texture")),
    ("mat", Some("Material")),
];

#[derive(Resource, Default)]
pub struct ContentState {
    /// Catalogue selection is independent of the open editor's entity selection.
    pub selected: Option<AssetId>,
    filter: String,
    kind: usize,
    visible: std::ops::Range<usize>,
}

impl ContentState {
    fn index(&self, slot: usize) -> usize {
        self.visible.start + slot
    }
}

#[derive(Component)]
struct AssetRow(usize);

#[derive(Component)]
enum Label {
    Count,
    Asset(usize),
}

#[derive(Component)]
struct MarkSlot(usize);

#[derive(Component)]
struct Tag(usize);

#[derive(Component)]
struct Filter;

#[derive(Component)]
struct ContentView;

pub struct ContentPlugin;

impl Plugin for ContentPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(ContentState::default());
        app.add_panel(PanelDescriptor {
            id: PANEL_ID,
            title: "Curiosities",
            region: Region::Rail,
        });
        app.add_system(Startup, build_panel)
            .add_system(LateUpdate, update_filter)
            .add_system(LateUpdate, sync_scroll)
            .add_system(LateUpdate, refresh_panel)
            .add_system(LateUpdate, render_marks.before(UiSet::Materials))
            .add_system(LateUpdate, render_tags.before(UiSet::Materials));
    }
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
            panel = panel.add_child_with(
                theme
                    .card()
                    .radius_sm()
                    .height(UIValue::Px(30.0))
                    .fixed()
                    .row()
                    .padding(UIRect::axes(0.0, 8.0))
                    .gap(6.0),
                |search| {
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
            );

            panel = panel.add_child_with(
                UINode::default()
                    .with_flex_shrink(0.0)
                    .with_flex_direction(FlexDirection::Row)
                    .with_gap(glam::Vec2::new(4.0, 0.0))
                    .with_margin(UIRect::axes(7.0, 0.0)),
                |mut tags| {
                    for (index, (name, _)) in KINDS.iter().enumerate() {
                        tags =
                            tags.add_child((theme.chip(*name).on_click(select_kind), Tag(index)));
                    }
                },
            );

            panel = panel.add_child_with(
                (
                    UINode::default()
                        .with_flex_grow(1.0)
                        .with_flex_shrink(1.0)
                        .with_flex_direction(FlexDirection::Column)
                        .clipped(),
                    Interactable,
                    ContentView,
                    UIVirtualList::new(0, ROW_HEIGHT).with_overscan(1),
                ),
                |mut view| {
                    let mut pool = view.spawn_child_queue(
                        UINode::default()
                            .with_flex_direction(FlexDirection::Column)
                            .with_flex_shrink(0.0),
                    );
                    let pool_entity = pool.entity();

                    for slot in 0..ROWS {
                        pool = pool.add_child_with(
                            (
                                theme
                                    .pressable()
                                    .height(UIValue::Px(ROW_HEIGHT))
                                    .row()
                                    .padding(UIRect::axes(4.0, 6.0))
                                    .gap(8.0)
                                    .on_click(select_asset)
                                    .on_double_click(open_asset),
                                AssetRow(slot),
                            ),
                            |row| {
                                let (mark_node, mark_material) = marks::node();
                                row.add_child((mark_node, mark_material, MarkSlot(slot)))
                                    .add_child((
                                        theme.label("").single_line().grow().shrink(1.0),
                                        Label::Asset(slot),
                                    ));
                            },
                        );
                    }

                    view.insert(UIScrollArea {
                        content: Some(pool_entity),
                        ..Default::default()
                    });
                },
            );

            panel.add_child((
                theme
                    .label("")
                    .height(UIValue::Px(26.0))
                    .fixed()
                    .padding(UIRect::axes(4.0, 10.0)),
                Label::Count,
            ));
        },
    );
}

fn sync_scroll(
    views: Query<(&ContentView, &mut UIScrollArea, &mut UIVirtualList)>,
    project: Res<ProjectState>,
    mut state: ResMut<ContentState>,
) {
    let Some((_, mut area, mut list)) = views.iter().next() else {
        return;
    };
    let count = visible_assets(&project, &state).len();
    list.item_count = count;
    area.content_extent = count as f32 * ROW_HEIGHT;
    state.visible = list.visible_range();
}

fn update_filter(
    mut changes: EventReader<UITextInputChanged>,
    filters: Query<&Filter>,
    views: Query<(&ContentView, &mut UIScrollArea)>,
    mut state: ResMut<ContentState>,
) {
    let mut reset = false;
    for change in changes.read() {
        if filters.get_entity(change.entity).is_some() {
            state.filter = change.value.clone();
            reset = true;
        }
    }
    if reset {
        if let Some((_, mut area)) = views.iter().next() {
            area.offset = 0.0;
        }
    }
}

fn select_kind(
    on: On<UIClick>,
    tags: Query<&Tag>,
    project: Res<ProjectState>,
    mut state: ResMut<ContentState>,
    views: Query<(&ContentView, &mut UIScrollArea)>,
) {
    if on.signal().button != MouseButton::Left || project.busy() {
        return;
    }
    let Some(tag) = tags.get_entity(on.entity()) else {
        return;
    };
    state.kind = tag.0;
    state.visible = 0..0;
    if let Some((_, mut area)) = views.iter().next() {
        area.offset = 0.0;
    }
}

fn row_asset(
    row: Entity,
    rows: &Query<&AssetRow>,
    project: &ProjectState,
    state: &ContentState,
) -> Option<AssetId> {
    let row = rows.get_entity(row)?;
    visible_assets(project, state)
        .get(state.index(row.0))
        .map(|asset| asset.id)
}

fn select_asset(
    on: On<UIClick>,
    rows: Query<&AssetRow>,
    project: Res<ProjectState>,
    mut state: ResMut<ContentState>,
) {
    if on.signal().button != MouseButton::Left || project.busy() {
        return;
    }
    if let Some(id) = row_asset(on.entity(), &rows, &project, &state) {
        state.selected = Some(id);
    }
}

fn open_asset(
    on: On<UIDoubleClick>,
    rows: Query<&AssetRow>,
    project: Res<ProjectState>,
    state: Res<ContentState>,
    mut commands: ResMut<EditorCommands>,
) {
    if on.signal().button != MouseButton::Left || project.busy() {
        return;
    }
    if let Some(id) = row_asset(on.entity(), &rows, &project, &state) {
        commands.0.push_back(EditorCommand::OpenAsset(id));
    }
}

fn refresh_panel(
    project: Res<ProjectState>,
    state: Res<ContentState>,
    labels: Query<(&Label, &mut UIText)>,
) {
    let assets = visible_assets(&project, &state);

    for (label, mut component) in labels.iter() {
        let value = match label {
            Label::Count => {
                if assets.is_empty() {
                    String::new()
                } else {
                    format!("{} assets", assets.len())
                }
            }
            Label::Asset(slot) => assets
                .get(state.index(*slot))
                .map(|asset| asset.display_name.clone())
                .unwrap_or_default(),
        };
        if component.text != value {
            component.text = value;
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

fn render_marks(
    project: Res<ProjectState>,
    state: Res<ContentState>,
    theme: Res<UITheme>,
    slots: Query<(&MarkSlot, &mut UINode, &mut UIMaterial)>,
    styles: Query<(&AssetRow, &mut UIInteractionStyle)>,
) {
    let assets = visible_assets(&project, &state);
    let selected = |slot: usize| {
        assets
            .get(state.index(slot))
            .is_some_and(|asset| state.selected == Some(asset.id))
    };
    for (slot, mut node, mut material) in slots.iter() {
        let mark = assets
            .get(state.index(slot.0))
            .map_or(Mark::None, |asset| asset_mark(&asset.kind));
        marks::apply(mark, &theme, selected(slot.0), &mut node, &mut material);
    }
    for (row, mut style) in styles.iter() {
        let wanted = theme.interaction(ButtonVariant::Ghost, selected(row.0));
        if style.normal != wanted.normal {
            **style = wanted;
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
    };
    use concerto_foundation::assets::content::AssetRegistry;
    use glam::Vec2;

    fn run<M>(world: &mut World, system: impl IntoSystem<(), M>) {
        let mut system = system.into_system();
        system.initialize(world);
        system.run_and_apply((), world);
    }

    fn build(mut cmd: CommandQueue, theme: Res<UITheme>) {
        let body = cmd.spawn(UINode::default()).entity();
        spawn_panel(&mut cmd, body, &theme);
    }

    fn fixture() -> (World, AssetId, Entity) {
        let mut world = World::new();
        world.register_component::<ChildOf>();
        world.register_component::<Children>();
        world.insert_resource(UITheme::default());
        world.insert_resource(EditorCommands::default());
        world.insert_resource(ContentState {
            visible: 0..1,
            ..Default::default()
        });
        let id = AssetId::new();
        let mut project = ProjectState::default();
        project.project = Some(Project {
            root: Default::default(),
            assets: vec![AssetEntry::from_address(
                id,
                "content/level.gasset",
                "Scene".into(),
                None,
            )],
            registry: AssetRegistry::default(),
        });
        world.insert_resource(project);
        run(&mut world, build);
        let row = world
            .query::<(Entity, &AssetRow), ()>()
            .iter(&mut world)
            .find(|(_, row)| row.0 == 0)
            .map(|(entity, _)| entity)
            .unwrap();
        (world, id, row)
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

    #[test]
    fn a_click_selects_an_asset_without_opening_it() {
        let (mut world, id, row) = fixture();

        world.trigger_on(
            row,
            UIClick {
                position: Vec2::ZERO,
                button: MouseButton::Left,
            },
        );

        assert_eq!(
            world.get_resource::<ContentState>().unwrap().selected,
            Some(id)
        );
        assert!(opened(&world).is_empty());
    }

    #[test]
    fn a_double_click_opens_the_asset() {
        let (mut world, id, row) = fixture();

        world.trigger_on(
            row,
            UIDoubleClick {
                position: Vec2::ZERO,
                button: MouseButton::Left,
            },
        );

        assert_eq!(opened(&world), [id]);
    }
}

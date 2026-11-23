use std::collections::HashSet;

use concerto_ecs::{
    IntoSystem, World,
    command::CommandQueue,
    component::{Component, removed::RemovedComponents},
    entity::{
        Entity,
        hierarchy::{ChildOf, Children},
    },
    events::event_writer::EventWriter,
    query::{
        Query,
        change_detection::DetectChanges,
        filter::{Added, Changed, Or, With},
    },
    resource::{Res, ResMut},
    signal::{On, lifecycle::Add, listener::IntoListener},
    system::BoxedSystem,
};
use concerto_window::input::MouseButton;
use glam::Vec2;

use crate::{
    focus::{FocusedWidget, UIFocusGained, UIFocusLost},
    interaction::{Interactable, UIClick},
    items::{
        UIHidden, UIItem, UIRowSpacer, UIWidgetPart, equip_items, fit_owner, keep_parts_first,
        keep_parts_last, park_items, place_rows, set_up_owner,
    },
    material::UIMaterial,
    node::{
        AlignContent, AlignItems, Overflow, Position, UIBox, UICulled, UIInset, UIIntrinsicHeight,
        UILayout, UINode,
    },
    scroll::UIScrollArea,
    text::UIText,
    theme::UITheme,
    transform::UIValue,
};

/// A vertical tree whose items are its children that carry `UITreeElement`, nested through theirs, one row each in depth-first order; it clips them, owns its overflow, draws each parent's caret and the connector lines between items, owns the carets' clicks, so put no click listener or `UIInteractionStyle` on an item; its natural height is its content extent, so a tree that must fill its slot needs zero height and a flex grow.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct UITree {
    /// Height of every row in logical pixels; `None` takes the theme's `row_height`.
    pub row_height: Option<f32>,
    /// Whether items can be highlighted, navigated by key, selected and activated; carets work either way.
    pub selectable: bool,
    /// While true every item shows as if expanded, caret clicks are ignored and `expanded` is left untouched.
    pub filtering: bool,
    /// Whether a click with Ctrl or Shift selects several items; the widget then sends `UIItemSelected` with a mode and leaves `UISelected` to its user.
    pub multi_select: bool,
}

impl Default for UITree {
    fn default() -> Self {
        Self {
            row_height: None,
            selectable: true,
            filtering: false,
            multi_select: false,
        }
    }
}

/// Makes a child of a `UITree`, or of another tree item, an item whose other children are its row content; put no click listener or `UIInteractionStyle` on it.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UITreeElement {
    /// Whether the item's child items are shown; the caret toggles it, and the caller may set it.
    pub expanded: bool,
}

#[derive(Component)]
pub(crate) struct UITreeCaret {
    glyph: Entity,
}

#[derive(Component)]
pub(crate) struct UITreeConnector;

const TICK_TO_CARET: f32 = 6.0;
const TICK_TO_CONTENT: f32 = 22.0;
const CONNECTOR_THICKNESS: f32 = 1.0;

pub(crate) struct TreeNode {
    pub(crate) children: Vec<Entity>,
    pub(crate) expanded: bool,
    pub(crate) hidden: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TreeRow {
    pub(crate) entity: Entity,
    pub(crate) depth: usize,
    pub(crate) has_children: bool,
    pub(crate) drawn_expanded: bool,
    pub(crate) last: bool,
    pub(crate) continues: Vec<bool>,
}

pub(crate) fn flatten_tree(
    roots: &[Entity],
    filtering: bool,
    mut node: impl FnMut(Entity) -> TreeNode,
) -> Vec<TreeRow> {
    let shown = shown_level(roots, &mut node);
    let mut rows = Vec::new();
    flatten_level(shown, filtering, 0, &mut Vec::new(), &mut rows);
    rows
}

struct ShownItem {
    entity: Entity,
    expanded: bool,
    children: Vec<ShownItem>,
}

fn shown_level(siblings: &[Entity], node: &mut impl FnMut(Entity) -> TreeNode) -> Vec<ShownItem> {
    siblings
        .iter()
        .filter_map(|&entity| {
            let node_of = node(entity);
            let children = shown_level(&node_of.children, &mut *node);
            (!node_of.hidden || !children.is_empty()).then_some(ShownItem {
                entity,
                expanded: node_of.expanded,
                children,
            })
        })
        .collect()
}

fn flatten_level(
    shown: Vec<ShownItem>,
    filtering: bool,
    depth: usize,
    continues: &mut Vec<bool>,
    rows: &mut Vec<TreeRow>,
) {
    let count = shown.len();
    for (position, item) in shown.into_iter().enumerate() {
        let last = position + 1 == count;
        let has_children = !item.children.is_empty();
        let drawn_expanded = has_children && (filtering || item.expanded);
        rows.push(TreeRow {
            entity: item.entity,
            depth,
            has_children,
            drawn_expanded,
            last,
            continues: continues.clone(),
        });
        if drawn_expanded {
            if depth > 0 {
                continues.push(!last);
            }
            flatten_level(item.children, filtering, depth + 1, continues, rows);
            if depth > 0 {
                continues.pop();
            }
        }
    }
}

fn marked_items(children: &Children, is_item: impl Fn(Entity) -> bool) -> Vec<Entity> {
    children
        .iter()
        .copied()
        .filter(|child| is_item(*child))
        .collect()
}

fn every_item(
    roots: &[Entity],
    marked: &mut impl FnMut(Entity) -> Vec<Entity>,
) -> Vec<(Entity, usize)> {
    let mut every = Vec::new();
    let mut pending = roots
        .iter()
        .rev()
        .map(|root| (*root, 0))
        .collect::<Vec<_>>();
    while let Some((entity, depth)) = pending.pop() {
        every.push((entity, depth));
        pending.extend(
            marked(entity)
                .into_iter()
                .rev()
                .map(|child| (child, depth + 1)),
        );
    }
    every
}

fn marked_children(
    entity: Entity,
    children: &Query<&Children>,
    elements: &Query<&mut UITreeElement>,
) -> Vec<Entity> {
    children
        .get_entity(entity)
        .map(|children| marked_items(children, |child| elements.contains_entity(child)))
        .unwrap_or_default()
}

pub(crate) fn has_shown_children(
    entity: Entity,
    elements: &Query<&mut UITreeElement>,
    hidden: &Query<&UIHidden>,
    children: &Query<&Children>,
) -> bool {
    let marked = |entity: Entity| {
        children
            .get_entity(entity)
            .map(|children| marked_items(children, |child| elements.contains_entity(child)))
            .unwrap_or_default()
    };
    !shown_level(&marked(entity), &mut |entity| TreeNode {
        children: marked(entity),
        expanded: false,
        hidden: hidden.contains_entity(entity),
    })
    .is_empty()
}

pub(crate) fn open_to(
    tree: Entity,
    filtering: bool,
    item: Entity,
    elements: &mut Query<&mut UITreeElement>,
    hidden: &Query<&UIHidden>,
    children: &Query<&Children>,
    parents: &Query<&ChildOf>,
) -> Option<usize> {
    let mut ancestor = parents.get_entity(item).map(ChildOf::parent);
    while !filtering
        && let Some(entity) = ancestor
        && entity != tree
    {
        if let Some(mut element) = elements.get_entity_mut(entity)
            && !element.expanded
        {
            element.expanded = true;
        }
        ancestor = parents.get_entity(entity).map(ChildOf::parent);
    }
    let roots = marked_children(tree, children, elements);
    flatten_tree(&roots, filtering, |entity| TreeNode {
        expanded: elements
            .get_entity_mut(entity)
            .is_some_and(|element| element.expanded),
        children: marked_children(entity, children, elements),
        hidden: hidden.contains_entity(entity),
    })
    .iter()
    .position(|row| row.entity == item)
}

/// Registers the listener that sets up each new `UITree` and tracks the removals `update_trees` reads, for a world run without `UIPlugin`.
pub fn install_listeners(world: &mut World) {
    world.add_listener(set_up_tree);
    world.track_removals::<Children>();
    world.track_removals::<UIHidden>();
    world.track_removals::<UITreeElement>();
}

/// The system that nests, places and draws each `UITree` whose inputs changed, for a world run without `UIPlugin`; it needs [`install_listeners`].
pub fn update_trees_system() -> BoxedSystem {
    update_trees.into_boxed_system()
}

pub(crate) fn set_up_tree(
    on: On<Add<UITree>>,
    areas: Query<&UIScrollArea>,
    interactables: Query<&Interactable>,
    children: Query<&Children>,
    spacers: Query<&UIRowSpacer>,
    mut cmd: CommandQueue,
) {
    set_up_owner(
        on.signal().entity,
        &areas,
        &interactables,
        &children,
        &spacers,
        &mut cmd,
    );
}

fn click_caret(
    on: On<UIClick>,
    parents: Query<&ChildOf>,
    mut elements: Query<&mut UITreeElement>,
    (items, trees): (Query<&UIItem>, Query<&UITree>),
    mut focused: ResMut<FocusedWidget>,
    mut lost: EventWriter<UIFocusLost>,
    mut gained: EventWriter<UIFocusGained>,
) {
    if on.signal().button != MouseButton::Left {
        return;
    }
    let Some(item) = parents.get_entity(on.entity()).map(ChildOf::parent) else {
        return;
    };
    let filtering = items
        .get_entity(item)
        .and_then(|item| trees.get_entity(item.owner()))
        .is_some_and(|tree| tree.filtering);
    if filtering {
        return;
    }
    if let Some(mut element) = elements.get_entity_mut(item) {
        element.expanded = !element.expanded;
    }
    let previous = **focused;
    if previous != Some(item) {
        **focused = Some(item);
        if let Some(previous) = previous {
            lost.write(UIFocusLost(previous));
        }
        gained.write(UIFocusGained(item));
    }
}

pub(crate) fn update_trees(
    mut trees: Query<(Entity, &UITree, &mut UIScrollArea, Option<&UILayout>)>,
    mut children: Query<(Entity, &mut Children)>,
    elements: Query<&UITreeElement>,
    hidden: Query<&UIHidden>,
    mut nodes: Query<&mut UINode>,
    parts: Query<&UIWidgetPart>,
    (carets, connectors, mut texts): (
        Query<&UITreeCaret>,
        Query<&UITreeConnector>,
        Query<&mut UIText>,
    ),
    mut spacers: Query<&mut UIIntrinsicHeight, With<UIRowSpacer>>,
    (items, culled, materials): (Query<&UIItem>, Query<&UICulled>, Query<&UIMaterial>),
    theme: Res<UITheme>,
    mut cmd: CommandQueue,
    (resized, changed_elements, added_hidden, parents, removed): (
        Query<Entity, (With<UITree>, Or<(Changed<UITree>, Changed<UILayout>)>)>,
        Query<Entity, Changed<UITreeElement>>,
        Query<Entity, Added<UIHidden>>,
        Query<&ChildOf>,
        (
            RemovedComponents<Children>,
            RemovedComponents<UITreeElement>,
            RemovedComponents<UIHidden>,
        ),
    ),
) {
    let (mut removed_children, mut removed_elements, mut removed_hidden) = removed;
    let touched = children
        .iter_mut()
        .filter(|(_, children)| children.is_changed())
        .map(|(entity, _)| entity)
        .chain(changed_elements.iter())
        .chain(added_hidden.iter())
        .chain(removed_children.read())
        .chain(removed_elements.read())
        .chain(removed_hidden.read())
        .collect::<Vec<_>>();
    let mut stale = resized.iter().collect::<HashSet<_>>();
    stale.extend(
        touched
            .into_iter()
            .filter_map(|entity| owning_tree(entity, &trees, &items, &parents)),
    );
    let every_tree = theme.is_changed();
    for (entity, tree, mut area, layout) in trees.iter_mut() {
        if !every_tree && !area.is_changed() && !stale.contains(&entity) {
            continue;
        }
        let row_height = tree.row_height.unwrap_or(theme.row_height);
        let mut roots = Vec::new();
        let mut spacer = None;
        if let Some((_, mut children)) = children.get_entity_mut(entity) {
            keep_parts_last(&mut children, &parts);
            for &child in children.iter() {
                if elements.contains_entity(child) {
                    roots.push(child);
                } else if spacer.is_none() {
                    spacer = spacers.get_entity_mut(child);
                }
            }
        }
        let mut marked = |entity: Entity| {
            children
                .get_entity_mut(entity)
                .map(|(_, children)| {
                    marked_items(&children, |child| elements.contains_entity(child))
                })
                .unwrap_or_default()
        };
        let rows = flatten_tree(&roots, tree.filtering, |entity| TreeNode {
            children: marked(entity),
            expanded: elements
                .get_entity(entity)
                .is_some_and(|element| element.expanded),
            hidden: hidden.contains_entity(entity),
        });
        let every = every_item(&roots, &mut marked);
        equip_items(
            every.iter().map(|(item, _)| *item),
            tree.selectable,
            &items,
            &materials,
            &mut cmd,
        );
        let placed = rows
            .iter()
            .map(|row| (row.entity, row.depth))
            .collect::<Vec<_>>();
        place_rows(
            &placed,
            entity,
            row_height,
            tree.selectable,
            layout.map(|layout| layout.content_rect.size.y),
            &mut area,
            &mut nodes,
            &items,
            &culled,
            &mut cmd,
        );
        let shown = rows.iter().map(|row| row.entity).collect::<HashSet<_>>();
        let parked = every
            .into_iter()
            .filter(|(item, _)| !shown.contains(item))
            .collect::<Vec<_>>();
        park_items(
            &parked,
            entity,
            row_height,
            tree.selectable,
            &items,
            &culled,
            &mut cmd,
        );
        for row in &rows {
            shape_item(row, &theme, &mut nodes);
            let (caret, drawn) = children
                .get_entity_mut(row.entity)
                .map(|(_, mut children)| {
                    keep_parts_first(&mut children, &parts);
                    let caret = children
                        .iter()
                        .copied()
                        .find(|child| carets.contains_entity(*child));
                    let drawn = children
                        .iter()
                        .copied()
                        .filter(|child| connectors.contains_entity(*child))
                        .collect::<Vec<_>>();
                    (caret, drawn)
                })
                .unwrap_or_default();
            let segments = connector_segments(row, &theme, row_height);
            sync_connectors(row.entity, &segments, &drawn, &theme, &mut nodes, &mut cmd);
            match (caret, row.has_children) {
                (None, true) => spawn_caret(row, &theme, &mut cmd),
                (Some(caret), false) => cmd.despawn(caret),
                (Some(caret), true) => {
                    sync_caret(caret, row, &theme, &carets, &mut texts, &mut nodes)
                }
                (None, false) => {}
            }
        }
        fit_owner(entity, spacer, area.content_extent, &mut nodes);
    }
}

fn owning_tree(
    entity: Entity,
    trees: &Query<(Entity, &UITree, &mut UIScrollArea, Option<&UILayout>)>,
    items: &Query<&UIItem>,
    parents: &Query<&ChildOf>,
) -> Option<Entity> {
    let direct = |entity: Entity| {
        if trees.contains_entity(entity) {
            Some(entity)
        } else {
            items.get_entity(entity).map(UIItem::owner)
        }
    };
    direct(entity).or_else(|| {
        parents
            .get_entity(entity)
            .and_then(|parent| direct(parent.parent()))
    })
}

fn shape_item(row: &TreeRow, theme: &UITheme, nodes: &mut Query<&mut UINode>) {
    let Some(mut node) = nodes.get_entity_mut(row.entity) else {
        return;
    };
    let left = caret_left(row, theme) + theme.tree_caret_column;
    if node.padding.left != left {
        node.padding.left = left;
    }
    if node.align_items != Some(AlignItems::Center) {
        node.align_items = Some(AlignItems::Center);
    }
    if node.overflow_x != Overflow::Visible {
        node.overflow_x = Overflow::Visible;
    }
    if node.overflow_y != Overflow::Visible {
        node.overflow_y = Overflow::Visible;
    }
}

fn caret_left(row: &TreeRow, theme: &UITheme) -> f32 {
    theme.tree_inset + row.depth as f32 * theme.tree_indent
}

fn caret_glyph(row: &TreeRow, theme: &UITheme) -> String {
    if row.drawn_expanded {
        theme.tree_caret_expanded
    } else {
        theme.tree_caret_collapsed
    }
    .to_string()
}

fn caret_inset(row: &TreeRow, theme: &UITheme) -> UIInset {
    UIInset {
        top: UIValue::Px(0.0),
        left: UIValue::Px(caret_left(row, theme)),
        ..Default::default()
    }
}

fn spawn_caret(row: &TreeRow, theme: &UITheme, cmd: &mut CommandQueue) {
    let glyph = cmd
        .spawn((
            UINode::default(),
            UIText {
                text: caret_glyph(row, theme),
                font_size: theme.tree_caret_font_size,
                line_height: theme.line_height(theme.tree_caret_font_size),
                font_family: theme.tree_caret_family.clone(),
                color: theme.text_muted,
                wrap: false,
                ..Default::default()
            },
            UIWidgetPart,
        ))
        .entity();
    let caret = cmd
        .spawn((
            UINode::default()
                .with_position(Position::Absolute)
                .with_inset(caret_inset(row, theme))
                .with_size(
                    UIValue::Px(theme.tree_caret_column),
                    UIValue::Percent(100.0),
                )
                .with_align_items(AlignItems::Center)
                .with_justify_content(AlignContent::Center),
            Interactable,
            click_caret.into_listener(),
            UIWidgetPart,
            UITreeCaret { glyph },
        ))
        .entity();
    cmd.add_child(caret, glyph);
    cmd.add_child(row.entity, caret);
}

fn sync_caret(
    caret: Entity,
    row: &TreeRow,
    theme: &UITheme,
    carets: &Query<&UITreeCaret>,
    texts: &mut Query<&mut UIText>,
    nodes: &mut Query<&mut UINode>,
) {
    let inset = caret_inset(row, theme);
    if let Some(mut node) = nodes.get_entity_mut(caret)
        && node.inset != inset
    {
        node.inset = inset;
    }
    let glyph = caret_glyph(row, theme);
    if let Some(caret) = carets.get_entity(caret)
        && let Some(mut text) = texts.get_entity_mut(caret.glyph)
        && text.text != glyph
    {
        text.text = glyph;
    }
}

pub(crate) fn connector_segments(row: &TreeRow, theme: &UITheme, row_height: f32) -> Vec<UIBox> {
    let Some(own) = row.depth.checked_sub(1) else {
        return Vec::new();
    };
    let centre = row_height / 2.0;
    let line_x = |level: usize| {
        theme.tree_inset + level as f32 * theme.tree_indent + theme.tree_caret_column / 2.0
    };
    let vertical = |level: usize, height: f32| UIBox {
        min: Vec2::new(line_x(level), 0.0),
        size: Vec2::new(CONNECTOR_THICKNESS, height),
    };
    let mut segments = (0..own)
        .filter(|level| row.continues.get(*level).copied().unwrap_or(false))
        .map(|level| vertical(level, row_height))
        .collect::<Vec<_>>();
    segments.push(vertical(own, if row.last { centre } else { row_height }));
    let tick = if row.has_children {
        TICK_TO_CARET
    } else {
        TICK_TO_CONTENT
    };
    segments.push(UIBox {
        min: Vec2::new(line_x(own), centre),
        size: Vec2::new(tick, CONNECTOR_THICKNESS),
    });
    segments
}

fn sync_connectors(
    row: Entity,
    segments: &[UIBox],
    drawn: &[Entity],
    theme: &UITheme,
    nodes: &mut Query<&mut UINode>,
    cmd: &mut CommandQueue,
) {
    for (index, segment) in segments.iter().enumerate() {
        let inset = UIInset {
            top: UIValue::Px(segment.min.y),
            left: UIValue::Px(segment.min.x),
            ..Default::default()
        };
        let width = UIValue::Px(segment.size.x);
        let height = UIValue::Px(segment.size.y);
        let Some(&connector) = drawn.get(index) else {
            let connector = cmd
                .spawn((
                    UINode::default()
                        .with_position(Position::Absolute)
                        .with_inset(inset)
                        .with_size(width, height),
                    UIMaterial::flat(theme.border),
                    UIWidgetPart,
                    UITreeConnector,
                ))
                .entity();
            cmd.add_child(row, connector);
            continue;
        };
        if let Some(mut node) = nodes.get_entity_mut(connector)
            && (node.inset != inset || node.width != width || node.height != height)
        {
            node.inset = inset;
            node.width = width;
            node.height = height;
        }
    }
    for &extra in drawn.iter().skip(segments.len()) {
        cmd.despawn(extra);
    }
}

#[cfg(test)]
mod tests {
    use concerto_ecs::{
        World,
        entity::{Entity, hierarchy::ChildOf},
        resource::{ResMut, Resource},
        signal::{On, listener::IntoListener},
    };
    use concerto_window::input::{Input, KeyCode, MouseButton, PhysicalKey};
    use glam::Vec2;

    use super::*;
    use crate::{
        items::{
            UIItem, UIItemSelected, UIListNext, UIReveal, UISelectAnchor, UISelectMode, UISelected,
            UITreeCollapse, UITreeExpand,
        },
        list::tests::Harness,
        node::{FlexDirection, UINode},
        transform::UIValue,
    };

    const ROW: f32 = 20.0;

    fn tree_of(harness: &mut Harness, height: f32) -> Entity {
        let root = harness.root();
        harness.world.spawn((
            UINode::default()
                .with_height(UIValue::Px(height))
                .with_flex_shrink(0.0)
                .with_flex_direction(FlexDirection::Column),
            UITree {
                row_height: Some(ROW),
                ..Default::default()
            },
            ChildOf::new(root),
        ))
    }

    fn item(harness: &mut Harness, parent: Entity, expanded: bool) -> Entity {
        harness.world.spawn((
            UINode::default(),
            UITreeElement { expanded },
            ChildOf::new(parent),
        ))
    }

    fn row(harness: &Harness, entity: Entity) -> Option<[usize; 2]> {
        harness
            .world
            .get_component_for_entity::<UIItem>(entity)
            .filter(|item| item.is_shown())
            .map(|item| [item.index(), item.depth()])
    }

    fn caret(harness: &Harness, entity: Entity) -> Option<Entity> {
        harness
            .world
            .get_component_for_entity::<Children>(entity)?
            .iter()
            .copied()
            .find(|child| {
                harness
                    .world
                    .get_component_for_entity::<UITreeCaret>(*child)
                    .is_some()
            })
    }

    fn expanded(harness: &Harness, entity: Entity) -> bool {
        harness
            .world
            .get_component_for_entity::<UITreeElement>(entity)
            .unwrap()
            .expanded
    }

    fn chain(harness: &mut Harness, tree: Entity, depth: usize) -> Vec<Entity> {
        let mut items = Vec::new();
        let mut parent = tree;
        for _ in 0..=depth {
            parent = item(harness, parent, true);
            items.push(parent);
        }
        items
    }

    fn content_inset(harness: &mut Harness, depth: usize) -> f32 {
        let tree = tree_of(harness, 600.0);
        let items = chain(harness, tree, depth);
        let content = harness.node(
            UINode::default().with_size(UIValue::Px(40.0), UIValue::Px(10.0)),
            items[depth],
        );
        harness.settle();
        assert_eq!(
            harness.layout(items[depth]).rect.min.y,
            harness.layout(tree).rect.min.y + depth as f32 * ROW
        );
        assert_eq!(
            harness.layout(items[depth]).rect.min.x,
            harness.layout(tree).rect.min.x
        );
        harness.layout(content).rect.min.x - harness.layout(tree).rect.min.x
    }

    fn hide(harness: &mut Harness, entities: &[Entity]) {
        for entity in entities {
            harness.world.insert(UIHidden, *entity);
        }
    }

    fn set_filtering(harness: &mut Harness, tree: Entity, filtering: bool) {
        harness
            .world
            .get_component_for_entity_mut::<UITree>(tree)
            .unwrap()
            .filtering = filtering;
    }

    fn rows_of(harness: &Harness, entities: &[Entity]) -> Vec<Option<[usize; 2]>> {
        entities
            .iter()
            .map(|entity| row(harness, *entity))
            .collect()
    }

    #[test]
    fn flatten_skips_collapsed_children() {
        let mut world = World::new();
        let [a, b, c, d] = [(); 4].map(|_| world.spawn(UINode::default()));
        let node = |children: Vec<Entity>, expanded: bool| TreeNode {
            children,
            expanded,
            hidden: false,
        };

        let rows = flatten_tree(&[a], false, |entity| match entity {
            entity if entity == a => node(vec![b, d], true),
            entity if entity == b => node(vec![c], false),
            _ => node(vec![], false),
        });

        let row = |entity, depth, has_children, drawn_expanded, last, continues: &[bool]| TreeRow {
            entity,
            depth,
            has_children,
            drawn_expanded,
            last,
            continues: continues.to_vec(),
        };
        assert_eq!(
            rows,
            [
                row(a, 0, true, true, true, &[]),
                row(b, 1, true, false, false, &[]),
                row(d, 1, false, false, true, &[]),
            ]
        );
    }

    #[derive(Resource, Default)]
    struct Modes(Vec<UISelectMode>);

    fn listen_for_modes(harness: &mut Harness, rows: &[Entity]) {
        harness.world.insert_resource(Modes::default());
        for row in rows {
            harness.world.insert(
                (move |on: On<UIItemSelected>, mut modes: ResMut<Modes>| {
                    modes.0.push(on.signal().mode.clone())
                })
                .into_listener(),
                *row,
            );
        }
    }

    fn hold(harness: &mut Harness, key: Option<KeyCode>) {
        let input = harness.world.get_resource_mut::<Input>().unwrap();
        input.update();
        for held in [KeyCode::ControlLeft, KeyCode::ShiftLeft] {
            input.update_key_input(PhysicalKey::Code(held), key == Some(held));
        }
    }

    fn click_with(harness: &mut Harness, key: Option<KeyCode>, row: Entity) -> Vec<UISelectMode> {
        hold(harness, key);
        harness.click(row, MouseButton::Left, Vec2::ZERO);
        std::mem::take(&mut harness.world.get_resource_mut::<Modes>().unwrap().0)
    }

    fn anchor(harness: &Harness, tree: Entity) -> Option<Entity> {
        harness
            .world
            .get_component_for_entity::<UISelectAnchor>(tree)
            .map(|anchor| anchor.0)
    }

    #[test]
    fn a_click_on_a_multi_select_tree_reports_its_mode_and_a_single_select_tree_ignores_modifiers()
    {
        let mut harness = Harness::new();
        let tree = tree_of(&mut harness, 200.0);
        harness
            .world
            .get_component_for_entity_mut::<UITree>(tree)
            .unwrap()
            .multi_select = true;
        let rows: Vec<Entity> = (0..5).map(|_| item(&mut harness, tree, false)).collect();
        listen_for_modes(&mut harness, &rows);
        harness.settle();
        let (ctrl, shift) = (Some(KeyCode::ControlLeft), Some(KeyCode::ShiftLeft));

        assert_eq!(
            click_with(&mut harness, None, rows[1]),
            [UISelectMode::Replace]
        );
        assert_eq!(anchor(&harness, tree), Some(rows[1]));
        assert!(harness.selected(&rows).is_empty());

        assert_eq!(
            click_with(&mut harness, ctrl, rows[3]),
            [UISelectMode::Toggle]
        );
        assert_eq!(anchor(&harness, tree), Some(rows[3]));

        assert_eq!(
            click_with(&mut harness, shift, rows[4]),
            [UISelectMode::Range(vec![rows[3], rows[4]])]
        );
        assert_eq!(
            click_with(&mut harness, shift, rows[0]),
            [UISelectMode::Range(vec![
                rows[3], rows[2], rows[1], rows[0]
            ])]
        );
        assert_eq!(anchor(&harness, tree), Some(rows[3]));

        harness.world.remove_component::<UISelectAnchor>(tree);
        assert_eq!(
            click_with(&mut harness, shift, rows[2]),
            [UISelectMode::Replace]
        );
        assert!(harness.selected(&rows).is_empty());

        harness.world.insert(UISelected, rows[0]);
        harness.world.insert(UISelected, rows[1]);
        assert_eq!(harness.selected(&rows), [0, 1]);

        let single = tree_of(&mut harness, 200.0);
        let singles: Vec<Entity> = (0..3).map(|_| item(&mut harness, single, false)).collect();
        listen_for_modes(&mut harness, &singles);
        harness.settle();
        assert_eq!(
            click_with(&mut harness, ctrl, singles[0]),
            [UISelectMode::Replace]
        );
        assert_eq!(
            click_with(&mut harness, ctrl, singles[2]),
            [UISelectMode::Replace]
        );
        assert_eq!(harness.selected(&singles), [2]);
        assert_eq!(anchor(&harness, single), None);
    }

    #[test]
    fn clicking_the_caret_toggles_expanded() {
        let mut harness = Harness::new();
        let tree = tree_of(&mut harness, 200.0);
        let parent = item(&mut harness, tree, false);
        let child = item(&mut harness, parent, false);
        harness.settle();
        assert_eq!(row(&harness, child), None);
        assert_eq!(harness.area(tree).content_extent, ROW);

        let shown = caret(&harness, parent).expect("a parent has a caret");
        harness.click(shown, MouseButton::Left, Vec2::ZERO);
        assert!(expanded(&harness, parent));
        harness.settle();

        assert_eq!(row(&harness, child), Some([1, 1]));
        assert!(harness.layout(child).is_shown());
        assert_eq!(harness.area(tree).content_extent, 2.0 * ROW);
        assert!(harness.selected(&[parent, child]).is_empty());

        harness.click(shown, MouseButton::Left, Vec2::ZERO);
        harness.settle();
        assert!(!expanded(&harness, parent));
        assert_eq!(row(&harness, child), None);
        assert!(!harness.layout(child).is_shown());
        assert_eq!(harness.area(tree).content_extent, ROW);
    }

    #[test]
    fn row_content_is_indented_by_depth_without_a_cap() {
        let mut harness = Harness::new();
        assert_eq!(content_inset(&mut harness, 3), 4.0 + 48.0 + 16.0);
        let mut harness = Harness::new();
        assert_eq!(content_inset(&mut harness, 20), 4.0 + 320.0 + 16.0);
    }

    #[test]
    fn next_skips_collapsed_children() {
        let mut harness = Harness::new();
        let tree = tree_of(&mut harness, 200.0);
        let parent = item(&mut harness, tree, false);
        item(&mut harness, parent, false);
        let sibling = item(&mut harness, tree, false);
        harness.settle();

        harness.focus(parent);
        harness.fire(UIListNext);

        assert_eq!(harness.focused(), Some(sibling));
    }

    #[test]
    fn expand_and_collapse_actions() {
        let mut harness = Harness::new();
        let tree = tree_of(&mut harness, 200.0);
        let parent = item(&mut harness, tree, false);
        let leaf = item(&mut harness, parent, false);
        let folder = item(&mut harness, parent, false);
        item(&mut harness, folder, false);
        harness.settle();

        harness.focus(parent);
        harness.fire(UITreeExpand);
        assert!(expanded(&harness, parent));
        assert_eq!(harness.focused(), Some(parent));
        harness.settle();

        harness.fire(UITreeExpand);
        assert_eq!(harness.focused(), Some(leaf));
        assert_eq!(harness.selected(&[parent, leaf, folder]), [1]);
        harness.settle();

        harness.fire(UITreeExpand);
        assert_eq!(harness.focused(), Some(leaf));

        harness.fire(UITreeCollapse);
        assert_eq!(harness.focused(), Some(parent));
        harness.settle();

        harness.focus(folder);
        harness.fire(UITreeCollapse);
        assert!(!expanded(&harness, folder));
        assert_eq!(harness.focused(), Some(parent));
        harness.settle();

        harness.fire(UITreeCollapse);
        assert!(!expanded(&harness, parent));
        assert_eq!(harness.focused(), Some(parent));
        harness.settle();

        harness.fire(UITreeCollapse);
        assert!(!expanded(&harness, parent));
        assert_eq!(harness.focused(), Some(parent));
    }

    #[test]
    fn reveal_expands_every_ancestor_then_scrolls() {
        let mut harness = Harness::new();
        let tree = tree_of(&mut harness, 100.0);
        for _ in 0..50 {
            item(&mut harness, tree, false);
        }
        let outer = item(&mut harness, tree, false);
        let inner = item(&mut harness, outer, false);
        let leaf = item(&mut harness, inner, false);
        harness.settle();
        assert_eq!(row(&harness, leaf), None);

        harness.world.trigger_on(leaf, UIReveal);
        assert!(expanded(&harness, outer));
        assert!(expanded(&harness, inner));
        harness.frame();

        assert_eq!(row(&harness, leaf), Some([52, 2]));
        assert_eq!(harness.area(tree).offset, 53.0 * ROW - 100.0);
        let content = harness.layout(tree).content_rect;
        let rect = harness.layout(leaf).rect;
        assert!(harness.layout(leaf).is_shown());
        assert!(rect.min.y >= content.min.y, "{rect:?} in {content:?}");
        assert!(
            rect.min.y + rect.size.y <= content.min.y + content.size.y,
            "{rect:?} in {content:?}"
        );
    }

    #[test]
    fn focus_leaves_an_item_whose_ancestor_collapses() {
        let mut harness = Harness::new();
        let tree = tree_of(&mut harness, 200.0);
        let items = chain(&mut harness, tree, 2);
        harness.settle();
        harness.focus(items[2]);

        harness
            .world
            .get_component_for_entity_mut::<UITreeElement>(items[0])
            .unwrap()
            .expanded = false;
        harness.settle();

        assert_eq!(harness.focused(), Some(items[0]));
    }

    #[test]
    fn a_hidden_item_with_a_visible_descendant_still_shows() {
        let mut harness = Harness::new();
        let tree = tree_of(&mut harness, 200.0);
        let parent = item(&mut harness, tree, true);
        let middle = item(&mut harness, parent, true);
        let leaf = item(&mut harness, middle, false);
        let sibling = item(&mut harness, parent, false);
        let other = item(&mut harness, tree, true);
        let hidden_child = item(&mut harness, other, false);
        hide(
            &mut harness,
            &[parent, middle, sibling, other, hidden_child],
        );

        harness.settle();

        assert_eq!(
            rows_of(
                &harness,
                &[parent, middle, leaf, sibling, other, hidden_child]
            ),
            [Some([0, 0]), Some([1, 1]), Some([2, 2]), None, None, None]
        );
        assert_eq!(harness.area(tree).content_extent, 3.0 * ROW);
    }

    #[test]
    fn filtering_opens_the_path_to_matches_without_writing_expanded() {
        let mut harness = Harness::new();
        let tree = tree_of(&mut harness, 200.0);
        let a = item(&mut harness, tree, false);
        let c = item(&mut harness, a, false);
        let d = item(&mut harness, c, false);
        let b = item(&mut harness, tree, false);
        let e = item(&mut harness, b, false);
        let every = [a, c, d, b, e];
        harness.settle();
        assert_eq!(
            rows_of(&harness, &every),
            [Some([0, 0]), None, None, Some([1, 0]), None]
        );

        hide(&mut harness, &[a, c, b, e]);
        set_filtering(&mut harness, tree, true);
        harness.settle();

        assert_eq!(
            rows_of(&harness, &every),
            [Some([0, 0]), Some([1, 1]), Some([2, 2]), None, None]
        );
        for entity in every {
            assert!(!expanded(&harness, entity));
        }

        for entity in every {
            harness.world.remove_component::<UIHidden>(entity);
        }
        set_filtering(&mut harness, tree, false);
        harness.settle();

        assert_eq!(
            rows_of(&harness, &every),
            [Some([0, 0]), None, None, Some([1, 0]), None]
        );
    }

    #[test]
    fn collapsing_while_scrolled_to_the_end_clamps_the_offset() {
        let mut harness = Harness::new();
        let tree = tree_of(&mut harness, 100.0);
        let parent = item(&mut harness, tree, true);
        for _ in 0..150 {
            item(&mut harness, parent, false);
        }
        let mut top = vec![parent];
        for _ in 0..49 {
            top.push(item(&mut harness, tree, false));
        }
        harness.settle();
        assert_eq!(harness.area(tree).content_extent, 200.0 * ROW);
        harness.set_offset(tree, 200.0 * ROW - 100.0);
        harness.settle();

        harness
            .world
            .get_component_for_entity_mut::<UITreeElement>(parent)
            .unwrap()
            .expanded = false;
        harness.settle();

        assert_eq!(harness.area(tree).offset, 50.0 * ROW - 100.0);
        let unculled = harness.unculled(&top);
        assert_eq!(unculled, (43..50).collect::<Vec<_>>());
        let content = harness.layout(tree).content_rect;
        for &row in &top[45..50] {
            let rect = harness.layout(row).rect;
            assert!(harness.layout(row).is_shown());
            assert!(rect.min.y >= content.min.y, "{rect:?} in {content:?}");
            assert!(
                rect.min.y + rect.size.y <= content.min.y + content.size.y,
                "{rect:?} in {content:?}"
            );
        }
    }

    fn tree_row(depth: usize, has_children: bool, last: bool, continues: &[bool]) -> TreeRow {
        TreeRow {
            entity: World::new().spawn(UINode::default()),
            depth,
            has_children,
            drawn_expanded: has_children,
            last,
            continues: continues.to_vec(),
        }
    }

    fn segment(x: f32, y: f32, width: f32, height: f32) -> UIBox {
        UIBox {
            min: Vec2::new(x, y),
            size: Vec2::new(width, height),
        }
    }

    fn segments_of(row: &TreeRow) -> Vec<UIBox> {
        connector_segments(row, &UITheme::default(), 24.0)
    }

    #[test]
    fn connectors_draw_a_vertical_and_a_tick_for_each_child_and_continue_ancestor_lines() {
        struct Case {
            name: &'static str,
            row: TreeRow,
            segments: Vec<UIBox>,
        }
        for case in [
            Case {
                name: "a middle child gets a full vertical and a tick",
                row: tree_row(1, false, false, &[]),
                segments: vec![
                    segment(12.0, 0.0, 1.0, 24.0),
                    segment(12.0, 12.0, 22.0, 1.0),
                ],
            },
            Case {
                name: "a last child gets an elbow",
                row: tree_row(1, false, true, &[]),
                segments: vec![
                    segment(12.0, 0.0, 1.0, 12.0),
                    segment(12.0, 12.0, 22.0, 1.0),
                ],
            },
            Case {
                name: "ancestor lines continue only while the ancestor has siblings below",
                row: tree_row(3, false, false, &[true, false]),
                segments: vec![
                    segment(12.0, 0.0, 1.0, 24.0),
                    segment(44.0, 0.0, 1.0, 24.0),
                    segment(44.0, 12.0, 22.0, 1.0),
                ],
            },
            Case {
                name: "a root has none",
                row: tree_row(0, false, false, &[]),
                segments: vec![],
            },
        ] {
            assert_eq!(segments_of(&case.row), case.segments, "{}", case.name);
        }
    }

    #[test]
    fn scrolling_from_outside_unculls_the_rows_in_view() {
        let mut harness = Harness::new();
        let tree = tree_of(&mut harness, 100.0);
        let items = (0..50)
            .map(|_| item(&mut harness, tree, false))
            .collect::<Vec<_>>();
        harness.settle();
        assert_eq!(harness.unculled(&items), (0..7).collect::<Vec<_>>());

        harness.set_offset(tree, 20.0 * ROW);
        harness.settle();

        assert_eq!(harness.unculled(&items), (18..27).collect::<Vec<_>>());
    }
}

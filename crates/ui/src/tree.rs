use std::collections::HashSet;

use concerto_ecs::{
    command::CommandQueue,
    component::Component,
    entity::{
        Entity,
        hierarchy::{ChildOf, Children},
    },
    events::event_writer::EventWriter,
    query::{Query, filter::With},
    resource::{Res, ResMut},
    signal::{On, lifecycle::Add, listener::IntoListener},
};
use concerto_window::input::MouseButton;

use crate::{
    focus::{FocusedWidget, UIFocusGained, UIFocusLost},
    interaction::{Interactable, UIClick},
    items::{
        UIHidden, UIItem, UIRowSpacer, UIWidgetPart, equip_items, fit_owner, keep_parts_first,
        keep_parts_last, park_items, place_rows, set_up_owner,
    },
    material::UIMaterial,
    node::{
        AlignContent, AlignItems, Overflow, Position, UICulled, UIInset, UIIntrinsicHeight,
        UILayout, UINode,
    },
    scroll::UIScrollArea,
    text::UIText,
    theme::UITheme,
    transform::UIValue,
};

/// A vertical tree whose items are its children that carry `UITreeElement`, nested through theirs, one row each in depth-first order; it clips them, owns its overflow, draws each parent's caret and owns their clicks, so put no click listener or `UIInteractionStyle` on an item.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct UITree {
    pub row_height: Option<f32>,
    pub selectable: bool,
    pub filtering: bool,
}

impl Default for UITree {
    fn default() -> Self {
        Self {
            row_height: None,
            selectable: true,
            filtering: false,
        }
    }
}

/// Makes a child of a `UITree`, or of another tree item, an item whose other children are its row content; put no click listener or `UIInteractionStyle` on it.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UITreeElement {
    pub expanded: bool,
}

#[derive(Component)]
pub(crate) struct UITreeCaret {
    glyph: Entity,
}

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
    node: impl Fn(Entity) -> TreeNode,
) -> Vec<TreeRow> {
    let shown = shown_level(roots, &node);
    let mut rows = Vec::new();
    flatten_level(shown, filtering, &mut Vec::new(), &mut rows);
    rows
}

struct ShownItem {
    entity: Entity,
    expanded: bool,
    children: Vec<ShownItem>,
}

fn shown_level(siblings: &[Entity], node: &impl Fn(Entity) -> TreeNode) -> Vec<ShownItem> {
    siblings
        .iter()
        .filter_map(|&entity| {
            let node_of = node(entity);
            let children = shown_level(&node_of.children, node);
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
            depth: continues.len(),
            has_children,
            drawn_expanded,
            last,
            continues: continues.clone(),
        });
        if drawn_expanded {
            continues.push(!last);
            flatten_level(item.children, filtering, continues, rows);
            continues.pop();
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

fn every_item(roots: &[Entity], marked: &impl Fn(Entity) -> Vec<Entity>) -> Vec<(Entity, usize)> {
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
    !shown_level(&marked(entity), &|entity| TreeNode {
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
    elements: &Query<&mut UITreeElement>,
    hidden: &Query<&UIHidden>,
    children: &Query<&Children>,
    parents: &Query<&ChildOf>,
) -> Option<usize> {
    let mut ancestor = parents.get_entity(item).map(ChildOf::parent);
    while !filtering
        && let Some(entity) = ancestor
        && entity != tree
    {
        if let Some(mut element) = elements.get_entity(entity)
            && !element.expanded
        {
            element.expanded = true;
        }
        ancestor = parents.get_entity(entity).map(ChildOf::parent);
    }
    let marked = |entity: Entity| {
        children
            .get_entity(entity)
            .map(|children| marked_items(children, |child| elements.contains_entity(child)))
            .unwrap_or_default()
    };
    flatten_tree(&marked(tree), filtering, |entity| TreeNode {
        children: marked(entity),
        expanded: elements
            .get_entity(entity)
            .is_some_and(|element| element.expanded),
        hidden: hidden.contains_entity(entity),
    })
    .iter()
    .position(|row| row.entity == item)
}

pub(crate) fn set_up_tree(
    on: On<Add<UITree>>,
    areas: Query<&UIScrollArea>,
    children: Query<&Children>,
    spacers: Query<&UIRowSpacer>,
    mut cmd: CommandQueue,
) {
    set_up_owner(on.signal().entity, &areas, &children, &spacers, &mut cmd);
}

fn click_caret(
    on: On<UIClick>,
    parents: Query<&ChildOf>,
    elements: Query<&mut UITreeElement>,
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
    if let Some(mut element) = elements.get_entity(item) {
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
    trees: Query<(Entity, &UITree, &mut UIScrollArea, Option<&UILayout>)>,
    children: Query<&mut Children>,
    elements: Query<&UITreeElement>,
    hidden: Query<&UIHidden>,
    nodes: Query<&mut UINode>,
    parts: Query<&UIWidgetPart>,
    (carets, texts): (Query<&UITreeCaret>, Query<&mut UIText>),
    spacers: Query<&mut UIIntrinsicHeight, With<UIRowSpacer>>,
    (items, culled, materials): (Query<&UIItem>, Query<&UICulled>, Query<&UIMaterial>),
    theme: Res<UITheme>,
    mut cmd: CommandQueue,
) {
    for (entity, tree, mut area, layout) in trees.iter() {
        let row_height = tree.row_height.unwrap_or(theme.row_height);
        let mut roots = Vec::new();
        let mut spacer = None;
        if let Some(mut children) = children.get_entity(entity) {
            keep_parts_last(&mut children, &parts);
            for &child in children.iter() {
                if elements.contains_entity(child) {
                    roots.push(child);
                } else if spacer.is_none() {
                    spacer = spacers.get_entity(child);
                }
            }
        }
        let marked = |entity: Entity| {
            children
                .get_entity(entity)
                .map(|children| marked_items(&children, |child| elements.contains_entity(child)))
                .unwrap_or_default()
        };
        let rows = flatten_tree(&roots, tree.filtering, |entity| TreeNode {
            children: marked(entity),
            expanded: elements
                .get_entity(entity)
                .is_some_and(|element| element.expanded),
            hidden: hidden.contains_entity(entity),
        });
        let every = every_item(&roots, &marked);
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
            &nodes,
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
            shape_item(row, &theme, &nodes);
            let caret = children.get_entity(row.entity).and_then(|mut children| {
                keep_parts_first(&mut children, &parts);
                children
                    .iter()
                    .copied()
                    .find(|child| carets.contains_entity(*child))
            });
            match (caret, row.has_children) {
                (None, true) => spawn_caret(row, &theme, &mut cmd),
                (Some(caret), false) => cmd.despawn(caret),
                (Some(caret), true) => sync_caret(caret, row, &theme, &carets, &texts, &nodes),
                (None, false) => {}
            }
        }
        fit_owner(entity, spacer, area.content_extent, &nodes);
    }
}

fn shape_item(row: &TreeRow, theme: &UITheme, nodes: &Query<&mut UINode>) {
    let Some(mut node) = nodes.get_entity(row.entity) else {
        return;
    };
    let left = caret_left(row, theme) + theme.tree_caret_column;
    if node.padding.left != left {
        node.padding.left = left;
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
                font_size: theme.font_size_md,
                line_height: theme.line_height(theme.font_size_md),
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
    texts: &Query<&mut UIText>,
    nodes: &Query<&mut UINode>,
) {
    let inset = caret_inset(row, theme);
    if let Some(mut node) = nodes.get_entity(caret)
        && node.inset != inset
    {
        node.inset = inset;
    }
    let glyph = caret_glyph(row, theme);
    if let Some(caret) = carets.get_entity(caret)
        && let Some(mut text) = texts.get_entity(caret.glyph)
        && text.text != glyph
    {
        text.text = glyph;
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use concerto_ecs::{
        World,
        entity::{Entity, hierarchy::ChildOf},
    };
    use concerto_window::input::MouseButton;
    use glam::Vec2;

    use super::*;
    use crate::{
        focus::update_focus,
        interaction::{HoveredNode, update_ui_interaction},
        items::{
            UIItem, UIListActivate, UIListFirst, UIListLast, UIListNext, UIListPrevious, UIReveal,
            UISelected, UITreeCollapse, UITreeExpand,
        },
        list::tests::Harness,
        node::{FlexDirection, UINode},
        scroll::UIScrollBar,
        text::UIText,
        transform::UIValue,
    };
    use concerto_ecs::{IntoSystem, System};
    use concerto_window::input::Input;

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

    fn row(harness: &Harness, entity: Entity) -> Option<(usize, usize)> {
        harness
            .world
            .get_component_for_entity::<UIItem>(entity)
            .filter(|item| item.is_shown())
            .map(|item| (item.index(), item.depth()))
    }

    fn press_and_release(harness: &mut Harness, entity: Entity) {
        let rect = harness.layout(entity).rect;
        let centre = rect.min + rect.size / 2.0;
        let mut interaction = update_ui_interaction.into_system();
        interaction.initialize(&mut harness.world);
        let mut focus = update_focus.into_system();
        focus.initialize(&mut harness.world);
        for pressed in [true, false] {
            {
                let input = harness.world.get_resource_mut::<Input>().unwrap();
                input.update();
                input.update_mouse_position(centre.x as f64, centre.y as f64);
                input.update_mouse_button(MouseButton::Left, pressed);
            }
            interaction.run_and_apply((), &mut harness.world);
            focus.run_and_apply((), &mut harness.world);
        }
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

    fn first_child(harness: &Harness, entity: Entity) -> Option<Entity> {
        harness
            .world
            .get_component_for_entity::<Children>(entity)?
            .iter()
            .next()
            .copied()
    }

    fn glyph(harness: &Harness, caret: Entity) -> String {
        let glyph = harness
            .world
            .get_component_for_entity::<UITreeCaret>(caret)
            .unwrap()
            .glyph;
        harness
            .world
            .get_component_for_entity::<UIText>(glyph)
            .unwrap()
            .text
            .clone()
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

    fn rows_of(harness: &Harness, entities: &[Entity]) -> Vec<Option<(usize, usize)>> {
        entities
            .iter()
            .map(|entity| row(harness, *entity))
            .collect()
    }

    #[test]
    fn flatten_skips_collapsed_children() {
        let mut world = World::new();
        let [a, b, c, d] = [(); 4].map(|_| world.spawn(UINode::default()));
        let nodes = HashMap::from([
            (a, (vec![b, d], true)),
            (b, (vec![c], false)),
            (c, (vec![], false)),
            (d, (vec![], false)),
        ]);

        let rows = flatten_tree(&[a], false, |entity| {
            let (children, expanded) = nodes[&entity].clone();
            TreeNode {
                children,
                expanded,
                hidden: false,
            }
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
                row(b, 1, true, false, false, &[false]),
                row(d, 1, false, false, true, &[false]),
            ]
        );
    }

    #[test]
    fn only_marked_children_are_items() {
        let mut harness = Harness::new();
        let tree = tree_of(&mut harness, 200.0);
        let parent = item(&mut harness, tree, true);
        let label = harness.node(UINode::default(), parent);
        let child = item(&mut harness, parent, false);

        harness.settle();

        assert_eq!(row(&harness, parent), Some((0, 0)));
        assert_eq!(row(&harness, child), Some((1, 1)));
        assert!(
            harness
                .world
                .get_component_for_entity::<UIItem>(label)
                .is_none()
        );
        assert_eq!(harness.area(tree).content_extent, 2.0 * ROW);
        assert_eq!(
            harness.layout(child).rect.min.y,
            harness.layout(tree).rect.min.y + ROW
        );
    }

    #[test]
    fn the_caret_appears_only_with_child_items() {
        let mut harness = Harness::new();
        let tree = tree_of(&mut harness, 200.0);
        let parent = item(&mut harness, tree, false);
        harness.node(UINode::default(), parent);
        harness.settle();
        assert_eq!(caret(&harness, parent), None);

        let child = item(&mut harness, parent, false);
        harness.settle();
        let shown = caret(&harness, parent).expect("a parent has a caret");
        assert_eq!(first_child(&harness, parent), Some(shown));
        assert!(
            harness
                .world
                .get_component_for_entity::<UIItem>(shown)
                .is_none()
        );
        assert_eq!(glyph(&harness, shown), "▸");

        harness.world.despawn(child);
        harness.settle();
        assert_eq!(caret(&harness, parent), None);
        assert!(
            harness
                .world
                .get_component_for_entity::<UINode>(shown)
                .is_none()
        );
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

        assert_eq!(row(&harness, child), Some((1, 1)));
        assert!(harness.layout(child).is_shown());
        assert_eq!(harness.area(tree).content_extent, 2.0 * ROW);
        assert_eq!(glyph(&harness, shown), "▾");
        assert!(harness.selected(&[parent, child]).is_empty());

        harness.click(shown, MouseButton::Left, Vec2::ZERO);
        harness.settle();
        assert!(!expanded(&harness, parent));
        assert_eq!(row(&harness, child), None);
        assert!(!harness.layout(child).is_shown());
        assert_eq!(harness.area(tree).content_extent, ROW);
    }

    #[test]
    fn row_content_is_indented_by_depth() {
        let mut harness = Harness::new();
        assert_eq!(content_inset(&mut harness, 3), 4.0 + 48.0 + 16.0);
    }

    #[test]
    fn no_indent_cap() {
        let mut harness = Harness::new();
        assert_eq!(content_inset(&mut harness, 20), 4.0 + 320.0 + 16.0);
    }

    #[test]
    fn the_caret_sits_in_its_column() {
        let mut harness = Harness::new();
        let tree = tree_of(&mut harness, 200.0);
        let items = chain(&mut harness, tree, 2);
        harness.settle();

        let caret = caret(&harness, items[1]).expect("a parent has a caret");
        let rect = harness.layout(caret).rect;
        let tree_rect = harness.layout(tree).rect;
        assert_eq!(rect.min.x - tree_rect.min.x, 4.0 + 16.0);
        assert_eq!(rect.size.x, 16.0);
        assert_eq!(rect.min.y, tree_rect.min.y + ROW);
        assert_eq!(rect.size.y, ROW);
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

        assert_eq!(row(&harness, leaf), Some((52, 2)));
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
    fn clicking_a_caret_leaves_keyboard_focus_on_its_item() {
        let mut harness = Harness::new();
        let tree = tree_of(&mut harness, 200.0);
        let parent = item(&mut harness, tree, false);
        let child = item(&mut harness, parent, false);
        harness.settle();
        let shown = caret(&harness, parent).expect("a parent has a caret");

        press_and_release(&mut harness, shown);
        assert!(expanded(&harness, parent));
        assert_eq!(harness.focused(), Some(parent));
        harness.settle();

        harness.fire(UIListNext);
        assert_eq!(harness.focused(), Some(child));
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
    fn a_parked_item_is_not_shown() {
        let mut harness = Harness::new();
        let tree = tree_of(&mut harness, 200.0);
        let parent = item(&mut harness, tree, false);
        let child = item(&mut harness, parent, false);
        harness.settle();

        let item_of = |entity| {
            *harness
                .world
                .get_component_for_entity::<UIItem>(entity)
                .unwrap()
        };
        assert!(item_of(parent).is_shown());
        assert!(!item_of(child).is_shown());
    }

    #[test]
    fn hovering_the_caret_highlights_its_row() {
        let mut harness = Harness::new();
        let tree = tree_of(&mut harness, 200.0);
        let parent = item(&mut harness, tree, false);
        item(&mut harness, parent, false);
        harness.settle();
        let shown = caret(&harness, parent).expect("a parent has a caret");

        **harness.world.get_resource_mut::<HoveredNode>().unwrap() = Some(shown);
        harness.frame();

        assert_eq!(
            harness.colour(parent),
            UITheme::default().surface_hovered.to_linear()
        );
    }

    #[test]
    fn a_clipping_item_does_not_clip_its_child_rows() {
        let mut harness = Harness::new();
        let tree = tree_of(&mut harness, 200.0);
        let parent = harness.world.spawn((
            UINode::default().clipped(),
            UITreeElement { expanded: true },
            ChildOf::new(tree),
        ));
        let child = item(&mut harness, parent, false);
        harness.settle();

        let layout = harness.layout(child);
        let centre = layout.rect.min + layout.rect.size / 2.0;
        assert!(
            layout.clip_rect.contains(centre),
            "{:?} clipped to {:?}",
            layout.rect,
            layout.clip_rect
        );
    }

    #[test]
    fn a_hidden_sibling_does_not_make_its_predecessor_not_last() {
        let mut world = World::new();
        let [a, b, c, d, e] = [(); 5].map(|_| world.spawn(UINode::default()));
        let nodes = HashMap::from([
            (a, (vec![b, d], true, false)),
            (b, (vec![c], true, false)),
            (c, (vec![], false, false)),
            (d, (vec![], false, true)),
            (e, (vec![], false, true)),
        ]);

        let rows = flatten_tree(&[a, e], false, |entity| {
            let (children, expanded, hidden) = nodes[&entity].clone();
            TreeNode {
                children,
                expanded,
                hidden,
            }
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
                row(b, 1, true, true, true, &[false]),
                row(c, 2, false, false, true, &[false, false]),
            ]
        );
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
            [Some((0, 0)), Some((1, 1)), Some((2, 2)), None, None, None]
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
            [Some((0, 0)), None, None, Some((1, 0)), None]
        );

        hide(&mut harness, &[a, c, b, e]);
        set_filtering(&mut harness, tree, true);
        harness.settle();

        assert_eq!(
            rows_of(&harness, &every),
            [Some((0, 0)), Some((1, 1)), Some((2, 2)), None, None]
        );
        for entity in every {
            assert!(!expanded(&harness, entity));
        }
        let shown = caret(&harness, a).expect("a parent has a caret");
        assert_eq!(glyph(&harness, shown), "▾");

        for entity in every {
            harness.world.remove_component::<UIHidden>(entity);
        }
        set_filtering(&mut harness, tree, false);
        harness.settle();

        assert_eq!(
            rows_of(&harness, &every),
            [Some((0, 0)), None, None, Some((1, 0)), None]
        );
        assert_eq!(glyph(&harness, shown), "▸");
    }

    #[test]
    fn a_filter_matching_everything_still_opens_the_tree() {
        let mut harness = Harness::new();
        let tree = tree_of(&mut harness, 200.0);
        let a = item(&mut harness, tree, false);
        let c = item(&mut harness, a, false);
        let d = item(&mut harness, c, false);
        let b = item(&mut harness, tree, false);
        let e = item(&mut harness, b, false);
        set_filtering(&mut harness, tree, true);

        harness.settle();

        assert_eq!(
            rows_of(&harness, &[a, c, d, b, e]),
            [
                Some((0, 0)),
                Some((1, 1)),
                Some((2, 2)),
                Some((3, 0)),
                Some((4, 1))
            ]
        );
    }

    #[test]
    fn carets_ignore_clicks_while_filtering() {
        let mut harness = Harness::new();
        let tree = tree_of(&mut harness, 200.0);
        let parent = item(&mut harness, tree, false);
        let child = item(&mut harness, parent, false);
        set_filtering(&mut harness, tree, true);
        harness.settle();
        let shown = caret(&harness, parent).expect("a parent has a caret");

        harness.click(shown, MouseButton::Left, Vec2::ZERO);
        harness.settle();

        assert!(!expanded(&harness, parent));
        assert_eq!(harness.focused(), None);
        assert_eq!(row(&harness, child), Some((1, 1)));
    }

    #[test]
    fn a_filter_that_hides_the_focused_item_clears_focus() {
        let mut harness = Harness::new();
        let tree = tree_of(&mut harness, 200.0);
        let a = item(&mut harness, tree, false);
        item(&mut harness, tree, false);
        harness.settle();
        harness.focus(a);

        hide(&mut harness, &[a]);
        set_filtering(&mut harness, tree, true);
        harness.settle();

        assert_eq!(harness.focused(), None);
    }

    #[test]
    fn a_filter_that_matches_nothing_shows_nothing_and_navigation_is_inert() {
        let mut harness = Harness::new();
        let tree = tree_of(&mut harness, 200.0);
        let a = item(&mut harness, tree, true);
        let c = item(&mut harness, a, false);
        let b = item(&mut harness, tree, false);
        let every = [a, c, b];
        harness.settle();
        harness.focus(c);
        harness.world.insert(UISelected, c);

        hide(&mut harness, &every);
        set_filtering(&mut harness, tree, true);
        harness.settle();

        assert_eq!(rows_of(&harness, &every), [None, None, None]);
        assert_eq!(harness.area(tree).content_extent, 0.0);
        let track = harness
            .world
            .get_component_for_entity::<Children>(tree)
            .unwrap()
            .iter()
            .copied()
            .find(|child| {
                harness
                    .world
                    .get_component_for_entity::<UIScrollBar>(*child)
                    .is_some()
            })
            .expect("the tree has a scrollbar track");
        assert!(
            !harness
                .world
                .get_component_for_entity::<UINode>(track)
                .unwrap()
                .visible
        );
        let focused = harness.focused();
        assert_eq!(
            focused, None,
            "hiding the focused item clears focus, so the actions below run with nothing focused"
        );
        for fire in [
            |harness: &mut Harness| harness.fire(UIListNext),
            |harness: &mut Harness| harness.fire(UIListPrevious),
            |harness: &mut Harness| harness.fire(UIListFirst),
            |harness: &mut Harness| harness.fire(UIListLast),
            |harness: &mut Harness| harness.fire(UIListActivate),
            |harness: &mut Harness| harness.fire(UITreeExpand),
            |harness: &mut Harness| harness.fire(UITreeCollapse),
        ] {
            fire(&mut harness);
            assert_eq!(harness.focused(), focused);
            assert_eq!(harness.selected(&every), [1]);
        }
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
        for index in 45..50 {
            let rect = harness.layout(top[index]).rect;
            assert!(harness.layout(top[index]).is_shown());
            assert!(rect.min.y >= content.min.y, "{rect:?} in {content:?}");
            assert!(
                rect.min.y + rect.size.y <= content.min.y + content.size.y,
                "{rect:?} in {content:?}"
            );
        }
    }

    #[test]
    fn navigation_and_reveal_while_filtering_leave_expanded_alone() {
        let mut harness = Harness::new();
        let tree = tree_of(&mut harness, 200.0);
        let parent = item(&mut harness, tree, false);
        let child = item(&mut harness, parent, false);
        let leaf = item(&mut harness, child, false);
        set_filtering(&mut harness, tree, true);
        harness.settle();

        harness.focus(parent);
        harness.fire(UITreeExpand);
        assert_eq!(harness.focused(), Some(child));
        harness.fire(UITreeCollapse);
        assert_eq!(harness.focused(), Some(parent));
        harness.fire(UITreeCollapse);
        assert_eq!(harness.focused(), Some(parent));
        harness.world.trigger_on(leaf, UIReveal);
        harness.settle();

        for entity in [parent, child, leaf] {
            assert!(!expanded(&harness, entity));
        }
        assert_eq!(row(&harness, leaf), Some((2, 2)));
    }

    #[test]
    fn navigation_treats_an_item_whose_children_are_all_hidden_as_a_leaf() {
        let mut harness = Harness::new();
        let tree = tree_of(&mut harness, 200.0);
        let grand = item(&mut harness, tree, true);
        let parent = item(&mut harness, grand, true);
        let children = [
            item(&mut harness, parent, false),
            item(&mut harness, parent, false),
        ];
        hide(&mut harness, &children);
        harness.settle();
        assert_eq!(caret(&harness, parent), None);

        harness.focus(parent);
        harness.fire(UITreeCollapse);
        assert_eq!(harness.focused(), Some(grand));
        assert!(expanded(&harness, parent));

        harness
            .world
            .get_component_for_entity_mut::<UITreeElement>(parent)
            .unwrap()
            .expanded = false;
        harness.settle();
        harness.focus(parent);
        harness.fire(UITreeExpand);
        assert_eq!(harness.focused(), Some(parent));
        assert!(!expanded(&harness, parent));
    }
}

use std::collections::HashMap;

use concerto_ecs::{
    IntoSystem,
    command::CommandQueue,
    component::Component,
    entity::{
        Entity,
        hierarchy::{ChildOf, Children},
    },
    events::event_reader::EventReader,
    query::{
        Query,
        filter::{Changed, With},
    },
    resource::Res,
    system::BoxedSystem,
};
use concerto_window::input::actions::ActionFired;

use crate::{
    anchor::UIAnchoredPanel,
    focus::FocusedWidget,
    items::{
        UIHidden, UIItemActivated, UIItemSelected, UIListNext, UIListPrevious, UISelected,
        UIWidgetPart,
    },
    list::UIList,
    node::{UILayout, UINode},
    scroll::UIScrollArea,
    text_input::{UITextInput, UITextInputCancelled, UITextInputSubmitted},
    theme::UITheme,
    tree::{TreeNode, UITree, UITreeElement, flatten_tree},
};

/// On a `UITextInput`: narrows the `UIList` or `UITree` `target` to the items whose `UISearchText` holds its text, and drives that list from the keyboard while focused.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct UISearchField {
    pub target: Entity,
}

/// The text a `UISearchField` matches an item against; an item without it is never hidden by a search.
#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct UISearchText(pub String);

/// The system that narrows and drives every `UISearchField`'s list, for a world run without `UIPlugin`.
pub fn update_search_fields_system() -> BoxedSystem {
    update_search_fields.into_boxed_system()
}

#[derive(Component, Default)]
pub(crate) struct UISearchState {
    query: Option<String>,
    reveal: Option<Entity>,
}

struct Target<'a, 'w, 's> {
    entity: Entity,
    children: &'a Query<'w, 's, &'static Children>,
    parts: &'a Query<'w, 's, &'static UIWidgetPart>,
    nodes: &'a Query<'w, 's, &'static UINode>,
    elements: &'a Query<'w, 's, &'static UITreeElement>,
    hidden: &'a Query<'w, 's, &'static UIHidden>,
    overrides: HashMap<Entity, bool>,
    filtering: Option<bool>,
}

impl Target<'_, '_, '_> {
    fn is_hidden(&self, entity: Entity) -> bool {
        self.overrides
            .get(&entity)
            .copied()
            .unwrap_or_else(|| self.hidden.contains_entity(entity))
    }

    fn child_items(&self, entity: Entity) -> Vec<Entity> {
        let Some(children) = self.children.get_entity(entity) else {
            return Vec::new();
        };
        children
            .iter()
            .copied()
            .filter(|&child| match self.filtering {
                Some(_) => self.elements.contains_entity(child),
                None => !self.parts.contains_entity(child) && self.nodes.contains_entity(child),
            })
            .collect()
    }

    fn every_item(&self) -> Vec<Entity> {
        let mut every = Vec::new();
        let mut pending: Vec<Entity> = self.child_items(self.entity).into_iter().rev().collect();
        while let Some(entity) = pending.pop() {
            every.push(entity);
            if self.filtering.is_some() {
                pending.extend(self.child_items(entity).into_iter().rev());
            }
        }
        every
    }

    fn shown_rows(&self) -> Vec<Entity> {
        let roots = self.child_items(self.entity);
        match self.filtering {
            Some(filtering) => flatten_tree(&roots, filtering, |entity| TreeNode {
                children: self.child_items(entity),
                expanded: self
                    .elements
                    .get_entity(entity)
                    .is_some_and(|element| element.expanded),
                hidden: self.is_hidden(entity),
            })
            .into_iter()
            .map(|row| row.entity)
            .collect(),
            None => roots
                .into_iter()
                .filter(|&entity| !self.is_hidden(entity))
                .collect(),
        }
    }

    fn navigable_rows(&self) -> Vec<Entity> {
        self.shown_rows()
            .into_iter()
            .filter(|&entity| !self.is_hidden(entity))
            .collect()
    }
}

pub(crate) fn update_search_fields<'w, 's>(
    mut fields: Query<(
        Entity,
        &UISearchField,
        &UITextInput,
        Option<&mut UISearchState>,
    )>,
    (children, parts, nodes, elements, hidden): (
        Query<'w, 's, &'static Children>,
        Query<'w, 's, &'static UIWidgetPart>,
        Query<'w, 's, &'static UINode>,
        Query<'w, 's, &'static UITreeElement>,
        Query<'w, 's, &'static UIHidden>,
    ),
    (texts, selected, changed): (
        Query<&UISearchText>,
        Query<Entity, With<UISelected>>,
        Query<Entity, Changed<Children>>,
    ),
    (lists, mut trees, mut areas): (
        Query<&UIList>,
        Query<&mut UITree>,
        Query<(&mut UIScrollArea, Option<&UILayout>)>,
    ),
    (mut actions, mut submitted, mut cancelled): (
        EventReader<ActionFired>,
        EventReader<UITextInputSubmitted>,
        EventReader<UITextInputCancelled>,
    ),
    (mut panels, parents): (Query<&mut UIAnchoredPanel>, Query<&ChildOf>),
    focused: Res<FocusedWidget>,
    theme: Res<UITheme>,
    mut cmd: CommandQueue,
) {
    let fired: Vec<ActionFired> = actions.read().cloned().collect();
    let submits: Vec<Entity> = submitted.read().map(|event| event.entity).collect();
    for event in cancelled.read() {
        if fields.contains_entity(event.entity) {
            close_panel(event.entity, &mut panels, &parents);
        }
    }
    for (field, search, input, state) in fields.iter_mut() {
        let Some(mut state) = state else {
            let reveal = selected
                .iter()
                .find(|&entity| is_child_item(entity, search.target, &parents));
            cmd.insert(
                UISearchState {
                    query: None,
                    reveal,
                },
                field,
            );
            continue;
        };
        let filtering = if lists.contains_entity(search.target) {
            None
        } else if let Some(tree) = trees.get_entity_mut(search.target) {
            Some(tree.filtering)
        } else {
            continue;
        };
        let mut target = Target {
            entity: search.target,
            children: &children,
            parts: &parts,
            nodes: &nodes,
            elements: &elements,
            hidden: &hidden,
            overrides: HashMap::new(),
            filtering,
        };
        let query_changed = state.query.as_deref() != Some(input.value.as_str());
        if query_changed || changed.contains_entity(search.target) {
            narrow(&mut target, &input.value, &texts, &mut cmd);
            if let Some(mut tree) = trees.get_entity_mut(search.target) {
                let filtering = !input.value.is_empty();
                if tree.filtering != filtering {
                    tree.filtering = filtering;
                }
                target.filtering = Some(filtering);
            }
        }
        let everything = target.every_item();
        let current = everything
            .iter()
            .copied()
            .find(|&entity| selected.contains_entity(entity));
        if query_changed {
            if let Some((mut area, _)) = areas.get_entity_mut(search.target)
                && area.offset != 0.0
            {
                area.offset = 0.0;
            }
            if state.query.is_some() && !input.value.is_empty() {
                let first = target
                    .navigable_rows()
                    .into_iter()
                    .find(|&entity| texts.contains_entity(entity));
                highlight(first, current, &mut cmd);
                state.reveal = first;
            }
            state.query = Some(input.value.clone());
        }
        if **focused == Some(field) {
            for action in &fired {
                if let Some(next) = step(&target, current, action) {
                    highlight(Some(next), current, &mut cmd);
                    cmd.entity(next).trigger(UIItemSelected);
                    state.reveal = Some(next);
                }
            }
        }
        if submits.contains(&field)
            && let Some(current) = current
            && target.navigable_rows().contains(&current)
        {
            cmd.entity(current).trigger(UIItemActivated);
        }
        if let Some(reveal) = state.reveal {
            let row_height = lists
                .get_entity(search.target)
                .and_then(|list| list.row_height)
                .or_else(|| {
                    trees
                        .get_entity_mut(search.target)
                        .and_then(|tree| tree.row_height)
                })
                .unwrap_or(theme.row_height);
            if let Some((mut area, layout)) = areas.get_entity_mut(search.target)
                && let Some(viewport) = layout
                    .map(|layout| layout.content_rect.size.y)
                    .filter(|viewport| *viewport > 0.0)
            {
                let rows = target.shown_rows();
                if let Some(index) = rows.iter().position(|&row| row == reveal) {
                    scroll_to_row(&mut area, index, rows.len(), row_height, viewport);
                }
                state.reveal = None;
            }
        }
    }
}

fn narrow(target: &mut Target, query: &str, texts: &Query<&UISearchText>, cmd: &mut CommandQueue) {
    let query = query.to_lowercase();
    for entity in target.every_item() {
        let Some(text) = texts.get_entity(entity) else {
            continue;
        };
        let hide = !query.is_empty() && !text.0.to_lowercase().contains(&query);
        let was = target.hidden.contains_entity(entity);
        if hide && !was {
            cmd.insert(UIHidden, entity);
        } else if !hide && was {
            cmd.remove::<UIHidden>(entity);
        }
        target.overrides.insert(entity, hide);
    }
}

fn highlight(next: Option<Entity>, current: Option<Entity>, cmd: &mut CommandQueue) {
    if next == current {
        return;
    }
    match next {
        Some(next) => cmd.insert(UISelected, next),
        None => {
            if let Some(current) = current {
                cmd.remove::<UISelected>(current);
            }
        }
    }
}

fn step(target: &Target, current: Option<Entity>, action: &ActionFired) -> Option<Entity> {
    let forward = if action.is(UIListNext) {
        true
    } else if action.is(UIListPrevious) {
        false
    } else {
        return None;
    };
    let rows = target.navigable_rows();
    let position = current.and_then(|current| rows.iter().position(|&row| row == current));
    let index = match (position, forward) {
        (None, true) => 0,
        (None, false) => rows.len().checked_sub(1)?,
        (Some(position), true) => position + 1,
        (Some(position), false) => position.checked_sub(1)?,
    };
    rows.get(index).copied().filter(|&row| Some(row) != current)
}

fn scroll_to_row(area: &mut UIScrollArea, index: usize, rows: usize, height: f32, viewport: f32) {
    let top = index as f32 * height;
    let bottom = top + height;
    let mut offset = area.offset;
    if top < offset {
        offset = top;
    } else if bottom > offset + viewport {
        offset = bottom - viewport;
    }
    let offset = offset.clamp(0.0, (rows as f32 * height - viewport).max(0.0));
    if area.offset != offset {
        area.offset = offset;
    }
}

fn is_child_item(entity: Entity, target: Entity, parents: &Query<&ChildOf>) -> bool {
    let mut next = parents.get_entity(entity).map(ChildOf::parent);
    while let Some(parent) = next {
        if parent == target {
            return true;
        }
        next = parents.get_entity(parent).map(ChildOf::parent);
    }
    false
}

fn close_panel(field: Entity, panels: &mut Query<&mut UIAnchoredPanel>, parents: &Query<&ChildOf>) {
    let mut next = Some(field);
    while let Some(entity) = next {
        if let Some(mut panel) = panels.get_entity_mut(entity) {
            if panel.open {
                panel.open = false;
            }
            return;
        }
        next = parents.get_entity(entity).map(ChildOf::parent);
    }
}

#[cfg(test)]
mod tests {
    use concerto_ecs::{
        entity::{Entity, hierarchy::ChildOf},
        events::event_channel::EventChannel,
    };
    use concerto_window::input::MouseButton;
    use glam::Vec2;

    use super::*;
    use crate::{
        anchor::UIAnchoredPanel,
        items::{UIHidden, UIListNext},
        list::{
            UIList,
            tests::{Harness, Heard, heard},
        },
        node::{FlexDirection, UINode},
        text_input::{UITextInput, UITextInputCancelled, UITextInputSubmitted},
        transform::UIValue,
        tree::{UITree, UITreeElement},
    };

    const ROW: f32 = 20.0;

    fn column(height: f32) -> UINode {
        UINode::default()
            .with_height(UIValue::Px(height))
            .with_flex_shrink(0.0)
            .with_flex_direction(FlexDirection::Column)
    }

    fn field(harness: &mut Harness, target: Entity, parent: Entity) -> Entity {
        harness.world.spawn((
            UINode::default(),
            UITextInput::new("Search"),
            UISearchField { target },
            ChildOf::new(parent),
        ))
    }

    fn item(harness: &mut Harness, parent: Entity, text: Option<&str>) -> Entity {
        let item = harness
            .world
            .spawn((UINode::default(), ChildOf::new(parent)));
        if let Some(text) = text {
            harness.world.insert(UISearchText(text.into()), item);
        }
        item
    }

    fn type_query(harness: &mut Harness, field: Entity, query: &str) {
        harness
            .world
            .get_component_for_entity_mut::<UITextInput>(field)
            .unwrap()
            .value = query.into();
        harness.settle();
    }

    fn submit(harness: &mut Harness, field: Entity) {
        harness
            .world
            .get_resource_mut::<EventChannel<UITextInputSubmitted>>()
            .unwrap()
            .push_event(UITextInputSubmitted {
                entity: field,
                value: String::new(),
            });
        harness.frame();
    }

    fn hidden(harness: &Harness, rows: &[Entity]) -> Vec<usize> {
        rows.iter()
            .enumerate()
            .filter(|(_, row)| {
                harness
                    .world
                    .get_component_for_entity::<UIHidden>(**row)
                    .is_some()
            })
            .map(|(index, _)| index)
            .collect()
    }

    #[test]
    fn a_search_field_narrows_its_list_and_picks_from_the_keyboard() {
        let mut harness = Harness::new();
        let root = harness.root();
        let panel = harness.world.spawn((
            column(400.0),
            UIAnchoredPanel::default().with_open(true),
            ChildOf::new(root),
        ));
        let list = harness.world.spawn((
            column(60.0),
            UIList {
                row_height: Some(ROW),
                click_activates: true,
                ..Default::default()
            },
        ));
        let search = field(&mut harness, list, panel);
        harness.world.insert(ChildOf::new(panel), list);
        let rows: Vec<Entity> = ["Grass", "Stone", "Moss Stone", "Sand", "Mossy Grass"]
            .into_iter()
            .map(|text| item(&mut harness, list, Some(text)))
            .collect();
        harness.settle();
        for row in &rows {
            harness.listen(*row);
        }
        harness.focus(search);

        type_query(&mut harness, search, "moss");
        assert_eq!(hidden(&harness, &rows), vec![0, 1, 3]);
        assert_eq!(harness.selected(&rows), vec![2]);
        assert!(
            harness.heard().is_empty(),
            "typing selects nothing for the user"
        );

        harness.fire(UIListNext);
        assert_eq!(harness.selected(&rows), vec![4]);
        assert_eq!(harness.heard(), vec![heard(rows[4], Heard::Selected)]);
        assert_eq!(harness.focused(), Some(search), "focus stays in the field");

        submit(&mut harness, search);
        assert_eq!(harness.heard(), vec![heard(rows[4], Heard::Activated)]);

        type_query(&mut harness, search, "STONE");
        assert_eq!(hidden(&harness, &rows), vec![0, 3, 4]);
        assert_eq!(
            harness.selected(&rows),
            vec![1],
            "a new query highlights the first match"
        );

        type_query(&mut harness, search, "lava");
        assert_eq!(hidden(&harness, &rows), vec![0, 1, 2, 3, 4]);
        assert!(harness.selected(&rows).is_empty());
        submit(&mut harness, search);
        assert!(harness.heard().is_empty(), "Enter picks nothing hidden");

        harness.fire(UIListNext);
        type_query(&mut harness, search, "");
        assert!(hidden(&harness, &rows).is_empty());
        harness.fire(UIListNext);
        assert_eq!(
            harness.selected(&rows),
            vec![0],
            "Down with no selection takes the first row"
        );
        harness.heard();

        harness.click(rows[3], MouseButton::Left, Vec2::ZERO);
        assert_eq!(
            harness.heard(),
            vec![
                heard(rows[3], Heard::Selected),
                heard(rows[3], Heard::Activated)
            ]
        );

        harness
            .world
            .get_resource_mut::<EventChannel<UITextInputCancelled>>()
            .unwrap()
            .push_event(UITextInputCancelled { entity: search });
        harness.frame();
        assert!(
            !harness
                .world
                .get_component_for_entity::<UIAnchoredPanel>(panel)
                .unwrap()
                .open,
            "Escape in the field closes its panel"
        );
    }

    #[test]
    fn a_tree_search_shows_the_ancestors_of_a_match_without_highlighting_them() {
        let mut harness = Harness::new();
        let root = harness.root();
        let tree = harness.world.spawn((
            column(200.0),
            UITree {
                row_height: Some(ROW),
                ..Default::default()
            },
            ChildOf::new(root),
        ));
        let search = field(&mut harness, tree, root);
        let none = item(&mut harness, tree, None);
        let player = item(&mut harness, tree, Some("Player"));
        let hand = item(&mut harness, player, Some("Hand"));
        let lamp = item(&mut harness, tree, Some("Lamp"));
        let rows = [none, player, hand, lamp];
        for row in rows {
            harness.world.insert(UITreeElement { expanded: false }, row);
        }
        harness.settle();

        type_query(&mut harness, search, "hand");
        assert_eq!(hidden(&harness, &rows), vec![1, 3]);
        assert!(
            harness
                .world
                .get_component_for_entity::<UITree>(tree)
                .unwrap()
                .filtering
        );
        assert_eq!(harness.selected(&rows), vec![2]);
        let shown: Vec<bool> = rows
            .iter()
            .map(|row| {
                harness
                    .world
                    .get_component_for_entity::<crate::items::UIItem>(*row)
                    .unwrap()
                    .is_shown()
            })
            .collect();
        assert_eq!(shown, vec![true, true, true, false]);
    }
}

//! The inspector's editor of a [`SceneEntityRef`]: a button naming the entity, which opens a menu
//! of the edited scene's entities.
use std::collections::{HashMap, HashSet};

use concerto_app::{App, Plugin, schedule_groups::LateUpdate};
use concerto_ecs::{
    Component, Entity, Query, Res, ResMut,
    command::{CommandQueue, EntityCommandQueue},
    component::{name::Name, scene::SceneEntityRef},
    entity::hierarchy::{ChildOf, Children},
    query::filter::{With, Without},
    signal::{
        On,
        lifecycle::Remove,
        listener::{GlobalListener, IntoListener, Listener},
    },
};
use concerto_ui::elements::{ListItem, Listening, Pressable, Text, prelude::*};
use concerto_ui::{
    anchor::UIAnchoredPanel, interaction::UIClick, items::UIItemActivated, text::UIText,
    theme::UITheme,
};
use concerto_window::input::MouseButton;

use crate::asset_editor::ActiveEditor;
use crate::asset_picker::{picker_button, picker_caret, picker_label};
use crate::fonts::{button_icon, glyph, icon_button};
use crate::hierarchy::{HierarchyState, ListRoot, ListedEntity, listed_entities};
use crate::hosted::{EditTarget, EditWorld, EditWorldSignals, PlayState};
use crate::inspector::{
    EditContext, EditError, PropertyCommits, PropertyEditor, PropertyRow, PropertyRowValue,
};
use crate::picker_menu::{picker_panel, picker_tree, show_picker_menu};
use crate::tools::{scene::SceneRoot, selection::Select};

const NOT_IN_SCENE: &str = "(not in this scene)";

/// Picks the entity a [`SceneEntityRef`] names from the entities of the scene being edited.
#[derive(Default)]
pub struct EntityPicker;

/// Keeps every entity picker row naming the entity it holds, and runs the menu the rows share.
pub struct EntityPickerPlugin;

impl Plugin for EntityPickerPlugin {
    fn build(&self, app: &mut App) {
        app.add_system(LateUpdate, refresh_entity_picker_labels)
            .add_listener(forget_scene);
    }
}

/// The text inside an [`EntityPicker`] row's button, naming the row's entity.
#[derive(Component)]
pub(crate) struct EntityPickerLabel {
    row: Entity,
}

pub(crate) fn register_defaults(registry: &mut crate::inspector::InspectorRegistry) {
    registry.register_property_editor::<SceneEntityRef, EntityPicker>();
}

impl PropertyEditor<SceneEntityRef> for EntityPicker {
    type Snapshot = SceneEntityRef;
    type Edit = SceneEntityRef;

    fn snapshot(&self, value: &SceneEntityRef) -> SceneEntityRef {
        *value
    }

    fn build(&self, row: &mut EntityCommandQueue, _: &SceneEntityRef, theme: &UITheme) {
        let entity = row.entity();
        row.add_child_with(
            picker_button(theme).on_click(toggle_entity_picker(entity)),
            |mut button| {
                button
                    .add_child((picker_label(theme), EntityPickerLabel { row: entity }))
                    .add_child(picker_caret(theme));
            },
        )
        .add_child(select_button(theme, entity));
    }

    fn apply(
        &self,
        value: &mut SceneEntityRef,
        edit: &SceneEntityRef,
        _: &EditContext,
    ) -> Result<(), EditError> {
        if edit.entity().is_none() && *edit != SceneEntityRef::default() {
            return Err(EditError::Rejected);
        }
        *value = *edit;
        Ok(())
    }

    fn edit_to(&self, value: &SceneEntityRef) -> Option<SceneEntityRef> {
        Some(*value)
    }

    fn follows_snapshot(&self) -> bool {
        true
    }
}

fn select_button(theme: &UITheme, row: Entity) -> (Listening<Pressable, UIClick>, Text) {
    (
        icon_button(theme).on_click(select_referenced(row)),
        button_icon(theme, glyph::CROSSHAIR),
    )
}

/// The menu every entity picker shares.
#[derive(Component)]
pub struct EntityPickerMenu;

fn held(values: &Query<&PropertyRowValue>, row: Entity) -> Option<SceneEntityRef> {
    values
        .get_entity(row)
        .and_then(|value| value.downcast::<SceneEntityRef>().copied())
}

fn toggle_entity_picker(row: Entity) -> Listener<UIClick> {
    (move |on: On<UIClick>,
           values: Query<&PropertyRowValue>,
           mut menus: Query<(Entity, &mut UIAnchoredPanel), With<EntityPickerMenu>>,
           roots: EditWorld<Query<(Entity, &SceneRoot)>>,
           top_level: EditWorld<Query<Entity, (Without<ChildOf>, Without<GlobalListener>)>>,
           children: EditWorld<Query<&Children>>,
           names: EditWorld<Query<Option<&Name>>>,
           active: Res<ActiveEditor>,
           states: Query<&PlayState>,
           theme: Res<UITheme>,
           mut cmd: CommandQueue| {
        if on.signal().button != MouseButton::Left {
            return;
        }
        let button = on.entity();
        let mut menu = None;
        if let Some((entity, mut panel)) = menus.iter_mut().next() {
            if panel.open && panel.owner == Some(button) {
                panel.open = false;
                return;
            }
            menu = Some(entity);
        }
        let candidates = listed_entities(
            &roots,
            &top_level,
            &children,
            &active,
            &states,
            ListRoot::Exclude,
        );
        let current = held(&values, row).unwrap_or_default();
        let none = SceneEntityRef::default();
        let list = picker_tree(&theme)
            .searchable("Search entities…")
            .item(
                theme
                    .tree_item("None")
                    .always_shown()
                    .selected(current == none)
                    .on_activate(pick_entity(row, none)),
            )
            .items(entity_items(row, current, &candidates, &names, &theme));
        show_picker_menu(
            menu,
            EntityPickerMenu,
            list,
            |field| picker_panel(button, field),
            &theme,
            &mut cmd,
        );
    })
    .into_listener()
}

fn entity_items(
    row: Entity,
    current: SceneEntityRef,
    candidates: &[ListedEntity],
    names: &Query<Option<&Name>>,
    theme: &UITheme,
) -> Vec<ListItem> {
    let listed: HashSet<Entity> = candidates.iter().map(|listed| listed.entity).collect();
    let mut children: HashMap<Option<Entity>, Vec<Entity>> = HashMap::new();
    for candidate in candidates {
        let parent = candidate.parent.filter(|parent| listed.contains(parent));
        children.entry(parent).or_default().push(candidate.entity);
    }
    let item = |entity: Entity| {
        let value = SceneEntityRef::Entity(entity);
        theme
            .tree_item(entity_label(entity, names.get_entity(entity).flatten()))
            .selected(current == value)
            .on_activate(pick_entity(row, value))
    };
    nest(None, &children, &item)
}

fn nest(
    parent: Option<Entity>,
    children: &HashMap<Option<Entity>, Vec<Entity>>,
    item: &impl Fn(Entity) -> ListItem,
) -> Vec<ListItem> {
    children
        .get(&parent)
        .into_iter()
        .flatten()
        .map(|&entity| item(entity).children(nest(Some(entity), children, item)))
        .collect()
}

fn pick_entity(row: Entity, value: SceneEntityRef) -> Listener<UIItemActivated> {
    (move |_: On<UIItemActivated>,
           mut menus: Query<&mut UIAnchoredPanel, With<EntityPickerMenu>>,
           rows: Query<(&PropertyRow, &PropertyRowValue)>,
           mut commits: ResMut<PropertyCommits>| {
        if let Some((property, held)) = rows.get_entity(row)
            && held.downcast::<SceneEntityRef>() != Some(&value)
            && let Err(error) = commits.push::<SceneEntityRef, EntityPicker>(property, value)
        {
            log::warn!("Entity pick dropped: {error}");
        }
        for mut panel in menus.iter_mut() {
            panel.open = false;
        }
    })
    .into_listener()
}

fn select_referenced(row: Entity) -> Listener<UIClick> {
    (move |on: On<UIClick>,
           values: Query<&PropertyRowValue>,
           roots: EditWorld<Query<(Entity, &SceneRoot)>>,
           top_level: EditWorld<Query<Entity, (Without<ChildOf>, Without<GlobalListener>)>>,
           children: EditWorld<Query<&Children>>,
           active: Res<ActiveEditor>,
           states: Query<&PlayState>,
           mut hierarchy: ResMut<HierarchyState>,
           mut scene: EditWorldSignals| {
        if on.signal().button != MouseButton::Left {
            return;
        }
        let Some(entity) = held(&values, row).and_then(|value| value.entity()) else {
            return;
        };
        let listed = listed_entities(
            &roots,
            &top_level,
            &children,
            &active,
            &states,
            ListRoot::Exclude,
        )
        .iter()
        .any(|listed| listed.entity == entity);
        if listed {
            scene.trigger(Select(Some(entity)));
            hierarchy.reveal_entity(entity);
        }
    })
    .into_listener()
}

fn forget_scene(
    _: On<Remove<EditTarget>>,
    mut menus: Query<&mut UIAnchoredPanel, With<EntityPickerMenu>>,
) {
    for mut panel in menus.iter_mut() {
        if panel.open {
            panel.open = false;
        }
    }
}

fn entity_label(entity: Entity, name: Option<&Name>) -> String {
    match name {
        Some(name) if !name.as_str().is_empty() => name.as_str().to_owned(),
        _ => format!("Entity {}", entity.index()),
    }
}

/// Writes the name of each picker row's entity into its label.
pub(crate) fn refresh_entity_picker_labels(
    values: Query<&PropertyRowValue>,
    mut labels: Query<(&EntityPickerLabel, &mut UIText)>,
    names: EditWorld<Query<Option<&Name>>>,
    roots: EditWorld<Query<(Entity, &SceneRoot)>>,
    top_level: EditWorld<Query<Entity, (Without<ChildOf>, Without<GlobalListener>)>>,
    children: EditWorld<Query<&Children>>,
    active: Res<ActiveEditor>,
    states: Query<&PlayState>,
) {
    if labels.iter_mut().next().is_none() {
        return;
    }
    let listed: HashSet<Entity> = listed_entities(
        &roots,
        &top_level,
        &children,
        &active,
        &states,
        ListRoot::Exclude,
    )
    .into_iter()
    .map(|listed| listed.entity)
    .collect();
    for (label, mut text) in labels.iter_mut() {
        let Some(reference) = held(&values, label.row) else {
            continue;
        };
        let wanted = match reference.entity() {
            Some(entity) if listed.contains(&entity) => {
                entity_label(entity, names.get_entity(entity).flatten())
            }
            _ => NOT_IN_SCENE.to_owned(),
        };
        if text.text != wanted {
            text.text = wanted;
        }
    }
}

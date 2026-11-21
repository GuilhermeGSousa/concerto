use std::collections::{HashMap, HashSet};

use concerto_app::{App, Plugin, schedule_groups::LateUpdate};
use concerto_ecs::{
    Component, Entity, Query, Res, ResMut,
    command::{CommandQueue, EntityCommandQueue},
    component::{name::Name, scene::SceneEntityRef},
    entity::hierarchy::{ChildOf, Children},
    events::event_reader::EventReader,
    query::filter::{With, Without},
    signal::{
        On,
        lifecycle::Remove,
        listener::{GlobalListener, IntoListener, Listener},
    },
};
use concerto_ui::elements::{Listening, Pressable, Text, prelude::*};
use concerto_ui::{
    anchor::UIAnchoredPanel,
    interaction::UIClick,
    node::{UINode, UIRect},
    scroll::UIScrollArea,
    text::UIText,
    text_input::UITextInputChanged,
    theme::UITheme,
    transform::UIValue,
};
use concerto_window::input::MouseButton;

use crate::asset_editor::ActiveEditor;
use crate::asset_picker::{
    ITEM_HEIGHT, LIST_MAX_HEIGHT, MENU_WIDTH, SCROLLBAR_GUTTER, menu_panel, picker_button,
    picker_caret, picker_label,
};
use crate::fonts::{button_icon, glyph, icon_button};
use crate::hierarchy::{HierarchyState, ListRoot, ListedEntity, listed_entities};
use crate::hosted::{EditTarget, EditWorld, EditWorldSignals, PlayState};
use crate::inspector::{
    EditContext, EditError, PropertyCommits, PropertyEditor, PropertyRow, PropertyRowValue,
};
use crate::tools::{scene::SceneRoot, selection::Select};

const NOT_IN_SCENE: &str = "(not in this scene)";
const INDENT: f32 = 14.0;

/// Picks the entity a [`SceneEntityRef`] names from the entities of the scene being edited.
#[derive(Default)]
pub struct EntityPicker;

/// Keeps every entity picker row naming the entity it holds, and runs the menu the rows share.
pub struct EntityPickerPlugin;

impl Plugin for EntityPickerPlugin {
    fn build(&self, app: &mut App) {
        app.add_system(LateUpdate, refresh_entity_picker_labels)
            .add_system(LateUpdate, filter_entity_picker)
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

/// The menu every entity picker shares, holding the text its filter field narrows the list by.
#[derive(Component)]
pub struct EntityPickerMenu {
    filter: String,
}

#[derive(Component)]
struct EntityPickerFilter;

#[derive(Component)]
struct EntityPickerView;

#[derive(Component, Clone)]
struct EntityPickerItem {
    entity: Option<Entity>,
    parent: Option<Entity>,
    label: String,
}

fn held(values: &Query<&PropertyRowValue>, row: Entity) -> Option<SceneEntityRef> {
    values
        .get_entity(row)
        .and_then(|value| value.downcast::<SceneEntityRef>().copied())
}

fn toggle_entity_picker(row: Entity) -> Listener<UIClick> {
    (move |on: On<UIClick>,
           values: Query<&PropertyRowValue>,
           menus: Query<(Entity, &mut EntityPickerMenu, &mut UIAnchoredPanel)>,
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
        let mut menu = match menus.iter().next() {
            Some((entity, mut menu, mut panel)) => {
                if panel.open && panel.owner == Some(button) {
                    panel.open = false;
                    return;
                }
                if !menu.filter.is_empty() {
                    menu.filter.clear();
                }
                **panel = menu_panel(button);
                cmd.entity(entity).despawn_children()
            }
            None => cmd.spawn((
                theme
                    .popup()
                    .width(UIValue::Px(MENU_WIDTH))
                    .padding(theme.spacing_xs)
                    .gap(theme.spacing_xs),
                menu_panel(button),
                EntityPickerMenu {
                    filter: String::new(),
                },
            )),
        };
        let candidates = listed_entities(
            &roots,
            &top_level,
            &children,
            &active,
            &states,
            ListRoot::Exclude,
        );
        let current = held(&values, row).unwrap_or_default();
        fill_menu(&mut menu, row, current, &candidates, &names, &theme);
    })
    .into_listener()
}

fn fill_menu(
    menu: &mut EntityCommandQueue,
    row: Entity,
    current: SceneEntityRef,
    candidates: &[ListedEntity],
    names: &Query<Option<&Name>>,
    theme: &UITheme,
) {
    menu.add_child((theme.text_field("Search entities…"), EntityPickerFilter));
    let items: Vec<EntityPickerItem> = std::iter::once(EntityPickerItem {
        entity: None,
        parent: None,
        label: "None".into(),
    })
    .chain(candidates.iter().map(|listed| EntityPickerItem {
        entity: Some(listed.entity),
        parent: listed.parent,
        label: entity_label(listed.entity, names.get_entity(listed.entity).flatten()),
    }))
    .collect();
    let depths: Vec<usize> = std::iter::once(0)
        .chain(candidates.iter().map(|listed| listed.depth))
        .collect();
    let extent = items.len() as f32 * ITEM_HEIGHT;
    menu.add_child_with(
        (
            theme
                .column()
                .fixed()
                .height(UIValue::Px(extent.min(LIST_MAX_HEIGHT)))
                .clipped(),
            EntityPickerView,
        ),
        |mut view| {
            let mut list = view.spawn_child(theme.column().gap(0.0).padding(UIRect {
                right: SCROLLBAR_GUTTER,
                ..UIRect::all(0.0)
            }));
            for (item, depth) in items.iter().zip(&depths) {
                let value = item
                    .entity
                    .map_or_else(SceneEntityRef::default, SceneEntityRef::Entity);
                let label = item.label.clone();
                list.add_child_with(
                    (
                        theme
                            .pressable()
                            .selected(current == value)
                            .row()
                            .fixed()
                            .height(UIValue::Px(ITEM_HEIGHT))
                            .padding(UIRect {
                                left: 8.0 + *depth as f32 * INDENT,
                                right: 8.0,
                                ..UIRect::all(0.0)
                            })
                            .on_click(pick_entity(row, value)),
                        item.clone(),
                    ),
                    |mut item| {
                        item.add_child(
                            theme
                                .label(label.clone())
                                .font_size(12.5)
                                .single_line()
                                .grow()
                                .shrink(1.0)
                                .min_width(UIValue::Px(0.0)),
                        );
                    },
                );
            }
            let list = list.entity();
            view.insert(UIScrollArea {
                content: Some(list),
                content_extent: extent,
                ..Default::default()
            });
        },
    );
}

fn pick_entity(row: Entity, value: SceneEntityRef) -> Listener<UIClick> {
    (move |on: On<UIClick>,
           menus: Query<&mut UIAnchoredPanel, With<EntityPickerMenu>>,
           rows: Query<(&PropertyRow, &PropertyRowValue)>,
           mut commits: ResMut<PropertyCommits>| {
        if on.signal().button != MouseButton::Left {
            return;
        }
        if let Some((property, held)) = rows.get_entity(row)
            && held.downcast::<SceneEntityRef>() != Some(&value)
            && let Err(error) = commits.push::<SceneEntityRef, EntityPicker>(property, value)
        {
            log::warn!("Entity pick dropped: {error}");
        }
        for mut panel in menus.iter() {
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

fn filter_entity_picker(
    mut events: EventReader<UITextInputChanged>,
    filters: Query<&EntityPickerFilter>,
    menus: Query<&mut EntityPickerMenu>,
    items: Query<(&EntityPickerItem, &mut UINode)>,
    views: Query<
        (&mut UIScrollArea, &mut UINode),
        (With<EntityPickerView>, Without<EntityPickerItem>),
    >,
) {
    let Some(text) = events
        .read()
        .filter(|event| filters.get_entity(event.entity).is_some())
        .last()
        .map(|event| event.value.clone())
    else {
        return;
    };
    for mut menu in menus.iter() {
        if menu.filter != text {
            menu.filter = text.clone();
        }
    }
    let filter = text.to_lowercase();
    let parents: HashMap<Entity, Option<Entity>> = items
        .iter()
        .filter_map(|(item, _)| item.entity.map(|entity| (entity, item.parent)))
        .collect();
    let mut shown = HashSet::new();
    for (item, _) in items.iter() {
        let Some(entity) = item.entity else {
            continue;
        };
        if !item.label.to_lowercase().contains(&filter) {
            continue;
        }
        let mut next = Some(entity);
        while let Some(entity) = next {
            if !shown.insert(entity) {
                break;
            }
            next = parents.get(&entity).copied().flatten();
        }
    }
    let mut count = 0;
    for (item, mut node) in items.iter() {
        let visible = item.entity.is_none_or(|entity| shown.contains(&entity));
        count += usize::from(visible);
        if node.visible != visible {
            node.visible = visible;
        }
    }
    let extent = count as f32 * ITEM_HEIGHT;
    let height = UIValue::Px(extent.min(LIST_MAX_HEIGHT));
    for (mut area, mut node) in views.iter() {
        if area.content_extent != extent || area.offset != 0.0 {
            area.content_extent = extent;
            area.offset = 0.0;
        }
        if node.height != height {
            node.height = height;
        }
    }
}

fn forget_scene(
    _: On<Remove<EditTarget>>,
    menus: Query<&mut UIAnchoredPanel, With<EntityPickerMenu>>,
) {
    for mut panel in menus.iter() {
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
    labels: Query<(&EntityPickerLabel, &mut UIText)>,
    names: EditWorld<Query<Option<&Name>>>,
    roots: EditWorld<Query<(Entity, &SceneRoot)>>,
    top_level: EditWorld<Query<Entity, (Without<ChildOf>, Without<GlobalListener>)>>,
    children: EditWorld<Query<&Children>>,
    active: Res<ActiveEditor>,
    states: Query<&PlayState>,
) {
    if labels.iter().next().is_none() {
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
    for (label, mut text) in labels.iter() {
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

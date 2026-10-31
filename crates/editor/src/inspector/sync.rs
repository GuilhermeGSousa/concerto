//! Reconciles the live target with card/row entities.
use super::*;

use super::add_component::{AddComponentMenu, spawn_add_component};
use super::component_menu::{ComponentMenu, ensure_component_menu, menu_button};
use super::list::remove_button;
use super::registry::InspectionSource;
use concerto_ecs::{component::Tick, reflect::AppTypeRegistry};
use concerto_reflect::PathSegment;
use std::collections::HashMap;

/// A component card in the inspector's UI hierarchy.
#[derive(Component, Clone, Copy)]
pub struct InspectedComponent {
    /// The entity the card shows a component of, in the edit `App`'s world.
    pub scene_entity: Entity,
    pub type_id: TypeId,
    pub name: &'static str,
    body: Entity,
    read: bool,
}

#[derive(Component)]
pub(super) struct BuildPropertyWidget {
    remove: Option<(Entity, usize)>,
}

pub(super) fn forget_scene(
    _: On<Remove<EditTarget>>,
    stacks: Query<(Entity, &ComponentStack, Option<&Children>)>,
    mut cmd: CommandQueue,
) {
    for (stack, _, children) in stacks.iter() {
        if children.is_some() {
            cmd.entity(stack).despawn_children();
        }
        cmd.insert(ComponentStack::default(), stack);
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn sync_inspected_components(
    source: EditWorld<InspectionSource>,
    registry: Res<InspectorRegistry>,
    data: Res<InspectorData>,
    theme: Res<UITheme>,
    stacks: Query<(Entity, &ComponentStack, Option<&Children>)>,
    cards: Query<&InspectedComponent>,
    bodies: Query<&Children>,
    rows: Query<(&PropertyRow, Option<&PropertyRowValue>)>,
    menus: Query<(Entity, &AddComponentMenu)>,
    component_menus: Query<&ComponentMenu>,
    mut cmd: CommandQueue,
) {
    let Some((stack_entity, stack, children)) = stacks.iter().next() else {
        return;
    };
    let target = data
        .scene_entity
        .filter(|&entity| source.entity_is_valid(entity));
    let structural_version = target.and_then(|entity| source.structural_version(entity));
    let registry_tick = Some(registry.changed_tick());
    let rebuild = stack.scene_target != target
        || stack.structural_version != structural_version
        || stack.registry_tick != registry_tick;
    if rebuild {
        if children.is_some() {
            cmd.entity(stack_entity).despawn_children();
        }
        cmd.insert(
            ComponentStack {
                scene_target: target,
                structural_version,
                registry_tick,
            },
            stack_entity,
        );
    }
    let Some(target) = target else {
        return;
    };

    if rebuild {
        let mut components = source.visible_components(target);
        components.sort_by(|(a_id, a), (b_id, b)| a.cmp(b).then(a_id.cmp(b_id)));
        for (type_id, name) in components {
            let removable = source.is_registered(type_id);
            let (entity, card) = spawn_card(
                &mut cmd,
                stack_entity,
                target,
                type_id,
                name,
                removable,
                &theme,
            );
            refresh_card(
                &source, &registry, &mut cmd, entity, card, &theme, &bodies, &rows,
            );
        }

        spawn_add_component(&mut cmd, stack_entity, &menus, &theme);
        ensure_component_menu(&mut cmd, &component_menus, &theme);
    } else {
        for &entity in children.into_iter().flat_map(|children| children.iter()) {
            if let Some(card) = cards.get_entity(entity) {
                refresh_card(
                    &source, &registry, &mut cmd, entity, *card, &theme, &bodies, &rows,
                );
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn refresh_card(
    source: &InspectionSource,
    registry: &Res<InspectorRegistry>,
    cmd: &mut CommandQueue,
    entity: Entity,
    mut card: InspectedComponent,
    theme: &UITheme,
    bodies: &Query<&Children>,
    rows: &Query<(&PropertyRow, Option<&PropertyRowValue>)>,
) {
    if card.read && !source.is_component_changed(card.scene_entity, card.type_id) {
        return;
    }
    let registry_tick = registry.changed_tick();
    let properties = source
        .collect_component(registry, card.scene_entity, card.type_id)
        .unwrap_or_default();
    let kept = card
        .read
        .then(|| {
            kept_rows(
                source,
                registry,
                registry_tick,
                card.body,
                &properties,
                bodies,
                rows,
            )
        })
        .flatten();
    match kept {
        Some(kept) => {
            for (row, property) in kept.into_iter().zip(properties) {
                if let (Some(row), PropertyKind::Value(value)) = (row, property.kind) {
                    cmd.insert(value, row);
                }
            }
        }
        None => {
            if card.read {
                cmd.entity(card.body).despawn_children();
            }
            let mut body = body_node(theme);
            body.visible = !properties.is_empty();
            cmd.insert(body, card.body);
            let mut lists = HashMap::new();
            for property in properties {
                let node = source.row_node(registry, &property, theme);
                let remove = list_element(&property.path)
                    .and_then(|(list, index)| Some((*lists.get(&list)?, index)));
                let path = property.list_length().map(|_| property.path.clone());
                let row = spawn_row(cmd, &card, property, registry_tick, node, remove, theme);
                if let Some(path) = path {
                    lists.insert(path, row);
                }
            }
        }
    }
    card.read = true;
    cmd.insert(card, entity);
}

/// The list a path's last step goes into, with the index it takes there.
fn list_element(path: &PropertyPath) -> Option<(PropertyPath, usize)> {
    let (&PathSegment::Index(index), list) = path.segments().split_last()? else {
        return None;
    };
    Some((PropertyPath::new(list.iter().copied()), index))
}

fn kept_rows(
    source: &InspectionSource,
    registry: &InspectorRegistry,
    registry_tick: Tick,
    body: Entity,
    properties: &[Property],
    bodies: &Query<&Children>,
    rows: &Query<(&PropertyRow, Option<&PropertyRowValue>)>,
) -> Option<Vec<Option<Entity>>> {
    let existing: Vec<Entity> = bodies
        .get_entity(body)
        .map(|children| children.iter().copied().collect())
        .unwrap_or_default();
    if existing.len() != properties.len() {
        return None;
    }
    existing
        .into_iter()
        .zip(properties)
        .map(|(entity, property)| {
            let (row, shown) = rows.get_entity(entity)?;
            let same_row = row.path == property.path
                && row.type_id == property.type_id
                && row.registry_tick == registry_tick;
            match (shown, property.value()) {
                _ if !same_row => None,
                (None, None) => Some(None),
                (Some(shown), Some(value)) if shown == value => Some(None),
                (Some(_), Some(_)) => source
                    .follows_snapshot(registry, row)
                    .then_some(Some(entity)),
                _ => None,
            }
        })
        .collect()
}

fn spawn_card(
    cmd: &mut CommandQueue,
    stack: Entity,
    target: Entity,
    type_id: TypeId,
    name: &'static str,
    removable: bool,
    theme: &UITheme,
) -> (Entity, InspectedComponent) {
    let mut stack_queue = cmd.entity(stack);
    let mut card_queue = stack_queue.spawn_child(
        theme
            .card()
            .column()
            .fixed()
            .padding(UIRect::axes(theme.spacing_xs + 2.0, theme.spacing_sm)),
    );
    let entity = card_queue.entity();

    card_queue.add_child_with(
        theme.row().fixed().gap(theme.spacing_xs + 2.0),
        |mut header| {
            header.add_child(theme.label(name).single_line().grow());
            if removable {
                header.add_child(menu_button(theme, target, type_id));
            }
            header.add_child((
                theme
                    .canvas()
                    .fill(theme.accent)
                    .radius(6.5)
                    .size(UIValue::Px(22.0), UIValue::Px(13.0))
                    .fixed(),
                UIDisabled,
            ));
        },
    );

    let body = card_queue.spawn_child(body_node(theme)).entity();

    let card = InspectedComponent {
        scene_entity: target,
        type_id,
        name,
        body,
        read: false,
    };
    card_queue.insert(card);
    (entity, card)
}

fn spawn_row(
    cmd: &mut CommandQueue,
    card: &InspectedComponent,
    property: Property,
    registry_tick: Tick,
    mut row_node: UINode,
    remove: Option<(Entity, usize)>,
    theme: &UITheme,
) -> Entity {
    if remove.is_some() {
        row_node.min_height = UIValue::Px(theme.control_height);
        if row_node.flex_wrap == concerto_ui::node::FlexWrap::NoWrap {
            row_node.padding.right = theme.control_height + theme.spacing_xs;
        }
    }
    let target = property.row(card.scene_entity, card.type_id, registry_tick);
    let label = if property.path.depth() == 0 {
        card.name.to_string()
    } else {
        label_for(&property.path)
    };
    let mut body_queue = cmd.entity(card.body);
    let mut row_queue = body_queue.spawn_child((row_node, target, BuildPropertyWidget { remove }));
    let row = row_queue.entity();
    row_queue.add_child(
        theme
            .label(label)
            .small()
            .muted()
            .single_line()
            .width(UIValue::Px(PROPERTY_LABEL_WIDTH))
            .fixed(),
    );
    match property.kind {
        PropertyKind::Value(value) => {
            row_queue.insert(value);
        }
        PropertyKind::Heading => {}
        PropertyKind::Unsupported => {
            row_queue.add_child(theme.label("Unsupported type"));
        }
    }
    row
}

/// The layout node every property row starts from.
pub fn standard_row_node(theme: &UITheme) -> UINode {
    theme.row().fixed().gap(theme.spacing_xs).into_bundle()
}

fn body_node(theme: &UITheme) -> UINode {
    UINode::default()
        .with_visible(false)
        .with_flex_shrink(0.0)
        .with_flex_direction(FlexDirection::Column)
        .with_gap(glam::Vec2::new(0.0, theme.spacing_xs))
        .with_padding(UIRect {
            top: theme.spacing_xs,
            ..Default::default()
        })
}

pub(super) fn build_property_widgets(
    rows: Query<(
        Entity,
        &PropertyRow,
        Option<&PropertyRowValue>,
        &BuildPropertyWidget,
    )>,
    registry: Res<InspectorRegistry>,
    types: EditWorld<Option<Res<AppTypeRegistry>>>,
    theme: Res<UITheme>,
    mut cmd: CommandQueue,
) {
    let types = types.as_deref().map(|types| types.read());
    let types = types.as_deref();
    for (entity, row, value, build) in rows.iter() {
        let editor = registry
            .editor(row.type_id, types)
            .filter(|editor| Some(editor.editor_type) == row.editor_type);
        if let (Some(editor), Some(value)) = (editor, value)
            && let Err(error) = editor.adapter.build(&mut cmd, entity, value, &theme)
        {
            log::warn!("Unable to build property widget: {error}");
        }
        if let Some((list, index)) = build.remove {
            cmd.entity(entity)
                .add_child(remove_button(&theme, list, index));
        }
        cmd.remove::<BuildPropertyWidget>(entity);
    }
}

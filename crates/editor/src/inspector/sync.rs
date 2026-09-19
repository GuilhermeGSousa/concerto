//! Reconciles the live target with card/row entities. Snapshots only live on rows;
//! collection creates temporary values which are moved into those components.
use super::*;

use ecs::{component::Tick, entity::hierarchy::ChildOf, query::filter::With, World};

/// A component card in the inspector's UI hierarchy. Query this component to
/// discover which live world component a card inspects. Its child property rows
/// own the snapshots and widget state.
#[derive(Component, Clone, Copy)]
pub struct InspectedComponent {
    pub entity: Entity,
    pub type_id: TypeId,
    pub name: &'static str,
    body: Entity,
    last_read_tick: Option<Tick>,
    registry_tick: Option<Tick>,
}

/// Widget creation is deferred to a regular system with a CommandQueue, keeping
/// the adapter API independent of the read-only reflection pass.
#[derive(Component)]
pub(super) struct BuildPropertyWidget;

/// World access is read-only because component discovery and snapshot reads
/// use runtime type IDs. Every presentation change is a deferred ECS command.
pub(super) fn sync_inspected_components(
    world: &World,
    data: Res<InspectorData>,
    registry: Res<InspectorRegistry>,
    theme: Res<UITheme>,
    stacks: Query<Entity, With<ComponentStack>>,
    cards: Query<(Entity, &InspectedComponent, Option<&ChildOf>)>,
    mut cmd: CommandQueue,
) {
    let target = data.entity.filter(|&entity| world.entity_is_valid(entity));
    let stack = stacks.iter().next();
    let cards: Vec<_> = cards
        .iter()
        .map(|(entity, card, parent)| (entity, *card, parent.map(ChildOf::parent)))
        .collect();

    // Descriptors are temporary discovery data, never another persistent model.
    let registry_tick = Some(registry.changed_tick());
    let mut desired: Vec<_> = target
        .into_iter()
        .flat_map(|entity| world.component_ids(entity))
        .filter_map(|&type_id| {
            let name = registry
                .component(type_id)
                .map(|c| c.name)
                .or_else(|| world.type_info(type_id).map(|info| info.short()))?;
            Some((type_id, name))
        })
        .collect();
    desired.sort_by(|(a_id, a), (b_id, b)| a.cmp(b).then(a_id.cmp(b_id)));

    let mut retained = Vec::new();
    for (entity, card, parent) in cards {
        if stack.is_none()
            || parent != stack
            || Some(card.entity) != target
            || !desired.iter().any(|(type_id, _)| *type_id == card.type_id)
        {
            cmd.despawn(entity);
        } else {
            retained.push((entity, card));
        }
    }
    let (Some(stack), Some(target)) = (stack, target) else {
        return;
    };
    let mut ordered = Vec::with_capacity(desired.len());
    for (type_id, name) in desired {
        let (entity, mut card) = retained
            .iter()
            .find(|(_, card)| card.type_id == type_id)
            .copied()
            .unwrap_or_else(|| spawn_card(&mut cmd, stack, target, type_id, name, &theme));
        ordered.push(entity);
        if card.last_read_tick.is_none()
            || card.registry_tick != registry_tick
            || card
                .last_read_tick
                .is_some_and(|tick| world.has_component_changed_since(target, type_id, tick))
        {
            let properties = registry
                .collect_component(world, target, type_id)
                .unwrap_or_default();
            reconcile_rows(world, &mut cmd, &card, properties, &theme);
            card.last_read_tick = Some(world.current_tick());
            card.registry_tick = registry_tick;
            cmd.insert(card, entity);
        }
    }
    order_children(world, &mut cmd, stack, ordered);
}

fn child_entities(world: &World, parent: Entity) -> Vec<Entity> {
    world
        .get_component_for_entity::<Children>(parent)
        .map(|children| children.iter().copied().collect())
        .unwrap_or_default()
}

/// A one-pass ordering request. Applied after deferred spawns/despawns so new
/// children participate in the same ordering as retained widgets.
#[derive(Component)]
pub(super) struct PendingChildOrder(Vec<Entity>);

fn order_children(world: &World, cmd: &mut CommandQueue, parent: Entity, ordered: Vec<Entity>) {
    if !child_entities(world, parent)
        .iter()
        .copied()
        .eq(ordered.iter().copied())
    {
        cmd.insert(PendingChildOrder(ordered), parent);
    }
}

pub(super) fn order_inspector_children(
    parents: Query<(Entity, &PendingChildOrder, Option<&mut Children>)>,
    mut cmd: CommandQueue,
) {
    for (entity, order, children) in parents.iter() {
        if let Some(mut children) = children {
            children.sort_by_key(|child| {
                order
                    .0
                    .iter()
                    .position(|&e| e == child)
                    .unwrap_or(usize::MAX)
            });
        }
        cmd.remove::<PendingChildOrder>(entity);
    }
}

fn reconcile_rows(
    world: &World,
    cmd: &mut CommandQueue,
    card: &InspectedComponent,
    properties: Vec<Property>,
    theme: &UITheme,
) {
    let visible = !properties.is_empty();
    if world
        .get_component_for_entity::<UINode>(card.body)
        .is_none_or(|node| node.visible != visible)
    {
        let mut node = world
            .get_component_for_entity::<UINode>(card.body)
            .cloned()
            .unwrap_or_else(|| body_node(theme));
        node.visible = visible;
        cmd.insert(node, card.body);
    }
    let mut existing = child_entities(world, card.body);
    let mut ordered = Vec::with_capacity(properties.len());
    for property in properties {
        let matching = existing.iter().position(|&entity| {
            world
                .get_component_for_entity::<PropertyRow>(entity)
                .is_some_and(|row| {
                    row.entity == card.entity
                        && row.component == card.type_id
                        && row.path == property.path
                        && row.type_id == property.type_id
                        && row.editor_type == property.editor_type
                        && row.registry_tick == property.registry_tick
                })
        });
        let row = if let Some(index) = matching {
            let row = existing.swap_remove(index);
            if world.get_component_for_entity::<PropertyRowValue>(row) != Some(&property.value) {
                cmd.insert(property.value, row);
            }
            row
        } else {
            spawn_row(cmd, card, property, theme)
        };
        ordered.push(row);
    }
    for row in existing {
        cmd.despawn(row);
    }
    order_children(world, cmd, card.body, ordered);
}

fn spawn_card(
    cmd: &mut CommandQueue,
    stack: Entity,
    target: Entity,
    type_id: TypeId,
    name: &'static str,
    theme: &UITheme,
) -> (Entity, InspectedComponent) {
    let entity = cmd
        .spawn((
            UINode {
                flex_shrink: 0.0,
                flex_direction: FlexDirection::Column,
                padding: UIRect::axes(theme.spacing_xs + 2.0, theme.spacing_sm),
                ..Default::default()
            },
            UIMaterial {
                corner_radius: theme.radius_md,
                ..UIMaterial::flat(theme.surface_raised)
            },
        ))
        .entity();
    cmd.add_child(stack, entity);
    let header = cmd
        .spawn(UINode {
            flex_shrink: 0.0,
            flex_direction: FlexDirection::Row,
            align_items: Some(taffy::AlignItems::Center),
            gap: glam::Vec2::new(theme.spacing_xs + 2.0, 0.0),
            ..Default::default()
        })
        .entity();
    cmd.add_child(entity, header);
    let label = cmd
        .spawn((
            UINode {
                flex_grow: 1.0,
                ..Default::default()
            },
            TextComponent {
                ellipsis: true,
                wrap: false,
                ..text(theme, name)
            },
        ))
        .entity();
    cmd.add_child(header, label);
    // Component enable/disable is not implemented; keep its visual disabled.
    let toggle = cmd
        .spawn((
            UINode {
                width: UIValue::Px(22.0),
                height: UIValue::Px(13.0),
                flex_shrink: 0.0,
                ..Default::default()
            },
            UIMaterial {
                corner_radius: 6.5,
                ..UIMaterial::flat(theme.accent)
            },
            UIDisabled,
        ))
        .entity();
    cmd.add_child(header, toggle);
    let body = cmd.spawn(body_node(theme)).entity();
    cmd.add_child(entity, body);
    let card = InspectedComponent {
        entity: target,
        type_id,
        name,
        body,
        last_read_tick: None,
        registry_tick: None,
    };
    cmd.insert(card, entity);
    (entity, card)
}

fn spawn_row(
    cmd: &mut CommandQueue,
    card: &InspectedComponent,
    property: Property,
    theme: &UITheme,
) -> Entity {
    let target = property.row(card.entity, card.type_id);
    let label = if property.path.depth() == 0 {
        card.name.to_string()
    } else {
        label_for(&property.path)
    };
    let row = cmd
        .spawn((
            UINode {
                flex_shrink: 0.0,
                flex_direction: FlexDirection::Row,
                align_items: Some(taffy::AlignItems::Center),
                gap: glam::Vec2::new(theme.spacing_xs, 0.0),
                ..Default::default()
            },
            target,
            property.value,
            BuildPropertyWidget,
        ))
        .entity();
    cmd.add_child(card.body, row);
    let label = cmd
        .spawn((
            UINode {
                width: UIValue::Px(PROPERTY_LABEL_WIDTH),
                flex_shrink: 0.0,
                ..Default::default()
            },
            TextComponent {
                color: theme.text_muted,
                font_size: theme.font_size_sm,
                line_height: theme.line_height(theme.font_size_sm),
                wrap: false,
                ellipsis: true,
                ..text(theme, &label)
            },
        ))
        .entity();
    cmd.add_child(row, label);
    row
}

fn body_node(theme: &UITheme) -> UINode {
    UINode {
        visible: false,
        flex_shrink: 0.0,
        flex_direction: FlexDirection::Column,
        gap: glam::Vec2::new(0.0, theme.spacing_xs),
        padding: UIRect {
            top: theme.spacing_xs,
            ..Default::default()
        },
        ..Default::default()
    }
}

pub(super) fn build_property_widgets(
    rows: Query<(Entity, &PropertyRow, &PropertyRowValue), With<BuildPropertyWidget>>,
    registry: Res<InspectorRegistry>,
    theme: Res<UITheme>,
    mut cmd: CommandQueue,
) {
    for (entity, row, value) in rows.iter() {
        if let Some(editor) = registry
            .editor(row.type_id)
            .filter(|editor| Some(editor.editor_type) == row.editor_type)
        {
            if let Err(error) = editor.adapter.build(&mut cmd, entity, value, &theme) {
                log::warn!("Unable to build property widget: {error}");
            }
        } else {
            let unsupported = cmd
                .spawn((UINode::default(), text(&theme, "Unsupported type")))
                .entity();
            cmd.add_child(entity, unsupported);
        }
        cmd.remove::<BuildPropertyWidget>(entity);
    }
}

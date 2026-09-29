//! Reconciles the live target with card/row entities.
use super::*;

use super::add_component::{AddComponentMenu, spawn_add_component};
use super::registry::InspectionSource;
use concerto_ecs::{component::Tick, query::filter::With};

/// A component card in the inspector's UI hierarchy.
#[derive(Component, Clone, Copy)]
pub struct InspectedComponent {
    pub entity: Entity,
    pub type_id: TypeId,
    pub name: &'static str,
    body: Entity,
    last_read_tick: Option<Tick>,
}

#[derive(Component)]
pub(super) struct BuildPropertyWidget;

pub(super) fn sync_inspected_components(
    source: InspectionSource,
    data: Res<InspectorData>,
    theme: Res<UITheme>,
    stacks: Query<(Entity, &ComponentStack, Option<&Children>)>,
    cards: Query<&InspectedComponent>,
    menus: Query<(Entity, &AddComponentMenu)>,
    mut cmd: CommandQueue,
) {
    let Some((stack_entity, stack, children)) = stacks.iter().next() else {
        return;
    };
    let target = data.entity.filter(|&entity| source.entity_is_valid(entity));
    let structural_version = target.and_then(|entity| source.structural_version(entity));
    let registry_tick = Some(source.registry_tick());
    let rebuild = stack.target != target
        || stack.structural_version != structural_version
        || stack.registry_tick != registry_tick;
    if rebuild {
        if children.is_some() {
            cmd.entity(stack_entity).despawn_children();
        }
        cmd.insert(
            ComponentStack {
                target,
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
            let (entity, card) = spawn_card(&mut cmd, stack_entity, target, type_id, name, &theme);
            refresh_card(&source, &mut cmd, entity, card, &theme);
        }

        spawn_add_component(&mut cmd, stack_entity, &menus, &theme);
    } else {
        for &entity in children.into_iter().flat_map(|children| children.iter()) {
            if let Some(card) = cards.get_entity(entity) {
                refresh_card(&source, &mut cmd, entity, *card, &theme);
            }
        }
    }
}

fn refresh_card(
    source: &InspectionSource,
    cmd: &mut CommandQueue,
    entity: Entity,
    mut card: InspectedComponent,
    theme: &UITheme,
) {
    if card
        .last_read_tick
        .is_some_and(|tick| !source.has_component_changed_since(card.entity, card.type_id, tick))
    {
        return;
    }
    let properties = source
        .collect_component(card.entity, card.type_id)
        .unwrap_or_default();
    if card.last_read_tick.is_some() {
        cmd.entity(card.body).despawn_children();
    }
    let mut body = body_node(theme);
    body.visible = !properties.is_empty();
    cmd.insert(body, card.body);
    for property in properties {
        spawn_row(cmd, &card, property, theme);
    }
    card.last_read_tick = Some(source.current_tick());
    cmd.insert(card, entity);
}

fn spawn_card(
    cmd: &mut CommandQueue,
    stack: Entity,
    target: Entity,
    type_id: TypeId,
    name: &'static str,
    theme: &UITheme,
) -> (Entity, InspectedComponent) {
    let mut stack_queue = cmd.entity(stack);
    let mut card_queue = stack_queue.spawn_child_queue((
        UINode::default()
            .with_flex_shrink(0.0)
            .with_flex_direction(FlexDirection::Column)
            .with_padding(UIRect::axes(theme.spacing_xs + 2.0, theme.spacing_sm)),
        UIMaterial {
            corner_radius: theme.radius_md,
            ..UIMaterial::flat(theme.surface_raised)
        },
    ));
    let entity = card_queue.entity();

    card_queue = card_queue.add_child_with(
        UINode::default()
            .with_flex_shrink(0.0)
            .with_flex_direction(FlexDirection::Row)
            .with_align_items(taffy::AlignItems::Center)
            .with_gap(glam::Vec2::new(theme.spacing_xs + 2.0, 0.0)),
        |header| {
            header
                .add_child((
                    UINode::default().with_flex_grow(1.0),
                    UIText {
                        ellipsis: true,
                        wrap: false,
                        ..text(theme, name)
                    },
                ))
                .add_child((
                    UINode::default()
                        .with_size(UIValue::Px(22.0), UIValue::Px(13.0))
                        .with_flex_shrink(0.0),
                    UIMaterial {
                        corner_radius: 6.5,
                        ..UIMaterial::flat(theme.accent)
                    },
                    UIDisabled,
                ));
        },
    );

    let body = card_queue.spawn_child_queue(body_node(theme)).entity();

    let card = InspectedComponent {
        entity: target,
        type_id,
        name,
        body,
        last_read_tick: None,
    };
    card_queue.insert(card);
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
    let mut body_queue = cmd.entity(card.body);
    let row_queue = body_queue.spawn_child_queue((
        UINode::default()
            .with_flex_shrink(0.0)
            .with_flex_direction(FlexDirection::Row)
            .with_align_items(taffy::AlignItems::Center)
            .with_gap(glam::Vec2::new(theme.spacing_xs, 0.0)),
        target,
        property.value,
        BuildPropertyWidget,
    ));
    let row = row_queue.entity();
    row_queue.add_child((
        UINode::default()
            .with_width(UIValue::Px(PROPERTY_LABEL_WIDTH))
            .with_flex_shrink(0.0),
        UIText {
            color: theme.text_muted,
            font_size: theme.font_size_sm,
            line_height: theme.line_height(theme.font_size_sm),
            wrap: false,
            ellipsis: true,
            ..text(theme, &label)
        },
    ));
    row
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
            cmd.entity(entity)
                .add_child((UINode::default(), text(&theme, "Unsupported type")));
        }
        cmd.remove::<BuildPropertyWidget>(entity);
    }
}

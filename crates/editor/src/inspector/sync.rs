//! Reconciles the live target with card/row entities.
use super::*;

use super::add_component::{AddComponentMenu, spawn_add_component};
use super::component_menu::{ComponentMenu, ensure_component_menu, menu_button};
use super::list::remove_button;
use super::registry::InspectionSource;
use concerto_ecs::{
    Changed,
    component::Tick,
    query::filter::{With, Without},
    reflect::AppTypeRegistry,
};
use concerto_reflect::{PathSegment, TypeRegistry};
use concerto_ui::text::UIText;
use std::{collections::HashMap, sync::Arc};

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

/// Marks a component card whose values cannot be edited here: the widgets in its body are disabled, and its menu still works.
#[derive(Component)]
pub struct ReadOnlyCard;

/// An empty column above the inspected entity's component cards, spawned again with them, for other plugins to fill.
#[derive(Component)]
pub struct InspectorHeader {
    /// The inspected entity, in the edit `App`'s world.
    pub scene_entity: Entity,
}

#[derive(Component)]
pub(super) struct LabelColumn {
    pub(super) body: Entity,
}

#[derive(Component)]
pub(super) struct RowLabel;

#[derive(Component)]
pub(super) struct BuildPropertyWidget {
    pub(super) world: TargetWorld,
    remove: Option<(Entity, usize)>,
}

pub(super) struct CardRows<'a> {
    pub(super) body: Entity,
    pub(super) name: &'a str,
    pub(super) target: Arc<dyn PropertyTarget>,
    pub(super) read: bool,
    pub(super) mixed: &'a [PropertyPath],
}

#[derive(Component)]
pub(super) struct MixedPaths(Vec<PropertyPath>);

#[derive(Clone, Copy)]
pub(super) struct CardMenu {
    pub(super) scene_entity: Entity,
    pub(super) component: TypeId,
    pub(super) enabled: bool,
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

fn inspected_targets(source: &InspectionSource, data: &InspectorData) -> Vec<Entity> {
    let Some(active) = data
        .scene_entity
        .filter(|&entity| source.entity_is_valid(entity))
    else {
        return Vec::new();
    };
    let mut targets: Vec<Entity> = data
        .scene_entities
        .iter()
        .copied()
        .filter(|&entity| entity != active && source.entity_is_valid(entity))
        .collect();
    targets.push(active);
    targets
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
    rows: Query<(
        Option<&PropertyRow>,
        Option<&PlainRow>,
        Option<&PropertyRowValue>,
    )>,
    menus: Query<(Entity, &AddComponentMenu)>,
    component_menus: Query<&ComponentMenu>,
    mut cmd: CommandQueue,
) {
    let Some((stack_entity, stack, children)) = stacks.iter().next() else {
        return;
    };
    let targets = inspected_targets(&source, &data);
    let structural_versions: Vec<Option<EntityStructuralVersion>> = targets
        .iter()
        .map(|&entity| source.structural_version(entity))
        .collect();
    let registry_tick = Some(registry.changed_tick());
    let rebuild = stack.scene_targets != targets
        || stack.structural_versions != structural_versions
        || stack.registry_tick != registry_tick;
    if rebuild {
        if children.is_some() {
            cmd.entity(stack_entity).despawn_children();
        }
        cmd.insert(
            ComponentStack {
                scene_targets: targets.clone(),
                structural_versions,
                registry_tick,
            },
            stack_entity,
        );
    }
    let Some((&target, others)) = targets.split_last() else {
        return;
    };
    let shown = Shown {
        source: &source,
        registry: &registry,
        others,
    };

    if rebuild {
        spawn_cards(
            &shown,
            &mut cmd,
            stack_entity,
            target,
            &theme,
            &bodies,
            &rows,
        );
        spawn_add_component(&mut cmd, stack_entity, &menus, others.is_empty(), &theme);
        ensure_component_menu(&mut cmd, &component_menus, &theme);
    } else {
        for &entity in children.into_iter().flat_map(|children| children.iter()) {
            if let Some(card) = cards.get_entity(entity) {
                refresh_card(&shown, &mut cmd, entity, *card, &theme, &bodies, &rows);
            }
        }
    }
}

struct Shown<'a, 'w> {
    source: &'a InspectionSource<'w>,
    registry: &'a Res<'w, InspectorRegistry>,
    others: &'a [Entity],
}

impl Shown<'_, '_> {
    fn mixed_paths(&self, component: TypeId, properties: &[Property]) -> Vec<PropertyPath> {
        let theirs: Vec<Vec<Property>> = self
            .others
            .iter()
            .map(|&other| {
                self.source
                    .collect_component(self.registry, other, component)
                    .unwrap_or_default()
            })
            .collect();
        let by_path: Vec<HashMap<&PropertyPath, Option<&PropertyRowValue>>> = theirs
            .iter()
            .map(|other| {
                other
                    .iter()
                    .map(|their| (&their.path, their.value()))
                    .collect()
            })
            .collect();
        properties
            .iter()
            .filter(|property| {
                property.value().is_some()
                    && by_path
                        .iter()
                        .any(|other| other.get(&property.path) != Some(&property.value()))
            })
            .map(|property| property.path.clone())
            .collect()
    }
}

fn spawn_cards(
    shown: &Shown,
    cmd: &mut CommandQueue,
    stack: Entity,
    target: Entity,
    theme: &UITheme,
    bodies: &Query<&Children>,
    rows: &Query<(
        Option<&PropertyRow>,
        Option<&PlainRow>,
        Option<&PropertyRowValue>,
    )>,
) {
    let source = shown.source;
    cmd.entity(stack).add_child((
        theme.column().fixed().gap(theme.spacing_xs + 2.0).hidden(),
        InspectorHeader {
            scene_entity: target,
        },
    ));
    let mut components = source.visible_components(target);
    components.retain(|&(id, _)| shown.others.iter().all(|&other| source.carries(other, id)));
    components.sort_by(|(a_id, a), (b_id, b)| a.cmp(b).then(a_id.cmp(b_id)));
    for (type_id, name) in components {
        let menu = source.is_registered(type_id).then_some(CardMenu {
            scene_entity: target,
            component: type_id,
            enabled: shown.others.is_empty(),
        });
        let (entity, body) = spawn_card(cmd, stack, name, menu, theme);
        let card = InspectedComponent {
            scene_entity: target,
            type_id,
            name,
            body,
            read: false,
        };
        refresh_card(shown, cmd, entity, card, theme, bodies, rows);
    }
}

#[allow(clippy::too_many_arguments)]
fn refresh_card(
    shown: &Shown,
    cmd: &mut CommandQueue,
    entity: Entity,
    mut card: InspectedComponent,
    theme: &UITheme,
    bodies: &Query<&Children>,
    rows: &Query<(
        Option<&PropertyRow>,
        Option<&PlainRow>,
        Option<&PropertyRowValue>,
    )>,
) {
    let source = shown.source;
    let changed = std::iter::once(card.scene_entity)
        .chain(shown.others.iter().copied())
        .any(|entity| source.is_component_changed(entity, card.type_id));
    if card.read && !changed {
        return;
    }
    let properties = source
        .collect_component(shown.registry, card.scene_entity, card.type_id)
        .unwrap_or_default();
    let mixed = shown.mixed_paths(card.type_id, &properties);
    show_properties(
        CardRows {
            body: card.body,
            name: card.name,
            target: Arc::new(ComponentTarget {
                entity: card.scene_entity,
                component: card.type_id,
            }),
            read: card.read,
            mixed: &mixed,
        },
        properties,
        &source.types(),
        shown.registry,
        cmd,
        theme,
        bodies,
        rows,
    );
    card.read = true;
    cmd.insert((card, MixedPaths(mixed)), entity);
}

#[allow(clippy::too_many_arguments)]
pub(super) fn show_properties(
    card: CardRows<'_>,
    properties: Vec<Property>,
    types: &TypeRegistry,
    registry: &Res<InspectorRegistry>,
    cmd: &mut CommandQueue,
    theme: &UITheme,
    bodies: &Query<&Children>,
    rows: &Query<(
        Option<&PropertyRow>,
        Option<&PlainRow>,
        Option<&PropertyRowValue>,
    )>,
) {
    let registry_tick = registry.changed_tick();
    let kept = card
        .read
        .then(|| {
            kept_rows(
                registry,
                types,
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
                let node = registry.row_node(types, &property, theme);
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
}

fn list_element(path: &PropertyPath) -> Option<(PropertyPath, usize)> {
    let (&PathSegment::Index(index), list) = path.segments().split_last()? else {
        return None;
    };
    Some((PropertyPath::new(list.iter().copied()), index))
}

fn kept_rows(
    registry: &InspectorRegistry,
    types: &TypeRegistry,
    registry_tick: Tick,
    body: Entity,
    properties: &[Property],
    bodies: &Query<&Children>,
    rows: &Query<(
        Option<&PropertyRow>,
        Option<&PlainRow>,
        Option<&PropertyRowValue>,
    )>,
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
            let (row, plain, shown) = rows.get_entity(entity)?;
            let (path, type_id) = shown_property(row, plain)?;
            let same_row = *path == property.path
                && type_id == property.type_id
                && row.is_none_or(|row| row.registry_tick == registry_tick);
            match (row, shown, property.value()) {
                _ if !same_row => None,
                (_, None, None) => Some(None),
                (_, Some(shown), Some(value)) if shown == value => Some(None),
                (Some(row), Some(_), Some(_)) => registry
                    .follows_snapshot(types, row)
                    .then_some(Some(entity)),
                _ => None,
            }
        })
        .collect()
}

fn shown_property<'a>(
    row: Option<&'a PropertyRow>,
    plain: Option<&'a PlainRow>,
) -> Option<(&'a PropertyPath, TypeId)> {
    row.map(|row| (&row.address.path, row.address.type_id))
        .or(plain.map(|plain| (&plain.path, plain.type_id)))
}

pub(super) fn spawn_card(
    cmd: &mut CommandQueue,
    stack: Entity,
    name: &str,
    menu: Option<CardMenu>,
    theme: &UITheme,
) -> (Entity, Entity) {
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
            if let Some(menu) = menu {
                let mut button =
                    header.spawn_child(menu_button(theme, menu.scene_entity, menu.component));
                if !menu.enabled {
                    button.insert(UIDisabled);
                }
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
    card_queue.insert(LabelColumn { body });
    (entity, body)
}

fn spawn_row(
    cmd: &mut CommandQueue,
    card: &CardRows<'_>,
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
    let label = row_label(
        card.name,
        &property.path,
        card.mixed.contains(&property.path),
    );
    let mut body_queue = cmd.entity(card.body);
    let mut row_queue = body_queue.spawn_child((
        row_node,
        BuildPropertyWidget {
            world: card.target.world(),
            remove,
        },
    ));
    let row = row_queue.entity();
    match property.row(card.target.clone(), registry_tick) {
        Some(editor_row) => row_queue.insert(editor_row),
        None => row_queue.insert(PlainRow {
            path: property.path.clone(),
            type_id: property.type_id,
        }),
    };
    row_queue.add_child((
        theme
            .label(label)
            .small()
            .muted()
            .single_line()
            .width(UIValue::Px(LABEL_MIN))
            .fixed(),
        RowLabel,
    ));
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

fn row_label(card: &str, path: &PropertyPath, mixed: bool) -> String {
    let mut label = if path.depth() == 0 {
        card.to_string()
    } else {
        label_for(path)
    };
    if mixed {
        label.push_str(" —");
    }
    label
}

pub(super) fn mark_mixed_rows(
    cards: Query<(&InspectedComponent, &MixedPaths), Changed<MixedPaths>>,
    children: Query<&Children>,
    rows: Query<(Option<&PropertyRow>, Option<&PlainRow>)>,
    mut labels: Query<&mut UIText, With<RowLabel>>,
) {
    for (card, mixed) in cards.iter() {
        for &row in children
            .get_entity(card.body)
            .into_iter()
            .flat_map(|rows| rows.iter())
        {
            if let Some((property, plain)) = rows.get_entity(row)
                && let Some((path, _)) = shown_property(property, plain)
            {
                let text = row_label(card.name, path, mixed.0.contains(path));
                relabel(&children, &mut labels, row, text);
            }
        }
    }
}

fn relabel(
    children: &Query<&Children>,
    labels: &mut Query<&mut UIText, With<RowLabel>>,
    row: Entity,
    text: String,
) {
    for &cell in children
        .get_entity(row)
        .into_iter()
        .flat_map(|cells| cells.iter())
    {
        if let Some(mut label) = labels.get_entity_mut(cell)
            && label.text != text
        {
            label.text = text.clone();
        }
    }
}

fn label_column(card_width: f32) -> f32 {
    (card_width * LABEL_SHARE).clamp(LABEL_MIN, LABEL_MAX)
}

pub(super) fn size_label_columns(
    cards: Query<(&LabelColumn, &UILayout)>,
    children: Query<&Children>,
    labels: Query<&RowLabel>,
    mut nodes: Query<&mut UINode>,
) {
    for (card, layout) in cards.iter() {
        let width = UIValue::Px(label_column(layout.rect.size.x));
        let rows = children.get_entity(card.body);
        for &row in rows.into_iter().flat_map(|rows| rows.iter()) {
            let cells = children.get_entity(row);
            for &cell in cells.into_iter().flat_map(|cells| cells.iter()) {
                if labels.get_entity(cell).is_some()
                    && let Some(mut node) = nodes.get_entity_mut(cell)
                    && node.width != width
                {
                    node.width = width;
                }
            }
        }
    }
}

pub(super) fn body_node(theme: &UITheme) -> UINode {
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
        Option<&PropertyRow>,
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
        if build.world == TargetWorld::EditTarget {
            build_row_widgets(
                entity, row, value, build, &registry, types, &theme, &mut cmd,
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn build_row_widgets(
    entity: Entity,
    row: Option<&PropertyRow>,
    value: Option<&PropertyRowValue>,
    build: &BuildPropertyWidget,
    registry: &InspectorRegistry,
    types: Option<&TypeRegistry>,
    theme: &UITheme,
    cmd: &mut CommandQueue,
) {
    let editor = row.and_then(|row| {
        registry
            .editor(row.address.type_id, types)
            .filter(|editor| editor.editor_type == row.editor_type)
    });
    if let (Some(editor), Some(value)) = (editor, value)
        && let Err(error) = editor.adapter.build(cmd, entity, value, theme)
    {
        log::warn!("Unable to build property widget: {error}");
    }
    if let Some((list, index)) = build.remove {
        cmd.entity(entity)
            .add_child(remove_button(theme, list, index));
    }
    cmd.remove::<BuildPropertyWidget>(entity);
}

pub(super) fn disable_read_only_cards(
    cards: Query<&InspectedComponent, With<ReadOnlyCard>>,
    children: Query<&Children>,
    enabled: Query<Entity, (With<Interactable>, Without<UIDisabled>)>,
    mut cmd: CommandQueue,
) {
    for card in cards.iter() {
        let mut pending = vec![card.body];
        while let Some(node) = pending.pop() {
            if enabled.contains_entity(node) {
                cmd.insert(UIDisabled, node);
            }
            pending.extend(
                children
                    .get_entity(node)
                    .into_iter()
                    .flat_map(|children| children.iter())
                    .copied(),
            );
        }
    }
}

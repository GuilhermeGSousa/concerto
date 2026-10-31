use super::*;

use super::sync::{BuildPropertyWidget, CardRows, build_row_widgets, show_properties, spawn_card};
use crate::{asset_editor::ActiveEditor, hosted::HostedApp};
use concerto_ecs::{
    Changed, With,
    reflect::AppTypeRegistry,
    signal::{Signal, lifecycle::Add},
};
use std::sync::Arc;

/// The values a document shows in the inspector while it is the active one.
#[derive(Component, Clone, Default)]
pub struct InspectedValues(pub Vec<Arc<dyn PropertyTarget>>);

/// Reads the active document's values again.
pub struct RefreshInspector;
impl Signal for RefreshInspector {}

/// One value as it was last read.
pub struct ValueSnapshot {
    pub target: Arc<dyn PropertyTarget>,
    pub name: String,
    /// The value's rows, empty when it is gone.
    pub properties: Vec<Property>,
    pub types: AppTypeRegistry,
}

/// The active document's values as they were last read.
#[derive(Resource, Default)]
pub struct InspectedSnapshots {
    pub document: Option<Entity>,
    pub values: Vec<ValueSnapshot>,
}

impl InspectedSnapshots {
    fn types_of(&self, target: &Arc<dyn PropertyTarget>) -> Option<&AppTypeRegistry> {
        self.values
            .iter()
            .find(|value| Arc::ptr_eq(&value.target, target))
            .map(|value| &value.types)
    }
}

/// A value's card in the inspector's UI hierarchy.
#[derive(Component, Clone)]
pub struct ValueCard {
    pub target: Arc<dyn PropertyTarget>,
    pub name: String,
    body: Entity,
}

fn read_value(
    world: &World,
    registry: &InspectorRegistry,
    target: &Arc<dyn PropertyTarget>,
) -> ValueSnapshot {
    let types = target.types(world).unwrap_or_default();
    let mut properties = Vec::new();
    {
        let table = types.read();
        target.read(world, &mut |value| {
            properties = registry.collect(value, &table);
        });
    }
    ValueSnapshot {
        target: target.clone(),
        name: target.name(),
        properties,
        types,
    }
}

pub(super) fn read_inspected_values(_: On<RefreshInspector>, world: &mut World) {
    let document = world
        .get_resource::<ActiveEditor>()
        .and_then(|active| active.0);
    let Some(registry) = world.get_resource::<InspectorRegistry>() else {
        return;
    };
    let values = document
        .and_then(|document| world.get_component_for_entity::<InspectedValues>(document))
        .map(|values| {
            values
                .0
                .iter()
                .map(|target| read_value(world, registry, target))
                .collect()
        })
        .unwrap_or_default();
    world.insert_resource(InspectedSnapshots { document, values });
}

pub(super) fn refresh_on_new_app(_: On<Add<HostedApp>>, mut cmd: CommandQueue) {
    cmd.trigger(RefreshInspector);
}

pub(super) fn refresh_inspected_values(
    active: Res<ActiveEditor>,
    registry: Res<InspectorRegistry>,
    changed: Query<&InspectedValues, Changed<InspectedValues>>,
    mut cmd: CommandQueue,
) {
    let values_changed = active
        .0
        .is_some_and(|document| changed.get_entity(document).is_some());
    if active.is_changed() || registry.is_changed() || values_changed {
        cmd.trigger(RefreshInspector);
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn sync_value_cards(
    snapshots: Res<InspectedSnapshots>,
    edit_targets: Query<Entity, With<EditTarget>>,
    registry: Res<InspectorRegistry>,
    theme: Res<UITheme>,
    stacks: Query<(Entity, Option<&Children>), With<ComponentStack>>,
    cards: Query<(Entity, &ValueCard)>,
    bodies: Query<&Children>,
    rows: Query<(
        Option<&PropertyRow>,
        Option<&PlainRow>,
        Option<&PropertyRowValue>,
    )>,
    views: Query<&mut UIScrollArea, With<DetailsView>>,
    mut cmd: CommandQueue,
) {
    if !snapshots.is_changed() || edit_targets.iter().next().is_some() {
        return;
    }
    let Some((stack, children)) = stacks.iter().next() else {
        return;
    };
    let shown: Vec<(Entity, &ValueCard)> = children
        .into_iter()
        .flat_map(|children| children.iter())
        .filter_map(|&card| cards.get_entity(card))
        .collect();
    let same = shown.len() == snapshots.values.len()
        && shown
            .iter()
            .zip(&snapshots.values)
            .all(|((_, card), value)| Arc::ptr_eq(&card.target, &value.target));
    if !same {
        for (card, _) in &shown {
            cmd.despawn(*card);
        }
        if let Some(mut view) = views.iter().next()
            && view.offset != 0.0
        {
            view.offset = 0.0;
        }
    }
    for (index, value) in snapshots.values.iter().enumerate() {
        let body = if same {
            shown[index].1.body
        } else {
            let (card, body) = spawn_card(&mut cmd, stack, &value.name, None, &theme);
            cmd.insert(
                ValueCard {
                    target: value.target.clone(),
                    name: value.name.clone(),
                    body,
                },
                card,
            );
            body
        };
        show_properties(
            CardRows {
                body,
                name: &value.name,
                target: value.target.clone(),
                read: same,
            },
            value.properties.clone(),
            &value.types.read(),
            &registry,
            &mut cmd,
            &theme,
            &bodies,
            &rows,
        );
    }
}

pub(super) fn build_value_widgets(
    rows: Query<(
        Entity,
        Option<&PropertyRow>,
        Option<&PropertyRowValue>,
        &BuildPropertyWidget,
    )>,
    registry: Res<InspectorRegistry>,
    snapshots: Res<InspectedSnapshots>,
    theme: Res<UITheme>,
    mut cmd: CommandQueue,
) {
    for (entity, row, value, build) in rows.iter() {
        if build.world == TargetWorld::Editor {
            let types = row.and_then(|row| snapshots.types_of(&row.address.target));
            let types = types.map(|types| types.read());
            build_row_widgets(
                entity,
                row,
                value,
                build,
                &registry,
                types.as_deref(),
                &theme,
                &mut cmd,
            );
        }
    }
}

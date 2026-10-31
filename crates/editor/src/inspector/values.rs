use super::*;

use super::sync::{BuildPropertyWidget, CardRows, build_row_widgets, show_properties, spawn_card};
use crate::{
    asset_editor::{ActiveEditor, EditorDocument},
    hosted::{Hosted, HostedApp},
    tools::{
        edits::{PropertyCommitted, PropertyRefusal, apply_at, dotted_name, with_edit_context},
        scene::write,
    },
};
use concerto_ecs::{
    Changed, With,
    reflect::AppTypeRegistry,
    signal::{Signal, lifecycle::Add},
};
use std::sync::Arc;

/// The values a document shows in the inspector while it is the active one; each lives in [`TargetWorld::Editor`] and has a name of its own.
#[derive(Component, Clone, Default)]
pub struct InspectedValues(pub Vec<Arc<dyn PropertyTarget>>);

/// Reads the active document's values again.
pub struct RefreshInspector;
impl Signal for RefreshInspector {}

/// One edit of a value in the editor's world, not yet checked against the widget registry.
pub struct CommitValue(pub PropertyCommit);
impl Signal for CommitValue {}

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
    target.read(world, &mut |value| {
        properties = registry.collect(value, &types.read());
    });
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
    world.insert_resource(InspectedSnapshots { values });
}

pub(super) fn send_value_commits(mut commits: ResMut<PropertyCommits>, mut cmd: CommandQueue) {
    if commits.holds_for(TargetWorld::Editor) {
        for commit in commits.take_for(TargetWorld::Editor) {
            cmd.trigger(CommitValue(commit));
        }
    }
}

pub(super) fn commit_value(on: On<CommitValue>, world: &mut World) {
    if let Err(refusal) = apply_value_commit(world, &on.signal().0) {
        log::warn!("Value commit refused: {refusal}");
    }
}

fn current_value_editor(
    world: &World,
    row: &PropertyRow,
) -> Result<Arc<dyn ErasedEditor>, EditError> {
    let registry = world
        .get_resource::<InspectorRegistry>()
        .ok_or(EditError::StaleEditor)?;
    let changed = world
        .resource_changed_tick::<InspectorRegistry>()
        .ok_or(EditError::StaleEditor)?;
    let types = row.address.target.types(world);
    let types = types.as_ref().map(|types| types.read());
    current_editor(registry, types.as_deref(), changed, row).map(|editor| editor.adapter)
}

fn write_value(world: &mut World, commit: &PropertyCommit) -> Result<(), EditError> {
    let editor = current_value_editor(world, &commit.row)?;
    let address = &commit.row.address;
    with_edit_context(world, |world, context| {
        let mut result = Err(EditError::NotFound);
        let found = address.target.edit(world, &mut |value| {
            result = apply_at(
                value,
                &address.path,
                editor.as_ref(),
                commit.edit.as_ref(),
                context,
            );
        });
        if found {
            result
        } else {
            Err(EditError::MissingTarget)
        }
    })
}

/// Tells the value's document what an edit did, and reads the values again.
pub(crate) fn report_value_edit(
    world: &mut World,
    address: &PropertyAddress,
    result: &Result<(), PropertyRefusal>,
) {
    if let Some(document) = address.target.document() {
        if let Err(refusal) = result
            && let Some(held) = world.get_component_for_entity_mut::<EditorDocument>(document)
        {
            held.status = refusal.status().to_string();
        }
        write(
            world,
            Hosted {
                document,
                event: PropertyCommitted {
                    address: address.clone(),
                    result: result.clone(),
                },
            },
        );
    }
    world.trigger(RefreshInspector);
}

/// Applies a commit to a value in the editor's world, reports it to the value's document, and reads the values again.
pub(crate) fn apply_value_commit(
    world: &mut World,
    commit: &PropertyCommit,
) -> Result<(), PropertyRefusal> {
    let address = &commit.row.address;
    let result = write_value(world, commit).map_err(|reason| {
        let property = dotted_name(&address.target.name(), &address.path);
        PropertyRefusal::Refused(format!("Set {property} failed: {reason}"))
    });
    report_value_edit(world, address, &result);
    result
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

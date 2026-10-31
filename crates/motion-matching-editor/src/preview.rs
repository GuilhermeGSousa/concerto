use std::any::TypeId;

use anyhow::Context;
use concerto_app::{App, edit_time::AtPreviewTime};
use concerto_ecs::{
    Component, Entity, Resource, World,
    reflect::{AppTypeRegistry, ReflectComponent},
    signal::{On, Signal},
    system::input::ComponentMetadata,
};
use concerto_editor::{
    clip_editor::preview::clip_preview,
    tools::{
        edits::{ComponentEdit, ComponentEdited},
        entity_ops::FixedEntities,
        scene::{SceneStatus, report_edit, write},
        selection::{Select, Selection},
    },
};
use concerto_motion_matching::{
    MotionDatabaseDefinition, ReflectFeatureChannel, SimulationBoneSettings, StoredChannel,
    TrajectorySettings,
};
use concerto_reflect::{Reflect, ReflectedValue, TypeRegistration, TypeRegistry, to_value};

use crate::playback::database_playback;

/// A definition's settings that belong to no channel, as the inspector edits them.
#[derive(Component, Reflect, Clone, Debug, PartialEq)]
#[reflect(Component)]
pub struct DatabaseSettings {
    pub rate: f32,
    pub speed: f32,
    pub simulation: SimulationBoneSettings,
    pub trajectory: TrajectorySettings,
}

impl DatabaseSettings {
    /// The settings `definition` holds.
    pub fn of(definition: &MotionDatabaseDefinition) -> Self {
        Self {
            rate: definition.rate,
            speed: definition.speed,
            simulation: definition.simulation.clone(),
            trajectory: definition.trajectory,
        }
    }
}

/// A channel entity's place in its definition's list.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ChannelOrder(pub usize);

/// A channel entity's registered name, which its row in the panel shows.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChannelKind(pub &'static str);

/// The bone names of the definition's skeleton, which a joint row offers; absent with no skeleton.
#[derive(Resource)]
pub struct SkeletonBones(pub Vec<String>);

/// Replaces the settings and channels a preview holds.
pub struct ShowDefinition {
    pub settings: DatabaseSettings,
    pub channels: Vec<StoredChannel>,
}

impl Signal for ShowDefinition {}

/// Replaces the preview's [`SkeletonBones`], or removes them when there is no skeleton.
pub struct ShowSkeletonBones(pub Option<Vec<String>>);

impl Signal for ShowSkeletonBones {}

/// Appends a channel of the registered type this names, at its default value.
pub struct AddChannel(pub String);

impl Signal for AddChannel {}

/// Removes a channel entity from the definition.
pub struct RemoveChannel(pub Entity);

impl Signal for RemoveChannel {}

pub(crate) struct LiveChannel {
    pub(crate) component: ReflectComponent,
    pub(crate) type_id: TypeId,
    pub(crate) kind: &'static str,
    pub(crate) value: Box<dyn Reflect>,
}

pub(crate) fn live_channel(
    stored: &StoredChannel,
    registry: &TypeRegistry,
) -> Result<LiveChannel, String> {
    let name = &stored.type_name;
    let registration = registry
        .get_by_name(name)
        .ok_or_else(|| format!("Channel {name} is not registered in the game"))?;
    let component = registration
        .data::<ReflectComponent>()
        .copied()
        .ok_or_else(|| format!("Channel {name} is not a component"))?;
    let channel = registration
        .data::<ReflectFeatureChannel>()
        .ok_or_else(|| format!("{name} is not a feature channel"))?;
    let value = (channel.from_value)(&stored.value)
        .map_err(|reason| format!("Channel {name} cannot be built: {reason}"))?;
    Ok(LiveChannel {
        component,
        type_id: (*registration).type_id(),
        kind: registration.short_name(),
        value,
    })
}

pub(crate) fn channel_types(registry: &TypeRegistry) -> Vec<String> {
    let mut names: Vec<String> = registry
        .iter()
        .filter(|registration| {
            registration.data::<ReflectFeatureChannel>().is_some()
                && registration.data::<ReflectComponent>().is_some()
        })
        .map(|registration| registration.short_name().to_owned())
        .collect();
    names.sort();
    names
}

fn database_entities(world: &mut World) -> Vec<Entity> {
    let mut entities: Vec<Entity> = world
        .query::<(Entity, &DatabaseSettings), ()>()
        .iter(world)
        .map(|(entity, _)| entity)
        .collect();
    entities.extend(
        world
            .query::<(Entity, &ChannelOrder), ()>()
            .iter(world)
            .map(|(entity, _)| entity),
    );
    entities
}

fn spawn_channel(
    world: &mut World,
    order: usize,
    stored: &StoredChannel,
) -> Result<(Entity, TypeId), String> {
    let channel = {
        let registry = world
            .get_resource::<AppTypeRegistry>()
            .ok_or("The preview has no type registry")?;
        live_channel(stored, &registry.read())?
    };
    let entity = world.spawn((ChannelOrder(order), ChannelKind(channel.kind)));
    (channel.component.insert_boxed)(world, entity, channel.value);
    Ok((entity, channel.type_id))
}

fn spawn_database_entities(on: On<ShowDefinition>, world: &mut World) {
    for entity in database_entities(world) {
        world.despawn(entity);
    }
    world.trigger(Select(None));
    world.spawn(on.signal().settings.clone());
    for (order, stored) in on.signal().channels.iter().enumerate() {
        if let Err(status) = spawn_channel(world, order, stored) {
            write(world, SceneStatus(status));
        }
    }
}

fn add_channel(on: On<AddChannel>, world: &mut World) {
    let order = world
        .query::<&ChannelOrder, ()>()
        .iter(world)
        .map(|order| order.0 + 1)
        .max()
        .unwrap_or(0);
    let stored = StoredChannel {
        type_name: on.signal().0.clone(),
        value: ReflectedValue::Struct(Vec::new()),
    };
    match spawn_channel(world, order, &stored) {
        Ok((entity, component)) => {
            report_edit(world);
            write(
                world,
                ComponentEdited {
                    edit: ComponentEdit::Add { entity, component },
                    result: Ok(()),
                },
            );
        }
        Err(status) => write(world, SceneStatus(status)),
    }
}

fn remove_channel(on: On<RemoveChannel>, world: &mut World) {
    let entity = on.signal().0;
    if world
        .get_component_for_entity::<ChannelOrder>(entity)
        .is_none()
    {
        return;
    }
    let component = channel_registration(&ComponentMetadata::new(world), entity)
        .map(|registration| (*registration).type_id());
    let selected = world
        .get_resource::<Selection>()
        .is_some_and(|selection| selection.entity() == Some(entity));
    if selected {
        world.trigger(Select(None));
    }
    world.despawn(entity);
    report_edit(world);
    if let Some(component) = component {
        write(
            world,
            ComponentEdited {
                edit: ComponentEdit::Remove { entity, component },
                result: Ok(()),
            },
        );
    }
}

fn show_skeleton_bones(on: On<ShowSkeletonBones>, world: &mut World) {
    match &on.signal().0 {
        Some(bones) => world.insert_resource(SkeletonBones(bones.clone())),
        None => {
            world.remove_resource::<SkeletonBones>();
        }
    }
}

fn channel_registration(
    components: &ComponentMetadata,
    entity: Entity,
) -> Option<std::sync::Arc<TypeRegistration>> {
    components
        .component_ids(entity)
        .iter()
        .filter_map(|&id| components.registration(id))
        .find(|registration| registration.data::<ReflectFeatureChannel>().is_some())
}

fn stored_channel(world: &World, entity: Entity) -> anyhow::Result<StoredChannel> {
    let components = ComponentMetadata::new(world);
    let registration =
        channel_registration(&components, entity).context("A channel entity lost its channel")?;
    let name = registration.short_name();
    let value = registration
        .data::<ReflectComponent>()
        .and_then(|component| (component.read)(&components, entity))
        .with_context(|| format!("Channel {name} cannot be read"))?;
    Ok(StoredChannel {
        type_name: name.to_owned(),
        value: to_value(value)?,
    })
}

pub(crate) fn capture_definition(
    world: &mut World,
) -> anyhow::Result<(DatabaseSettings, Vec<StoredChannel>)> {
    let settings = world
        .query::<&DatabaseSettings, ()>()
        .iter(world)
        .next()
        .cloned()
        .context("The preview holds no settings")?;
    let mut channels = world
        .query::<(Entity, &ChannelOrder), ()>()
        .iter(world)
        .map(|(entity, order)| (*order, entity))
        .collect::<Vec<_>>();
    channels.sort_by_key(|(order, _)| *order);
    let channels = channels
        .into_iter()
        .map(|(_, entity)| stored_channel(world, entity))
        .collect::<anyhow::Result<_>>()?;
    Ok((settings, channels))
}

/// What a definition's preview adds to the game's recipe.
pub fn database_preview(app: &mut App) {
    clip_preview(app);
    database_playback(app);
    app.register_type::<DatabaseSettings>()
        .insert_resource(FixedEntities)
        .add_listener(spawn_database_entities.at_preview_time())
        .add_listener(show_skeleton_bones.at_preview_time())
        .add_listener(add_channel.at_preview_time())
        .add_listener(remove_channel.at_preview_time());
}

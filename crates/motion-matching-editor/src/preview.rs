use anyhow::Context;
use concerto_app::{App, edit_time::AtPreviewTime};
use concerto_ecs::{
    Component, Entity, World,
    reflect::{AppTypeRegistry, ReflectComponent},
    signal::{On, Signal},
    system::input::ComponentMetadata,
};
use concerto_editor::tools::{
    entity_ops::FixedEntities,
    scene::{SceneStatus, write},
    selection::Select,
};
use concerto_motion_matching::{
    MotionDatabaseDefinition, ReflectFeatureChannel, SimulationBoneSettings, StoredChannel,
    TrajectorySettings,
};
use concerto_reflect::{Reflect, TypeRegistry, to_value};

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

/// Replaces the settings and channels a preview holds.
pub struct ShowDefinition {
    pub settings: DatabaseSettings,
    pub channels: Vec<StoredChannel>,
}

impl Signal for ShowDefinition {}

pub(crate) fn live_channel(
    stored: &StoredChannel,
    registry: &TypeRegistry,
) -> Result<(ReflectComponent, Box<dyn Reflect>), String> {
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
    Ok((component, value))
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

fn spawn_channel(world: &mut World, order: usize, stored: &StoredChannel) -> Result<(), String> {
    let (component, value) = {
        let registry = world
            .get_resource::<AppTypeRegistry>()
            .ok_or("The preview has no type registry")?;
        live_channel(stored, &registry.read())?
    };
    let entity = world.spawn(ChannelOrder(order));
    (component.insert_boxed)(world, entity, value);
    Ok(())
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

fn stored_channel(world: &World, entity: Entity) -> anyhow::Result<StoredChannel> {
    let components = ComponentMetadata::new(world);
    let registration = components
        .component_ids(entity)
        .iter()
        .filter_map(|&id| components.registration(id))
        .find(|registration| registration.data::<ReflectFeatureChannel>().is_some())
        .context("A channel entity lost its channel")?;
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
    app.register_type::<DatabaseSettings>()
        .insert_resource(FixedEntities)
        .add_listener(spawn_database_entities.at_preview_time());
}

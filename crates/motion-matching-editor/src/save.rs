use anyhow::Context;
use concerto_ecs::{
    Entity, Query, With, World,
    command::CommandQueue,
    signal::{On, Signal},
};
use concerto_editor::{
    asset_editor::{EditorDocument, SaveRequested, Unsaved},
    hosted::HostedApp,
    project::ProjectState,
    scene::{SaveFailed, SceneSaved},
    tools::scene::write,
};
use concerto_foundation::assets::{
    content::save_content_asset_into,
    registry::{AssetEntry, AssetRegistry},
};
use concerto_motion_matching::MotionDatabaseDefinition;

use crate::{document::DefinitionDocument, preview::capture_definition};

pub(crate) struct SaveDefinition {
    document: Entity,
}

impl Signal for SaveDefinition {}

pub(crate) fn request_definition_saves(
    requested: Query<Entity, (With<SaveRequested>, With<DefinitionDocument>)>,
    mut cmd: CommandQueue,
) {
    for document in requested.iter() {
        cmd.remove::<SaveRequested>(document);
        cmd.trigger(SaveDefinition { document });
    }
}

fn captured(world: &mut World, document: Entity) -> anyhow::Result<MotionDatabaseDefinition> {
    let hosted = world
        .get_component_for_entity_mut::<HostedApp>(document)
        .context("The preview is not running, so its settings and channels cannot be read")?;
    let (settings, channels) = capture_definition(hosted.world_mut())?;
    let state = world
        .get_component_for_entity::<DefinitionDocument>(document)
        .context("The document was closed")?;
    Ok(MotionDatabaseDefinition {
        skeleton: state.skeleton.clone(),
        entries: state.entries.clone(),
        rate: settings.rate,
        speed: settings.speed,
        simulation: settings.simulation,
        trajectory: settings.trajectory,
        channels,
    })
}

fn write_definition(
    world: &mut World,
    definition: &MotionDatabaseDefinition,
    address: &str,
) -> anyhow::Result<AssetEntry> {
    let root = world
        .get_resource::<ProjectState>()
        .and_then(|state| state.project.as_ref())
        .context("Open a project before saving")?
        .root
        .clone();
    let registry = world
        .get_resource_mut::<AssetRegistry>()
        .context("The editor has no asset registry")?;
    let id = save_content_asset_into(definition, &root, address, registry)?;
    registry
        .get(id)
        .cloned()
        .context("The saved definition was not registered")
}

fn save(world: &mut World, document: Entity) -> anyhow::Result<AssetEntry> {
    let address = world
        .get_component_for_entity::<EditorDocument>(document)
        .and_then(|document| document.current.as_ref())
        .context("No definition is open")?
        .address
        .clone();
    let definition = captured(world, document)?;
    let saved = write_definition(world, &definition, &address)?;
    if let Some(document) = world.get_component_for_entity_mut::<EditorDocument>(document) {
        document.title = saved.name().to_owned();
        document.current = Some(saved.clone());
    }
    if let Some(state) = world.get_component_for_entity_mut::<DefinitionDocument>(document) {
        state.loaded = Some(definition);
    }
    if let Some(hosted) = world.get_component_for_entity_mut::<HostedApp>(document) {
        hosted.mark_saved();
    }
    world.remove_component::<Unsaved>(document);
    Ok(saved)
}

fn set_status(world: &mut World, document: Entity, status: String) {
    if let Some(document) = world.get_component_for_entity_mut::<EditorDocument>(document) {
        document.status = status;
    }
}

pub(crate) fn save_definition(on: On<SaveDefinition>, world: &mut World) {
    let document = on.signal().document;
    match save(world, document) {
        Ok(asset) => {
            set_status(world, document, format!("Saved {}", asset.address));
            write(world, SceneSaved { document, asset });
        }
        Err(error) => {
            let message = format!("Save failed: {error:#}");
            set_status(world, document, message.clone());
            write(world, SaveFailed { document, message });
        }
    }
}

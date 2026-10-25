use std::time::{Duration, Instant};

use concerto_app::{App, schedule_groups::LateUpdate};
use concerto_ecs::{
    Component, Entity, IntoSystemConfig, Query, Res, Resource, With, command::CommandQueue,
    events::event_reader::EventReader,
};
use concerto_foundation::assets::{asset_server::AssetServer, asset_store::AssetStore};
use concerto_mcp::{AppMcpExt, McpTool, Needs, PendingCall, ToolCalls, ToolError};
use concerto_scene::{scene::Scene, spawner::SceneSpawnerComponent};
use schemars::JsonSchema;
use serde::Deserialize;

use super::act::{describe_document, unknown_document};
use crate::{
    asset_editor::{ActiveEditor, EditorDocument, SaveRequested, Unsaved},
    hosted::{EditTarget, EditWorld, PlayFailed, PlayStarted, PlayState, PlayStopped, StopPlay},
    scene::{self, SaveFailed, SceneSaved},
};

/// Saves a document's unsaved changes to its file, and waits until they are written.
#[derive(Deserialize, JsonSchema)]
pub struct Save {
    /// The document's id, such as 3v1, as status lists it; the active document when left out.
    pub document: Option<String>,
}

/// Starts the active scene's game in the viewport, saving the scene's unsaved changes first, and waits until it plays and has loaded.
#[derive(Deserialize, JsonSchema)]
pub struct Play {}

/// Stops the running game and brings back the scene as it was before play.
#[derive(Deserialize, JsonSchema)]
pub struct Stop {}

#[doc(hidden)]
#[derive(Resource)]
pub struct PlayTimeout(pub Duration);

#[derive(Component)]
struct Saving(Entity);

#[derive(Component)]
struct Starting {
    document: Entity,
    saved: bool,
    playing: Option<(Instant, String)>,
}

#[derive(Component)]
struct Stopping(Entity);

pub(super) fn build(app: &mut App) {
    app.add_tool::<Save, _>(McpTool::new("save").needs(Needs::Scene), save)
        .add_tool::<Play, _>(McpTool::new("play").needs(Needs::Scene), play)
        .add_tool::<Stop, _>(McpTool::new("stop").needs(Needs::Playing), stop)
        .add_system(
            LateUpdate,
            (
                answer_saves,
                answer_plays,
                answer_loaded_plays.after(answer_plays),
                answer_stops,
            ),
        );
}

fn save(
    mut calls: ToolCalls<Save>,
    active: Res<ActiveEditor>,
    documents: Query<&EditorDocument>,
    unsaved: Query<&Unsaved>,
    states: Query<&PlayState>,
    mut cmd: CommandQueue,
) {
    for (arguments, reply) in calls.read() {
        let document = match &arguments.document {
            Some(id) => {
                match Entity::parse_id(id).filter(|&entity| documents.get_entity(entity).is_some())
                {
                    Some(document) => document,
                    None => {
                        reply.err(unknown_document(id));
                        continue;
                    }
                }
            }
            None => match active.0 {
                Some(document) => document,
                None => continue,
            },
        };
        let Some(open) = documents.get_entity(document) else {
            continue;
        };
        let playing = states
            .get_entity(document)
            .is_some_and(|state| state.is_on());
        if !playing && unsaved.get_entity(document).is_none() {
            reply.ok(format!(
                "Nothing to save in document {}.",
                describe_document(document, &documents)
            ));
            continue;
        }
        if open.pending.is_some() {
            reply.err(ToolError::new(
                "busy",
                "The scene is still loading; call status until it says busy: no, then save.",
            ));
            continue;
        }
        cmd.insert(SaveRequested, document);
        cmd.spawn((
            PendingCall::new(reply, "the document to save"),
            Saving(document),
        ));
    }
}

fn answer_saves(
    mut saved: EventReader<SceneSaved>,
    mut failed: EventReader<SaveFailed>,
    waiting: Query<(Entity, &PendingCall, &Saving)>,
    mut cmd: CommandQueue,
) {
    for event in saved.read() {
        for (entity, pending, saving) in waiting.iter() {
            if saving.0 == event.document {
                pending.reply.ok(format!("Saved {}.", event.asset.address));
                cmd.despawn(entity);
            }
        }
    }
    for event in failed.read() {
        for (entity, pending, saving) in waiting.iter() {
            if saving.0 == event.document {
                pending
                    .reply
                    .err(ToolError::new("save_failed", event.message.clone()));
                cmd.despawn(entity);
            }
        }
    }
}

fn play(
    mut calls: ToolCalls<Play>,
    active: Res<ActiveEditor>,
    timeout: Option<Res<PlayTimeout>>,
    mut cmd: CommandQueue,
) {
    for (_, reply) in calls.read() {
        let Some(document) = active.0 else {
            continue;
        };
        let pending = PendingCall::new(reply, "the game to start");
        cmd.spawn((
            match &timeout {
                Some(timeout) => pending.timeout(timeout.0),
                None => pending,
            },
            Starting {
                document,
                saved: false,
                playing: None,
            },
        ));
        cmd.trigger(scene::Play(document));
    }
}

fn answer_plays(
    mut saved: EventReader<SceneSaved>,
    mut started: EventReader<PlayStarted>,
    mut failed: EventReader<PlayFailed>,
    documents: Query<&EditorDocument>,
    waiting: Query<(Entity, &mut PendingCall, &mut Starting)>,
    mut cmd: CommandQueue,
) {
    for event in saved.read() {
        for (_, _, mut starting) in waiting.iter() {
            if starting.document == event.document {
                starting.saved = true;
            }
        }
    }
    for event in started.read() {
        for (_, mut pending, mut starting) in waiting.iter() {
            if starting.document != event.document {
                continue;
            }
            let shown = documents
                .get_entity(event.document)
                .and_then(|document| document.current.as_ref())
                .map_or("the game", |asset| asset.address.as_str());
            let saved = if starting.saved {
                "; its unsaved changes were saved first"
            } else {
                ""
            };
            let playing = format!(
                "Playing {shown} in document {}{saved}",
                event.document.to_id_string()
            );
            pending.at_deadline = Some(format!(
                "{playing}; it is still loading, so scene_tree and screenshot show it as it arrives."
            ));
            starting.playing = Some((Instant::now(), playing));
        }
    }
    for event in failed.read() {
        for (entity, pending, starting) in waiting.iter() {
            if starting.document == event.document {
                pending
                    .reply
                    .err(ToolError::new("play_failed", event.message.clone()));
                cmd.despawn(entity);
            }
        }
    }
}

fn answer_loaded_plays(
    assets: EditWorld<Option<Res<AssetServer>>>,
    scenes: EditWorld<Option<Res<AssetStore<Scene>>>>,
    spawners: EditWorld<Query<&SceneSpawnerComponent>>,
    targets: Query<Entity, With<EditTarget>>,
    waiting: Query<(Entity, &PendingCall, &Starting)>,
    mut cmd: CommandQueue,
) {
    let spawning = scenes.as_deref().is_some_and(|scenes| {
        spawners
            .iter()
            .any(|spawner| scenes.get(&spawner.0).is_some())
    });
    if spawning || assets.as_deref().is_some_and(AssetServer::is_loading) {
        return;
    }
    for (entity, pending, starting) in waiting.iter() {
        let Some((started, playing)) = &starting.playing else {
            continue;
        };
        if targets.get_entity(starting.document).is_none() {
            continue;
        }
        pending.reply.ok(format!(
            "{playing}; it loaded in {:.2} s.",
            started.elapsed().as_secs_f64()
        ));
        cmd.despawn(entity);
    }
}

fn stop(
    mut calls: ToolCalls<Stop>,
    active: Res<ActiveEditor>,
    states: Query<(Entity, &PlayState)>,
    mut cmd: CommandQueue,
) {
    let playing: Vec<Entity> = states
        .iter()
        .filter(|(_, state)| state.is_on())
        .map(|(document, _)| document)
        .collect();
    let document = playing
        .iter()
        .copied()
        .find(|&document| active.0 == Some(document))
        .or(playing.first().copied());
    for (_, reply) in calls.read() {
        let Some(document) = document else {
            continue;
        };
        cmd.spawn((
            PendingCall::new(reply, "the game to stop"),
            Stopping(document),
        ));
        cmd.trigger(StopPlay(document));
    }
}

fn answer_stops(
    mut stopped: EventReader<PlayStopped>,
    waiting: Query<(Entity, &PendingCall, &Stopping)>,
    mut cmd: CommandQueue,
) {
    for event in stopped.read() {
        for (entity, pending, stopping) in waiting.iter() {
            if stopping.0 == event.document {
                pending.reply.ok(format!(
                    "Stopped the game in document {}; the scene is back as it was before play.",
                    event.document.to_id_string()
                ));
                cmd.despawn(entity);
            }
        }
    }
}

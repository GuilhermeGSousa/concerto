use std::collections::VecDeque;

use concerto_animation::{
    animate_targets, clip::AnimationClip, player::AnimationPlayer, update_animation_players,
};
use concerto_app::{plugins::Plugin, schedule_groups::LateUpdate};
use concerto_color::Color;
use concerto_debug_gizmos::{DebugGizmos, DebugGizmosPlugin};
use concerto_ecs::{Component, IntoSystemConfig, Query, Res, reflect::ReflectComponent};
use concerto_foundation::{
    assets::asset_store::AssetStore,
    time::Time,
    transform::{GlobalTransform, Transform, systems::propagate_global_transforms},
};
use concerto_mesh::Skeleton;
use concerto_motion_matching::{
    CharacterLink, CharacterSimulation, DynCloneChannel, FeatureChannel, FeatureShape,
    MotionDatabase, MotionMatchingDebug, preview_features,
};
use concerto_reflect::{Reflect, ReflectDefault};
use concerto_render::plugin::RenderPlugin;
use glam::Vec3;

use crate::{
    BakedFrame, SIMULATION_COLOR, SIMULATION_WIDTH, channel_color, draw_placed, draw_simulation,
    local_pose, placed_shape,
};

const HISTORY_SECONDS: f32 = 1.0;

/// Which layers of a character's motion matching are drawn; goes on the entity that holds `CharacterLink`.
#[derive(Component, Reflect, Clone, Debug, PartialEq)]
#[reflect(Component, Default)]
pub struct MotionMatchingGizmos {
    pub simulation: bool,
    pub query: bool,
    pub matched: bool,
    pub history: bool,
    #[reflect(skip)]
    trail: VecDeque<(f32, Vec3)>,
}

impl Default for MotionMatchingGizmos {
    fn default() -> Self {
        Self {
            simulation: true,
            query: true,
            matched: true,
            history: true,
            trail: VecDeque::new(),
        }
    }
}

/// Draws `MotionMatchingGizmos` in game time; register it after `DefaultPlugins`.
pub struct MotionMatchingGizmosPlugin;

impl Plugin for MotionMatchingGizmosPlugin {
    fn build(&self, app: &mut concerto_app::App) {
        app.register_type::<MotionMatchingGizmos>();
        if app.is_plugin_added::<RenderPlugin>() && !app.is_plugin_added::<DebugGizmosPlugin>() {
            app.register_plugin(DebugGizmosPlugin);
        }
        if app.is_plugin_added::<DebugGizmosPlugin>() {
            app.add_system(
                LateUpdate,
                draw_motion_matching
                    .after(update_animation_players)
                    .after(animate_targets)
                    .after(propagate_global_transforms),
            );
        }
    }
}

fn placed_channels(
    debug: &MotionMatchingDebug,
    database: &MotionDatabase,
    skeleton: &Skeleton,
    frame: &BakedFrame,
    features: &[f32],
    bone: &Transform,
) -> Option<Vec<Vec<FeatureShape>>> {
    if debug.query_dims != database.feature_dims()? {
        return None;
    }
    let channels: Vec<Box<dyn FeatureChannel>> = database
        .live_channels()
        .ok()?
        .into_iter()
        .map(DynCloneChannel::dyn_clone)
        .collect();
    let dims: usize = channels.iter().map(|channel| channel.dims()).sum();
    if features.len() != database.feature_dims()? || features.len() != dims {
        return None;
    }
    let shapes = preview_features(&channels, &frame.context(skeleton), features);
    Some(
        shapes
            .into_iter()
            .map(|shapes| {
                shapes
                    .into_iter()
                    .map(|shape| placed_shape(bone, shape))
                    .collect()
            })
            .collect(),
    )
}

/// The shapes of the frame a node published it plays, per channel and placed at `bone`; `None` when they can't be read.
pub fn matched_shapes(
    debug: &MotionMatchingDebug,
    database: &MotionDatabase,
    skeleton: &Skeleton,
    bone: &Transform,
) -> Option<Vec<Vec<FeatureShape>>> {
    let frame = debug.frame;
    database.range_of(frame)?;
    let baked = BakedFrame::placed(
        skeleton,
        debug.pose.clone(),
        database.simulation_bone(frame)?,
        database.trajectory_samples(frame)?,
    );
    placed_channels(
        debug,
        database,
        skeleton,
        &baked,
        database.features(frame)?,
        bone,
    )
}

/// The shapes of the last search's query a node published, per channel and placed at `bone`; `None` before a search or when they can't be read.
pub fn query_shapes(
    debug: &MotionMatchingDebug,
    database: &MotionDatabase,
    skeleton: &Skeleton,
    clips: &[&AnimationClip],
    bone: &Transform,
) -> Option<Vec<Vec<FeatureShape>>> {
    let query = debug.last_query.as_ref()?;
    database.range_of(query.frame)?;
    let baked = BakedFrame::placed(
        skeleton,
        local_pose(database, skeleton, clips, query.frame)?,
        query.simulation.clone(),
        query.trajectory.clone(),
    );
    placed_channels(debug, database, skeleton, &baked, &query.features, bone)
}

fn remember(trail: &mut VecDeque<(f32, Vec3)>, delta: f32, position: Vec3) {
    if delta <= 0.0 {
        return;
    }
    for (age, _) in trail.iter_mut() {
        *age += delta;
    }
    trail.push_back((0.0, position));
    while trail.front().is_some_and(|(age, _)| *age > HISTORY_SECONDS) {
        trail.pop_front();
    }
}

fn draw_channels(
    gizmos: &mut DebugGizmos,
    bone: &Transform,
    channels: &[Vec<FeatureShape>],
    color: impl Fn(usize) -> Color,
) {
    for (channel, shapes) in channels.iter().enumerate() {
        for shape in shapes {
            draw_placed(gizmos, bone.local_y(), *shape, color(channel));
        }
    }
}

/// Draws what each character's `MotionMatchingGizmos` turns on, at where its capsule is drawn this frame.
pub fn draw_motion_matching(
    characters: Query<(&mut MotionMatchingGizmos, &CharacterLink, &GlobalTransform)>,
    players: Query<&AnimationPlayer>,
    simulations: Query<&CharacterSimulation>,
    globals: Query<&GlobalTransform>,
    databases: Res<AssetStore<MotionDatabase>>,
    clips: Res<AssetStore<AnimationClip>>,
    skeletons: Res<AssetStore<Skeleton>>,
    time: Res<Time>,
    mut gizmos: DebugGizmos,
) {
    gizmos.set_line_width(SIMULATION_WIDTH);
    for (mut shown, link, character) in characters.iter() {
        let (Some(capsule), Some(skeleton)) = (link.capsule.entity(), link.skeleton.entity())
        else {
            continue;
        };
        let (Some(capsule), Some(simulation), Some(player)) = (
            globals.get_entity(capsule),
            simulations.get_entity(capsule),
            players.get_entity(skeleton),
        ) else {
            continue;
        };
        let bone = Transform::from_translation_rotation(
            capsule.translation(),
            simulation.interpolated_facing(time.fixed_alpha()),
        );
        if shown.simulation {
            draw_simulation(&mut gizmos, &bone);
            gizmos.line(bone.translation, character.translation(), SIMULATION_COLOR);
        }
        remember(
            &mut shown.trail,
            time.delta().as_secs_f32(),
            bone.translation,
        );
        if shown.history {
            gizmos.linestrip(
                shown.trail.iter().map(|(_, position)| *position),
                SIMULATION_COLOR,
            );
        }
        if !shown.matched && !shown.query {
            continue;
        }
        let Some(debug) = player
            .debug()
            .get::<MotionMatchingDebug>()
            .iter()
            .find(|debug| debug.initialised && !debug.failed)
        else {
            continue;
        };
        let Some(database) = databases.get(&debug.database) else {
            continue;
        };
        let Some(skeleton) = skeletons.get(&database.skeleton) else {
            continue;
        };
        let Some(sampled) = database
            .clips()
            .map(|handle| clips.get(handle))
            .collect::<Option<Vec<_>>>()
        else {
            continue;
        };
        if shown.matched
            && let Some(channels) = matched_shapes(debug, database, skeleton, &bone)
        {
            draw_channels(&mut gizmos, &bone, &channels, |channel| {
                channel_color(channel, Some(channel))
            });
        }
        if shown.query
            && let Some(channels) = query_shapes(debug, database, skeleton, &sampled, &bone)
        {
            draw_channels(&mut gizmos, &bone, &channels, |channel| {
                channel_color(channel, None)
            });
        }
    }
}

#[cfg(test)]
mod tests;

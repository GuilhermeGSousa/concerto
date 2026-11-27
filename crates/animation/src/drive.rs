use concerto_ecs::{Entity, Query, Res, SystemSet, system::input::InputSet};
use concerto_foundation::{assets::asset_store::AssetStore, time::Time};
use concerto_mesh::skeleton::{Skeleton, SkeletonComponent};

use crate::{graph::AnimationGraph, node::DriveNodeInputs, player::AnimationPlayer};

/// The set `drive_animation_players` runs in, for ordering systems against the drive pass.
#[derive(SystemSet, Clone, PartialEq, Eq, Hash, Debug)]
pub struct AnimationDriveSet;

/// Each fixed step, walks the drive pass of every animation player that has a skeleton, paused or not.
pub fn drive_animation_players(
    mut players: Query<(Entity, &mut AnimationPlayer, &SkeletonComponent)>,
    graphs: Res<AssetStore<AnimationGraph>>,
    skeletons: Res<AssetStore<Skeleton>>,
    inputs: InputSet<DriveNodeInputs>,
) {
    let dt = Time::fixed_delta_time().as_secs_f32();
    for (entity, mut player, component) in players.iter_mut() {
        player.drive(
            entity,
            &graphs,
            skeletons.get(component.skeleton()),
            &inputs,
            dt,
        );
    }
}

use concerto_ecs::{
    Entity,
    command::CommandQueue,
    query::{
        Query,
        filter::{Added, Changed},
    },
    resource::Res,
};
use concerto_foundation::{assets::asset_store::AssetStore, time::Time, transform::Transform};
use concerto_mesh::skeleton::SkeletonComponent;

use crate::{
    clip::AnimationClip,
    graph::AnimationGraph,
    player::{AnimationHandleComponent, AnimationPlayer},
    root::AnimationRootBone,
};

pub(crate) fn initialize_skeletons(
    skeletons: Query<(Entity, &SkeletonComponent), Added<SkeletonComponent>>,
    mut cmd: CommandQueue,
) {
    for (entity, skeleton) in skeletons.iter() {
        let Some(root) = skeleton.root() else {
            continue;
        };
        cmd.insert(AnimationPlayer::new(skeleton.bones().len()), entity);
        cmd.insert(AnimationRootBone::default(), root);
    }
}

pub(crate) fn animate_targets(
    animation_players: Query<(&mut AnimationPlayer, &SkeletonComponent)>,
    transforms: Query<&mut Transform>,
    root_bones: Query<&mut AnimationRootBone>,
    animation_graphs: Res<AssetStore<AnimationGraph>>,
    animation_clips: Res<AssetStore<AnimationClip>>,
) {
    for (mut animation_player, skeleton) in animation_players.iter() {
        if animation_player.is_paused() {
            continue;
        }
        animation_player.evaluate(
            &animation_clips,
            &animation_graphs,
            skeleton.bone_ids(),
            skeleton.bones(),
            &transforms,
            &root_bones,
        );
    }
}

pub(crate) fn update_animation_players(
    animation_players: Query<&mut AnimationPlayer>,
    animation_clips: Res<AssetStore<AnimationClip>>,
    animation_graphs: Res<AssetStore<AnimationGraph>>,
    time: Res<Time>,
) {
    let delta_time = time.delta().as_secs_f32();
    for mut animation_player in animation_players.iter() {
        if animation_player.is_paused() {
            continue;
        }
        animation_player.update(delta_time, &animation_clips, &animation_graphs);
    }
}

pub(crate) fn initialize_animation_players(
    animation_players: Query<
        (&mut AnimationPlayer, &AnimationHandleComponent),
        Changed<AnimationHandleComponent>,
    >,
    animation_graphs: Res<AssetStore<AnimationGraph>>,
    animation_clips: Res<AssetStore<AnimationClip>>,
) {
    for (mut animation_player, graph_handle) in animation_players.iter() {
        animation_player.initialize_graph(
            (*graph_handle).clone(),
            &animation_clips,
            &animation_graphs,
        );
    }
}

#[cfg(test)]
mod tests {
    use concerto_ecs::{IntoSystem, System, World, component::scene::SceneEntityRef};
    use concerto_foundation::assets::{AssetId, handle::AssetHandle};
    use concerto_mesh::skeleton::{Skeleton, SkeletonComponent};

    use super::initialize_skeletons;
    use crate::{player::AnimationPlayer, root::AnimationRootBone};

    #[test]
    fn rooted_skeleton_initializes_its_player_and_root_marker() {
        let mut world = World::default();
        let root = world.spawn(());
        let owner = world.spawn(SkeletonComponent {
            skeleton: AssetHandle::<Skeleton>::weak(AssetId::new()),
            bones: vec![SceneEntityRef::Entity(root)],
            bone_ids: vec![uuid::Uuid::new_v4()],
            root: Some(SceneEntityRef::Entity(root)),
        });

        let mut system = initialize_skeletons.into_system();
        system.initialize(&mut world);
        system.run_and_apply(&mut world);

        assert!(
            world
                .get_component_for_entity::<AnimationPlayer>(owner)
                .is_some()
        );
        assert!(
            world
                .get_component_for_entity::<AnimationRootBone>(root)
                .is_some()
        );
    }
}

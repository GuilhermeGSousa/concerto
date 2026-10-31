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
use concerto_mesh::skeleton::{Skeleton, SkeletonComponent};

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

pub fn animate_targets(
    animation_players: Query<(&mut AnimationPlayer, &SkeletonComponent)>,
    transforms: Query<&mut Transform>,
    root_bones: Query<&mut AnimationRootBone>,
    animation_graphs: Res<AssetStore<AnimationGraph>>,
    animation_clips: Res<AssetStore<AnimationClip>>,
    skeletons: Res<AssetStore<Skeleton>>,
) {
    for (mut animation_player, skeleton_component) in animation_players.iter() {
        if animation_player.is_paused() {
            continue;
        }
        let Some(skeleton) = skeletons.get(skeleton_component.skeleton()) else {
            continue;
        };
        animation_player.evaluate(
            &animation_clips,
            &animation_graphs,
            &skeleton.bone_ids,
            skeleton_component.bones(),
            &transforms,
            &root_bones,
        );
    }
}

pub fn update_animation_players(
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
    use concerto_foundation::{
        assets::{
            AssetId, asset_server::AssetServer, asset_store::AssetStore, handle::AssetHandle,
        },
        transform::Transform,
    };
    use concerto_mesh::skeleton::{JointPose, Skeleton, SkeletonComponent};
    use glam::{Mat4, Vec3};
    use uuid::Uuid;

    use super::{animate_targets, initialize_skeletons};
    use crate::{
        clip::{AnimationChanelOutput, AnimationChannel, AnimationClip},
        graph::{AnimationGraph, made_live},
        node::AnimationClipNode,
        player::AnimationPlayer,
        root::AnimationRootBone,
    };

    #[test]
    fn rooted_skeleton_initializes_its_player_and_root_marker() {
        let mut world = World::default();
        let root = world.spawn(());
        let owner = world.spawn(SkeletonComponent {
            skeleton: AssetHandle::<Skeleton>::weak(AssetId::new()),
            bones: vec![SceneEntityRef::Entity(root)],
            root: Some(SceneEntityRef::Entity(root)),
        });

        let mut system = initialize_skeletons.into_system();
        system.initialize(&mut world);
        system.run_and_apply((), &mut world);

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

    fn one_bone_skeleton(bone_id: Uuid) -> Skeleton {
        Skeleton::new(
            vec![Mat4::IDENTITY],
            vec![None],
            vec!["bone".to_string()],
            vec![JointPose::IDENTITY],
            vec![bone_id],
        )
        .unwrap()
    }

    #[test]
    fn a_player_whose_skeleton_asset_is_not_loaded_waits_and_runs_once_it_is() {
        let bone_id = Uuid::from_u128(7);
        let target = Vec3::new(1.0, 2.0, 3.0);

        let mut server = AssetServer::new();
        let mut clips = AssetStore::<AnimationClip>::new();
        let mut graphs = AssetStore::<AnimationGraph>::new();
        let skeletons = AssetStore::<Skeleton>::new();
        server.register_asset(&clips);
        server.register_asset(&graphs);
        server.register_asset(&skeletons);

        let mut clip = AnimationClip::default();
        clip.add_channel(
            bone_id,
            AnimationChannel::new(
                vec![0.0, 1.0],
                AnimationChanelOutput::from_translation([target.to_array(); 2].into_iter()),
            ),
        );
        let clip_handle = server.add(AnimationClip::default());
        clips.insert(clip_handle.id(), clip);
        let graph_handle = server.add(AnimationGraph::default());
        graphs.insert(
            graph_handle.id(),
            made_live(AnimationGraph::from_node(AnimationClipNode::new(
                clip_handle,
            ))),
        );
        let skeleton_handle = server.add(one_bone_skeleton(bone_id));

        let mut player = AnimationPlayer::new(1);
        player.initialize_graph(graph_handle, &clips, &graphs);

        let mut world = World::default();
        let bone = world.spawn(Transform::IDENTITY);
        world.spawn((
            player,
            SkeletonComponent {
                skeleton: skeleton_handle.clone(),
                bones: vec![SceneEntityRef::Entity(bone)],
                root: None,
            },
        ));
        world.insert_resource(clips);
        world.insert_resource(graphs);
        world.insert_resource(skeletons);

        let mut system = animate_targets.into_system();
        system.initialize(&mut world);

        system.run_and_apply((), &mut world);
        assert_eq!(
            world
                .get_component_for_entity::<Transform>(bone)
                .unwrap()
                .translation,
            Vec3::ZERO,
            "a player whose skeleton asset is missing must not pose its bones"
        );

        world
            .get_resource_mut::<AssetStore<Skeleton>>()
            .unwrap()
            .insert(skeleton_handle.id(), one_bone_skeleton(bone_id));
        system.run_and_apply((), &mut world);
        assert_eq!(
            world
                .get_component_for_entity::<Transform>(bone)
                .unwrap()
                .translation,
            target,
            "once the skeleton loads, the clip channel keyed by its bone id drives the bone"
        );
    }
}

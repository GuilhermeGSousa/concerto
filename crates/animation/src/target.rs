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
    pose::RootMotion,
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
        let evaluated = !animation_player.is_paused()
            && skeletons
                .get(skeleton_component.skeleton())
                .is_some_and(|skeleton| {
                    animation_player.evaluate(
                        &animation_clips,
                        &animation_graphs,
                        &skeleton.bone_ids,
                        &skeleton.rest_pose,
                        skeleton_component.bones(),
                        &transforms,
                        &root_bones,
                    )
                });
        if evaluated {
            continue;
        }
        let root_bone = skeleton_component
            .root()
            .and_then(|root| root_bones.get_entity(root));
        if let Some(mut root_bone) = root_bone {
            root_bone.root_motion = RootMotion::IDENTITY;
        }
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
    use glam::{Mat4, Quat, Vec2, Vec3};
    use uuid::Uuid;

    use super::{animate_targets, initialize_skeletons};
    use crate::{
        clip::{AnimationChanelOutput, AnimationChannel, AnimationClip},
        graph::{AnimationGraph, made_live},
        node::{AnimationClipNode, state_machine::AnimationStateMachine},
        player::AnimationPlayer,
        pose::RootMotion,
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
    const ROOT_ID: Uuid = Uuid::from_u128(9);

    struct Rig {
        server: AssetServer,
        clips: AssetStore<AnimationClip>,
        graphs: AssetStore<AnimationGraph>,
        skeletons: AssetStore<Skeleton>,
        skeleton: AssetHandle<Skeleton>,
        rest: JointPose,
    }

    impl Rig {
        fn new(rest_rotation: Quat) -> Self {
            let mut server = AssetServer::new();
            let clips = AssetStore::<AnimationClip>::new();
            let graphs = AssetStore::<AnimationGraph>::new();
            let skeletons = AssetStore::<Skeleton>::new();
            server.register_asset(&clips);
            server.register_asset(&graphs);
            server.register_asset(&skeletons);
            let rest = JointPose {
                rotation: rest_rotation,
                ..JointPose::IDENTITY
            };
            let skeleton = server.add(one_bone_skeleton(ROOT_ID));
            Self {
                server,
                clips,
                graphs,
                skeletons,
                skeleton,
                rest,
            }
        }

        fn loaded_skeleton(mut self) -> Self {
            let skeleton = Skeleton::new(
                vec![Mat4::IDENTITY],
                vec![None],
                vec!["root".to_string()],
                vec![self.rest.clone()],
                vec![ROOT_ID],
            )
            .unwrap();
            self.skeletons.insert(self.skeleton.id(), skeleton);
            self
        }

        fn clip(&mut self, channels: Vec<AnimationChannel>) -> AssetHandle<AnimationClip> {
            let mut clip = AnimationClip::default();
            for channel in channels {
                clip.add_channel(ROOT_ID, channel);
            }
            let handle = self.server.add(AnimationClip::default());
            self.clips.insert(handle.id(), clip);
            handle
        }

        fn moving_clip(&mut self, per_second: Vec3) -> AssetHandle<AnimationClip> {
            self.clip(vec![moving_channel(per_second)])
        }

        fn graph(&mut self, graph: AnimationGraph) -> AssetHandle<AnimationGraph> {
            let handle = self.server.add(AnimationGraph::default());
            self.graphs.insert(handle.id(), made_live(graph));
            handle
        }

        fn player(&self, graph: AssetHandle<AnimationGraph>) -> AnimationPlayer {
            let mut player = AnimationPlayer::new(1);
            player.initialize_graph(graph, &self.clips, &self.graphs);
            player
        }

        fn update(&self, player: &mut AnimationPlayer, delta_time: f32) {
            player.update(delta_time, &self.clips, &self.graphs);
        }

        fn root_motion_after_animating(self, player: AnimationPlayer) -> RootMotion {
            let mut world = World::default();
            let root = world.spawn((
                Transform::IDENTITY,
                AnimationRootBone {
                    root_motion: stale_motion(),
                },
            ));
            world.spawn((
                player,
                SkeletonComponent {
                    skeleton: self.skeleton,
                    bones: vec![SceneEntityRef::Entity(root)],
                    root: Some(SceneEntityRef::Entity(root)),
                },
            ));
            world.insert_resource(self.clips);
            world.insert_resource(self.graphs);
            world.insert_resource(self.skeletons);

            let mut system = animate_targets.into_system();
            system.initialize(&mut world);
            system.run_and_apply((), &mut world);

            world
                .get_component_for_entity::<AnimationRootBone>(root)
                .unwrap()
                .root_motion
        }
    }

    fn stale_motion() -> RootMotion {
        RootMotion {
            translation: Vec3::new(5.0, 0.0, 0.0),
            rotation: Quat::from_rotation_y(0.5),
        }
    }

    fn moving_channel(per_second: Vec3) -> AnimationChannel {
        AnimationChannel::new(
            vec![0.0, 1.0],
            AnimationChanelOutput::Translation(vec![Vec3::ZERO, per_second]),
        )
    }

    #[test]
    fn a_blend_space_gives_the_weighted_delta_of_its_inputs() {
        let mut rig = Rig::new(Quat::IDENTITY).loaded_skeleton();
        let along_x = rig.moving_clip(Vec3::new(4.0, 0.0, 0.0));
        let along_y = rig.moving_clip(Vec3::new(0.0, 4.0, 0.0));
        let along_z = rig.moving_clip(Vec3::new(0.0, 0.0, 4.0));
        let mut graph = AnimationGraph::new();
        graph
            .result_node()
            .with_blend_space_2d_input("at", |space| {
                space
                    .animation_clip_input(along_x, Vec2::ZERO)
                    .animation_clip_input(along_y, Vec2::X)
                    .animation_clip_input(along_z, Vec2::Y);
            });
        let graph = rig.graph(graph);
        let mut player = rig.player(graph);
        player.set_vec2_param("at", Vec2::new(0.25, 0.25));
        rig.update(&mut player, 0.5);

        let motion = rig.root_motion_after_animating(player);

        assert!(
            motion
                .translation
                .abs_diff_eq(Vec3::new(1.0, 0.5, 0.5), 1e-4),
            "got {}",
            motion.translation
        );
    }

    #[test]
    fn a_nested_graph_delta_reaches_the_root_bone_measured_from_the_skeleton_rest() {
        let rest = Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2);
        let mut rig = Rig::new(rest).loaded_skeleton();
        let clip = rig.clip(vec![
            moving_channel(Vec3::new(2.0, 0.0, 0.0)),
            AnimationChannel::new(
                vec![0.0, 1.0],
                AnimationChanelOutput::Rotation(vec![rest, Quat::from_rotation_y(1.0) * rest]),
            ),
        ]);
        let inner = rig.graph(AnimationGraph::from_node(AnimationClipNode::new(clip)));
        let machine = AnimationStateMachine::from_initial_state("only", inner, |_| {}).build();
        let outer = rig.graph(AnimationGraph::from_node(machine));
        let mut player = rig.player(outer);
        rig.update(&mut player, 0.5);

        let motion = rig.root_motion_after_animating(player);

        assert!(
            motion
                .translation
                .abs_diff_eq(Vec3::new(1.0, 0.0, 0.0), 1e-4),
            "got {}",
            motion.translation
        );
        assert!(
            motion.rotation.angle_between(Quat::from_rotation_y(0.5)) < 1e-3,
            "got {}",
            motion.rotation
        );
    }

    #[test]
    fn a_paused_player_leaves_no_root_motion_on_the_root() {
        let mut rig = Rig::new(Quat::IDENTITY).loaded_skeleton();
        let clip = rig.moving_clip(Vec3::X);
        let graph = rig.graph(AnimationGraph::from_node(AnimationClipNode::new(clip)));
        let mut player = rig.player(graph);
        rig.update(&mut player, 0.5);
        player.set_paused(true);

        assert_eq!(
            rig.root_motion_after_animating(player),
            RootMotion::IDENTITY
        );
    }

    #[test]
    fn an_unloaded_skeleton_leaves_no_root_motion_on_the_root() {
        let mut rig = Rig::new(Quat::IDENTITY);
        let clip = rig.moving_clip(Vec3::X);
        let graph = rig.graph(AnimationGraph::from_node(AnimationClipNode::new(clip)));
        let mut player = rig.player(graph);
        rig.update(&mut player, 0.5);

        assert_eq!(
            rig.root_motion_after_animating(player),
            RootMotion::IDENTITY
        );
    }

    #[test]
    fn a_swap_to_an_unloaded_graph_leaves_no_root_motion_on_the_root() {
        let mut rig = Rig::new(Quat::IDENTITY).loaded_skeleton();
        let clip = rig.moving_clip(Vec3::X);
        let graph = rig.graph(AnimationGraph::from_node(AnimationClipNode::new(clip)));
        let mut player = rig.player(graph);
        rig.update(&mut player, 0.5);
        player.initialize_graph(AssetHandle::weak(AssetId::new()), &rig.clips, &rig.graphs);

        assert_eq!(
            rig.root_motion_after_animating(player),
            RootMotion::IDENTITY
        );
    }
}

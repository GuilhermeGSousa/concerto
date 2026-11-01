use concerto_ecs::{
    Entity,
    command::CommandQueue,
    entity::hierarchy::ChildOf,
    query::{
        Query,
        filter::{Added, Changed},
    },
    resource::Res,
};
use concerto_foundation::{
    assets::asset_store::AssetStore,
    time::Time,
    transform::{GlobalTransform, Transform},
};
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
    parents: Query<&ChildOf>,
    globals: Query<&GlobalTransform>,
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
                        &parents,
                        &globals,
                    )
                });
        if evaluated {
            continue;
        }
        for bone in skeleton_component.bones() {
            if let Some(mut root_bone) = bone.entity().and_then(|bone| root_bones.get_entity(bone))
            {
                root_bone.root_motion = RootMotion::IDENTITY;
            }
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
    use concerto_ecs::{
        Entity, IntoSystem, System, World, component::scene::SceneEntityRef,
        entity::hierarchy::ChildOf,
    };
    use concerto_foundation::{
        assets::{
            AssetId, asset_server::AssetServer, asset_store::AssetStore, handle::AssetHandle,
        },
        transform::{GlobalTransform, Transform},
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
    const CHILD_ID: Uuid = Uuid::from_u128(10);
    const CHILD_REST: Vec3 = Vec3::new(0.0, 0.5, 0.0);

    struct Rig {
        server: AssetServer,
        clips: AssetStore<AnimationClip>,
        graphs: AssetStore<AnimationGraph>,
        skeletons: AssetStore<Skeleton>,
        skeleton: AssetHandle<Skeleton>,
        rest: JointPose,
        names_root: bool,
        leading_root_bone: bool,
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
                names_root: true,
                leading_root_bone: false,
            }
        }

        fn resting_at(mut self, translation: Vec3) -> Self {
            self.rest.translation = translation;
            self
        }

        fn unnamed_root(mut self) -> Self {
            self.names_root = false;
            self
        }

        fn with_leading_root_bone(mut self) -> Self {
            self.leading_root_bone = true;
            self
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

        fn loaded_two_bone_skeleton(mut self) -> Self {
            let skeleton = Skeleton::new(
                vec![Mat4::IDENTITY; 2],
                vec![None, Some(0)],
                vec!["root".to_string(), "child".to_string()],
                vec![
                    self.rest.clone(),
                    JointPose {
                        translation: CHILD_REST,
                        ..JointPose::IDENTITY
                    },
                ],
                vec![ROOT_ID, CHILD_ID],
            )
            .unwrap();
            self.skeletons.insert(self.skeleton.id(), skeleton);
            self
        }

        fn clip(&mut self, channels: Vec<AnimationChannel>) -> AssetHandle<AnimationClip> {
            self.clip_of(ROOT_ID, channels)
        }

        fn child_clip(&mut self, channels: Vec<AnimationChannel>) -> AssetHandle<AnimationClip> {
            self.clip_of(CHILD_ID, channels)
        }

        fn clip_of(
            &mut self,
            bone_id: Uuid,
            channels: Vec<AnimationChannel>,
        ) -> AssetHandle<AnimationClip> {
            let mut clip = AnimationClip::default();
            for channel in channels {
                clip.add_channel(bone_id, channel);
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
            let bone_count = self
                .skeletons
                .get(&self.skeleton)
                .map_or(1, Skeleton::bone_count);
            let mut player = AnimationPlayer::new(bone_count);
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
            let mut bones = vec![SceneEntityRef::Entity(root)];
            if self.leading_root_bone {
                let leading = world.spawn(AnimationRootBone::default());
                bones.insert(0, SceneEntityRef::Entity(leading));
            }
            world.spawn((
                player,
                SkeletonComponent {
                    skeleton: self.skeleton,
                    bones,
                    root: self.names_root.then_some(SceneEntityRef::Entity(root)),
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

        fn spawn_two_bones(self, player: AnimationPlayer) -> TwoBones {
            let mut world = World::default();
            let root = world.spawn((Transform::IDENTITY, AnimationRootBone::default()));
            let child = world.spawn(Transform::IDENTITY);
            let owner = world.spawn((
                player,
                SkeletonComponent {
                    skeleton: self.skeleton,
                    bones: vec![SceneEntityRef::Entity(root), SceneEntityRef::Entity(child)],
                    root: Some(SceneEntityRef::Entity(root)),
                },
            ));
            world.insert_resource(self.clips);
            world.insert_resource(self.graphs);
            world.insert_resource(self.skeletons);
            TwoBones {
                world,
                owner,
                root,
                child,
            }
        }

        fn playing_on_the_root(mut self, channels: Vec<AnimationChannel>) -> TwoBones {
            let clip = self.clip(channels);
            let graph = self.graph(AnimationGraph::from_node(AnimationClipNode::new(clip)));
            let player = self.player(graph);
            self.spawn_two_bones(player)
        }
    }

    struct TwoBones {
        world: World,
        owner: Entity,
        root: Entity,
        child: Entity,
    }

    impl TwoBones {
        fn animate(&mut self) {
            let mut system = animate_targets.into_system();
            system.initialize(&mut self.world);
            system.run_and_apply((), &mut self.world);
        }

        fn play(&mut self, graph: AssetHandle<AnimationGraph>) {
            let owner = self.owner;
            self.world
                .resource_scope(|world, clips: &AssetStore<AnimationClip>| {
                    world.resource_scope(|world, graphs: &AssetStore<AnimationGraph>| {
                        world
                            .get_component_for_entity_mut::<AnimationPlayer>(owner)
                            .unwrap()
                            .initialize_graph(graph, clips, graphs);
                    });
                });
        }

        fn put_root_under(&mut self, space: Mat4) {
            let parent = self.world.spawn(GlobalTransform::new(space));
            self.world.insert(ChildOf::new(parent), self.root);
        }

        fn root(&self) -> &Transform {
            self.world
                .get_component_for_entity::<Transform>(self.root)
                .unwrap()
        }

        fn child(&self) -> &Transform {
            self.world
                .get_component_for_entity::<Transform>(self.child)
                .unwrap()
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

    fn held(output: AnimationChanelOutput) -> AnimationChannel {
        AnimationChannel::new(vec![0.0, 1.0], output)
    }

    fn lying_rest() -> Quat {
        Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)
    }

    #[test]
    fn a_root_shows_its_pitch_and_its_yaw_does_not_reach_the_transform() {
        let pitch = Quat::from_rotation_x(0.3);
        let turned = Quat::from_rotation_y(1.1) * pitch * lying_rest();
        let mut bones = Rig::new(lying_rest())
            .loaded_two_bone_skeleton()
            .playing_on_the_root(vec![held(AnimationChanelOutput::Rotation(vec![
                turned, turned,
            ]))]);

        bones.animate();

        let shown = bones.root().rotation;
        assert!(
            shown.angle_between(pitch * lying_rest()) < 1e-3,
            "got {shown}"
        );
    }

    #[test]
    fn a_roots_height_follows_the_clip_and_its_ground_translation_stays_at_rest() {
        let keyed = Vec3::new(3.0, 1.25, 4.0);
        let mut bones = Rig::new(lying_rest())
            .resting_at(Vec3::new(1.0, 0.9, -2.0))
            .loaded_two_bone_skeleton()
            .playing_on_the_root(vec![held(AnimationChanelOutput::Translation(vec![
                keyed, keyed,
            ]))]);

        bones.animate();

        let shown = bones.root().translation;
        assert!(
            shown.abs_diff_eq(Vec3::new(1.0, 1.25, -2.0), 1e-5),
            "got {shown}"
        );
    }

    #[test]
    fn a_root_with_a_scale_channel_shows_its_scale() {
        let keyed = Vec3::new(1.5, 2.0, 0.5);
        let mut bones = Rig::new(lying_rest())
            .loaded_two_bone_skeleton()
            .playing_on_the_root(vec![held(AnimationChanelOutput::Scale(vec![keyed, keyed]))]);

        bones.animate();

        assert_eq!(bones.root().scale, keyed);
    }

    #[test]
    fn under_a_rotated_parent_a_roots_height_is_read_along_the_parents_up() {
        let keyed = Vec3::new(1.0, 2.0, 3.0);
        let mut bones = Rig::new(lying_rest())
            .loaded_two_bone_skeleton()
            .playing_on_the_root(vec![held(AnimationChanelOutput::Translation(vec![
                keyed, keyed,
            ]))]);
        bones.put_root_under(Mat4::from_quat(lying_rest()));

        bones.animate();

        let shown = bones.root().translation;
        assert!(
            shown.abs_diff_eq(Vec3::new(0.0, 0.0, 3.0), 1e-5),
            "got {shown}"
        );
    }

    #[test]
    fn under_a_parent_scaled_to_nothing_a_roots_height_is_read_along_y() {
        let keyed = Vec3::new(1.0, 2.0, 3.0);
        let mut bones = Rig::new(lying_rest())
            .loaded_two_bone_skeleton()
            .playing_on_the_root(vec![held(AnimationChanelOutput::Translation(vec![
                keyed, keyed,
            ]))]);
        bones.put_root_under(Mat4::from_scale(Vec3::ZERO));

        bones.animate();

        let shown = bones.root().translation;
        assert!(
            shown.abs_diff_eq(Vec3::new(0.0, 2.0, 0.0), 1e-5),
            "got {shown}"
        );
    }

    #[test]
    fn a_child_joint_with_no_position_channel_keeps_its_rest_translation() {
        let turn = Quat::from_rotation_z(0.4);
        let mut rig = Rig::new(Quat::IDENTITY).loaded_two_bone_skeleton();
        let clip = rig.child_clip(vec![AnimationChannel::new(
            vec![0.0, 1.0],
            AnimationChanelOutput::Rotation(vec![turn, turn]),
        )]);
        let graph = rig.graph(AnimationGraph::from_node(AnimationClipNode::new(clip)));
        let player = rig.player(graph);
        let mut bones = rig.spawn_two_bones(player);

        bones.animate();

        assert_eq!(bones.child().translation, CHILD_REST);
        assert!(bones.child().rotation.abs_diff_eq(turn, 1e-5));
    }

    #[test]
    fn a_joint_with_no_channels_in_a_second_clip_shows_rest_not_the_first_clips_value() {
        let keyed = Vec3::new(1.0, 2.0, 3.0);
        let mut rig = Rig::new(Quat::IDENTITY).loaded_two_bone_skeleton();
        let first = rig.child_clip(vec![AnimationChannel::new(
            vec![0.0, 1.0],
            AnimationChanelOutput::Translation(vec![keyed, keyed]),
        )]);
        let second = rig.moving_clip(Vec3::X);
        let first = rig.graph(AnimationGraph::from_node(AnimationClipNode::new(first)));
        let second = rig.graph(AnimationGraph::from_node(AnimationClipNode::new(second)));
        let player = rig.player(first);
        let mut bones = rig.spawn_two_bones(player);

        bones.animate();
        bones.animate();
        assert_eq!(
            bones.child().translation,
            keyed,
            "every pose in the pool holds the first clip's value"
        );

        bones.play(second);
        bones.animate();

        assert_eq!(bones.child().translation, CHILD_REST);
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
    fn a_paused_player_clears_a_root_bone_the_skeleton_does_not_name_as_its_root() {
        let mut rig = Rig::new(Quat::IDENTITY).loaded_skeleton();
        let clip = rig.moving_clip(Vec3::X);
        let graph = rig.graph(AnimationGraph::from_node(AnimationClipNode::new(clip)));
        let mut player = rig.player(graph);
        rig.update(&mut player, 0.5);
        player.set_paused(true);

        assert_eq!(
            rig.unnamed_root().root_motion_after_animating(player),
            RootMotion::IDENTITY
        );
    }

    #[test]
    fn a_paused_player_clears_every_root_bone() {
        let mut rig = Rig::new(Quat::IDENTITY).loaded_skeleton();
        let clip = rig.moving_clip(Vec3::X);
        let graph = rig.graph(AnimationGraph::from_node(AnimationClipNode::new(clip)));
        let mut player = rig.player(graph);
        rig.update(&mut player, 0.5);
        player.set_paused(true);

        assert_eq!(
            rig.with_leading_root_bone()
                .root_motion_after_animating(player),
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

use concerto_ecs::{
    Entity,
    command::CommandQueue,
    entity::hierarchy::ChildOf,
    query::{
        Query,
        filter::{Added, Changed},
    },
    resource::Res,
    system::input::InputSet,
};
use concerto_foundation::{
    assets::{asset_store::AssetStore, handle::AssetHandle},
    time::Time,
    transform::{GlobalTransform, Transform},
};
use concerto_mesh::skeleton::{Skeleton, SkeletonComponent};

use crate::{
    clip::{AnimationClip, ClipEventData},
    event::Fired,
    graph::AnimationGraph,
    node::{CreateNodeInputs, EvaluateNodeInputs, UpdateNodeInputs},
    player::{AnimationHandleComponent, AnimationPlayer},
    pose::{FiredEvent, RootMotion},
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
    animation_players: Query<(Entity, &mut AnimationPlayer, &SkeletonComponent)>,
    transforms: Query<&mut Transform>,
    root_bones: Query<&mut AnimationRootBone>,
    parents: Query<&ChildOf>,
    globals: Query<&GlobalTransform>,
    animation_graphs: Res<AssetStore<AnimationGraph>>,
    clips: Res<AssetStore<AnimationClip>>,
    skeletons: Res<AssetStore<Skeleton>>,
    inputs: InputSet<EvaluateNodeInputs>,
    mut cmd: CommandQueue,
) {
    for (entity, mut animation_player, skeleton_component) in animation_players.iter() {
        let evaluated = !animation_player.is_paused()
            && skeletons
                .get(skeleton_component.skeleton())
                .is_some_and(|skeleton| {
                    animation_player.evaluate(
                        &animation_graphs,
                        &inputs,
                        skeleton,
                        skeleton_component.bones(),
                        &transforms,
                        &root_bones,
                        &parents,
                        &globals,
                    )
                });
        if evaluated {
            for event in animation_player.fired.drain(..) {
                trigger(&clips, event, entity, &mut cmd);
            }
            continue;
        }
        if !animation_player.fired.is_empty() {
            animation_player.fired.clear();
        }
        for bone in skeleton_component.bones() {
            if let Some(mut root_bone) = bone.entity().and_then(|bone| root_bones.get_entity(bone))
            {
                root_bone.root_motion = RootMotion::IDENTITY;
            }
        }
    }
}

fn trigger(
    clips: &AssetStore<AnimationClip>,
    event: FiredEvent,
    entity: Entity,
    cmd: &mut CommandQueue,
) {
    let Some(key) = clips
        .get(&AssetHandle::weak(event.clip))
        .and_then(|clip| clip.events().get(event.key as usize))
    else {
        return;
    };
    if let ClipEventData::Live { value, data, .. } = &key.event {
        let fired = Fired {
            weight: event.weight,
            time: event.time,
            mirrored: event.mirrored,
        };
        (data.trigger)(&**value, fired, entity, cmd);
    }
}

pub fn update_animation_players(
    animation_players: Query<(&mut AnimationPlayer, Option<&SkeletonComponent>)>,
    animation_graphs: Res<AssetStore<AnimationGraph>>,
    skeletons: Res<AssetStore<Skeleton>>,
    time: Res<Time>,
    inputs: InputSet<UpdateNodeInputs>,
) {
    let delta_time = time.delta().as_secs_f32();
    for (mut animation_player, component) in animation_players.iter() {
        if animation_player.is_paused() {
            continue;
        }
        let skeleton = component.and_then(|component| skeletons.get(component.skeleton()));
        animation_player.update(delta_time, &animation_graphs, skeleton, &inputs);
    }
}

pub(crate) fn initialize_animation_players(
    animation_players: Query<
        (&mut AnimationPlayer, &AnimationHandleComponent),
        Changed<AnimationHandleComponent>,
    >,
    animation_graphs: Res<AssetStore<AnimationGraph>>,
    inputs: InputSet<CreateNodeInputs>,
) {
    for (mut animation_player, graph_handle) in animation_players.iter() {
        animation_player.initialize_graph((*graph_handle).clone(), &animation_graphs, &inputs);
    }
}

#[cfg(test)]
mod tests {
    use concerto_ecs::{
        Entity, IntoSystem, System, World,
        component::scene::SceneEntityRef,
        entity::hierarchy::ChildOf,
        resource::{ResMut, Resource},
        signal::{
            On,
            listener::{IntoListener, Listener},
        },
    };
    use concerto_foundation::{
        assets::{
            AssetId, asset_server::AssetServer, asset_store::AssetStore, handle::AssetHandle,
        },
        transform::{GlobalTransform, Transform},
    };
    use concerto_mesh::skeleton::{JointPose, Skeleton, SkeletonComponent};
    use concerto_name::{Name, name};
    use concerto_reflect::{Reflect, TypeRegistry};
    use glam::{Affine3A, Mat4, Quat, Vec2, Vec3};
    use uuid::Uuid;

    use super::{animate_targets, initialize_animation_players, initialize_skeletons};
    use crate::{
        clip::{AnimationChanelOutput, AnimationChannel, AnimationClip},
        evaluation::AnimationGraphContext,
        event::{AnimationEvent, AnimationSignal},
        graph::{AnimationGraph, AnimationNodeIndex, made_live},
        node::{
            AnimationClipNode, AnimationNode, BlendNode, IkGoal, IkTarget, ReflectAnimationNode,
            TwoBoneIkNode, register_node_inputs, state_machine::AnimationStateMachine,
        },
        player::{AnimationHandleComponent, AnimationPlayer},
        plugin::register_animation_types,
        pose::{AnimationFrame, EvaluatedFrame, FiredEvent, FramePool, RootMotion},
        root::AnimationRootBone,
        test_walks::Walks,
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
            vec!["bone".into()],
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
        let walks = Walks::new(clips, graphs);
        walks.initialize(&mut player, graph_handle);

        let mut world = walks.into_world();
        let bone = world.spawn(Transform::IDENTITY);
        world.spawn((
            player,
            SkeletonComponent {
                skeleton: skeleton_handle.clone(),
                bones: vec![SceneEntityRef::Entity(bone)],
                root: None,
            },
        ));
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
        walks: Walks,
        skeletons: AssetStore<Skeleton>,
        skeleton: AssetHandle<Skeleton>,
        rest: JointPose,
        names_root: bool,
        leading_root_bone: bool,
    }

    impl Rig {
        fn new(rest_rotation: Quat) -> Self {
            Self::registering(rest_rotation, |_| {})
        }

        fn probing() -> Self {
            Self::registering(Quat::IDENTITY, register_node_inputs::<Probe>)
        }

        fn registering(rest_rotation: Quat, register: impl FnOnce(&mut World)) -> Self {
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
                walks: Walks::with(clips, graphs, register),
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
                vec!["root".into()],
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
                vec!["root".into(), "child".into()],
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
            self.walks.clips_mut().insert(handle.id(), clip);
            handle
        }

        fn moving_clip(&mut self, per_second: Vec3) -> AssetHandle<AnimationClip> {
            self.clip(vec![moving_channel(per_second)])
        }

        fn graph(&mut self, graph: AnimationGraph) -> AssetHandle<AnimationGraph> {
            let handle = self.server.add(AnimationGraph::default());
            self.walks
                .graphs_mut()
                .insert(handle.id(), made_live(graph));
            handle
        }

        fn probe_graph(&mut self) -> AssetHandle<AnimationGraph> {
            let mut registry = TypeRegistry::default();
            register_animation_types(&mut registry);
            registry.register::<Probe>();
            let mut graph = AnimationGraph::from_node(Probe {});
            graph.make_live(&registry, None).unwrap();
            let handle = self.server.add(AnimationGraph::default());
            self.walks.graphs_mut().insert(handle.id(), graph);
            handle
        }

        fn player(&self, graph: AssetHandle<AnimationGraph>) -> AnimationPlayer {
            let bone_count = self
                .skeletons
                .get(&self.skeleton)
                .map_or(1, Skeleton::bone_count);
            let mut player = AnimationPlayer::new(bone_count);
            self.walks.initialize(&mut player, graph);
            player
        }

        fn update(&self, player: &mut AnimationPlayer, delta_time: f32) {
            self.walks.advance(player, delta_time);
        }

        fn root_motion_after_animating(self, player: AnimationPlayer) -> RootMotion {
            let mut world = self.walks.into_world();
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
            world.insert_resource(self.skeletons);

            let mut system = animate_targets.into_system();
            system.initialize(&mut world);
            system.run_and_apply((), &mut world);

            world
                .get_component_for_entity::<AnimationRootBone>(root)
                .unwrap()
                .root_motion
        }

        fn firing_clip(&mut self, time: f32, name: &str) -> AssetHandle<AnimationClip> {
            let mut clip = AnimationClip::default();
            clip.add_channel(ROOT_ID, moving_channel(Vec3::X));
            clip.add_event(
                time,
                AnimationEvent {
                    name: Name::new(name),
                },
            );
            let handle = self.server.add(AnimationClip::default());
            self.walks.clips_mut().insert(handle.id(), clip);
            handle
        }

        fn blending_left_and_right(&mut self) -> AnimationPlayer {
            let left = self.firing_clip(0.25, "left");
            let right = self.firing_clip(0.25, "right");
            let mut left_index = None;
            let mut right_index = None;
            let mut graph = AnimationGraph::new();
            graph.result_node().with_input(BlendNode {}, |mut blend| {
                blend
                    .with_input(AnimationClipNode::new(left), |node| {
                        left_index = Some(node.index())
                    })
                    .with_input(AnimationClipNode::new(right), |node| {
                        right_index = Some(node.index())
                    });
            });
            let graph = self.graph(graph);
            let mut player = self.player(graph);
            let weight = |index: Option<AnimationNodeIndex>| index.expect("the input was added");
            player.set_node_weight(&weight(left_index), 0.25);
            player.set_node_weight(&weight(right_index), 0.75);
            player
        }

        fn deliver(self, player: AnimationPlayer) -> Delivery {
            let mut world = self.walks.into_world();
            world.insert_resource(Heard::default());
            world.insert_resource(HeardGlobally::default());
            world.register_component::<Listener<AnimationSignal<AnimationEvent>>>();
            world.add_listener(
                |_: On<AnimationSignal<AnimationEvent>>, mut heard: ResMut<HeardGlobally>| {
                    heard.0 += 1;
                },
            );
            let root = world.spawn((Transform::IDENTITY, AnimationRootBone::default()));
            let owner = world.spawn((
                player,
                SkeletonComponent {
                    skeleton: self.skeleton,
                    bones: vec![SceneEntityRef::Entity(root)],
                    root: Some(SceneEntityRef::Entity(root)),
                },
                hear().into_listener(),
            ));
            world.insert_resource(self.skeletons);

            let mut system = animate_targets.into_system();
            system.initialize(&mut world);
            system.run_and_apply((), &mut world);

            let mut heard = std::mem::take(&mut world.get_resource_mut::<Heard>().unwrap().0);
            heard.sort_by_key(|event| event.name.as_str().to_owned());
            Delivery {
                heard,
                heard_globally: world.get_resource::<HeardGlobally>().unwrap().0,
                still_fired: world
                    .get_component_for_entity::<AnimationPlayer>(owner)
                    .unwrap()
                    .fired
                    .len(),
            }
        }

        fn spawn_two_bones(self, player: AnimationPlayer) -> TwoBones {
            let mut world = self.walks.into_world();
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

    #[derive(Debug, PartialEq)]
    struct HeardEvent {
        name: Name,
        weight: f32,
        time: f32,
    }

    fn heard_event(name: Name, weight: f32, time: f32) -> HeardEvent {
        HeardEvent { name, weight, time }
    }

    #[derive(Resource, Default)]
    struct Heard(Vec<HeardEvent>);

    #[derive(Resource, Default)]
    struct HeardGlobally(usize);

    struct Delivery {
        heard: Vec<HeardEvent>,
        heard_globally: usize,
        still_fired: usize,
    }

    fn hear() -> Listener<AnimationSignal<AnimationEvent>> {
        (|on: On<AnimationSignal<AnimationEvent>>, mut heard: ResMut<Heard>| {
            let fired = on.signal();
            heard
                .0
                .push(heard_event(fired.signal.name, fired.weight, fired.time));
        })
        .into_listener()
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
            self.world
                .insert(AnimationHandleComponent::new(graph), self.owner);
            let mut system = initialize_animation_players.into_system();
            system.initialize(&mut self.world);
            system.run_and_apply((), &mut self.world);
        }

        fn put_root_under(&mut self, space: Mat4) {
            let parent = self.world.spawn(GlobalTransform::new(space));
            self.world.insert(ChildOf::new(parent), self.root);
        }

        fn put_root_under_a_parent_with_no_global(&mut self) {
            let parent = self.world.spawn(());
            self.world.insert(ChildOf::new(parent), self.root);
        }

        fn model_to_world_seen(mut self) -> Option<Affine3A> {
            let graphs = self
                .world
                .remove_resource::<AssetStore<AnimationGraph>>()
                .unwrap();
            let player = self
                .world
                .get_component_for_entity_mut::<AnimationPlayer>(self.owner)
                .unwrap();
            let mut seen = None;
            player.for_each_state_mut::<Probe>(&graphs, |_, state, _| seen = *state);
            seen
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

    #[derive(Default, Reflect)]
    #[reflect(AnimationNode)]
    struct Probe {}

    impl AnimationNode for Probe {
        type State = Option<Affine3A>;
        type CreateInput = ();
        type UpdateInput = ();
        type EvaluateInput = ();

        fn create_state(&self, _: &(), _: &AnimationGraphContext<'_>) -> Option<Affine3A> {
            None
        }

        fn update(&self, _: &mut Option<Affine3A>, _: f32, _: &(), _: &AnimationGraphContext<'_>) {}

        fn evaluate(
            &self,
            seen: &mut Option<Affine3A>,
            _: &(),
            context: &AnimationGraphContext<'_>,
            _: &[Uuid],
            _: &[EvaluatedFrame],
            _: &mut FramePool,
            _: &mut AnimationFrame,
        ) {
            *seen = context.model_to_world();
        }

        fn reset(&self, _: &mut Option<Affine3A>) {}
    }

    fn probed_bones() -> TwoBones {
        let mut rig = Rig::probing().loaded_two_bone_skeleton();
        let graph = rig.probe_graph();
        let player = rig.player(graph);
        rig.spawn_two_bones(player)
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
            .with_blend_space_2d_input(name!("at"), |space| {
                space
                    .animation_clip_input(along_x, Vec2::ZERO)
                    .animation_clip_input(along_y, Vec2::X)
                    .animation_clip_input(along_z, Vec2::Y);
            });
        let graph = rig.graph(graph);
        let mut player = rig.player(graph);
        player.set_vec2_param(name!("at"), Vec2::new(0.25, 0.25));
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
        let machine =
            AnimationStateMachine::from_initial_state(name!("only"), inner, |_| {}).build();
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
        rig.walks
            .initialize(&mut player, AssetHandle::weak(AssetId::new()));

        assert_eq!(
            rig.root_motion_after_animating(player),
            RootMotion::IDENTITY
        );
    }

    #[test]
    fn a_crossed_event_reaches_the_listener_on_the_player_with_its_weight_and_time() {
        let mut rig = Rig::new(Quat::IDENTITY).loaded_skeleton();
        let mut player = rig.blending_left_and_right();
        rig.update(&mut player, 0.5);

        let delivery = rig.deliver(player);

        assert_eq!(
            delivery.heard,
            [
                heard_event(name!("left"), 0.25, 0.25),
                heard_event(name!("right"), 0.75, 0.25)
            ]
        );
        assert_eq!(delivery.still_fired, 0);
        assert_eq!(
            delivery.heard_globally, 0,
            "a global listener never hears a signal triggered on an entity"
        );
    }

    #[test]
    fn a_graph_sees_the_global_of_bone_zeros_parent_as_model_to_world() {
        let space = Mat4::from_scale_rotation_translation(
            Vec3::new(2.0, 1.0, 0.5),
            Quat::from_rotation_y(0.7),
            Vec3::new(1.0, 2.0, 3.0),
        );
        let mut bones = probed_bones();
        bones.put_root_under(space);

        bones.animate();

        let seen = bones.model_to_world_seen().unwrap();
        assert!(
            seen.abs_diff_eq(Affine3A::from_mat4(space), 1e-5),
            "got {seen:?}"
        );
    }

    #[test]
    fn a_threshold_drops_the_sources_at_or_below_it() {
        let mut rig = Rig::new(Quat::IDENTITY).loaded_skeleton();
        let mut player = rig.blending_left_and_right();
        player.set_event_threshold(0.25);
        rig.update(&mut player, 0.5);

        assert_eq!(
            rig.deliver(player).heard,
            [heard_event(name!("right"), 0.75, 0.25)]
        );
    }

    fn left_fired(clip: AssetId) -> FiredEvent {
        FiredEvent {
            clip,
            key: 0,
            time: 0.25,
            weight: 1.0,
            mirrored: false,
        }
    }

    #[test]
    fn a_paused_player_clears_what_it_fired_and_triggers_nothing() {
        let mut rig = Rig::new(Quat::IDENTITY).loaded_skeleton();
        let clip = rig.firing_clip(0.25, "left");
        let graph = rig.graph(AnimationGraph::from_node(AnimationClipNode::new(
            clip.clone(),
        )));
        let mut player = rig.player(graph);
        player.fired.push(left_fired(clip.id()));
        player.set_paused(true);

        let delivery = rig.deliver(player);

        assert!(delivery.heard.is_empty());
        assert_eq!(delivery.still_fired, 0);
    }

    #[test]
    fn a_player_whose_graph_is_not_loaded_clears_what_it_fired_and_triggers_nothing() {
        let mut rig = Rig::new(Quat::IDENTITY).loaded_skeleton();
        let clip = rig.firing_clip(0.25, "left");
        let mut player = rig.player(AssetHandle::weak(AssetId::new()));
        player.fired.push(left_fired(clip.id()));

        let delivery = rig.deliver(player);

        assert!(delivery.heard.is_empty());
        assert_eq!(delivery.still_fired, 0);
    }

    #[test]
    fn a_graph_sees_identity_as_model_to_world_when_bone_zero_has_no_parent() {
        let mut bones = probed_bones();

        bones.animate();

        assert_eq!(bones.model_to_world_seen(), Some(Affine3A::IDENTITY));
    }

    #[test]
    fn a_graph_sees_identity_as_model_to_world_when_bone_zeros_parent_has_no_global() {
        let mut bones = probed_bones();
        bones.put_root_under_a_parent_with_no_global();

        bones.animate();

        assert_eq!(bones.model_to_world_seen(), Some(Affine3A::IDENTITY));
    }

    #[test]
    fn a_two_bone_ik_graph_puts_the_hand_on_a_world_target_under_the_skeletons_parent() {
        let mut server = AssetServer::new();
        let clips = AssetStore::<AnimationClip>::new();
        let graphs = AssetStore::<AnimationGraph>::new();
        let mut skeletons = AssetStore::<Skeleton>::new();
        server.register_asset(&clips);
        server.register_asset(&graphs);
        server.register_asset(&skeletons);
        let arm = || {
            Skeleton::new(
                vec![Mat4::IDENTITY; 3],
                vec![None, Some(0), Some(1)],
                vec!["upper".into(), "lower".into(), "hand".into()],
                vec![
                    JointPose::IDENTITY,
                    JointPose {
                        translation: Vec3::new(0.0, -1.0, 0.0),
                        ..JointPose::IDENTITY
                    },
                    JointPose {
                        translation: Vec3::new(0.3, -0.8, 0.0),
                        ..JointPose::IDENTITY
                    },
                ],
                vec![Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(3)],
            )
            .unwrap()
        };
        let skeleton = server.add(arm());
        skeletons.insert(skeleton.id(), arm());

        let mut walks = Walks::new(clips, graphs);
        let graph = server.add(AnimationGraph::default());
        walks.graphs_mut().insert(
            graph.id(),
            made_live(AnimationGraph::from_node(TwoBoneIkNode {
                tip: "hand".into(),
                target: IkGoal::Param {
                    param: name!("hand_target"),
                },
                ..TwoBoneIkNode::default()
            })),
        );
        let space = Affine3A::from_rotation_translation(
            Quat::from_rotation_y(0.7),
            Vec3::new(1.0, 2.0, 3.0),
        );
        let target = space.transform_point3(Vec3::new(0.6, -1.2, 0.4));
        let mut player = AnimationPlayer::new(3);
        walks.initialize(&mut player, graph);
        player.set_param(
            name!("hand_target"),
            IkTarget {
                position: target,
                rotation: None,
            },
        );

        let mut world = walks.into_world();
        let parent = world.spawn(GlobalTransform::new(Mat4::from(space)));
        let bones: Vec<Entity> = (0..3).map(|_| world.spawn(Transform::IDENTITY)).collect();
        world.insert(ChildOf::new(parent), bones[0]);
        world.spawn((
            player,
            SkeletonComponent {
                skeleton,
                bones: bones.iter().copied().map(SceneEntityRef::Entity).collect(),
                root: None,
            },
        ));
        world.insert_resource(skeletons);
        let mut system = animate_targets.into_system();
        system.initialize(&mut world);
        system.run_and_apply((), &mut world);

        let model = bones.iter().fold(Mat4::IDENTITY, |model, bone| {
            model
                * world
                    .get_component_for_entity::<Transform>(*bone)
                    .unwrap()
                    .compute_matrix()
        });
        let hand = space.transform_point3(model.w_axis.truncate());
        assert!(hand.distance(target) < 1e-4, "hand {hand}, target {target}");
    }
}

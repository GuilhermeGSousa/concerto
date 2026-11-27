//! Skeletal animation for Concerto: clips, animation graphs and the player that evaluates them
//! on a skeleton's bones each frame.
//!
//! An [`AnimationPlayer`](player::AnimationPlayer) plays an
//! [`AnimationGraph`](graph::AnimationGraph), a tree of nodes such as clips, blends, layers, state
//! machines, IK and retargeting. Add [`AnimationPlugin`](plugin::AnimationPlugin), give an entity
//! with a skeleton an [`AnimationHandleComponent`](player::AnimationHandleComponent), and set the
//! parameters its nodes read on the player.

/// Typed per-bone values that travel with a pose and blend through every combiner.
pub mod attribute;
/// The named parameters a player gives its graph's nodes.
pub mod blackboard;
/// Animation clips: keyed joint channels, clip events and additive settings.
pub mod clip;
/// What a graph walk hands each node: the evaluation stack and the player's context.
pub mod evaluation;
/// Clip events: values a clip triggers as signals on the player's entity when playback crosses
/// them.
pub mod event;
/// Animation graphs: trees of nodes, built in code or loaded as assets, and their per-player
/// state.
pub mod graph;
/// Two-bone IK: the chain solve and where its hinge comes from.
pub mod ik;
/// Pose velocities, which inertialization decays.
pub mod inertialization;
/// The yaw and pitch that turn a bone's look direction onto a target.
pub mod look_at;
/// Bone masks, which limit a layer to some bones, and their resolution against a skeleton.
pub mod mask;
/// Mirroring a pose across a plane through the skeleton's model-space origin.
pub mod mirror;
/// The animation nodes a graph is built from, and the trait a new kind of node implements.
pub mod node;
/// Typed names of blackboard parameters.
pub mod param;
/// The [`AnimationPlayer`](player::AnimationPlayer) component, which plays a graph on a
/// skeleton.
pub mod player;
/// The plugin that registers animation assets, types and systems, and the
/// [`App`](concerto_app::App) methods that register new nodes and clip events.
pub mod plugin;
/// Poses, the frames a graph evaluates to, and root motion.
pub mod pose;
/// Retargeting: rigs that describe skeletons, and playing a pose of one skeleton on another.
pub mod retarget;
/// The component that carries a skeleton's root motion.
pub mod root;
/// The systems that advance players and write their poses to bones.
pub mod target;
/// How a state machine blends from one state to the next.
pub mod transition;

pub use target::{animate_targets, update_animation_players};
pub use transition::TransitionBlender;

#[cfg(test)]
mod testing;

#[cfg(test)]
mod tests {
    use concerto_foundation::assets::{
        LoadableAsset, asset_server::AssetServer, asset_store::AssetStore, handle::AssetHandle,
    };
    use concerto_name::{Name, name};
    use glam::Vec3;
    use uuid::Uuid;

    use crate::blackboard::AnimationBlackboard;
    use crate::clip::{AnimationChanelOutput, AnimationChannel, AnimationClip};
    use crate::evaluation::RootBone;
    use crate::event::AnimationEvent;
    use crate::graph::AnimationGraph;
    use crate::node::AnimationPlayMode::PlayOnce;
    use crate::node::state_machine::{
        AnimationFSMTrigger, AnimationStateMachine, AnimationStateMachineState,
    };
    use crate::node::{
        AnimationClipNode, AnimationClipNodeState, AnimationNode, ErasedAnimationNode, NodeState,
    };
    use crate::pose::{FiredEvent, FramePool, JointPose, RootMotion};
    use crate::testing::{Walks, made_live};
    use crate::transition::TransitionBlender;

    fn unit_duration_clip() -> AnimationClip {
        let mut clip = AnimationClip::default();
        clip.add_channel(
            Uuid::new_v4(),
            AnimationChannel::new(
                vec![0.0, 1.0],
                AnimationChanelOutput::from_translation(
                    [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0]].into_iter(),
                ),
            ),
        );
        clip
    }

    fn erased<T: AnimationNode>(node: T) -> Box<dyn ErasedAnimationNode> {
        Box::new(node)
    }

    fn current_time(node_state: &NodeState) -> f32 {
        node_state
            .downcast_ref::<AnimationClipNodeState>()
            .unwrap()
            .current_time()
    }

    fn state_name(node_state: &NodeState) -> Name {
        node_state
            .downcast_ref::<AnimationStateMachineState>()
            .unwrap()
            .current_state_name()
            .unwrap()
    }

    #[test]
    fn a_clip_node_starts_at_its_start_time_wraps_when_looping_finishes_once_and_resets_to_it() {
        let mut server = AssetServer::new();
        let mut clips = AssetStore::<AnimationClip>::new();
        server.register_asset(&clips);
        let handle = server.add(AnimationClip::default());
        clips.insert(handle.id(), unit_duration_clip());
        let walks = Walks::new(clips, AssetStore::<AnimationGraph>::new());
        let blackboard = AnimationBlackboard::default();
        let update = |node: &dyn ErasedAnimationNode, node_state: &mut NodeState, dt: f32| {
            walks.update(&blackboard, |context| node.update(node_state, dt, context));
        };

        let looping = erased(AnimationClipNode::new(handle.clone()).with_start_time(0.4));
        let mut node_state = walks
            .create(&blackboard, |context| looping.create_state(context))
            .unwrap();
        assert_eq!(current_time(&node_state), 0.4);

        update(looping.as_ref(), &mut node_state, 0.3);
        assert!((current_time(&node_state) - 0.7).abs() < 1e-5);
        assert!(!looping.is_finished(&node_state));

        update(looping.as_ref(), &mut node_state, 0.4);
        assert_eq!(current_time(&node_state), 0.4);
        assert!(looping.is_finished(&node_state));

        update(looping.as_ref(), &mut node_state, 0.1);
        assert!((current_time(&node_state) - 0.5).abs() < 1e-5);
        assert!(!looping.is_finished(&node_state));

        looping.reset(&mut node_state);
        assert_eq!(current_time(&node_state), 0.4);
        assert!(!looping.is_finished(&node_state));

        let once = erased(
            AnimationClipNode::new(handle)
                .with_play_mode(PlayOnce)
                .with_start_time(0.4),
        );
        let mut node_state = walks
            .create(&blackboard, |context| once.create_state(context))
            .unwrap();
        assert_eq!(current_time(&node_state), 0.4);

        update(once.as_ref(), &mut node_state, 0.7);
        assert!(once.is_finished(&node_state));

        once.reset(&mut node_state);
        assert_eq!(current_time(&node_state), 0.4);
        assert!(!once.is_finished(&node_state));
    }

    fn clip_with_duration(duration: f32) -> AnimationClip {
        let mut clip = AnimationClip::default();
        clip.add_channel(
            Uuid::new_v4(),
            AnimationChannel::new(
                vec![0.0, duration],
                AnimationChanelOutput::from_translation(
                    [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0]].into_iter(),
                ),
            ),
        );
        clip
    }

    fn add_asset<A: LoadableAsset + Default + 'static>(
        server: &AssetServer,
        store: &mut AssetStore<A>,
        asset: A,
    ) -> AssetHandle<A> {
        let handle = server.add(A::default());
        store.insert(handle.id(), asset);
        handle
    }

    #[test]
    fn a_state_entered_mid_transition_plays_out_and_a_state_entered_again_replays_its_clip() {
        const FRAME: f32 = 1.0 / 60.0;

        let mut server = AssetServer::new();
        let mut clips = AssetStore::<AnimationClip>::new();
        let mut graphs = AssetStore::<AnimationGraph>::new();
        server.register_asset(&clips);
        server.register_asset(&graphs);

        let movement_clip = add_asset(&server, &mut clips, clip_with_duration(1.0));
        let jump_start_clip = add_asset(&server, &mut clips, clip_with_duration(0.2));
        let jump_loop_clip = add_asset(&server, &mut clips, clip_with_duration(0.5));
        let jump_land_clip = add_asset(&server, &mut clips, clip_with_duration(0.3));

        let movement = add_asset(
            &server,
            &mut graphs,
            made_live(AnimationGraph::from_node(AnimationClipNode::new(
                movement_clip,
            ))),
        );
        let jump_start = add_asset(
            &server,
            &mut graphs,
            made_live(AnimationGraph::from_node(
                AnimationClipNode::new(jump_start_clip).with_play_mode(PlayOnce),
            )),
        );
        let air = add_asset(
            &server,
            &mut graphs,
            made_live(AnimationGraph::from_node(AnimationClipNode::new(
                jump_loop_clip,
            ))),
        );
        let land = add_asset(
            &server,
            &mut graphs,
            made_live(AnimationGraph::from_node(
                AnimationClipNode::new(jump_land_clip).with_play_mode(PlayOnce),
            )),
        );

        // The air state fades in slowly (0.1s) while land snaps in (0.01s), so landing early
        // leaves the air fade in flight after the machine has already moved on to land.
        let fsm =
            AnimationStateMachine::from_initial_state(name!("movement"), movement, |transition| {
                transition.to(
                    name!("jump_start"),
                    AnimationFSMTrigger::on_bool(name!("jumped"), true),
                    0.01,
                );
            })
            .state(name!("jump_start"), jump_start, |transition| {
                transition.to(name!("air"), AnimationFSMTrigger::OnAnimationEnd, 0.1);
            })
            .state(name!("air"), air, |transition| {
                transition.to(
                    name!("land"),
                    AnimationFSMTrigger::on_bool(name!("is_grounded"), true),
                    0.01,
                );
            })
            .state(name!("land"), land, |transition| {
                transition.to(name!("movement"), AnimationFSMTrigger::OnAnimationEnd, 0.1);
            })
            .build();
        let fsm = erased(fsm);

        let mut blackboard = AnimationBlackboard::default();
        let walks = Walks::new(clips, graphs);
        let mut node_state = walks
            .create(&blackboard, |context| fsm.create_state(context))
            .unwrap();

        let step = |node_state: &mut NodeState, blackboard: &AnimationBlackboard| {
            walks.update(blackboard, |context| fsm.update(node_state, FRAME, context));
            state_name(node_state)
        };

        let until = |node_state: &mut NodeState, blackboard: &AnimationBlackboard, target: &str| {
            let mut frames = 0;
            loop {
                frames += 1;
                if step(node_state, blackboard) == target {
                    return frames;
                }
                assert!(frames < 120, "{target} was never reached");
            }
        };
        let jump = |node_state: &mut NodeState, blackboard: &mut AnimationBlackboard| {
            blackboard.set(name!("jumped"), true);
            assert_eq!(step(node_state, blackboard), "jump_start");
            blackboard.set(name!("jumped"), false);
            until(node_state, blackboard, "air")
        };

        let first_jump = jump(&mut node_state, &mut blackboard);
        blackboard.set(name!("is_grounded"), true);
        assert_eq!(step(&mut node_state, &blackboard), "land");
        until(&mut node_state, &blackboard, "movement");
        blackboard.set(name!("is_grounded"), false);
        let second_jump = jump(&mut node_state, &mut blackboard);

        assert!(
            first_jump >= 10,
            "jump_start ended after {first_jump} frames"
        );
        assert_eq!(second_jump, first_jump);
    }

    fn constant_x_clip(bone: Uuid, x: f32) -> AnimationClip {
        let mut clip = AnimationClip::default();
        clip.add_channel(
            bone,
            AnimationChannel::new(
                vec![0.0, 1.0],
                AnimationChanelOutput::from_translation([[x, 0.0, 0.0], [x, 0.0, 0.0]].into_iter()),
            ),
        );
        clip
    }

    const MACHINE_FRAME: f32 = 1.0 / 60.0;

    fn linear_x_clip(bone: Uuid, slope: f32) -> AnimationClip {
        let mut clip = AnimationClip::default();
        clip.add_channel(
            bone,
            AnimationChannel::new(
                vec![0.0, 100.0],
                AnimationChanelOutput::from_translation(
                    [[0.0, 0.0, 0.0], [slope * 100.0, 0.0, 0.0]].into_iter(),
                ),
            ),
        );
        clip
    }

    struct Machine {
        _server: AssetServer,
        walks: Walks,
        fsm: Box<dyn ErasedAnimationNode>,
        blackboard: AnimationBlackboard,
        node_state: NodeState,
        bone: Uuid,
        pool: FramePool,
        root_motion: RootMotion,
        events: Vec<FiredEvent>,
    }

    impl Machine {
        fn new(
            bone: Uuid,
            a_clip: AnimationClip,
            b_clip: AnimationClip,
            blend_time: f32,
            blender: TransitionBlender,
        ) -> Self {
            let mut server = AssetServer::new();
            let mut clips = AssetStore::<AnimationClip>::new();
            let mut graphs = AssetStore::<AnimationGraph>::new();
            server.register_asset(&clips);
            server.register_asset(&graphs);

            let mut graph_for = |clip: AnimationClip| {
                let clip = add_asset(&server, &mut clips, clip);
                add_asset(
                    &server,
                    &mut graphs,
                    made_live(AnimationGraph::from_node(AnimationClipNode::new(clip))),
                )
            };
            let a = graph_for(a_clip);
            let b = graph_for(b_clip);

            let fsm = AnimationStateMachine::from_initial_state(name!("a"), a, |transition| {
                transition.to(
                    name!("b"),
                    AnimationFSMTrigger::on_bool(name!("to_b"), true),
                    blend_time,
                );
            })
            .state(name!("b"), b, |_| {})
            .blender(blender)
            .build();
            let fsm = erased(fsm);

            let blackboard = AnimationBlackboard::default();
            let walks = Walks::new(clips, graphs);
            let node_state = walks
                .create(&blackboard, |context| fsm.create_state(context))
                .unwrap();

            Self {
                _server: server,
                walks,
                fsm,
                blackboard,
                node_state,
                bone,
                pool: FramePool::new(1),
                root_motion: RootMotion::IDENTITY,
                events: Vec::new(),
            }
        }

        fn set(&mut self, name: Name, value: bool) {
            self.blackboard.set(name, value);
        }

        fn step(&mut self) -> f32 {
            let root = RootBone {
                index: 0,
                rest: JointPose::IDENTITY,
                up: Vec3::X,
            };
            self.walks.update(&self.blackboard, |context| {
                self.fsm
                    .update(&mut self.node_state, MACHINE_FRAME, context)
            });
            let mut frame = self.pool.acquire();
            let passed = self
                .walks
                .pass(&self.blackboard, Some(root.clone()), |context| {
                    self.fsm
                        .root_motion(&mut self.node_state, context, &[self.bone], &[])
                });
            self.walks
                .evaluate(&self.blackboard, Some(root), |context| {
                    self.fsm.evaluate(
                        &mut self.node_state,
                        context,
                        &[self.bone],
                        &[],
                        &mut self.pool,
                        &mut frame,
                    )
                });
            let x = frame.joint(0).unwrap().translation.x;
            self.root_motion = passed;
            self.events = frame.events().to_vec();
            self.pool.release(frame);
            x
        }

        fn run(&mut self, frames: usize) -> f32 {
            let mut x = 0.0;
            for _ in 0..frames {
                x = self.step();
            }
            x
        }

        fn state(&self) -> Name {
            state_name(&self.node_state)
        }
    }

    #[test]
    fn inertialize_velocity_step_is_bounded_by_the_destination_velocity() {
        let bone = Uuid::new_v4();
        let mut machine = Machine::new(
            bone,
            linear_x_clip(bone, 2.0),
            constant_x_clip(bone, 10.0),
            1.0,
            TransitionBlender::Inertialization,
        );
        machine.run(9);
        let before_last = machine.step();
        let last = machine.step();
        let velocity_before = (last - before_last) / MACHINE_FRAME;
        assert!((velocity_before - 2.0).abs() < 1e-2);

        machine.set(name!("to_b"), true);
        let on_transition = machine.step();
        machine.set(name!("to_b"), false);
        let after = machine.step();
        let velocity_after = (after - on_transition) / MACHINE_FRAME;

        let destination_velocity = 0.0;
        assert!(
            (velocity_after - velocity_before).abs() <= destination_velocity + 0.5,
            "velocity went from {velocity_before} to {velocity_after}"
        );
    }

    fn with_event(mut clip: AnimationClip, time: f32) -> AnimationClip {
        clip.add_event(
            time,
            AnimationEvent {
                name: name!("step"),
            },
        );
        clip
    }

    #[derive(Debug)]
    struct TimeWeight {
        time: f32,
        weight: f32,
    }

    fn time_weight(time: f32, weight: f32) -> TimeWeight {
        TimeWeight { time, weight }
    }

    fn fired_by_time(machine: &Machine) -> Vec<TimeWeight> {
        let mut fired: Vec<_> = machine
            .events
            .iter()
            .map(|event| time_weight(event.time, event.weight))
            .collect();
        fired.sort_by(|a, b| a.time.total_cmp(&b.time));
        fired
    }

    fn assert_fired(actual: &[TimeWeight], expected: &[TimeWeight]) {
        assert_eq!(actual.len(), expected.len(), "fired {actual:?}");
        for (fired, expected_fired) in actual.iter().zip(expected) {
            assert!(
                (fired.time - expected_fired.time).abs() < 1e-6
                    && (fired.weight - expected_fired.weight).abs() < 1e-5,
                "expected {expected:?}, fired {actual:?}"
            );
        }
    }

    struct BlenderCase {
        blender: TransitionBlender,
        x: f32,
        root_x: f32,
        fired: Vec<TimeWeight>,
    }

    #[test]
    fn each_blender_weighs_the_pose_root_motion_and_events_its_own_way_and_settles_on_the_target() {
        let entered = MACHINE_FRAME / 0.1;
        let cases = [
            BlenderCase {
                blender: TransitionBlender::CrossFade,
                x: (12.0 + (8.0 - 12.0) * entered) * MACHINE_FRAME,
                root_x: (2.0 + (8.0 - 2.0) * entered) * MACHINE_FRAME,
                fired: vec![
                    time_weight(0.0, entered),
                    time_weight(5.5 * MACHINE_FRAME, 1.0 - entered),
                ],
            },
            BlenderCase {
                blender: TransitionBlender::Inertialization,
                x: 10.0 * MACHINE_FRAME,
                root_x: 8.0 * MACHINE_FRAME,
                fired: vec![time_weight(0.0, 1.0)],
            },
        ];

        for case in cases {
            let bone = Uuid::new_v4();
            let mut machine = Machine::new(
                bone,
                with_event(linear_x_clip(bone, 2.0), 5.5 * MACHINE_FRAME),
                with_event(linear_x_clip(bone, 8.0), 0.0),
                0.1,
                case.blender,
            );
            machine.run(5);
            machine.set(name!("to_b"), true);

            let x = machine.step();

            assert!(
                (x - case.x).abs() < 1e-4,
                "{:?}: expected the pose at {}, got {x}",
                case.blender,
                case.x
            );
            assert!(
                (machine.root_motion.translation.x - case.root_x).abs() < 1e-4,
                "{:?}: expected {}, got {}",
                case.blender,
                case.root_x,
                machine.root_motion.translation.x
            );
            assert_fired(&fired_by_time(&machine), &case.fired);

            let settled = machine.run(60);
            assert_eq!(machine.state(), "b");
            assert!(
                (settled - 8.0 * 61.0 * MACHINE_FRAME).abs() < 1e-3,
                "{:?}: settled at {settled}",
                case.blender
            );
        }
    }
}

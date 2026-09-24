pub mod blackboard;
pub mod clip;
pub mod evaluation;
pub mod graph;
pub mod node;
pub mod player;
pub mod plugin;
pub mod pose;
pub mod root;
pub mod target;
pub mod transition;

#[cfg(test)]
mod tests {
    use concerto_foundation::assets::{
        Asset, asset_server::AssetServer, asset_store::AssetStore, handle::AssetHandle,
    };
    use uuid::Uuid;

    use crate::blackboard::{AnimationBlackboard, AnimationBlackboardValue};
    use crate::clip::{AnimationChanelOutput, AnimationChannel, AnimationClip};
    use crate::evaluation::AnimationGraphContext;
    use crate::graph::AnimationGraph;
    use crate::node::AnimationPlayMode::PlayOnce;
    use crate::node::state_machine::{
        AnimationFSMTrigger, AnimationStateMachine, AnimationStateMachineInstance,
    };
    use crate::node::{
        AnimationClipNode, AnimationClipNodeInstance, AnimationNodeInstance, AnimationNodeKind,
    };
    use crate::pose::PosePool;

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

    fn current_time(instance: &dyn AnimationNodeInstance) -> f32 {
        instance
            .as_any()
            .downcast_ref::<AnimationClipNodeInstance>()
            .unwrap()
            .current_time()
    }

    #[test]
    fn looping_clip_starts_and_wraps_at_start_time() {
        let mut server = AssetServer::new();
        let mut clips = AssetStore::<AnimationClip>::new();
        server.register_asset(&clips);
        let handle = server.add(AnimationClip::default());
        clips.insert(handle.id(), unit_duration_clip());
        let graphs = AssetStore::<AnimationGraph>::new();
        let blackboard = AnimationBlackboard::default();

        let context = AnimationGraphContext {
            animation_clips: &clips,
            animation_graphs: &graphs,
            blackboard: &blackboard,
        };

        let node = AnimationNodeKind::Clip(AnimationClipNode::new(handle).with_start_time(0.4));
        let mut instance = node.create_instance(&context);
        assert_eq!(current_time(instance.as_ref()), 0.4);

        instance.update(&node, 0.3, &context);
        assert!((current_time(instance.as_ref()) - 0.7).abs() < 1e-5);
        assert!(!instance.is_finished());

        instance.update(&node, 0.4, &context);
        assert_eq!(current_time(instance.as_ref()), 0.4);
        assert!(instance.is_finished());

        instance.update(&node, 0.1, &context);
        assert!((current_time(instance.as_ref()) - 0.5).abs() < 1e-5);
        assert!(!instance.is_finished());

        instance.reset();
        assert_eq!(current_time(instance.as_ref()), 0.4);
        assert!(!instance.is_finished());
    }

    #[test]
    fn play_once_clip_starts_at_start_time_and_finishes() {
        let mut server = AssetServer::new();
        let mut clips = AssetStore::<AnimationClip>::new();
        server.register_asset(&clips);
        let handle = server.add(AnimationClip::default());
        clips.insert(handle.id(), unit_duration_clip());
        let graphs = AssetStore::<AnimationGraph>::new();
        let blackboard = AnimationBlackboard::default();
        let context = AnimationGraphContext {
            animation_clips: &clips,
            animation_graphs: &graphs,
            blackboard: &blackboard,
        };

        let node = AnimationNodeKind::Clip(
            AnimationClipNode::new(handle)
                .with_play_mode(PlayOnce)
                .with_start_time(0.4),
        );
        let mut instance = node.create_instance(&context);
        assert_eq!(current_time(instance.as_ref()), 0.4);

        instance.update(&node, 0.7, &context);
        assert!(instance.is_finished());

        instance.reset();
        assert_eq!(current_time(instance.as_ref()), 0.4);
        assert!(!instance.is_finished());
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

    fn add_asset<A: Asset + Default + 'static>(
        server: &AssetServer,
        store: &mut AssetStore<A>,
        asset: A,
    ) -> AssetHandle<A> {
        let handle = server.add(A::default());
        store.insert(handle.id(), asset);
        handle
    }

    /// A state entered while an earlier transition is still fading in must not be stranded
    /// when that older fade completes: landing mid-air-blend used to hand the blend stack
    /// back to the air state while the machine stayed in `land`, freezing the character.
    #[test]
    fn state_entered_mid_transition_is_not_superseded_by_the_older_fade() {
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
            AnimationGraph::from_node(AnimationNodeKind::Clip(AnimationClipNode::new(
                movement_clip,
            ))),
        );
        let jump_start = add_asset(
            &server,
            &mut graphs,
            AnimationGraph::from_node(AnimationNodeKind::Clip(
                AnimationClipNode::new(jump_start_clip).with_play_mode(PlayOnce),
            )),
        );
        let air = add_asset(
            &server,
            &mut graphs,
            AnimationGraph::from_node(AnimationNodeKind::Clip(AnimationClipNode::new(
                jump_loop_clip,
            ))),
        );
        let land = add_asset(
            &server,
            &mut graphs,
            AnimationGraph::from_node(AnimationNodeKind::Clip(
                AnimationClipNode::new(jump_land_clip).with_play_mode(PlayOnce),
            )),
        );

        // The air state fades in slowly (0.1s) while land snaps in (0.01s), so landing early
        // leaves the air fade in flight after the machine has already moved on to land.
        let fsm = AnimationStateMachine::from_initial_state("movement", movement, |transition| {
            transition.to(
                "jump_start",
                AnimationFSMTrigger::on_bool("jumped", true),
                0.01,
            );
        })
        .state("jump_start", jump_start, |transition| {
            transition.to("air", AnimationFSMTrigger::OnAnimationEnd, 0.1);
        })
        .state("air", air, |transition| {
            transition.to(
                "land",
                AnimationFSMTrigger::on_bool("is_grounded", true),
                0.01,
            );
        })
        .state("land", land, |transition| {
            transition.to("movement", AnimationFSMTrigger::OnAnimationEnd, 0.1);
        })
        .build();
        let fsm = AnimationNodeKind::StateMachine(fsm);

        let mut blackboard = AnimationBlackboard::default();
        let mut instance = {
            let context = AnimationGraphContext {
                animation_clips: &clips,
                animation_graphs: &graphs,
                blackboard: &blackboard,
            };
            fsm.create_instance(&context)
        };

        let step = |instance: &mut Box<dyn AnimationNodeInstance>,
                    blackboard: &AnimationBlackboard| {
            let context = AnimationGraphContext {
                animation_clips: &clips,
                animation_graphs: &graphs,
                blackboard,
            };
            instance.update(&fsm, FRAME, &context);
            instance
                .as_any()
                .downcast_ref::<AnimationStateMachineInstance>()
                .unwrap()
                .current_state_name()
                .to_string()
        };

        blackboard.set("jumped", AnimationBlackboardValue::Bool(true));
        assert_eq!(step(&mut instance, &blackboard), "jump_start");
        blackboard.set("jumped", AnimationBlackboardValue::Bool(false));

        let mut state = String::new();
        for _ in 0..30 {
            state = step(&mut instance, &blackboard);
            if state == "air" {
                break;
            }
        }
        assert_eq!(state, "air", "jump_start should end and hand off to air");

        // Land immediately, while the 0.1s air fade is only ~1 frame in.
        blackboard.set("is_grounded", AnimationBlackboardValue::Bool(true));
        assert_eq!(step(&mut instance, &blackboard), "land");

        // The land clip must actually play out and end the state, rather than being frozen
        // by the stale air fade completing underneath it.
        for _ in 0..60 {
            state = step(&mut instance, &blackboard);
            if state == "movement" {
                break;
            }
        }
        assert_eq!(
            state, "movement",
            "land clip never finished: the blend stack stranded the land state"
        );
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

    /// A transition still cross-fades (not a hard cut), and once it completes the blend stack
    /// collapses to exactly the target — proving the newest entry is the single source of truth
    /// rather than one authority lagging behind another.
    #[test]
    fn transition_cross_fades_then_settles_on_the_target() {
        const FRAME: f32 = 1.0 / 60.0;

        let mut server = AssetServer::new();
        let mut clips = AssetStore::<AnimationClip>::new();
        let mut graphs = AssetStore::<AnimationGraph>::new();
        server.register_asset(&clips);
        server.register_asset(&graphs);

        let bone = Uuid::new_v4();
        let a_clip = add_asset(&server, &mut clips, constant_x_clip(bone, 0.0));
        let b_clip = add_asset(&server, &mut clips, constant_x_clip(bone, 10.0));
        let a = add_asset(
            &server,
            &mut graphs,
            AnimationGraph::from_node(AnimationNodeKind::Clip(AnimationClipNode::new(a_clip))),
        );
        let b = add_asset(
            &server,
            &mut graphs,
            AnimationGraph::from_node(AnimationNodeKind::Clip(AnimationClipNode::new(b_clip))),
        );

        let fsm = AnimationStateMachine::from_initial_state("a", a, |transition| {
            transition.to("b", AnimationFSMTrigger::on_bool("go", true), 0.1);
        })
        .state("b", b, |_| {})
        .build();
        let fsm = AnimationNodeKind::StateMachine(fsm);

        let mut blackboard = AnimationBlackboard::default();
        blackboard.set("go", AnimationBlackboardValue::Bool(true));

        let mut instance = {
            let context = AnimationGraphContext {
                animation_clips: &clips,
                animation_graphs: &graphs,
                blackboard: &blackboard,
            };
            fsm.create_instance(&context)
        };

        let step = |instance: &mut Box<dyn AnimationNodeInstance>| {
            let context = AnimationGraphContext {
                animation_clips: &clips,
                animation_graphs: &graphs,
                blackboard: &blackboard,
            };
            instance.update(&fsm, FRAME, &context);
        };

        let sample_x = |instance: &Box<dyn AnimationNodeInstance>| -> f32 {
            let context = AnimationGraphContext {
                animation_clips: &clips,
                animation_graphs: &graphs,
                blackboard: &blackboard,
            };
            let mut pool = PosePool::new(1);
            let mut pose = pool.acquire();
            instance.evaluate(&fsm, &context, &[bone], &[], &mut pool, &mut pose);
            pose.get_joint_pose(0).unwrap().translation.x
        };

        // One frame into the 0.1s fade: a genuine blend between the two poses, not a hard cut.
        step(&mut instance);
        let mid = sample_x(&instance);
        assert!(
            mid > 0.0 && mid < 10.0,
            "expected a cross-fade, got x={mid}"
        );

        // After the fade completes the stack collapses to the target and samples it exactly.
        for _ in 0..30 {
            step(&mut instance);
        }
        let settled = sample_x(&instance);
        assert!(
            (settled - 10.0).abs() < 1e-4,
            "expected to settle on the target pose, got x={settled}"
        );

        let name = instance
            .as_any()
            .downcast_ref::<AnimationStateMachineInstance>()
            .unwrap()
            .current_state_name()
            .to_string();
        assert_eq!(name, "b");
    }

    /// Re-entering a state restarts its clip: `OnAnimationEnd` is anchored to entry, so a second
    /// visit plays the whole clip again instead of firing instantly on a stale completion flag.
    #[test]
    fn re_entering_a_state_replays_its_clip_from_the_start() {
        const FRAME: f32 = 1.0 / 60.0;

        let mut server = AssetServer::new();
        let mut clips = AssetStore::<AnimationClip>::new();
        let mut graphs = AssetStore::<AnimationGraph>::new();
        server.register_asset(&clips);
        server.register_asset(&graphs);

        let movement_clip = add_asset(&server, &mut clips, clip_with_duration(1.0));
        let jump_clip = add_asset(&server, &mut clips, clip_with_duration(0.3));
        let movement = add_asset(
            &server,
            &mut graphs,
            AnimationGraph::from_node(AnimationNodeKind::Clip(AnimationClipNode::new(
                movement_clip,
            ))),
        );
        let jump = add_asset(
            &server,
            &mut graphs,
            AnimationGraph::from_node(AnimationNodeKind::Clip(
                AnimationClipNode::new(jump_clip).with_play_mode(PlayOnce),
            )),
        );

        let fsm = AnimationStateMachine::from_initial_state("movement", movement, |transition| {
            transition.to("jump", AnimationFSMTrigger::on_bool("jump", true), 0.01);
        })
        .state("jump", jump, |transition| {
            transition.to("movement", AnimationFSMTrigger::OnAnimationEnd, 0.01);
        })
        .build();
        let fsm = AnimationNodeKind::StateMachine(fsm);

        let mut blackboard = AnimationBlackboard::default();
        let mut instance = {
            let context = AnimationGraphContext {
                animation_clips: &clips,
                animation_graphs: &graphs,
                blackboard: &blackboard,
            };
            fsm.create_instance(&context)
        };

        let step = |instance: &mut Box<dyn AnimationNodeInstance>,
                    blackboard: &AnimationBlackboard|
         -> String {
            let context = AnimationGraphContext {
                animation_clips: &clips,
                animation_graphs: &graphs,
                blackboard,
            };
            instance.update(&fsm, FRAME, &context);
            instance
                .as_any()
                .downcast_ref::<AnimationStateMachineInstance>()
                .unwrap()
                .current_state_name()
                .to_string()
        };

        // Enter jump, then count frames from entry until OnAnimationEnd hands back to movement.
        let frames_in_jump = |instance: &mut Box<dyn AnimationNodeInstance>,
                              blackboard: &mut AnimationBlackboard|
         -> usize {
            blackboard.set("jump", AnimationBlackboardValue::Bool(true));
            assert_eq!(step(instance, blackboard), "jump");
            blackboard.set("jump", AnimationBlackboardValue::Bool(false));

            let mut frames = 0;
            loop {
                let state = step(instance, blackboard);
                frames += 1;
                if state == "movement" {
                    break;
                }
                assert!(frames < 120, "jump state never ended");
            }
            frames
        };

        let first = frames_in_jump(&mut instance, &mut blackboard);
        let second = frames_in_jump(&mut instance, &mut blackboard);

        // The 0.3s clip at 60fps is ~18 frames; a stale completion flag would end it in ~1.
        assert!(first >= 12, "first jump ended too fast: {first} frames");
        assert!(
            second >= 12,
            "re-entered jump replayed instantly ({second} frames): clip not anchored to entry"
        );
        assert!(
            (first as i32 - second as i32).abs() <= 3,
            "re-entry length {second} should match the first play {first}"
        );
    }
}

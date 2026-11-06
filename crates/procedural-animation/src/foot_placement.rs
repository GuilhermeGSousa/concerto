use concerto_animation::{
    evaluation::AnimationGraphContext,
    node::{
        AnimationNode, ReflectAnimationNode,
        model_space::{WorldToModel, bone_under_first, parent},
    },
    pose::{AnimationFrame, EvaluatedFrame, FramePool},
};
use concerto_ecs::Res;
use concerto_mesh::{JointName, Skeleton};
use concerto_name::Name;
use concerto_physics::{body::BodyId, physics_state::PhysicsState};
use concerto_reflect::Reflect;
use log::warn;
use uuid::Uuid;

/// Puts each leg's foot on the ground under it and lowers the pelvis for the lowest; passes its first input through first.
#[derive(Clone, Debug, PartialEq, Reflect, serde::Serialize, serde::Deserialize)]
#[reflect(AnimationNode)]
pub struct FootPlacementNode {
    pub pelvis: JointName,
    pub legs: Vec<FootPlacementLeg>,
    pub max_step_up: f32,
    pub max_step_down: f32,
    pub max_pelvis_drop: f32,
    pub max_foot_angle: f32,
    pub foot_halflife: f32,
    pub pelvis_halflife: f32,
    pub rotation_halflife: f32,
    pub softness: f32,
    pub weight: f32,
    pub weight_param: Option<Name>,
    pub weight_halflife: f32,
    pub ignore_param: Option<Name>,
}

impl Default for FootPlacementNode {
    fn default() -> Self {
        Self {
            pelvis: JointName::default(),
            legs: Vec::new(),
            max_step_up: 0.5,
            max_step_down: 0.5,
            max_pelvis_drop: 0.4,
            max_foot_angle: 30.0_f32.to_radians(),
            foot_halflife: 0.05,
            pelvis_halflife: 0.1,
            rotation_halflife: 0.05,
            softness: 0.0,
            weight: 1.0,
            weight_param: None,
            weight_halflife: 0.2,
            ignore_param: None,
        }
    }
}

/// A leg: the foot bone names the chain (its parent the knee, its grandparent the hip), and an optional toe bone adds a second ray.
#[derive(Clone, Debug, Default, PartialEq, Reflect, serde::Serialize, serde::Deserialize)]
pub struct FootPlacementLeg {
    pub foot: JointName,
    pub toe: Option<JointName>,
}

/// The body a foot placement node's rays skip, set on the blackboard by the game: the character's own capsule.
#[derive(Clone, Copy, Debug, Default, Reflect)]
pub struct IgnoredBody {
    #[reflect(skip)]
    pub body: Option<BodyId>,
}

/// The time a [`FootPlacementNode`] has to ease over, and whether it has warned about legs it cannot resolve.
#[derive(Debug, Default)]
pub struct FootPlacementState {
    warned: bool,
    elapsed: f32,
}

impl FootPlacementNode {
    fn resolve(&self, skeleton: Option<&Skeleton>) -> Result<(), String> {
        let skeleton = skeleton.ok_or("the player has no skeleton")?;
        let pelvis = bone_under_first(skeleton, self.pelvis)?;
        for leg in &self.legs {
            let foot = bone_under_first(skeleton, leg.foot)?;
            let knee = parent(skeleton, foot)
                .ok_or_else(|| format!("foot `{}` has no parent", &*leg.foot))?;
            let hip = parent(skeleton, knee)
                .ok_or_else(|| format!("foot `{}` has no grandparent", &*leg.foot))?;
            if !descends_from(skeleton, hip, pelvis) {
                return Err(format!(
                    "the hip of foot `{}` does not descend from pelvis `{}`",
                    &*leg.foot, &*self.pelvis
                ));
            }
            if let Some(toe) = leg.toe {
                bone_under_first(skeleton, toe)?;
            }
        }
        Ok(())
    }
}

fn descends_from(skeleton: &Skeleton, bone: usize, ancestor: usize) -> bool {
    std::iter::successors(parent(skeleton, bone), |&up| parent(skeleton, up))
        .any(|up| up == ancestor)
}

impl AnimationNode for FootPlacementNode {
    type State = FootPlacementState;
    type CreateInput = ();
    type UpdateInput = ();
    type EvaluateInput = Option<Res<'static, PhysicsState>>;

    fn create_state(&self, _: &(), _context: &AnimationGraphContext<'_>) -> FootPlacementState {
        FootPlacementState::default()
    }

    fn update(
        &self,
        state: &mut FootPlacementState,
        delta_time: f32,
        _: &(),
        _: &AnimationGraphContext<'_>,
    ) {
        if delta_time.is_finite() && delta_time > 0.0 {
            state.elapsed += delta_time;
        }
    }

    fn evaluate(
        &self,
        state: &mut FootPlacementState,
        _physics: &Option<Res<PhysicsState>>,
        context: &AnimationGraphContext<'_>,
        _: &[Uuid],
        evaluated_inputs: &[EvaluatedFrame],
        _: &mut FramePool,
        output: &mut AnimationFrame,
    ) {
        if let Some(input) = evaluated_inputs.first() {
            output.copy_from(&input.frame);
        }
        if self.legs.is_empty() {
            return;
        }
        if let Err(reason) = self.resolve(context.skeleton()) {
            if !state.warned {
                warn!("foot placement on `{}` skipped: {reason}", &*self.pelvis);
                state.warned = true;
            }
            return;
        }
        let model_to_world = context.model_to_world().unwrap_or_default();
        if !model_to_world.is_finite() || !WorldToModel::new(model_to_world).is_finite() {
            return;
        }
        state.elapsed = 0.0;
    }

    fn reset(&self, state: &mut FootPlacementState) {
        state.elapsed = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use concerto_animation::{
        evaluation::AnimationGraphContext,
        graph::AnimationGraph,
        node::{AnimationNode, ReflectAnimationNode},
        player::{AnimationHandleComponent, AnimationPlayer},
        plugin::{AnimationAppExt, AnimationPlugin, register_animation_types},
        pose::{AnimationFrame, EvaluatedFrame, FramePool, JointPose, RootMotion},
        root::AnimationRootBone,
    };
    use concerto_app::{App, plugins::AssetManagerPlugin, schedule_groups::LateUpdate};
    use concerto_ecs::{
        Entity,
        component::scene::SceneEntityRef,
        system::{
            executor::single_thread::SingleThreadedExecutor,
            schedule::{Schedule, Schedules},
        },
        world::World,
    };
    use concerto_foundation::{
        assets::{AssetId, asset_store::AssetStore, handle::AssetHandle},
        time::Time,
        transform::Transform,
    };
    use concerto_mesh::{
        JointName,
        skeleton::{Skeleton, SkeletonComponent},
    };
    use concerto_physics::{
        collider::{Collider, register_colliders},
        physics_state::PhysicsState,
    };
    use concerto_reflect::{Reflect, TypeRegistry};
    use glam::{Mat4, Quat, Vec3};
    use uuid::Uuid;

    use super::{FootPlacementLeg, FootPlacementNode, FootPlacementState};
    use crate::plugin::ProceduralAnimationPlugin;

    const EPS: f32 = 1.0e-4;
    const KNEE_BEND: f32 = 0.1;

    #[derive(Default, Reflect)]
    #[reflect(AnimationNode)]
    struct ConstantPose {
        translations: Vec<Vec3>,
        rotations: Vec<Quat>,
        root_translation: Vec3,
        root_rotation: Quat,
    }

    impl AnimationNode for ConstantPose {
        type State = ();
        type CreateInput = ();
        type UpdateInput = ();
        type EvaluateInput = ();

        fn create_state(&self, _: &(), _context: &AnimationGraphContext<'_>) {}

        fn update(&self, _: &mut (), _: f32, _: &(), _: &AnimationGraphContext<'_>) {}

        fn evaluate(
            &self,
            _: &mut (),
            _: &(),
            _: &AnimationGraphContext<'_>,
            _: &[Uuid],
            _: &[EvaluatedFrame],
            _: &mut FramePool,
            output: &mut AnimationFrame,
        ) {
            for (bone, translation) in self.translations.iter().enumerate() {
                if let Some(joint) = output.joint_mut(bone) {
                    joint.translation = *translation;
                }
            }
            for (bone, rotation) in self.rotations.iter().enumerate() {
                if let Some(joint) = output.joint_mut(bone) {
                    joint.rotation = *rotation;
                }
            }
            output.set_root_motion(RootMotion {
                translation: self.root_translation,
                rotation: self.root_rotation,
            });
        }

        fn reset(&self, _: &mut ()) {}
    }

    impl ConstantPose {
        fn rest_of(skeleton: &Skeleton) -> Self {
            Self {
                translations: skeleton
                    .rest_pose
                    .iter()
                    .map(|joint| joint.translation)
                    .collect(),
                rotations: skeleton
                    .rest_pose
                    .iter()
                    .map(|joint| joint.rotation)
                    .collect(),
                ..Default::default()
            }
        }

        fn shifted_of(skeleton: &Skeleton) -> Self {
            let mut pose = Self::rest_of(skeleton);
            pose.translations[0] += Vec3::new(0.02, -0.03, 0.01);
            pose.rotations[0] = Quat::from_rotation_y(0.1);
            pose.rotations[2] = Quat::from_rotation_x(0.05);
            pose
        }
    }

    struct Bone {
        name: &'static str,
        parent: Option<u16>,
        translation: Vec3,
    }

    fn skeleton_of(bones: &[Bone]) -> Skeleton {
        Skeleton::new(
            vec![Mat4::IDENTITY; bones.len()],
            bones.iter().map(|bone| bone.parent).collect(),
            bones.iter().map(|bone| bone.name.into()).collect(),
            bones
                .iter()
                .map(|bone| JointPose {
                    translation: bone.translation,
                    ..JointPose::IDENTITY
                })
                .collect(),
            (0..bones.len() as u128).map(Uuid::from_u128).collect(),
        )
        .unwrap()
    }

    fn leg_bones(names: [&'static str; 4], hip_x: f32, first: u16) -> [Bone; 4] {
        let half = 0.45;
        [
            Bone {
                name: names[0],
                parent: Some(0),
                translation: Vec3::new(hip_x, 0.0, 0.0),
            },
            Bone {
                name: names[1],
                parent: Some(first),
                translation: Vec3::new(0.0, -half * KNEE_BEND.cos(), half * KNEE_BEND.sin()),
            },
            Bone {
                name: names[2],
                parent: Some(first + 1),
                translation: Vec3::new(0.0, -half * KNEE_BEND.cos(), -half * KNEE_BEND.sin()),
            },
            Bone {
                name: names[3],
                parent: Some(first + 2),
                translation: Vec3::new(0.0, 0.0, 0.15),
            },
        ]
    }

    fn rig() -> Skeleton {
        let mut bones = vec![Bone {
            name: "pelvis",
            parent: None,
            translation: Vec3::new(0.0, 0.9, 0.0),
        }];
        bones.extend(leg_bones(
            ["left_hip", "left_knee", "left_foot", "left_toe"],
            0.1,
            1,
        ));
        bones.extend(leg_bones(
            ["right_hip", "right_knee", "right_foot", "right_toe"],
            -0.1,
            5,
        ));
        skeleton_of(&bones)
    }

    fn leg(foot: &str, toe: Option<&str>) -> FootPlacementLeg {
        FootPlacementLeg {
            foot: foot.into(),
            toe: toe.map(JointName::from),
        }
    }

    fn placement() -> FootPlacementNode {
        FootPlacementNode {
            pelvis: "pelvis".into(),
            legs: vec![
                leg("left_foot", Some("left_toe")),
                leg("right_foot", Some("right_toe")),
            ],
            ..Default::default()
        }
    }

    struct Stage {
        app: App,
        bones: Vec<Entity>,
        player: Entity,
        input: ConstantPose,
    }

    impl Stage {
        fn new(skeleton: Skeleton, input: ConstantPose, node: FootPlacementNode) -> Self {
            let mut app = App::new();
            app.register_plugin(AssetManagerPlugin);
            app.insert_resource(Time::new());
            app.register_plugin(AnimationPlugin);
            app.register_plugin(ProceduralAnimationPlugin);
            app.register_animation_node::<ConstantPose>();

            let bone_count = skeleton.bone_count();
            let world = app.main_mut().world_mut();
            world.register_component::<Transform>();
            world.register_component::<Collider>();
            let skeleton_handle = AssetHandle::<Skeleton>::weak(AssetId::new());
            world
                .get_resource_mut::<AssetStore<Skeleton>>()
                .unwrap()
                .insert(skeleton_handle.id(), skeleton);
            let graph_handle = add_graph(world, &input, node);
            let bones: Vec<Entity> = (0..bone_count)
                .map(|_| world.spawn(Transform::IDENTITY))
                .collect();
            let player = world.spawn((
                AnimationPlayer::new(bone_count),
                AnimationHandleComponent::new(graph_handle),
                SkeletonComponent::new(
                    skeleton_handle,
                    bones
                        .iter()
                        .map(|bone| SceneEntityRef::Entity(*bone))
                        .collect(),
                    None,
                ),
            ));
            let schedules = world.remove_resource::<Schedules>().unwrap();
            let compiled = schedules.compile::<SingleThreadedExecutor>(world);
            world.insert_resource(compiled);

            Self {
                app,
                bones,
                player,
                input,
            }
        }

        fn world(&mut self) -> &mut World {
            self.app.main_mut().world_mut()
        }

        fn with_physics(mut self) -> Self {
            self.world().insert_resource(PhysicsState::new());
            self
        }

        fn add_cuboid(&mut self, half_extents: Vec3, center: Vec3, rotation: Quat) -> Entity {
            let world = self.world();
            let cuboid = world.spawn((
                Collider::cuboid(half_extents.x, half_extents.y, half_extents.z),
                Transform::from_translation_rotation(center, rotation),
            ));
            let mut registration = Schedule::new();
            registration.add_system(register_colliders);
            registration
                .compile::<SingleThreadedExecutor>(world)
                .run(world);
            cuboid
        }

        fn add_floor(&mut self, height: f32) {
            self.add_cuboid(
                Vec3::new(2.0, 0.5, 2.0),
                Vec3::new(0.0, height - 0.5, 0.0),
                Quat::IDENTITY,
            );
        }

        fn run(&mut self) {
            self.world().run_schedule(LateUpdate);
        }

        fn settle(&mut self) {
            self.run();
            self.run();
        }

        fn with_state<R>(&mut self, read: impl FnOnce(&mut FootPlacementState) -> R) -> R {
            let player = self.player;
            let mut read = Some(read);
            let mut result = None;
            self.world()
                .resource_scope::<AssetStore<AnimationGraph>>(|world, graphs| {
                    world
                        .get_component_for_entity_mut::<AnimationPlayer>(player)
                        .unwrap()
                        .for_each_state_mut::<FootPlacementNode>(graphs, |_, state, _| {
                            if let Some(read) = read.take() {
                                result = Some(read(state));
                            }
                        });
                });
            result.expect("the graph holds a foot placement node")
        }

        fn warned(&mut self) -> bool {
            self.with_state(|state| state.warned)
        }

        fn local(&mut self, bone: usize) -> Transform {
            let entity = self.bones[bone];
            self.world()
                .get_component_for_entity::<Transform>(entity)
                .unwrap()
                .clone()
        }

        fn assert_input_pose(&mut self) {
            for bone in 0..self.bones.len() {
                let local = self.local(bone);
                let translation = self.input.translations[bone];
                let rotation = self.input.rotations[bone];
                assert!(
                    local.translation.distance(translation) < EPS,
                    "bone {bone}: {:?} vs {translation:?}",
                    local.translation
                );
                assert!(
                    local.rotation.angle_between(rotation) < EPS,
                    "bone {bone}: {:?} vs {rotation:?}",
                    local.rotation
                );
            }
        }
    }

    fn add_graph(
        world: &mut World,
        input: &ConstantPose,
        node: FootPlacementNode,
    ) -> AssetHandle<AnimationGraph> {
        let mut registry = TypeRegistry::default();
        register_animation_types(&mut registry);
        registry.register::<FootPlacementNode>();
        registry.register::<ConstantPose>();
        let mut graph = AnimationGraph::new();
        graph.result_node().with_input(node, |mut placement| {
            placement.with_input(
                ConstantPose {
                    translations: input.translations.clone(),
                    rotations: input.rotations.clone(),
                    root_translation: input.root_translation,
                    root_rotation: input.root_rotation,
                },
                |_| {},
            );
        });
        graph.make_live(&registry, None).unwrap();
        let handle = AssetHandle::<AnimationGraph>::weak(AssetId::new());
        world
            .get_resource_mut::<AssetStore<AnimationGraph>>()
            .unwrap()
            .insert(handle.id(), graph);
        handle
    }

    #[test]
    fn a_leg_the_node_cannot_resolve_leaves_the_input_pose_and_warns_once() {
        let unresolvable = [
            FootPlacementNode {
                legs: vec![leg("missing_foot", None)],
                ..placement()
            },
            FootPlacementNode {
                legs: vec![leg("left_hip", None)],
                ..placement()
            },
            FootPlacementNode {
                pelvis: "right_hip".into(),
                ..placement()
            },
        ];

        for node in unresolvable {
            let skeleton = rig();
            let input = ConstantPose::shifted_of(&skeleton);
            let mut stage = Stage::new(skeleton, input, node.clone()).with_physics();
            stage.add_floor(0.1);
            stage.settle();

            stage.assert_input_pose();
            assert!(stage.warned(), "{node:?}");
        }
    }

    #[test]
    fn a_resolvable_node_does_not_warn() {
        let skeleton = rig();
        let input = ConstantPose::shifted_of(&skeleton);
        let mut stage = Stage::new(skeleton, input, placement()).with_physics();
        stage.add_floor(0.0);
        stage.settle();

        assert!(!stage.warned());
    }

    #[test]
    fn without_physics_the_output_is_the_input_pose() {
        let skeleton = rig();
        let input = ConstantPose::shifted_of(&skeleton);
        let mut stage = Stage::new(skeleton, input, placement());
        stage.settle();

        stage.assert_input_pose();
        assert!(!stage.warned());
    }

    #[test]
    fn the_inputs_root_motion_reaches_the_output_unchanged() {
        let skeleton = rig();
        let moved = RootMotion {
            translation: Vec3::new(0.01, 0.0, 0.04),
            rotation: Quat::from_rotation_y(0.03),
        };
        let input = ConstantPose {
            root_translation: moved.translation,
            root_rotation: moved.rotation,
            ..ConstantPose::shifted_of(&skeleton)
        };
        let mut stage = Stage::new(skeleton, input, placement()).with_physics();
        stage.add_floor(0.1);
        let root = stage.bones[0];
        stage.world().insert(AnimationRootBone::default(), root);
        stage.settle();

        let root_motion = stage
            .world()
            .get_component_for_entity::<AnimationRootBone>(root)
            .unwrap()
            .root_motion;
        assert_eq!(root_motion, moved);
    }

    #[test]
    fn evaluate_takes_the_elapsed_time() {
        let skeleton = rig();
        let input = ConstantPose::rest_of(&skeleton);
        let mut stage = Stage::new(skeleton, input, placement()).with_physics();
        stage.add_floor(0.0);
        stage.settle();

        stage.with_state(|state| state.elapsed = 0.3);
        stage.run();

        assert_eq!(stage.with_state(|state| state.elapsed), 0.0);
    }
}

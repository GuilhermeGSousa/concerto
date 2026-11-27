use concerto_mesh::{JointName, Skeleton};
use concerto_name::name;
use concerto_reflect::{Reflect, ReflectDefault};
use glam::{Quat, Vec3};
use log::warn;
use uuid::Uuid;

use crate::{
    blackboard::AnimationBlackboard,
    evaluation::AnimationGraphContext,
    ik::{Hinge, TwoBoneChain, solve_two_bone},
    node::{
        AnimationNode, EditorAnimationNode, InputSlots, ReflectAnimationNode, effective_weight,
        model_space::{ModelJoint, WorldToModel, bone_under_first, model_joint, parent},
        pass_through,
    },
    param::Param,
    pose::{AnimationFrame, EvaluatedFrame, FramePool, JointPose},
};

/// Bends the chain ending at `tip` so the tip reaches `target`; passes its first input through first.
#[derive(Clone, Debug, PartialEq, Reflect, serde::Serialize, serde::Deserialize)]
#[reflect(AnimationNode, Default)]
pub struct TwoBoneIkNode {
    /// The end bone of the chain; its parent is the mid joint and its grandparent the root.
    pub tip: JointName,
    /// The point the tip reaches for.
    pub target: IkGoal,
    /// The point the mid joint bends toward, when given.
    pub pole: Option<IkGoal>,
    /// What rotation the tip ends with.
    pub tip_rotation: TipRotation,
    /// How far short of full reach, in model-space units, the chain starts easing toward straight
    /// instead of snapping; 0 for none.
    pub softness: f32,
    /// The axis the mid joint bends about where the chain is too straight to show its own bend.
    pub hinge: Hinge,
    /// How much of the solve is applied, before the param scales it; clamped to 0 to 1.
    pub weight: f32,
    /// The `f32` blackboard param the weight is multiplied by, when it holds one.
    pub weight_param: Option<Param<f32>>,
}

impl Default for TwoBoneIkNode {
    fn default() -> Self {
        Self {
            tip: JointName::default(),
            target: IkGoal::default(),
            pole: None,
            tip_rotation: TipRotation::KeepInput,
            softness: 0.0,
            hinge: Hinge::Off,
            weight: 1.0,
            weight_param: None,
        }
    }
}

/// Where an IK goal is.
#[derive(Clone, Debug, PartialEq, Reflect, serde::Serialize, serde::Deserialize)]
pub enum IkGoal {
    /// A point fixed to a bone of the input pose: `translation` in the bone's space, with the bone's rotation times `rotation`.
    Bone {
        /// The bone the goal follows.
        bone: JointName,
        /// The goal's offset in the bone's space.
        translation: Vec3,
        /// The goal's rotation relative to the bone's.
        rotation: Quat,
    },
    /// The `IkTarget` the game set under `param`, in world space.
    Param {
        /// The blackboard param holding the target.
        ///
        /// When it is missing, a target leaves the chain as it is and a pole is ignored.
        param: Param<IkTarget>,
    },
}

impl Default for IkGoal {
    fn default() -> Self {
        IkGoal::Bone {
            bone: JointName::default(),
            translation: Vec3::ZERO,
            rotation: Quat::IDENTITY,
        }
    }
}

/// A world-space IK goal a game puts on the blackboard.
#[derive(Clone, Copy, Debug, Default, PartialEq, Reflect)]
#[reflect(Default)]
pub struct IkTarget {
    /// The goal's position in world space.
    pub position: Vec3,
    /// The rotation the tip takes in world space under [`TipRotation::FromTarget`], if any.
    pub rotation: Option<Quat>,
}

/// What rotation the tip ends with.
#[derive(Clone, Debug, Default, PartialEq, Reflect, serde::Serialize, serde::Deserialize)]
pub enum TipRotation {
    /// The tip's model-space rotation from the input pose.
    #[default]
    KeepInput,
    /// The goal's rotation, or the input's when the goal has none.
    FromTarget,
}

/// Whether a [`TwoBoneIkNode`] has already warned about a chain it cannot resolve.
#[derive(Debug, Default)]
pub struct TwoBoneIkState {
    warned: bool,
}

struct ResolvedChain<'a> {
    skeleton: &'a Skeleton,
    root: usize,
    mid: usize,
    tip: usize,
    target: ResolvedGoal,
    pole: Option<ResolvedGoal>,
    hinge: Option<Vec3>,
}

enum ResolvedGoal {
    Bone {
        bone: usize,
        translation: Vec3,
        rotation: Quat,
    },
    Param(Param<IkTarget>),
}

struct Goal {
    position: Vec3,
    rotation: Option<Quat>,
}

impl TwoBoneIkNode {
    fn resolve<'a>(&self, skeleton: Option<&'a Skeleton>) -> Result<ResolvedChain<'a>, String> {
        let skeleton = skeleton.ok_or("the player has no skeleton")?;
        let tip = bone_under_first(skeleton, self.tip)?;
        let mid = parent(skeleton, tip).ok_or("the tip bone has no parent")?;
        let root = parent(skeleton, mid).ok_or("the tip bone has no grandparent")?;
        Ok(ResolvedChain {
            skeleton,
            root,
            mid,
            tip,
            target: resolve_goal(skeleton, &self.target)?,
            pole: self
                .pole
                .as_ref()
                .map(|pole| resolve_goal(skeleton, pole))
                .transpose()?,
            hinge: self.hinge.local(skeleton, root, mid, tip),
        })
    }

    fn solve(
        &self,
        chain: &ResolvedChain<'_>,
        context: &AnimationGraphContext<'_>,
        joints: &[JointPose],
    ) -> Option<[(usize, Quat); 3]> {
        let skeleton = chain.skeleton;
        let model = |bone| model_joint(skeleton, joints, bone);
        let world_to_model = WorldToModel::new(context.model_to_world().unwrap_or_default());
        let blackboard = context.blackboard();

        let target = chain
            .target
            .read(model, &world_to_model, blackboard, true)??;
        let pole = match &chain.pole {
            Some(pole) => pole.read(model, &world_to_model, blackboard, false)?,
            None => None,
        };

        let root = model(chain.root)?;
        let mid = model(chain.mid)?;
        let tip = model(chain.tip)?;
        let parent_rotation = match parent(skeleton, chain.root) {
            Some(parent) => model(parent)?.rotation,
            None => Quat::IDENTITY,
        };

        let solution = solve_two_bone(
            &TwoBoneChain {
                root: root.transform.translation.into(),
                mid: mid.transform.translation.into(),
                tip: tip.transform.translation.into(),
                hinge: chain.hinge.map(|axis| mid.rotation * axis),
            },
            target.position,
            pole.map(|pole| pole.position),
            self.softness,
        );

        let root_rotation = solution.root * root.rotation;
        let mid_rotation = solution.mid * mid.rotation;
        let tip_rotation = match self.tip_rotation {
            TipRotation::KeepInput => tip.rotation,
            TipRotation::FromTarget => target.rotation.unwrap_or(tip.rotation),
        };
        Some([
            (
                chain.root,
                (parent_rotation.inverse() * root_rotation).normalize(),
            ),
            (
                chain.mid,
                (root_rotation.inverse() * mid_rotation).normalize(),
            ),
            (
                chain.tip,
                (mid_rotation.inverse() * tip_rotation).normalize(),
            ),
        ])
    }
}

impl ResolvedGoal {
    fn read(
        &self,
        model: impl Fn(usize) -> Option<ModelJoint>,
        world_to_model: &WorldToModel,
        blackboard: &AnimationBlackboard,
        is_target: bool,
    ) -> Option<Option<Goal>> {
        let goal = match self {
            ResolvedGoal::Bone {
                bone,
                translation,
                rotation,
            } => {
                let joint = model(*bone)?;
                Goal {
                    position: joint.transform.transform_point3(*translation),
                    rotation: Some(joint.rotation * *rotation),
                }
            }
            ResolvedGoal::Param(param) => {
                let Some(target) = blackboard.read(param) else {
                    return if is_target { None } else { Some(None) };
                };
                if !world_to_model.is_finite() {
                    return None;
                }
                Goal {
                    position: world_to_model.transform.transform_point3(target.position),
                    rotation: target
                        .rotation
                        .map(|rotation| world_to_model.rotation * rotation),
                }
            }
        };
        let finite = goal.position.is_finite()
            && (!is_target || goal.rotation.is_none_or(|rotation| rotation.is_finite()));
        finite.then_some(Some(goal))
    }
}

fn resolve_goal(skeleton: &Skeleton, goal: &IkGoal) -> Result<ResolvedGoal, String> {
    Ok(match goal {
        IkGoal::Bone {
            bone,
            translation,
            rotation,
        } => ResolvedGoal::Bone {
            bone: bone_under_first(skeleton, *bone)?,
            translation: *translation,
            rotation: *rotation,
        },
        IkGoal::Param { param } => ResolvedGoal::Param(*param),
    })
}

impl AnimationNode for TwoBoneIkNode {
    type State = TwoBoneIkState;
    type CreateInput = ();
    type UpdateInput = ();
    type EvaluateInput = ();

    fn create_state(&self, _: &(), _context: &AnimationGraphContext<'_>) -> TwoBoneIkState {
        TwoBoneIkState::default()
    }

    fn update(&self, _: &mut TwoBoneIkState, _: f32, _: &(), _: &mut AnimationGraphContext<'_>) {}

    fn evaluate(
        &self,
        state: &mut TwoBoneIkState,
        _: &(),
        context: &AnimationGraphContext<'_>,
        _: &[Uuid],
        evaluated_inputs: &[EvaluatedFrame],
        _: &mut FramePool,
        output: &mut AnimationFrame,
    ) {
        pass_through(evaluated_inputs, output);
        let weight = effective_weight(self.weight, self.weight_param, context.blackboard());
        if weight <= 0.0 {
            return;
        }
        let chain = match self.resolve(context.skeleton()) {
            Ok(chain) => chain,
            Err(reason) => {
                if !state.warned {
                    warn!("two-bone IK on `{}` skipped: {reason}", &*self.tip);
                    state.warned = true;
                }
                return;
            }
        };
        if let Some(solved) = self.solve(&chain, context, output.joints()) {
            for (bone, rotation) in solved {
                if let Some(joint) = output.joint_mut(bone) {
                    joint.rotation = joint.rotation.slerp(rotation, weight);
                }
            }
        }
    }

    fn reset(&self, _: &mut TwoBoneIkState) {}
}

impl EditorAnimationNode for TwoBoneIkNode {
    fn input_slots(&self) -> InputSlots {
        InputSlots::Fixed(vec![name!("pose")])
    }
}

#[cfg(test)]
mod tests {
    use concerto_foundation::assets::{AssetId, asset_store::AssetStore};
    use concerto_mesh::{Skeleton, skeleton::JointPose};
    use concerto_name::name;
    use glam::{Affine3A, Mat4, Quat, Vec3};

    use super::{Hinge, IkGoal, IkTarget, TipRotation, TwoBoneIkNode, TwoBoneIkState};
    use crate::{
        blackboard::AnimationBlackboard,
        evaluation::AnimationGraphContext,
        node::AnimationNode,
        pose::{AnimationFrame, EvaluatedFrame, FiredEvent, FramePool, Pose},
        testing::Walks,
    };

    const UPPER: usize = 1;
    const LOWER: usize = 2;
    const HAND: usize = 3;
    const GRIP: usize = 4;
    const CHAIN: [usize; 3] = [UPPER, LOWER, HAND];

    fn joint(translation: Vec3, rotation: Quat) -> JointPose {
        JointPose {
            translation,
            rotation,
            scale: Vec3::ONE,
        }
    }

    struct Bone {
        name: &'static str,
        parent: Option<u16>,
        joint: JointPose,
    }

    fn bone(name: &'static str, parent: Option<u16>, joint: JointPose) -> Bone {
        Bone {
            name,
            parent,
            joint,
        }
    }

    fn skeleton() -> Skeleton {
        let bones = [
            bone("pelvis", None, joint(Vec3::Y, Quat::from_rotation_y(0.7))),
            bone(
                "upper",
                Some(0),
                joint(Vec3::new(0.3, 0.0, 0.0), Quat::from_rotation_z(0.3)),
            ),
            bone(
                "lower",
                Some(1),
                joint(Vec3::new(0.0, -1.0, 0.0), Quat::from_rotation_z(-0.6)),
            ),
            bone(
                "hand",
                Some(2),
                joint(Vec3::new(0.0, -0.8, 0.0), Quat::from_rotation_x(0.2)),
            ),
            bone(
                "grip",
                Some(0),
                joint(Vec3::new(0.2, -0.9, 0.5), Quat::from_rotation_x(0.4)),
            ),
            bone("prop", None, joint(Vec3::X, Quat::IDENTITY)),
            bone("prop_a", Some(5), joint(Vec3::Y, Quat::IDENTITY)),
            bone(
                "prop_b",
                Some(6),
                joint(Vec3::Y, Quat::from_rotation_z(0.5)),
            ),
            bone("prop_tip", Some(7), joint(Vec3::Y, Quat::IDENTITY)),
        ];
        Skeleton::new(
            vec![Mat4::IDENTITY; bones.len()],
            bones.iter().map(|bone| bone.parent).collect(),
            bones.iter().map(|bone| bone.name.into()).collect(),
            bones.iter().map(|bone| bone.joint.clone()).collect(),
            (0..bones.len() as u128)
                .map(uuid::Uuid::from_u128)
                .collect(),
        )
        .unwrap()
    }

    fn input_event() -> FiredEvent {
        FiredEvent {
            clip: AssetId::from_path("ik_input"),
            key: 2,
            time: 0.4,
            weight: 0.5,
            mirrored: false,
        }
    }

    struct Rig {
        walks: Walks,
        blackboard: AnimationBlackboard,
        skeleton: Skeleton,
        model_to_world: Option<Affine3A>,
    }

    impl Rig {
        fn new() -> Self {
            Self {
                walks: Walks::new(AssetStore::new(), AssetStore::new()),
                blackboard: AnimationBlackboard::default(),
                skeleton: skeleton(),
                model_to_world: None,
            }
        }

        fn input(&self) -> AnimationFrame {
            let mut frame = AnimationFrame::new(Pose::from_joints(self.skeleton.rest_pose.clone()));
            frame.push_event(input_event());
            frame
        }

        fn solve(&self, node: &TwoBoneIkNode) -> AnimationFrame {
            let input = [EvaluatedFrame {
                frame: self.input(),
                weight: 1.0,
            }];
            let mut pool = FramePool::new(self.skeleton.bone_count());
            let mut output = pool.acquire();
            let mut state = TwoBoneIkState::default();
            self.walks.evaluate_posed(
                &self.blackboard,
                &self.skeleton,
                self.model_to_world,
                |context: &AnimationGraphContext<'_>| {
                    node.evaluate(
                        &mut state,
                        &(),
                        context,
                        &[],
                        &input,
                        &mut pool,
                        &mut output,
                    )
                },
            );
            output
        }

        fn model(&self, frame: &AnimationFrame, bone: usize) -> Mat4 {
            self.skeleton.forward_kinematics(frame.joints()).unwrap()[bone]
        }

        fn model_position(&self, frame: &AnimationFrame, bone: usize) -> Vec3 {
            self.model(frame, bone).w_axis.truncate()
        }

        fn model_rotation(&self, frame: &AnimationFrame, bone: usize) -> Quat {
            self.model(frame, bone).to_scale_rotation_translation().1
        }

        fn grip_point(&self, translation: Vec3) -> Vec3 {
            self.model(&self.input(), GRIP)
                .transform_point3(translation)
        }
    }

    fn node(target: IkGoal) -> TwoBoneIkNode {
        TwoBoneIkNode {
            tip: "hand".into(),
            target,
            ..TwoBoneIkNode::default()
        }
    }

    fn on_grip(translation: Vec3, rotation: Quat) -> IkGoal {
        IkGoal::Bone {
            bone: "grip".into(),
            translation,
            rotation,
        }
    }

    fn on_param() -> IkGoal {
        IkGoal::Param {
            param: name!("hand_target").into(),
        }
    }

    fn assert_near(actual: Vec3, expected: Vec3) {
        assert!(
            actual.abs_diff_eq(expected, 1e-4),
            "expected {expected}, got {actual}"
        );
    }

    fn assert_rotation(actual: Quat, expected: Quat) {
        assert!(
            actual.angle_between(expected) < 1e-3,
            "expected rotation {expected}, got {actual}"
        );
    }

    fn turned_world() -> Affine3A {
        Affine3A::from_rotation_translation(Quat::from_rotation_y(1.1), Vec3::new(3.0, 0.0, -2.0))
    }

    fn across(v: Vec3, axis: Vec3) -> Vec3 {
        let axis = axis.normalize();
        v - axis * v.dot(axis)
    }

    #[test]
    fn a_bone_or_world_goal_puts_the_tip_on_it_and_leaves_every_other_bone_and_the_events_alone() {
        let rig = Rig::new();
        let offset = Vec3::new(0.05, 0.02, 0.1);

        let frame = rig.solve(&node(on_grip(offset, Quat::IDENTITY)));

        assert_near(rig.model_position(&frame, HAND), rig.grip_point(offset));
        assert_eq!(frame.events(), [input_event()]);
        let input = rig.input();
        for bone in (0..rig.skeleton.bone_count()).filter(|bone| !CHAIN.contains(bone)) {
            assert_eq!(frame.joints()[bone], input.joints()[bone], "bone {bone}");
        }
        for bone in CHAIN {
            assert_eq!(
                frame.joints()[bone].translation,
                input.joints()[bone].translation
            );
        }

        let mut rig = Rig::new();
        rig.model_to_world = Some(turned_world());
        let goal = rig.grip_point(Vec3::new(-0.1, 0.1, 0.0));
        rig.blackboard.set(
            name!("hand_target"),
            IkTarget {
                position: turned_world().transform_point3(goal),
                rotation: None,
            },
        );

        let frame = rig.solve(&node(on_param()));

        assert_near(rig.model_position(&frame, HAND), goal);
    }

    fn aiming_at_the_grip(rig: &mut Rig, rotation: Option<Quat>) {
        rig.model_to_world = Some(turned_world());
        rig.blackboard.set(
            name!("hand_target"),
            IkTarget {
                position: turned_world().transform_point3(rig.grip_point(Vec3::ZERO)),
                rotation,
            },
        );
    }

    struct TipCase {
        name: &'static str,
        setup: fn(&mut Rig) -> TwoBoneIkNode,
        rotation: fn(&Rig) -> Quat,
    }

    #[test]
    fn the_tip_keeps_its_input_model_rotation_or_takes_the_goals() {
        let cases = [
            TipCase {
                name: "keep input keeps the model rotation of the tip",
                setup: |_| node(on_grip(Vec3::ZERO, Quat::from_rotation_y(0.9))),
                rotation: |rig| rig.model_rotation(&rig.input(), HAND),
            },
            TipCase {
                name: "from target with a param goal maps its rotation into the model",
                setup: |rig| {
                    aiming_at_the_grip(rig, Some(Quat::from_rotation_x(0.8)));
                    TwoBoneIkNode {
                        tip_rotation: TipRotation::FromTarget,
                        ..node(on_param())
                    }
                },
                rotation: |_| Quat::from_rotation_y(1.1).inverse() * Quat::from_rotation_x(0.8),
            },
        ];

        for case in cases {
            let mut rig = Rig::new();
            let node = (case.setup)(&mut rig);

            let frame = rig.solve(&node);

            let actual = rig.model_rotation(&frame, HAND);
            let expected = (case.rotation)(&rig);
            assert!(
                actual.angle_between(expected) < 1e-3,
                "{}: expected rotation {expected}, got {actual}",
                case.name
            );
        }
    }

    struct PoleCase {
        root: Vec3,
        target: Vec3,
        pole: Vec3,
        unpoled_side: Vec3,
    }

    fn pole_case(rig: &Rig) -> PoleCase {
        let target = rig.grip_point(Vec3::ZERO);
        let unpoled = rig.solve(&node(on_grip(Vec3::ZERO, Quat::IDENTITY)));
        let root = rig.model_position(&unpoled, UPPER);
        let unpoled_side = across(rig.model_position(&unpoled, LOWER) - root, target - root);
        PoleCase {
            root,
            target,
            pole: root + (target - root) * 0.5 - unpoled_side.normalize(),
            unpoled_side,
        }
    }

    #[test]
    fn a_pole_puts_the_mid_on_its_world_side() {
        let mut rig = Rig::new();
        rig.model_to_world = Some(turned_world());
        let case = pole_case(&rig);
        rig.blackboard.set(
            name!("elbow_pole"),
            IkTarget {
                position: turned_world().transform_point3(case.pole),
                rotation: None,
            },
        );
        let poled = TwoBoneIkNode {
            pole: Some(IkGoal::Param {
                param: name!("elbow_pole").into(),
            }),
            ..node(on_grip(Vec3::ZERO, Quat::IDENTITY))
        };

        let frame = rig.solve(&poled);

        let axis = case.target - case.root;
        let side = across(rig.model_position(&frame, LOWER) - case.root, axis).normalize();
        let pole_side = across(case.pole - case.root, axis).normalize();
        assert!(
            side.dot(pole_side) > 0.999,
            "the mid is on {side}, the pole on {pole_side}"
        );
        assert!(case.unpoled_side.dot(pole_side) < 0.0);

        let input = rig.input();
        let input_bend = (rig.model_position(&input, LOWER) - rig.model_position(&input, UPPER))
            .cross(rig.model_position(&input, HAND) - rig.model_position(&input, LOWER))
            .normalize();
        let against = -(rig.model_rotation(&input, LOWER).inverse() * input_bend);
        let hinged = TwoBoneIkNode {
            hinge: Hinge::Axis { axis: against },
            ..node(on_grip(Vec3::ZERO, Quat::IDENTITY))
        };
        let hinge_side = axis.cross(-input_bend).normalize();

        let frame = rig.solve(&hinged);

        let side = across(rig.model_position(&frame, LOWER) - case.root, axis).normalize();
        assert!(
            side.dot(hinge_side) > 0.99,
            "the mid is on {side}, the hinge side is {hinge_side}"
        );

        rig.blackboard.set(
            name!("elbow_pole"),
            IkTarget {
                position: turned_world()
                    .transform_point3(case.root + axis * 0.5 + case.unpoled_side.normalize()),
                rotation: None,
            },
        );
        let frame = rig.solve(&TwoBoneIkNode {
            pole: poled.pole,
            ..hinged
        });

        let side = across(rig.model_position(&frame, LOWER) - case.root, axis).normalize();
        assert!(
            side.dot(case.unpoled_side.normalize()) > 0.99,
            "with a pole on the input side the mid is on {side}"
        );
    }

    #[test]
    fn a_partial_weight_slerps_each_chain_bone_from_the_input_toward_the_solve() {
        let rig = Rig::new();
        let full = node(on_grip(Vec3::ZERO, Quat::IDENTITY));
        let solved = rig.solve(&full);
        let input = rig.input();

        let by_weight = rig.solve(&TwoBoneIkNode {
            weight: 0.5,
            ..full.clone()
        });

        for bone in CHAIN {
            let halfway = input.joints()[bone]
                .rotation
                .slerp(solved.joints()[bone].rotation, 0.5);
            assert_rotation(by_weight.joints()[bone].rotation, halfway);
        }
    }
}

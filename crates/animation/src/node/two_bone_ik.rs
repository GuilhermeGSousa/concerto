use concerto_mesh::{JointName, Skeleton};
use concerto_name::Name;
use concerto_reflect::Reflect;
use glam::{Affine3A, Quat, Vec3};
use log::warn;
use uuid::Uuid;

use crate::{
    blackboard::AnimationBlackboard,
    evaluation::AnimationGraphContext,
    ik::{TwoBoneChain, solve_two_bone},
    node::{AnimationNode, ReflectAnimationNode, pass_through},
    pose::{EvaluatedPose, JointPose, Pose, PosePool},
};

/// Bends the chain ending at `tip` so the tip reaches `target`; passes its first input through first.
#[derive(Clone, Debug, PartialEq, Reflect, serde::Serialize, serde::Deserialize)]
#[reflect(AnimationNode)]
pub struct TwoBoneIkNode {
    pub tip: JointName,
    pub target: IkGoal,
    pub pole: Option<IkGoal>,
    pub tip_rotation: TipRotation,
    pub softness: f32,
    pub weight: f32,
    pub weight_param: Option<Name>,
}

impl Default for TwoBoneIkNode {
    fn default() -> Self {
        Self {
            tip: JointName::default(),
            target: IkGoal::default(),
            pole: None,
            tip_rotation: TipRotation::KeepInput,
            softness: 0.0,
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
        bone: JointName,
        translation: Vec3,
        rotation: Quat,
    },
    /// The `IkTarget` the game set under `param`, in world space.
    Param { param: Name },
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
#[derive(Clone, Copy, Debug, PartialEq, Reflect)]
pub struct IkTarget {
    pub position: Vec3,
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
}

enum ResolvedGoal {
    Bone {
        bone: usize,
        translation: Vec3,
        rotation: Quat,
    },
    Param(Name),
}

struct ModelJoint {
    transform: Affine3A,
    rotation: Quat,
}

struct Goal {
    position: Vec3,
    rotation: Option<Quat>,
}

impl TwoBoneIkNode {
    fn effective_weight(&self, blackboard: &AnimationBlackboard) -> f32 {
        let scale = self
            .weight_param
            .and_then(|param| blackboard.get_float(param))
            .unwrap_or(1.0);
        let weight = self.weight * scale;
        if weight.is_nan() {
            0.0
        } else {
            weight.clamp(0.0, 1.0)
        }
    }

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

struct WorldToModel {
    transform: Affine3A,
    rotation: Quat,
}

impl WorldToModel {
    fn new(model_to_world: Affine3A) -> Self {
        let (_, rotation, _) = model_to_world.to_scale_rotation_translation();
        Self {
            transform: model_to_world.inverse(),
            rotation: rotation.inverse(),
        }
    }

    fn is_finite(&self) -> bool {
        self.transform.is_finite() && self.rotation.is_finite()
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
                let Some(target) = blackboard.get::<IkTarget>(*param) else {
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

fn parent(skeleton: &Skeleton, bone: usize) -> Option<usize> {
    skeleton
        .parents
        .get(bone)
        .copied()
        .flatten()
        .map(usize::from)
}

fn bone_under_first(skeleton: &Skeleton, name: JointName) -> Result<usize, String> {
    let bone = skeleton
        .bone_index(name.0)
        .ok_or_else(|| format!("the skeleton has no bone `{}`", &*name))?;
    let mut top = bone;
    while let Some(up) = parent(skeleton, top) {
        top = up;
    }
    if top == 0 {
        Ok(bone)
    } else {
        Err(format!("bone `{}` does not descend from bone 0", &*name))
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

fn model_joint(skeleton: &Skeleton, joints: &[JointPose], bone: usize) -> Option<ModelJoint> {
    let joint = joints.get(bone)?;
    let local =
        Affine3A::from_scale_rotation_translation(joint.scale, joint.rotation, joint.translation);
    Some(match parent(skeleton, bone) {
        Some(up) => {
            let parent = model_joint(skeleton, joints, up)?;
            ModelJoint {
                transform: parent.transform * local,
                rotation: parent.rotation * joint.rotation,
            }
        }
        None => ModelJoint {
            transform: local,
            rotation: joint.rotation,
        },
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

    fn update(&self, _: &mut TwoBoneIkState, _: f32, _: &(), _: &AnimationGraphContext<'_>) {}

    fn evaluate(
        &self,
        state: &mut TwoBoneIkState,
        _: &(),
        context: &AnimationGraphContext<'_>,
        _: &[Uuid],
        evaluated_inputs: &[EvaluatedPose],
        _: &mut PosePool,
        output: &mut Pose,
    ) {
        pass_through(evaluated_inputs, output);
        let weight = self.effective_weight(context.blackboard());
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
                if let Some(joint) = output.get_joint_pose_mut(bone) {
                    joint.rotation = joint.rotation.slerp(rotation, weight);
                }
            }
        }
    }

    fn reset(&self, _: &mut TwoBoneIkState) {}
}

#[cfg(test)]
mod tests {
    use concerto_foundation::assets::asset_store::AssetStore;
    use concerto_mesh::{JointName, Skeleton, skeleton::JointPose};
    use concerto_name::name;
    use glam::{Affine3A, Mat4, Quat, Vec3};

    use super::{IkGoal, IkTarget, TipRotation, TwoBoneIkNode, TwoBoneIkState};
    use crate::{
        blackboard::AnimationBlackboard,
        evaluation::AnimationGraphContext,
        node::AnimationNode,
        pose::{EvaluatedPose, Pose, PosePool, RootMotion},
        test_walks::Walks,
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

    fn skeleton() -> Skeleton {
        let bones = [
            ("pelvis", None, joint(Vec3::Y, Quat::from_rotation_y(0.7))),
            (
                "upper",
                Some(0),
                joint(Vec3::new(0.3, 0.0, 0.0), Quat::from_rotation_z(0.3)),
            ),
            (
                "lower",
                Some(1),
                joint(Vec3::new(0.0, -1.0, 0.0), Quat::from_rotation_z(-0.6)),
            ),
            (
                "hand",
                Some(2),
                joint(Vec3::new(0.0, -0.8, 0.0), Quat::from_rotation_x(0.2)),
            ),
            (
                "grip",
                Some(0),
                joint(Vec3::new(0.2, -0.9, 0.5), Quat::from_rotation_x(0.4)),
            ),
            ("prop", None, joint(Vec3::X, Quat::IDENTITY)),
            ("prop_a", Some(5), joint(Vec3::Y, Quat::IDENTITY)),
            (
                "prop_b",
                Some(6),
                joint(Vec3::Y, Quat::from_rotation_z(0.5)),
            ),
            ("prop_tip", Some(7), joint(Vec3::Y, Quat::IDENTITY)),
        ];
        Skeleton::new(
            vec![Mat4::IDENTITY; bones.len()],
            bones.iter().map(|(_, parent, _)| *parent).collect(),
            bones.iter().map(|(name, _, _)| (*name).into()).collect(),
            bones.iter().map(|(_, _, joint)| joint.clone()).collect(),
            (0..bones.len() as u128)
                .map(uuid::Uuid::from_u128)
                .collect(),
        )
        .unwrap()
    }

    fn input_motion() -> RootMotion {
        RootMotion {
            translation: Vec3::new(0.1, 0.0, 0.3),
            rotation: Quat::from_rotation_y(0.05),
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

        fn input(&self) -> Pose {
            let mut pose = Pose::from_joints(self.skeleton.rest_pose.clone());
            pose.set_root_motion(input_motion());
            pose
        }

        fn solve(&self, node: &TwoBoneIkNode) -> Pose {
            self.run(node, true)
        }

        fn solve_without_skeleton(&self, node: &TwoBoneIkNode) -> Pose {
            self.run(node, false)
        }

        fn run(&self, node: &TwoBoneIkNode, with_skeleton: bool) -> Pose {
            let input = [EvaluatedPose {
                pose: self.input(),
                weight: 1.0,
            }];
            let mut pool = PosePool::new(self.skeleton.bone_count());
            let mut output = pool.acquire();
            let mut state = TwoBoneIkState::default();
            let evaluate = |context: &AnimationGraphContext<'_>| {
                node.evaluate(
                    &mut state,
                    &(),
                    context,
                    &[],
                    &input,
                    &mut pool,
                    &mut output,
                )
            };
            if with_skeleton {
                self.walks.evaluate_posed(
                    &self.blackboard,
                    &self.skeleton,
                    self.model_to_world,
                    evaluate,
                );
            } else {
                self.walks.evaluate(&self.blackboard, None, evaluate);
            }
            output
        }

        fn model(&self, pose: &Pose, bone: usize) -> Mat4 {
            self.skeleton.forward_kinematics(pose.joints()).unwrap()[bone]
        }

        fn model_position(&self, pose: &Pose, bone: usize) -> Vec3 {
            self.model(pose, bone).w_axis.truncate()
        }

        fn model_rotation(&self, pose: &Pose, bone: usize) -> Quat {
            self.model(pose, bone).to_scale_rotation_translation().1
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
            param: name!("hand_target"),
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

    fn assert_unchanged(rig: &Rig, pose: &Pose) {
        assert_eq!(pose.joints(), rig.input().joints());
        assert_eq!(pose.root_motion(), input_motion());
    }

    fn turned_world() -> Affine3A {
        Affine3A::from_rotation_translation(Quat::from_rotation_y(1.1), Vec3::new(3.0, 0.0, -2.0))
    }

    fn across(v: Vec3, axis: Vec3) -> Vec3 {
        let axis = axis.normalize();
        v - axis * v.dot(axis)
    }

    #[test]
    fn a_bone_goal_puts_the_tip_on_the_goal_and_leaves_every_other_bone_alone() {
        let rig = Rig::new();
        let offset = Vec3::new(0.05, 0.02, 0.1);

        let pose = rig.solve(&node(on_grip(offset, Quat::IDENTITY)));

        assert_near(rig.model_position(&pose, HAND), rig.grip_point(offset));
        assert_eq!(pose.root_motion(), input_motion());
        let input = rig.input();
        for bone in (0..rig.skeleton.bone_count()).filter(|bone| !CHAIN.contains(bone)) {
            assert_eq!(pose.joints()[bone], input.joints()[bone], "bone {bone}");
        }
        for bone in CHAIN {
            assert_eq!(
                pose.joints()[bone].translation,
                input.joints()[bone].translation
            );
        }
    }

    #[test]
    fn keep_input_keeps_the_model_rotation_of_the_tip() {
        let rig = Rig::new();

        let pose = rig.solve(&node(on_grip(Vec3::ZERO, Quat::from_rotation_y(0.9))));

        assert_rotation(
            rig.model_rotation(&pose, HAND),
            rig.model_rotation(&rig.input(), HAND),
        );
    }

    #[test]
    fn from_target_gives_the_tip_the_grip_rotation_times_the_goal_rotation() {
        let rig = Rig::new();
        let turn = Quat::from_rotation_y(0.9);

        let pose = rig.solve(&TwoBoneIkNode {
            tip_rotation: TipRotation::FromTarget,
            ..node(on_grip(Vec3::ZERO, turn))
        });

        assert_rotation(
            rig.model_rotation(&pose, HAND),
            rig.model_rotation(&rig.input(), GRIP) * turn,
        );
    }

    #[test]
    fn a_param_goal_reaches_its_world_position_under_a_turned_model() {
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

        let pose = rig.solve(&node(on_param()));

        assert_near(rig.model_position(&pose, HAND), goal);
    }

    #[test]
    fn from_target_with_a_param_goal_that_has_no_rotation_keeps_the_input_tip_rotation() {
        let mut rig = Rig::new();
        rig.model_to_world = Some(turned_world());
        rig.blackboard.set(
            name!("hand_target"),
            IkTarget {
                position: turned_world().transform_point3(rig.grip_point(Vec3::ZERO)),
                rotation: None,
            },
        );

        let pose = rig.solve(&TwoBoneIkNode {
            tip_rotation: TipRotation::FromTarget,
            ..node(on_param())
        });

        assert_rotation(
            rig.model_rotation(&pose, HAND),
            rig.model_rotation(&rig.input(), HAND),
        );
    }

    #[test]
    fn from_target_with_a_param_goal_maps_its_rotation_into_the_model() {
        let mut rig = Rig::new();
        rig.model_to_world = Some(turned_world());
        let world_rotation = Quat::from_rotation_x(0.8);
        rig.blackboard.set(
            name!("hand_target"),
            IkTarget {
                position: turned_world().transform_point3(rig.grip_point(Vec3::ZERO)),
                rotation: Some(world_rotation),
            },
        );

        let pose = rig.solve(&TwoBoneIkNode {
            tip_rotation: TipRotation::FromTarget,
            ..node(on_param())
        });

        assert_rotation(
            rig.model_rotation(&pose, HAND),
            Quat::from_rotation_y(1.1).inverse() * world_rotation,
        );
    }

    #[test]
    fn a_target_param_that_is_missing_or_of_another_type_leaves_the_input_pose() {
        let mut rig = Rig::new();
        assert_unchanged(&rig, &rig.solve(&node(on_param())));

        rig.blackboard
            .set(name!("hand_target"), Vec3::new(0.3, 0.2, 0.1));
        assert_unchanged(&rig, &rig.solve(&node(on_param())));
    }

    #[test]
    fn a_param_goal_under_a_model_scaled_to_nothing_leaves_the_input_pose() {
        let mut rig = Rig::new();
        rig.model_to_world = Some(Affine3A::from_scale(Vec3::ZERO));
        rig.blackboard.set(
            name!("hand_target"),
            IkTarget {
                position: Vec3::ZERO,
                rotation: None,
            },
        );

        assert_unchanged(&rig, &rig.solve(&node(on_param())));
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

    fn assert_on_the_pole_side(rig: &Rig, pose: &Pose, case: &PoleCase) {
        let axis = case.target - case.root;
        let side = across(rig.model_position(pose, LOWER) - case.root, axis).normalize();
        let pole_side = across(case.pole - case.root, axis).normalize();
        assert!(
            side.dot(pole_side) > 0.999,
            "the mid is on {side}, the pole on {pole_side}"
        );
        assert!(case.unpoled_side.dot(pole_side) < 0.0);
    }

    #[test]
    fn a_param_pole_under_a_turned_model_puts_the_mid_on_its_world_side() {
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

        let pose = rig.solve(&TwoBoneIkNode {
            pole: Some(IkGoal::Param {
                param: name!("elbow_pole"),
            }),
            ..node(on_grip(Vec3::ZERO, Quat::IDENTITY))
        });

        assert_on_the_pole_side(&rig, &pose, &case);
    }

    #[test]
    fn a_bone_pole_puts_the_mid_on_its_side() {
        let rig = Rig::new();
        let case = pole_case(&rig);
        let grip = rig.model(&rig.input(), GRIP);

        let pose = rig.solve(&TwoBoneIkNode {
            pole: Some(on_grip(
                grip.inverse().transform_point3(case.pole),
                Quat::IDENTITY,
            )),
            ..node(on_grip(Vec3::ZERO, Quat::IDENTITY))
        });

        assert_on_the_pole_side(&rig, &pose, &case);
    }

    #[test]
    fn a_missing_pole_param_solves_as_no_pole() {
        let rig = Rig::new();
        let unpoled = node(on_grip(Vec3::ZERO, Quat::IDENTITY));

        let pose = rig.solve(&TwoBoneIkNode {
            pole: Some(IkGoal::Param {
                param: name!("elbow_pole"),
            }),
            ..unpoled.clone()
        });

        assert_eq!(pose.joints(), rig.solve(&unpoled).joints());
    }

    #[test]
    fn a_partial_weight_slerps_each_chain_bone_from_the_input_toward_the_solve() {
        let mut rig = Rig::new();
        rig.blackboard.set(name!("ik_weight"), 0.5_f32);
        let full = node(on_grip(Vec3::ZERO, Quat::IDENTITY));
        let solved = rig.solve(&full);
        let input = rig.input();

        let by_weight = rig.solve(&TwoBoneIkNode {
            weight: 0.5,
            ..full.clone()
        });
        let by_param = rig.solve(&TwoBoneIkNode {
            weight_param: Some(name!("ik_weight")),
            ..full.clone()
        });

        for bone in CHAIN {
            let halfway = input.joints()[bone]
                .rotation
                .slerp(solved.joints()[bone].rotation, 0.5);
            assert_rotation(by_weight.joints()[bone].rotation, halfway);
            assert_rotation(by_param.joints()[bone].rotation, halfway);
        }
    }

    #[test]
    fn zero_weight_leaves_the_input_pose() {
        let rig = Rig::new();

        let pose = rig.solve(&TwoBoneIkNode {
            weight: 0.0,
            ..node(on_grip(Vec3::ZERO, Quat::IDENTITY))
        });

        assert_unchanged(&rig, &pose);
    }

    #[test]
    fn a_chain_the_node_cannot_resolve_leaves_the_input_pose() {
        let rig = Rig::new();
        let reaching = node(on_grip(Vec3::ZERO, Quat::IDENTITY));
        let with_tip = |tip: &str| TwoBoneIkNode {
            tip: tip.into(),
            ..reaching.clone()
        };
        let unresolvable = [
            with_tip("missing"),
            with_tip("upper"),
            with_tip("prop_tip"),
            TwoBoneIkNode {
                target: IkGoal::Bone {
                    bone: JointName::from("missing"),
                    translation: Vec3::ZERO,
                    rotation: Quat::IDENTITY,
                },
                ..reaching.clone()
            },
            TwoBoneIkNode {
                target: IkGoal::Bone {
                    bone: JointName::from("prop_b"),
                    translation: Vec3::ZERO,
                    rotation: Quat::IDENTITY,
                },
                ..reaching.clone()
            },
        ];

        for node in &unresolvable {
            assert_unchanged(&rig, &rig.solve(node));
        }
        assert_unchanged(&rig, &rig.solve_without_skeleton(&reaching));
    }
}

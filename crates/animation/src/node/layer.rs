use concerto_ecs::Res;
use concerto_foundation::assets::{asset_store::AssetStore, handle::AssetHandle};
use concerto_mesh::Skeleton;
use concerto_name::Name;
use concerto_reflect::Reflect;
use glam::Quat;
use log::warn;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    blackboard::AnimationBlackboard,
    evaluation::AnimationGraphContext,
    mask::{BoneMask, MaskResolution, resolve_mask},
    node::{AnimationNode, ReflectAnimationNode, model_space::parent},
    pose::{AnimationFrame, EvaluatedFrame, FiredEvent, FramePool, JointPose},
};

/// Lays its second input over its first on the bones of a mask, by the mask's weights times the layer weight.
#[derive(Clone, Debug, PartialEq, Reflect, Serialize, Deserialize)]
#[reflect(AnimationNode)]
pub struct LayerNode {
    pub mask: AssetHandle<BoneMask>,
    pub space: LayerSpace,
    pub weight: f32,
    pub weight_param: Option<Name>,
}

/// Where a layer blends rotations.
#[derive(Clone, Copy, Debug, Default, PartialEq, Reflect, Serialize, Deserialize)]
pub enum LayerSpace {
    Local,
    #[default]
    Mesh,
}

impl Default for LayerNode {
    fn default() -> Self {
        Self {
            mask: AssetHandle::default(),
            space: LayerSpace::default(),
            weight: 1.0,
            weight_param: None,
        }
    }
}

impl LayerNode {
    /// A layer over the bones of `mask`, blended in mesh space at full weight.
    pub fn new(mask: AssetHandle<BoneMask>) -> Self {
        Self {
            mask,
            ..Self::default()
        }
    }

    /// Sets where the layer blends rotations.
    pub fn with_space(mut self, space: LayerSpace) -> Self {
        self.space = space;
        self
    }

    /// Sets the layer weight, before the param scales it.
    pub fn with_weight(mut self, weight: f32) -> Self {
        self.weight = weight;
        self
    }

    /// Multiplies the layer weight by the `f32` blackboard param named `param`, when it holds one.
    pub fn with_weight_param(mut self, param: Name) -> Self {
        self.weight_param = Some(param);
        self
    }

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
}

/// A [`LayerNode`]'s resolved mask, its model-space buffers, and which warnings it has given.
#[derive(Default)]
pub struct LayerState {
    mask: MaskResolution,
    base_model: Vec<Quat>,
    overlay_model: Vec<Quat>,
    output_model: Vec<Quat>,
    warned_missing: bool,
    warned_root: bool,
    warned_inputs: bool,
}

impl AnimationNode for LayerNode {
    type State = LayerState;
    type CreateInput = ();
    type UpdateInput = ();
    type EvaluateInput = Res<'static, AssetStore<BoneMask>>;

    fn create_state(&self, _: &(), _context: &AnimationGraphContext<'_>) -> LayerState {
        LayerState::default()
    }

    fn update(&self, _: &mut LayerState, _: f32, _: &(), _: &AnimationGraphContext<'_>) {}

    fn evaluate(
        &self,
        state: &mut LayerState,
        masks: &Res<AssetStore<BoneMask>>,
        context: &AnimationGraphContext<'_>,
        _: &[Uuid],
        evaluated_inputs: &[EvaluatedFrame],
        _: &mut FramePool,
        output: &mut AnimationFrame,
    ) {
        if evaluated_inputs.len() > 2 && !state.warned_inputs {
            warn!(
                "a layer node has {} inputs and ignores all after the second",
                evaluated_inputs.len()
            );
            state.warned_inputs = true;
        }
        let Some(base) = evaluated_inputs.first() else {
            return;
        };
        output.copy_from(&base.frame);
        let Some(overlay) = evaluated_inputs.get(1) else {
            return;
        };
        let weight = self.effective_weight(context.blackboard());
        if weight <= 0.0 {
            return;
        }
        for event in overlay.frame.events() {
            output.push_event(FiredEvent {
                weight: event.weight * weight,
                ..*event
            });
        }

        let Some(mask) = masks.get(&self.mask) else {
            return;
        };
        let Some(skeleton) = context.skeleton() else {
            return;
        };
        resolve_mask(
            mask,
            skeleton,
            context.root().map(|root| root.index),
            &mut state.mask,
        );
        if !state.mask.missing.is_empty() && !state.warned_missing {
            let names: Vec<&str> = state.mask.missing.iter().map(|name| &**name).collect();
            warn!(
                "layer mask {:?} names bones the skeleton lacks: {}",
                self.mask.id(),
                names.join(", ")
            );
            state.warned_missing = true;
        }
        if state.mask.names_root && !state.warned_root {
            warn!(
                "layer mask {:?} names the root bone, which keeps the base's joint",
                self.mask.id()
            );
            state.warned_root = true;
        }

        match self.space {
            LayerSpace::Local => blend_local(&state.mask.weights, weight, &overlay.frame, output),
            LayerSpace::Mesh => {
                blend_mesh(state, skeleton, weight, &base.frame, &overlay.frame, output)
            }
        }
    }

    fn reset(&self, _: &mut LayerState) {}
}

fn blend_joint(joint: &mut JointPose, target: &JointPose, weight: f32) {
    joint.translation = joint.translation.lerp(target.translation, weight);
    joint.rotation = joint.rotation.slerp(target.rotation, weight);
    joint.scale = joint.scale.lerp(target.scale, weight);
}

fn blend_local(weights: &[f32], layer: f32, overlay: &AnimationFrame, output: &mut AnimationFrame) {
    for (bone, mask_weight) in weights.iter().enumerate() {
        let weight = mask_weight * layer;
        if weight <= 0.0 {
            continue;
        }
        let Some(target) = overlay.joint(bone) else {
            continue;
        };
        let Some(joint) = output.joint_mut(bone) else {
            continue;
        };
        blend_joint(joint, target, weight);
    }
}

fn parent_below(skeleton: &Skeleton, bone: usize) -> Option<usize> {
    parent(skeleton, bone).filter(|parent| *parent < bone)
}

fn blend_mesh(
    state: &mut LayerState,
    skeleton: &Skeleton,
    layer: f32,
    base: &AnimationFrame,
    overlay: &AnimationFrame,
    output: &mut AnimationFrame,
) {
    let bones = state
        .mask
        .weights
        .len()
        .min(base.len())
        .min(overlay.len())
        .min(output.len());
    state.base_model.clear();
    state.overlay_model.clear();
    state.output_model.clear();
    for bone in 0..bones {
        let up = parent_below(skeleton, bone);
        let above = |models: &[Quat]| up.map_or(Quat::IDENTITY, |up| models[up]);
        let base_model = above(&state.base_model) * base.joints()[bone].rotation;
        let overlay_model = above(&state.overlay_model) * overlay.joints()[bone].rotation;
        let output_parent = above(&state.output_model);
        let weight = state.mask.weights[bone] * layer;
        let output_model = match output.joint_mut(bone) {
            Some(joint) if weight > 0.0 => {
                let target = base_model.slerp(overlay_model, weight);
                let overlay_joint = &overlay.joints()[bone];
                joint.translation = joint.translation.lerp(overlay_joint.translation, weight);
                joint.scale = joint.scale.lerp(overlay_joint.scale, weight);
                joint.rotation = (output_parent.inverse() * target).normalize();
                target
            }
            Some(joint) => output_parent * joint.rotation,
            None => output_parent,
        };
        state.base_model.push(base_model);
        state.overlay_model.push(overlay_model);
        state.output_model.push(output_model);
    }
}

#[cfg(test)]
mod tests {
    use concerto_foundation::assets::{AssetId, asset_store::AssetStore, handle::AssetHandle};
    use concerto_mesh::Skeleton;
    use concerto_name::{Name, name};
    use glam::{Mat4, Quat, Vec3};
    use uuid::Uuid;

    use super::{LayerNode, LayerSpace, LayerState};
    use crate::{
        blackboard::AnimationBlackboard,
        evaluation::RootBone,
        mask::{BoneMask, BoneMaskBranch},
        node::{ErasedAnimationNode, NodeState, model_space::parent},
        pose::{
            AnimationFrame, EvaluatedFrame, FiredEvent, FramePool, JointPose, Pose, RootMotion,
        },
        test_walks::Walks,
    };

    const NAMES: [&str; 10] = [
        "root",
        "spine1",
        "spine2",
        "spine3",
        "left_upper",
        "left_lower",
        "left_hand",
        "right_upper",
        "right_lower",
        "right_hand",
    ];
    const BONES: usize = NAMES.len();
    const LEFT_ARM: [usize; 3] = [4, 5, 6];

    fn joint(translation: Vec3, rotation: Quat, scale: Vec3) -> JointPose {
        JointPose {
            translation,
            rotation,
            scale,
        }
    }

    fn skeleton() -> Skeleton {
        Skeleton::new(
            vec![Mat4::IDENTITY; BONES],
            vec![
                None,
                Some(0),
                Some(1),
                Some(2),
                Some(3),
                Some(4),
                Some(5),
                Some(3),
                Some(7),
                Some(8),
            ],
            NAMES.iter().map(|name| Name::new(name)).collect(),
            vec![joint(Vec3::Y, Quat::IDENTITY, Vec3::ONE); BONES],
            (0..BONES as u128).map(Uuid::from_u128).collect(),
        )
        .unwrap()
    }

    fn base_joints() -> Vec<JointPose> {
        (0..BONES)
            .map(|bone| {
                let at = bone as f32;
                joint(
                    Vec3::new(0.0, 0.1 * at, 0.0),
                    Quat::from_rotation_x(0.1 * at),
                    Vec3::ONE,
                )
            })
            .collect()
    }

    fn overlay_joints() -> Vec<JointPose> {
        (0..BONES)
            .map(|bone| {
                let at = bone as f32;
                joint(
                    Vec3::new(0.2 * at, 0.0, 0.1),
                    Quat::from_rotation_z(0.3 + 0.1 * at),
                    Vec3::splat(1.5),
                )
            })
            .collect()
    }

    fn base_motion() -> RootMotion {
        RootMotion {
            translation: Vec3::new(0.0, 0.0, 0.2),
            rotation: Quat::from_rotation_y(0.1),
        }
    }

    fn overlay_motion() -> RootMotion {
        RootMotion {
            translation: Vec3::new(0.4, 0.0, 0.0),
            rotation: Quat::from_rotation_y(-0.3),
        }
    }

    fn base_event() -> FiredEvent {
        FiredEvent {
            clip: AssetId::from_path("layer_base"),
            key: 1,
            time: 0.2,
            weight: 1.0,
            mirrored: false,
        }
    }

    fn overlay_event() -> FiredEvent {
        FiredEvent {
            clip: AssetId::from_path("layer_overlay"),
            key: 2,
            time: 0.5,
            weight: 0.8,
            mirrored: false,
        }
    }

    fn frame(joints: Vec<JointPose>, motion: RootMotion, event: FiredEvent) -> AnimationFrame {
        let mut frame = AnimationFrame::new(Pose::from_joints(joints));
        frame.set_root_motion(motion);
        frame.push_event(event);
        frame
    }

    fn base() -> AnimationFrame {
        frame(base_joints(), base_motion(), base_event())
    }

    fn overlay() -> AnimationFrame {
        frame(overlay_joints(), overlay_motion(), overlay_event())
    }

    fn weighted(frame: AnimationFrame, weight: f32) -> EvaluatedFrame {
        EvaluatedFrame { frame, weight }
    }

    fn pair(base: AnimationFrame, overlay: AnimationFrame) -> Vec<EvaluatedFrame> {
        vec![weighted(base, 1.0), weighted(overlay, 1.0)]
    }

    fn layered() -> Vec<EvaluatedFrame> {
        pair(base(), overlay())
    }

    fn fresh_state() -> NodeState {
        Box::new(LayerState::default())
    }

    fn typed(state: &NodeState) -> &LayerState {
        state
            .downcast_ref::<LayerState>()
            .expect("a layer node's state is a LayerState")
    }

    struct Rig {
        walks: Walks,
        blackboard: AnimationBlackboard,
        skeleton: Skeleton,
        root: Option<RootBone>,
        with_skeleton: bool,
    }

    impl Rig {
        fn new() -> Self {
            Self {
                walks: Walks::new(AssetStore::new(), AssetStore::new()),
                blackboard: AnimationBlackboard::default(),
                skeleton: skeleton(),
                root: None,
                with_skeleton: true,
            }
        }

        fn add_mask(&mut self, path: &str, mask: BoneMask) -> AssetHandle<BoneMask> {
            let id = AssetId::from_path(path);
            self.walks.masks_mut().insert(id, mask);
            AssetHandle::weak(id)
        }

        fn left_arm(&mut self) -> AssetHandle<BoneMask> {
            self.add_mask(
                "left_arm",
                BoneMask::new().with_branch(BoneMaskBranch::new("left_upper")),
            )
        }

        fn layer(&self, node: &LayerNode, inputs: &[EvaluatedFrame]) -> AnimationFrame {
            self.layer_with(node, &mut fresh_state(), inputs)
        }

        fn layer_with(
            &self,
            node: &LayerNode,
            state: &mut NodeState,
            inputs: &[EvaluatedFrame],
        ) -> AnimationFrame {
            let erased: &dyn ErasedAnimationNode = node;
            let mut pool = FramePool::new(BONES);
            pool.set_rest(&self.skeleton.rest_pose);
            let mut output = pool.acquire();
            let skeleton = self.with_skeleton.then_some(&self.skeleton);
            self.walks
                .evaluate_with(&self.blackboard, self.root.clone(), skeleton, |context| {
                    erased.evaluate(state, context, &[], inputs, &mut pool, &mut output)
                });
            output
        }

        fn model_rotation(&self, frame: &AnimationFrame, bone: usize) -> Quat {
            let mut rotation = frame.joints()[bone].rotation;
            let mut at = bone;
            while let Some(up) = parent(&self.skeleton, at) {
                rotation = frame.joints()[up].rotation * rotation;
                at = up;
            }
            rotation
        }
    }

    fn root_bone() -> RootBone {
        RootBone {
            index: 0,
            rest: JointPose::IDENTITY,
            up: Vec3::Y,
        }
    }

    fn assert_rotation_near(actual: Quat, expected: Quat) {
        assert!(
            actual.abs_diff_eq(expected, 1e-5) || actual.abs_diff_eq(-expected, 1e-5),
            "expected rotation {expected}, got {actual}"
        );
    }

    fn assert_joint_near(actual: &JointPose, expected: &JointPose) {
        assert!(
            actual.translation.abs_diff_eq(expected.translation, 1e-5),
            "expected translation {}, got {}",
            expected.translation,
            actual.translation
        );
        assert!(
            actual.scale.abs_diff_eq(expected.scale, 1e-5),
            "expected scale {}, got {}",
            expected.scale,
            actual.scale
        );
        assert_rotation_near(actual.rotation, expected.rotation);
    }

    fn blended(bone: usize, weight: f32) -> JointPose {
        let from = &base_joints()[bone];
        let to = &overlay_joints()[bone];
        joint(
            from.translation.lerp(to.translation, weight),
            from.rotation.slerp(to.rotation, weight),
            from.scale.lerp(to.scale, weight),
        )
    }

    fn assert_base_pose(frame: &AnimationFrame) {
        assert_eq!(frame.joints(), base_joints());
        assert_eq!(frame.root_motion(), base_motion());
    }

    fn local(mask: AssetHandle<BoneMask>) -> LayerNode {
        LayerNode::new(mask).with_space(LayerSpace::Local)
    }

    #[test]
    fn local_at_full_weight_lays_the_overlay_on_the_masked_arm_alone() {
        let mut rig = Rig::new();
        let node = local(rig.left_arm());

        let frame = rig.layer(&node, &layered());

        let overlay = overlay_joints();
        let base = base_joints();
        for bone in 0..BONES {
            if LEFT_ARM.contains(&bone) {
                assert_joint_near(&frame.joints()[bone], &overlay[bone]);
            } else {
                assert_eq!(frame.joints()[bone], base[bone], "bone {bone}");
            }
        }
    }

    #[test]
    fn half_layer_weight_puts_a_masked_joint_halfway() {
        let mut rig = Rig::new();
        let node = local(rig.left_arm()).with_weight(0.5);

        let frame = rig.layer(&node, &layered());

        for bone in LEFT_ARM {
            assert_joint_near(&frame.joints()[bone], &blended(bone, 0.5));
        }
    }

    struct ParamCase {
        param: Name,
        weight: f32,
    }

    impl ParamCase {
        fn new(param: Name, weight: f32) -> Self {
            Self { param, weight }
        }
    }

    #[test]
    fn the_layer_weight_is_the_weight_times_the_param_and_clamps_above_1() {
        let mut rig = Rig::new();
        let mask = rig.left_arm();
        rig.blackboard.set(name!("aim"), 0.5_f32);
        rig.blackboard.set(name!("boost"), 3.0_f32);
        rig.blackboard.set(name!("flag"), true);
        let node = local(mask).with_weight(0.8);
        let cases = [
            ParamCase::new(name!("aim"), 0.8 * 0.5),
            ParamCase::new(name!("unset"), 0.8),
            ParamCase::new(name!("flag"), 0.8),
            ParamCase::new(name!("boost"), 1.0),
        ];

        for case in cases {
            let frame = rig.layer(&node.clone().with_weight_param(case.param), &layered());

            assert_joint_near(&frame.joints()[4], &blended(4, case.weight));
        }
    }

    #[test]
    fn mesh_keeps_the_overlay_model_rotation_of_the_masked_arm_over_a_bent_spine() {
        let mut rig = Rig::new();
        let node = LayerNode::new(rig.left_arm());
        let mut base_joints = vec![joint(Vec3::Y, Quat::IDENTITY, Vec3::ONE); BONES];
        base_joints[1].rotation = Quat::from_rotation_x(40f32.to_radians());
        let mut overlay_joints = vec![joint(Vec3::Y, Quat::IDENTITY, Vec3::ONE); BONES];
        overlay_joints[4].rotation = Quat::from_rotation_z(0.5);
        overlay_joints[5].rotation = Quat::from_rotation_z(0.3);
        overlay_joints[6].rotation = Quat::from_rotation_y(0.2);
        overlay_joints[7].rotation = Quat::from_rotation_x(0.7);
        let base = frame(base_joints.clone(), base_motion(), base_event());
        let overlay = frame(overlay_joints, overlay_motion(), overlay_event());
        let overlay_models: Vec<Quat> = LEFT_ARM
            .iter()
            .map(|bone| rig.model_rotation(&overlay, *bone))
            .collect();

        let frame = rig.layer(&node, &pair(base, overlay));

        for (bone, expected) in LEFT_ARM.iter().zip(overlay_models) {
            assert_rotation_near(rig.model_rotation(&frame, *bone), expected);
        }
        for bone in (0..BONES).filter(|bone| !LEFT_ARM.contains(bone)) {
            assert_eq!(
                frame.joints()[bone].rotation,
                base_joints[bone].rotation,
                "bone {bone}"
            );
        }
    }

    #[test]
    fn mesh_and_local_agree_when_the_branch_ancestors_match() {
        let mut rig = Rig::new();
        let mask = rig.left_arm();
        let mut overlay_joints = overlay_joints();
        for (bone, joint) in base_joints().iter().enumerate().take(4) {
            overlay_joints[bone].rotation = joint.rotation;
        }
        let inputs = || {
            pair(
                base(),
                frame(overlay_joints.clone(), overlay_motion(), overlay_event()),
            )
        };

        let in_mesh = rig.layer(&LayerNode::new(mask.clone()), &inputs());
        let in_local = rig.layer(&local(mask), &inputs());

        for bone in 0..BONES {
            assert_joint_near(&in_mesh.joints()[bone], &in_local.joints()[bone]);
        }
    }

    #[test]
    fn root_motion_and_the_root_joint_are_the_base_even_under_a_mask_naming_the_root() {
        let mut rig = Rig::new();
        rig.root = Some(root_bone());
        let mask = rig.add_mask(
            "everything",
            BoneMask::new().with_branch(BoneMaskBranch::new("root")),
        );

        for space in [LayerSpace::Local, LayerSpace::Mesh] {
            let mut state = fresh_state();
            let node = LayerNode::new(mask.clone()).with_space(space);

            let frame = rig.layer_with(&node, &mut state, &layered());

            assert_eq!(frame.root_motion(), base_motion());
            assert_eq!(frame.joints()[0], base_joints()[0]);
            assert!(typed(&state).warned_root);
            if space == LayerSpace::Local {
                assert_joint_near(&frame.joints()[1], &overlay_joints()[1]);
            }
        }
    }

    #[test]
    fn overlay_events_are_scaled_by_the_layer_weight_whatever_the_mask() {
        let mut rig = Rig::new();
        let faint = rig.add_mask(
            "faint",
            BoneMask::new().with_branch(BoneMaskBranch::new("left_upper").with_weight(0.25)),
        );
        let unloaded = AssetHandle::weak(AssetId::from_path("unloaded"));
        let expected = [
            base_event(),
            FiredEvent {
                weight: overlay_event().weight * 0.5,
                ..overlay_event()
            },
        ];

        for mask in [faint.clone(), unloaded, AssetHandle::default()] {
            let frame = rig.layer(&local(mask).with_weight(0.5), &layered());
            assert_eq!(frame.events(), expected);
        }
        rig.with_skeleton = false;
        let frame = rig.layer(&local(faint).with_weight(0.5), &layered());
        assert_eq!(frame.events(), expected);
    }

    #[test]
    fn a_nan_layer_weight_gives_the_base_and_infinity_gives_the_overlay_on_masked_bones() {
        let mut rig = Rig::new();
        let mask = rig.left_arm();

        let nan = rig.layer(&local(mask.clone()).with_weight(f32::NAN), &layered());
        let infinite = rig.layer(&local(mask).with_weight(f32::INFINITY), &layered());

        assert_base_pose(&nan);
        assert_eq!(nan.events(), [base_event()]);
        for bone in LEFT_ARM {
            assert_joint_near(&infinite.joints()[bone], &overlay_joints()[bone]);
        }
    }

    #[test]
    fn the_node_weights_of_its_inputs_do_not_change_the_result() {
        let mut rig = Rig::new();
        let node = LayerNode::new(rig.left_arm()).with_weight(0.7);

        let even = rig.layer(&node, &layered());
        let uneven = rig.layer(&node, &[weighted(base(), 0.0), weighted(overlay(), 0.25)]);

        assert_eq!(uneven.joints(), even.joints());
        assert_eq!(uneven.root_motion(), even.root_motion());
        assert_eq!(uneven.events(), even.events());
    }

    #[test]
    fn an_unusable_mask_no_skeleton_or_no_overlay_passes_the_base_through() {
        let mut rig = Rig::new();
        let mask = rig.left_arm();
        let unloaded = AssetHandle::weak(AssetId::from_path("unloaded"));

        assert_base_pose(&rig.layer(&LayerNode::default(), &layered()));
        assert_base_pose(&rig.layer(&LayerNode::new(unloaded), &layered()));
        assert_base_pose(&rig.layer(&LayerNode::new(mask.clone()), &[weighted(base(), 1.0)]));
        rig.with_skeleton = false;
        assert_base_pose(&rig.layer(&LayerNode::new(mask), &layered()));
    }

    #[test]
    fn no_input_gives_the_rest_pose() {
        let mut rig = Rig::new();
        let node = LayerNode::new(rig.left_arm());

        let frame = rig.layer(&node, &[]);

        assert_eq!(frame.joints(), &*rig.skeleton.rest_pose);
    }

    #[test]
    fn a_third_input_warns_once_and_is_ignored() {
        let mut rig = Rig::new();
        let node = local(rig.left_arm()).with_weight(0.6);
        let third = frame(
            vec![joint(Vec3::X, Quat::from_rotation_y(1.0), Vec3::splat(2.0)); BONES],
            overlay_motion(),
            overlay_event(),
        );
        let mut inputs = layered();
        inputs.push(weighted(third, 1.0));
        let mut state = fresh_state();

        let first = rig.layer_with(&node, &mut state, &inputs);
        assert!(typed(&state).warned_inputs);
        let second = rig.layer_with(&node, &mut state, &inputs);

        let expected = rig.layer(&node, &layered());
        for frame in [first, second] {
            assert_eq!(frame.joints(), expected.joints());
            assert_eq!(frame.events(), expected.events());
        }
        assert!(typed(&state).warned_inputs);
        assert!(!typed(&state).warned_missing);
        assert!(!typed(&state).warned_root);
    }

    #[test]
    fn a_missing_bone_sets_its_own_warning_flag_alone() {
        let mut rig = Rig::new();
        let mask = rig.add_mask(
            "tailed",
            BoneMask::new()
                .with_branch(BoneMaskBranch::new("tail"))
                .with_branch(BoneMaskBranch::new("left_upper")),
        );
        let mut state = fresh_state();

        let frame = rig.layer_with(&local(mask), &mut state, &layered());

        assert!(typed(&state).warned_missing);
        assert!(!typed(&state).warned_root);
        assert!(!typed(&state).warned_inputs);
        assert_joint_near(&frame.joints()[4], &overlay_joints()[4]);
    }
}

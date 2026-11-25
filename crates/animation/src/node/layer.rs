use concerto_ecs::Res;
use concerto_foundation::assets::{asset_store::AssetStore, handle::AssetHandle};
use concerto_mesh::Skeleton;
use concerto_name::name;
use concerto_reflect::{Reflect, ReflectDefault};
use glam::Quat;
use log::warn;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    evaluation::AnimationGraphContext,
    mask::{BoneMask, MaskResolution, resolve_mask},
    node::{
        AnimationNode, InputSlots, ReflectAnimationNode, effective_weight, model_space::parent,
    },
    param::Param,
    pose::{AnimationFrame, EvaluatedFrame, FiredEvent, FramePool, JointPose},
};

/// Lays its second input over its first on the bones of a mask, by the mask's weights times the layer weight.
#[derive(Clone, Debug, PartialEq, Reflect, Serialize, Deserialize)]
#[reflect(AnimationNode, Default)]
pub struct LayerNode {
    pub mask: AssetHandle<BoneMask>,
    pub space: LayerSpace,
    pub weight: f32,
    pub weight_param: Option<Param<f32>>,
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
    pub fn with_weight_param(mut self, param: impl Into<Param<f32>>) -> Self {
        self.weight_param = Some(param.into());
        self
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

    fn input_slots(&self) -> InputSlots {
        InputSlots::Fixed(vec![name!("base"), name!("overlay")])
    }

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
        let weight = effective_weight(self.weight, self.weight_param, context.blackboard());
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
        blend_attributes(&state.mask.weights, weight, &overlay.frame, output);
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

fn blend_attributes(
    weights: &[f32],
    layer: f32,
    overlay: &AnimationFrame,
    output: &mut AnimationFrame,
) {
    for (bone, mask_weight) in weights.iter().enumerate() {
        let weight = mask_weight * layer;
        if weight > 0.0 {
            output
                .attributes_mut()
                .blend_bone(overlay.attributes(), bone, weight);
        }
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
    use concerto_name::Name;
    use glam::{Mat4, Quat, Vec3};
    use uuid::Uuid;

    use super::{LayerNode, LayerSpace, LayerState};
    use crate::{
        blackboard::AnimationBlackboard,
        mask::{BoneMask, BoneMaskBranch},
        node::{ErasedAnimationNode, NodeState, model_space::parent},
        pose::{AnimationFrame, EvaluatedFrame, FiredEvent, FramePool, JointPose, Pose},
        testing::Walks,
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

    fn frame(joints: Vec<JointPose>, event: FiredEvent) -> AnimationFrame {
        let mut frame = AnimationFrame::new(Pose::from_joints(joints));
        frame.push_event(event);
        frame
    }

    fn base() -> AnimationFrame {
        frame(base_joints(), base_event())
    }

    fn overlay() -> AnimationFrame {
        frame(overlay_joints(), overlay_event())
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

    struct Rig {
        walks: Walks,
        blackboard: AnimationBlackboard,
        skeleton: Skeleton,
    }

    impl Rig {
        fn new() -> Self {
            Self {
                walks: Walks::new(AssetStore::new(), AssetStore::new()),
                blackboard: AnimationBlackboard::default(),
                skeleton: skeleton(),
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
            let erased: &dyn ErasedAnimationNode = node;
            let mut state: NodeState = Box::<LayerState>::default();
            let mut pool = FramePool::new(BONES);
            pool.set_rest(&self.skeleton.rest_pose);
            let mut output = pool.acquire();
            self.walks
                .evaluate_with(&self.blackboard, None, Some(&self.skeleton), |context| {
                    erased.evaluate(&mut state, context, &[], inputs, &mut pool, &mut output)
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

    fn local(mask: AssetHandle<BoneMask>) -> LayerNode {
        LayerNode::new(mask).with_space(LayerSpace::Local)
    }

    #[test]
    fn the_masked_arm_moves_by_the_layer_weight_and_the_rest_keeps_the_base() {
        let mut rig = Rig::new();
        let node = local(rig.left_arm()).with_weight(0.5);

        let frame = rig.layer(&node, &layered());

        for bone in 0..BONES {
            if LEFT_ARM.contains(&bone) {
                assert_joint_near(&frame.joints()[bone], &blended(bone, 0.5));
            } else {
                assert_eq!(frame.joints()[bone], base_joints()[bone], "bone {bone}");
            }
        }
        assert_eq!(
            frame.events(),
            [
                base_event(),
                FiredEvent {
                    weight: overlay_event().weight * 0.5,
                    ..overlay_event()
                }
            ]
        );
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
        let base = frame(base_joints.clone(), base_event());
        let overlay = frame(overlay_joints, overlay_event());
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
}

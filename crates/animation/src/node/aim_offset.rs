use std::f32::consts::{PI, TAU};

use concerto_ecs::Res;
use concerto_foundation::{
    assets::{asset_store::AssetStore, handle::AssetHandle},
    geometry::delauney::Triangulation2D,
    math::{Spring, SpringValue},
};
use concerto_mesh::{JointName, Skeleton};
use concerto_name::name;
use concerto_reflect::{Reflect, ReflectDefault};
use glam::{Vec2, Vec3};
use log::warn;
use uuid::Uuid;

use crate::{
    blackboard::AnimationBlackboard,
    clip::{Additive, AnimationClip},
    evaluation::AnimationGraphContext,
    look_at::{LookFrame, yaw_pitch_to},
    node::{
        AnimationNode, InputSlots, ReflectAnimationNode,
        model_space::{WorldToModel, bone_under_first, model_joint},
        pass_through,
    },
    param::Param,
    pose::{AnimationFrame, EvaluatedFrame, FramePool, JointPose, apply_additive},
};

const DUPLICATE_TOLERANCE: f32 = 1e-4;
const LINE_TOLERANCE: f32 = 1e-3;
const SNAP_WEIGHT: f32 = 1.0e-4;

/// Layers additive aim clips, chosen by the target's yaw and pitch from a chosen bone, over its first input.
#[derive(Clone, Debug, PartialEq, Reflect, serde::Serialize, serde::Deserialize)]
#[reflect(AnimationNode, Default)]
pub struct AimOffsetNode {
    /// The additive aim clips, each at the yaw and pitch it aims at.
    pub points: Vec<AimPoint>,
    /// The bone the target's yaw and pitch are measured from.
    pub frame_bone: JointName,
    /// The model-space axis the frame bone aims along in its rest pose.
    pub forward: Vec3,
    /// The model-space up of the frame bone in its rest pose.
    pub up: Vec3,
    /// The `Vec3` blackboard param holding the world-space point to aim at.
    pub target_param: Param<Vec3>,
    /// The weight the aim is layered at, before the param scales it; clamped to 0 to 1.
    pub weight: f32,
    /// The `f32` blackboard param the weight is multiplied by, when it holds one.
    pub weight_param: Option<Param<f32>>,
    /// The halflife, in seconds, the weight eases in and out with.
    pub weight_halflife: f32,
    /// The halflife, in seconds, the yaw and pitch ease toward the target with; 0 snaps.
    pub angle_halflife: f32,
}

impl Default for AimOffsetNode {
    fn default() -> Self {
        Self {
            points: Vec::new(),
            frame_bone: JointName::default(),
            forward: Vec3::Z,
            up: Vec3::Y,
            target_param: Param::default(),
            weight: 1.0,
            weight_param: None,
            weight_halflife: 0.2,
            angle_halflife: 0.1,
        }
    }
}

/// An additive aim clip and the yaw and pitch, in radians, it aims at.
#[derive(Clone, Debug, Default, PartialEq, Reflect, serde::Serialize, serde::Deserialize)]
pub struct AimPoint {
    /// The additive clip, which must be imported as additive.
    pub clip: AssetHandle<AnimationClip>,
    /// The yaw it aims at, in radians; positive turns toward the frame's side, `up × forward`.
    pub yaw: f32,
    /// The pitch it aims at, in radians; positive turns toward the frame's up.
    pub pitch: f32,
}

impl AimPoint {
    /// The additive `clip`, aiming at `yaw` and `pitch` in radians.
    pub fn new(clip: AssetHandle<AnimationClip>, yaw: f32, pitch: f32) -> Self {
        Self { clip, yaw, pitch }
    }
}

impl AimOffsetNode {
    /// A node with the default settings and no points, aiming at the world `Vec3` under `target_param` as seen from `frame_bone`.
    pub fn new(target_param: impl Into<Param<Vec3>>, frame_bone: impl Into<JointName>) -> Self {
        Self {
            target_param: target_param.into(),
            frame_bone: frame_bone.into(),
            ..Self::default()
        }
    }

    /// Adds an aim point.
    pub fn with_point(mut self, point: AimPoint) -> Self {
        self.points.push(point);
        self
    }

    /// Sets the model-space axis the frame bone aims along in its rest pose.
    pub fn with_forward(mut self, forward: Vec3) -> Self {
        self.forward = forward;
        self
    }

    /// Sets the model-space up of the frame bone in its rest pose.
    pub fn with_up(mut self, up: Vec3) -> Self {
        self.up = up;
        self
    }

    /// Sets the weight, before the param scales it.
    pub fn with_weight(mut self, weight: f32) -> Self {
        self.weight = weight;
        self
    }

    /// Multiplies the weight by the `f32` blackboard param named `param`, when it holds one.
    pub fn with_weight_param(mut self, param: impl Into<Param<f32>>) -> Self {
        self.weight_param = Some(param.into());
        self
    }

    /// Sets the halflife, in seconds, the weight eases in and out with.
    pub fn with_weight_halflife(mut self, weight_halflife: f32) -> Self {
        self.weight_halflife = weight_halflife;
        self
    }

    /// Sets the halflife, in seconds, the yaw and pitch ease toward the target with; 0 snaps.
    pub fn with_angle_halflife(mut self, angle_halflife: f32) -> Self {
        self.angle_halflife = angle_halflife;
        self
    }

    fn sample_points(&self) -> Vec<Vec2> {
        self.points
            .iter()
            .map(|point| Vec2::new(point.yaw, point.pitch))
            .collect()
    }

    fn effective_weight(&self, blackboard: &AnimationBlackboard) -> f32 {
        let scale = self
            .weight_param
            .and_then(|param| blackboard.read(&param).copied())
            .unwrap_or(1.0);
        let weight = self.weight * scale;
        if weight.is_nan() {
            0.0
        } else {
            weight.clamp(0.0, 1.0)
        }
    }

    fn resolve<'a>(&self, skeleton: Option<&'a Skeleton>) -> Result<AimFrame<'a>, String> {
        let skeleton = skeleton.ok_or("the player has no skeleton")?;
        let bone = bone_under_first(skeleton, self.frame_bone)?;
        let rest = model_joint(skeleton, &skeleton.rest_pose, bone)
            .ok_or("the skeleton's rest pose is missing a bone")?;
        let inverse = rest.rotation.inverse();
        let frame = LookFrame::new(inverse * self.forward, inverse * self.up)
            .ok_or("`forward` or `up` is zero or not finite, or they are parallel")?;
        Ok(AimFrame {
            skeleton,
            bone,
            frame,
        })
    }

    fn target(&self, context: &AnimationGraphContext<'_>) -> Option<Vec3> {
        let target = *context.blackboard().read(&self.target_param)?;
        let world_to_model = WorldToModel::new(context.model_to_world().unwrap_or_default());
        if !world_to_model.is_finite() {
            return None;
        }
        let target = world_to_model.transform.transform_point3(target);
        target.is_finite().then_some(target)
    }

    fn step_clock(
        &self,
        state: &mut AimOffsetState,
        clips: &AssetStore<AnimationClip>,
        delta_time: f32,
    ) {
        let longest = self
            .points
            .iter()
            .filter_map(|point| clips.get(&point.clip))
            .map(AnimationClip::additive_duration)
            .fold(0.0, f32::max);
        state.clock = if longest > 0.0 {
            (state.clock + delta_time).rem_euclid(longest)
        } else {
            0.0
        };
    }

    fn warn_plain_clips(&self, state: &mut AimOffsetState, clips: &AssetStore<AnimationClip>) {
        if state.warned_clip {
            return;
        }
        let plain = self.points.iter().find(|point| {
            clips
                .get(&point.clip)
                .is_some_and(|clip| matches!(clip.additive(), Additive::None))
        });
        if let Some(point) = plain {
            warn!(
                "aim offset toward `{}` plays clip {:?} as no change: it is not additive",
                self.target_param.0.as_str(),
                point.clip.id()
            );
            state.warned_clip = true;
        }
    }

    fn combined_delta(
        &self,
        state: &AimOffsetState,
        clips: &AssetStore<AnimationClip>,
        bone: &Uuid,
    ) -> Option<JointPose> {
        let mut combined: Option<JointPose> = None;
        let mut weight_so_far = 0.0;
        for (point, &weight) in self.points.iter().zip(&state.point_weights) {
            if weight.is_nan() || weight <= 0.0 {
                continue;
            }
            let delta = clips.get(&point.clip).map_or(JointPose::IDENTITY, |clip| {
                clip.sample_additive(sample_time(state.clock, clip.additive_duration()), bone)
            });
            weight_so_far += weight;
            combined = Some(match combined {
                None => JointPose {
                    translation: delta.translation * weight,
                    ..delta
                },
                Some(mut sum) => {
                    sum.rotation = sum.rotation.slerp(delta.rotation, weight / weight_so_far);
                    sum.translation += delta.translation * weight;
                    sum
                }
            });
        }
        combined
    }
}

struct AimFrame<'a> {
    skeleton: &'a Skeleton,
    bone: usize,
    frame: LookFrame,
}

impl AimFrame<'_> {
    fn measure(&self, joints: &[JointPose], target: Vec3) -> Option<Vec2> {
        let joint = model_joint(self.skeleton, joints, self.bone)?;
        let direction =
            joint.rotation.inverse() * (target - Vec3::from(joint.transform.translation));
        if direction.length_squared() <= 0.0 {
            return None;
        }
        let angles = yaw_pitch_to(&self.frame, direction);
        let angles = Vec2::new(angles.yaw, angles.pitch);
        angles.is_finite().then_some(angles)
    }
}

/// The time an [`AimOffsetNode`] owes since its last evaluate, its eased weight and aim angles, its clock, its point weights, and which warnings it has given.
pub struct AimOffsetState {
    elapsed: f32,
    fresh: bool,
    snap_angles: bool,
    clock: f32,
    weight: Spring<f32>,
    angles: Spring<Vec2>,
    weighting: AimWeights,
    point_weights: Vec<f32>,
    warned: bool,
    warned_clip: bool,
}

impl AimOffsetState {
    fn take_time(&mut self) -> f32 {
        let elapsed = std::mem::take(&mut self.elapsed);
        if std::mem::take(&mut self.fresh) {
            0.0
        } else {
            elapsed
        }
    }

    fn ease_angles(&mut self, goal: Vec2, halflife: f32, delta_time: f32) {
        if std::mem::take(&mut self.snap_angles) {
            self.angles.reset_to(goal);
            return;
        }
        let eased = self.angles.value;
        let goal = Vec2::new(eased.x + wrapped(goal.x - eased.x), goal.y);
        ease(&mut self.angles, goal, halflife, delta_time);
        if self.angles.value.x.abs() > PI {
            self.angles.value.x = wrapped(self.angles.value.x);
        }
    }
}

fn halflife(halflife: f32) -> f32 {
    if halflife.is_finite() && halflife > 0.0 {
        halflife
    } else {
        0.0
    }
}

fn ease<T: SpringValue>(spring: &mut Spring<T>, goal: T, halflife: f32, delta_time: f32) -> T {
    if halflife > 0.0 {
        spring.set_halflife(halflife);
        spring.update(goal, delta_time)
    } else {
        spring.reset_to(goal);
        goal
    }
}

fn wrapped(angle: f32) -> f32 {
    (angle + PI).rem_euclid(TAU) - PI
}

fn sample_time(clock: f32, duration: f32) -> f32 {
    if duration > 0.0 {
        clock.rem_euclid(duration)
    } else {
        0.0
    }
}

impl AnimationNode for AimOffsetNode {
    type State = AimOffsetState;
    type CreateInput = ();
    type UpdateInput = ();
    type EvaluateInput = Res<'static, AssetStore<AnimationClip>>;

    fn input_slots(&self) -> InputSlots {
        InputSlots::Fixed(vec![name!("pose")])
    }

    fn create_state(&self, _: &(), _context: &AnimationGraphContext<'_>) -> AimOffsetState {
        AimOffsetState {
            elapsed: 0.0,
            fresh: true,
            snap_angles: true,
            clock: 0.0,
            weight: Spring::critically_damped(0.0, 0.0),
            angles: Spring::critically_damped(Vec2::ZERO, 0.0),
            weighting: AimWeights::new(&self.sample_points()),
            point_weights: vec![0.0; self.points.len()],
            warned: false,
            warned_clip: false,
        }
    }

    fn update(
        &self,
        state: &mut AimOffsetState,
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
        state: &mut AimOffsetState,
        clips: &Res<AssetStore<AnimationClip>>,
        context: &AnimationGraphContext<'_>,
        bone_ids: &[Uuid],
        evaluated_inputs: &[EvaluatedFrame],
        _: &mut FramePool,
        output: &mut AnimationFrame,
    ) {
        pass_through(evaluated_inputs, output);
        let delta_time = state.take_time();
        let aim = match self.resolve(context.skeleton()) {
            Ok(aim) => aim,
            Err(reason) => {
                if !state.warned {
                    warn!(
                        "aim offset toward `{}` skipped: {reason}",
                        self.target_param.0.as_str()
                    );
                    state.warned = true;
                }
                return;
            }
        };
        if state.point_weights.len() != self.points.len() {
            state.weighting = AimWeights::new(&self.sample_points());
            state.point_weights = vec![0.0; self.points.len()];
        }
        self.step_clock(state, clips, delta_time);
        self.warn_plain_clips(state, clips);

        let measured = self
            .target(context)
            .and_then(|target| aim.measure(output.joints(), target));
        match measured {
            Some(goal) => state.ease_angles(goal, halflife(self.angle_halflife), delta_time),
            None => {
                let held = state.angles.value;
                state.angles.reset_to(held);
            }
        }
        let goal = if measured.is_some() {
            self.effective_weight(context.blackboard())
        } else {
            0.0
        };
        let weight = ease(
            &mut state.weight,
            goal,
            halflife(self.weight_halflife),
            delta_time,
        )
        .clamp(0.0, 1.0);
        state.weight.value = weight;
        if goal <= 0.0 && weight < SNAP_WEIGHT {
            state.weight.reset_to(0.0);
            state.snap_angles = true;
            return;
        }
        if weight <= 0.0 {
            return;
        }

        state
            .weighting
            .weigh(state.angles.value, &mut state.point_weights);
        for (bone, id) in bone_ids.iter().enumerate() {
            let Some(delta) = self.combined_delta(state, clips, id) else {
                continue;
            };
            if let Some(joint) = output.joint_mut(bone) {
                apply_additive(joint, &delta, weight);
            }
        }
    }

    fn reset(&self, state: &mut AimOffsetState) {
        state.elapsed = 0.0;
        state.fresh = true;
        state.snap_angles = true;
        state.clock = 0.0;
        state.weight.reset_to(0.0);
    }
}

pub(crate) struct AimWeights {
    shape: AimShape,
}

enum AimShape {
    Empty,
    Single(usize),
    Line(AimLine),
    Plane(AimPlane),
}

struct AimLine {
    origin: Vec2,
    direction: Vec2,
    stops: Vec<AimStop>,
}

struct AimStop {
    point: usize,
    along: f32,
}

struct AimPlane {
    triangulation: Triangulation2D,
    kept: Vec<usize>,
}

impl AimWeights {
    pub(crate) fn new(points: &[Vec2]) -> Self {
        let mut kept: Vec<usize> = Vec::new();
        for (index, point) in points.iter().enumerate() {
            let duplicate = kept
                .iter()
                .any(|&earlier| points[earlier].distance(*point) < DUPLICATE_TOLERANCE);
            if !duplicate {
                kept.push(index);
            }
        }
        let shape = match kept.len() {
            0 => AimShape::Empty,
            1 => AimShape::Single(kept[0]),
            _ => {
                let line = AimLine::through_farthest(points, &kept);
                if line.holds(points, &kept) {
                    AimShape::Line(line)
                } else {
                    let triangulation =
                        Triangulation2D::build(kept.iter().map(|&index| points[index]).collect());
                    if triangulation.triangles().is_empty() {
                        AimShape::Line(line)
                    } else {
                        AimShape::Plane(AimPlane {
                            triangulation,
                            kept,
                        })
                    }
                }
            }
        };
        Self { shape }
    }

    pub(crate) fn weigh(&self, sample: Vec2, weights: &mut [f32]) {
        weights.fill(0.0);
        match &self.shape {
            AimShape::Empty => {}
            AimShape::Single(point) => set(weights, *point, 1.0),
            AimShape::Line(line) => line.weigh(sample, weights),
            AimShape::Plane(plane) => plane.weigh(sample, weights),
        }
    }
}

impl AimLine {
    fn through_farthest(points: &[Vec2], kept: &[usize]) -> Self {
        let mut first = kept[0];
        let mut second = kept[1];
        let mut farthest = points[first].distance_squared(points[second]);
        for (position, &a) in kept.iter().enumerate() {
            for &b in &kept[position + 1..] {
                let distance = points[a].distance_squared(points[b]);
                if distance > farthest {
                    farthest = distance;
                    first = a;
                    second = b;
                }
            }
        }
        let origin = points[first];
        let direction = (points[second] - origin).normalize_or_zero();
        let mut stops: Vec<AimStop> = kept
            .iter()
            .map(|&point| AimStop {
                point,
                along: (points[point] - origin).dot(direction),
            })
            .collect();
        stops.sort_by(|a, b| a.along.total_cmp(&b.along));
        Self {
            origin,
            direction,
            stops,
        }
    }

    fn holds(&self, points: &[Vec2], kept: &[usize]) -> bool {
        kept.iter().all(|&point| {
            (points[point] - self.origin).perp_dot(self.direction).abs() <= LINE_TOLERANCE
        })
    }

    fn weigh(&self, sample: Vec2, weights: &mut [f32]) {
        let along = (sample - self.origin).dot(self.direction);
        let next = self.stops.partition_point(|stop| stop.along <= along);
        if next == 0 {
            set(weights, self.stops[0].point, 1.0);
        } else if next == self.stops.len() {
            set(weights, self.stops[next - 1].point, 1.0);
        } else {
            let lower = &self.stops[next - 1];
            let upper = &self.stops[next];
            let t = (along - lower.along) / (upper.along - lower.along);
            set(weights, lower.point, 1.0 - t);
            set(weights, upper.point, t);
        }
    }
}

impl AimPlane {
    fn weigh(&self, sample: Vec2, weights: &mut [f32]) {
        let located = self.triangulation.locate_or_nearest(sample);
        let triangle = self.triangulation.triangles()[located.triangle];
        set(weights, self.kept[triangle.a], located.lambda_a);
        set(weights, self.kept[triangle.b], located.lambda_b);
        set(weights, self.kept[triangle.c], located.lambda_c);
    }
}

fn set(weights: &mut [f32], index: usize, weight: f32) {
    if let Some(slot) = weights.get_mut(index) {
        *slot = weight;
    }
}

#[cfg(test)]
mod tests {
    use concerto_ecs::Res;
    use concerto_foundation::assets::{AssetId, asset_store::AssetStore, handle::AssetHandle};
    use concerto_mesh::Skeleton;
    use concerto_name::name;
    use glam::{Affine3A, Mat4, Quat, Vec2, Vec3};
    use proptest::prelude::*;
    use uuid::Uuid;

    use super::{AimOffsetNode, AimOffsetState, AimPoint, AimWeights};
    use crate::{
        blackboard::AnimationBlackboard,
        clip::{Additive, AdditiveBase, AnimationChanelOutput, AnimationChannel, AnimationClip},
        evaluation::AnimationGraphContext,
        node::AnimationNode,
        pose::{
            AnimationFrame, EvaluatedFrame, FiredEvent, FramePool, JointPose, Pose, apply_additive,
        },
        testing::Walks,
    };

    fn weigh(points: &[Vec2], sample: Vec2) -> Vec<f32> {
        let mut weights = vec![f32::NAN; points.len()];
        AimWeights::new(points).weigh(sample, &mut weights);
        weights
    }

    struct Weighed {
        sample: Vec2,
        weights: Vec<f32>,
    }

    fn weighed(sample: Vec2, weights: &[f32]) -> Weighed {
        Weighed {
            sample,
            weights: weights.to_vec(),
        }
    }

    struct PointsCase {
        name: &'static str,
        points: Vec<Vec2>,
        samples: Vec<Weighed>,
    }

    fn pitch_line(name: &'static str, points: Vec<Vec2>, extra: Vec<Weighed>) -> PointsCase {
        let at = |weights: [f32; 3]| {
            let mut all = vec![0.0; points.len()];
            all[..3].copy_from_slice(&weights);
            all
        };
        let mut samples = vec![
            weighed(Vec2::new(0.0, 0.25), &at([0.0, 0.5, 0.5])),
            weighed(Vec2::new(0.3, -0.25), &at([0.5, 0.5, 0.0])),
            weighed(Vec2::new(0.0, 1.5), &at([0.0, 0.0, 1.0])),
            weighed(Vec2::new(-0.2, -2.0), &at([1.0, 0.0, 0.0])),
        ];
        samples.extend(extra);
        PointsCase {
            name,
            points,
            samples,
        }
    }

    #[test]
    fn aim_weights_are_barycentric_inside_the_points_and_clamp_to_their_hull() {
        let cases = [
            pitch_line(
                "a pitch only set on a line",
                vec![
                    Vec2::new(0.0, -0.5),
                    Vec2::new(0.0, 0.0),
                    Vec2::new(0.0, 0.5),
                ],
                Vec::new(),
            ),
            PointsCase {
                name: "a sample inside a triangle is weighed by its barycentric coordinates",
                points: vec![Vec2::ZERO, Vec2::X, Vec2::Y],
                samples: vec![
                    weighed(Vec2::new(0.25, 0.25), &[0.5, 0.25, 0.25]),
                    weighed(Vec2::new(0.1, 0.6), &[0.3, 0.1, 0.6]),
                ],
            },
        ];

        for case in cases {
            for sample in &case.samples {
                let weights = weigh(&case.points, sample.sample);

                assert_eq!(weights.len(), sample.weights.len(), "{}", case.name);
                for (index, (actual, expected)) in weights.iter().zip(&sample.weights).enumerate() {
                    assert!(
                        (actual - expected).abs() < 1e-5,
                        "{} at {}: weight {index} is {actual}, not {expected}",
                        case.name,
                        sample.sample
                    );
                }
            }
        }
    }

    fn well_separated(points: &[Vec2]) -> bool {
        let count = points.len();
        (0..count).all(|a| {
            (a + 1..count).all(|b| {
                points[a].distance(points[b]) >= 0.05
                    && (b + 1..count).all(|c| {
                        (points[b] - points[a])
                            .perp_dot(points[c] - points[a])
                            .abs()
                            >= 0.01
                    })
            })
        })
    }

    proptest! {
        #[test]
        fn aim_weights_are_not_negative_beyond_rounding_and_sum_to_1(
            points in prop::collection::vec(
                prop::array::uniform2(-1.0f32..1.0).prop_map(Vec2::from_array),
                1..7,
            )
            .prop_filter("well separated", |points| well_separated(points)),
            sample in prop::array::uniform2(-3.0f32..3.0).prop_map(Vec2::from_array),
        ) {
            let weights = weigh(&points, sample);

            let total: f32 = weights.iter().sum();
            prop_assert!((total - 1.0).abs() < 1e-4, "{sample}: {weights:?}");
            prop_assert!(weights.iter().all(|weight| *weight >= -1e-4), "{weights:?}");
        }
    }

    const HIPS: usize = 0;
    const SPINE: usize = 1;
    const CHEST: usize = 2;
    const ARM: usize = 3;
    const PROP: usize = 4;
    const DT: f32 = 1.0 / 60.0;
    const DISTANCE: f32 = 2.0;

    struct RigBone {
        name: &'static str,
        parent: Option<u16>,
        joint: JointPose,
    }

    fn rig_bone(
        name: &'static str,
        parent: Option<u16>,
        translation: Vec3,
        rotation: Quat,
    ) -> RigBone {
        RigBone {
            name,
            parent,
            joint: JointPose {
                translation,
                rotation,
                scale: Vec3::ONE,
            },
        }
    }

    fn skeleton() -> Skeleton {
        let bones = [
            rig_bone("hips", None, Vec3::Y, Quat::from_rotation_y(0.4)),
            rig_bone(
                "spine",
                Some(0),
                Vec3::new(0.0, 0.3, 0.0),
                Quat::from_rotation_x(0.1),
            ),
            rig_bone(
                "chest",
                Some(1),
                Vec3::new(0.0, 0.3, 0.0),
                Quat::from_rotation_z(0.15),
            ),
            rig_bone(
                "arm",
                Some(2),
                Vec3::new(0.2, 0.1, 0.0),
                Quat::from_rotation_z(-1.0),
            ),
            rig_bone("prop", None, Vec3::X, Quat::IDENTITY),
        ];
        Skeleton::new(
            vec![Mat4::IDENTITY; bones.len()],
            bones.iter().map(|bone| bone.parent).collect(),
            bones.iter().map(|bone| bone.name.into()).collect(),
            bones.iter().map(|bone| bone.joint.clone()).collect(),
            (0..bones.len()).map(bone_id).collect(),
        )
        .unwrap()
    }

    fn bone_id(bone: usize) -> Uuid {
        Uuid::from_u128(bone as u128)
    }

    fn input_event() -> FiredEvent {
        FiredEvent {
            clip: AssetId::from_path("aim_input"),
            key: 3,
            time: 0.2,
            weight: 0.5,
            mirrored: false,
        }
    }

    struct Delta {
        rotation: Quat,
        translation: Vec3,
    }

    impl Delta {
        fn pose(&self) -> JointPose {
            JointPose {
                translation: self.translation,
                rotation: self.rotation,
                scale: Vec3::ONE,
            }
        }
    }

    fn low() -> Delta {
        Delta {
            rotation: Quat::from_rotation_x(0.4),
            translation: Vec3::new(0.0, -0.05, 0.0),
        }
    }

    fn level() -> Delta {
        Delta {
            rotation: Quat::from_rotation_y(0.2),
            translation: Vec3::new(0.02, 0.0, 0.0),
        }
    }

    fn high() -> Delta {
        Delta {
            rotation: Quat::from_rotation_x(-0.5),
            translation: Vec3::new(0.0, 0.04, 0.03),
        }
    }

    fn still(clip: &mut AnimationClip, bone: usize, delta: &Delta, duration: f32) {
        clip.add_channel(
            bone_id(bone),
            AnimationChannel::new(
                vec![0.0, duration],
                AnimationChanelOutput::Rotation(vec![delta.rotation, delta.rotation]),
            ),
        );
        clip.add_channel(
            bone_id(bone),
            AnimationChannel::new(
                vec![0.0, duration],
                AnimationChanelOutput::Translation(vec![delta.translation, delta.translation]),
            ),
        );
    }

    fn unmoved() -> Delta {
        Delta {
            rotation: Quat::IDENTITY,
            translation: Vec3::ZERO,
        }
    }

    fn neutral() -> AnimationClip {
        let mut clip = AnimationClip::default();
        still(&mut clip, HIPS, &unmoved(), 1.0);
        still(&mut clip, CHEST, &unmoved(), 1.0);
        clip.set_frame_rate(10.0);
        clip
    }

    fn additive(mut clip: AnimationClip) -> AnimationClip {
        clip.set_frame_rate(10.0);
        clip.set_additive(Additive::LocalSpace(AdditiveBase::new(AssetHandle::weak(
            AssetId::from_path("neutral"),
        ))));
        clip.with_delta_against(&neutral())
    }

    fn held(delta: &Delta, duration: f32) -> AnimationClip {
        let mut clip = AnimationClip::default();
        still(&mut clip, HIPS, delta, duration);
        still(&mut clip, CHEST, delta, duration);
        clip
    }

    fn clips<'a>(context: &'a AnimationGraphContext<'a>) -> &'a Res<'a, AssetStore<AnimationClip>> {
        context
            .inputs
            .get::<Res<'static, AssetStore<AnimationClip>>>()
            .unwrap()
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

        fn add_clip(&mut self, name: &str, clip: AnimationClip) -> AssetHandle<AnimationClip> {
            let id = AssetId::from_path(name);
            self.walks.clips_mut().insert(id, clip);
            AssetHandle::weak(id)
        }

        fn add_aim(&mut self, name: &str, delta: &Delta) -> AssetHandle<AnimationClip> {
            self.add_clip(name, additive(held(delta, 1.0)))
        }

        fn input(&self) -> AnimationFrame {
            let mut frame = AnimationFrame::new(Pose::from_joints(self.skeleton.rest_pose.clone()));
            frame.push_event(input_event());
            frame
        }

        fn toward(&self, yaw: f32, pitch: f32) -> Vec3 {
            let chest = self
                .skeleton
                .forward_kinematics(&self.skeleton.rest_pose)
                .unwrap()[CHEST]
                .w_axis
                .truncate();
            let direction = Vec3::new(
                yaw.sin() * pitch.cos(),
                pitch.sin(),
                yaw.cos() * pitch.cos(),
            );
            chest + direction * DISTANCE
        }

        fn aim_at(&mut self, yaw: f32, pitch: f32) {
            let target = self
                .model_to_world
                .unwrap_or_default()
                .transform_point3(self.toward(yaw, pitch));
            self.blackboard.set(name!("aim_target"), target);
        }

        fn forget_target(&mut self) {
            self.blackboard = AnimationBlackboard::default();
        }

        fn start(&self, node: &AimOffsetNode) -> AimOffsetState {
            self.walks.create(&self.blackboard, |context| {
                AnimationNode::create_state(node, &(), context)
            })
        }

        fn step(
            &self,
            node: &AimOffsetNode,
            state: &mut AimOffsetState,
            delta_time: f32,
        ) -> AnimationFrame {
            self.walks.update(&self.blackboard, |context| {
                node.update(state, delta_time, &(), context)
            });
            self.evaluate(node, state)
        }

        fn evaluate(&self, node: &AimOffsetNode, state: &mut AimOffsetState) -> AnimationFrame {
            let input = [EvaluatedFrame {
                frame: self.input(),
                weight: 1.0,
            }];
            let mut pool = FramePool::new(self.skeleton.bone_count());
            let mut output = pool.acquire();
            let evaluate = |context: &AnimationGraphContext<'_>| {
                node.evaluate(
                    state,
                    clips(context),
                    context,
                    &self.skeleton.bone_ids,
                    &input,
                    &mut pool,
                    &mut output,
                )
            };
            self.walks.evaluate_posed(
                &self.blackboard,
                &self.skeleton,
                self.model_to_world,
                evaluate,
            );
            output
        }

        fn run(&self, node: &AimOffsetNode) -> AnimationFrame {
            let mut state = self.start(node);
            self.step(node, &mut state, 0.0)
        }

        fn expected(&self, bone: usize, delta: &JointPose) -> JointPose {
            let mut joint = self.skeleton.rest_pose[bone].clone();
            apply_additive(&mut joint, delta, 1.0);
            joint
        }
    }

    fn aim_node() -> AimOffsetNode {
        AimOffsetNode::new(name!("aim_target"), "chest")
            .with_weight_halflife(0.0)
            .with_angle_halflife(0.0)
    }

    fn with_points(points: &[AimPoint]) -> AimOffsetNode {
        points
            .iter()
            .cloned()
            .fold(aim_node(), AimOffsetNode::with_point)
    }

    struct PitchClips {
        low: AssetHandle<AnimationClip>,
        level: AssetHandle<AnimationClip>,
        high: AssetHandle<AnimationClip>,
    }

    fn pitch_clips(rig: &mut Rig) -> PitchClips {
        PitchClips {
            low: rig.add_aim("low", &low()),
            level: rig.add_aim("level", &level()),
            high: rig.add_aim("high", &high()),
        }
    }

    fn pitch_node(clips: &PitchClips) -> AimOffsetNode {
        with_points(&[
            AimPoint::new(clips.low.clone(), 0.0, -0.5),
            AimPoint::new(clips.level.clone(), 0.0, 0.0),
            AimPoint::new(clips.high.clone(), 0.0, 0.5),
        ])
    }

    fn blended(a: &Delta, b: &Delta, t: f32) -> JointPose {
        JointPose {
            translation: a.translation * (1.0 - t) + b.translation * t,
            rotation: a.rotation.slerp(b.rotation, t),
            scale: Vec3::ONE,
        }
    }

    fn assert_joint(actual: &JointPose, expected: &JointPose) {
        assert!(
            actual.translation.abs_diff_eq(expected.translation, 1e-4)
                && actual.rotation.angle_between(expected.rotation) < 1e-3,
            "expected {expected:?}, got {actual:?}"
        );
    }

    fn assert_aimed(rig: &Rig, frame: &AnimationFrame, delta: &JointPose) {
        for bone in [HIPS, CHEST] {
            assert_joint(&frame.joints()[bone], &rig.expected(bone, delta));
        }
        let input = rig.input();
        for bone in [SPINE, ARM, PROP] {
            assert_eq!(frame.joints()[bone], input.joints()[bone], "bone {bone}");
        }
    }

    fn assert_unchanged(rig: &Rig, frame: &AnimationFrame) {
        assert_eq!(frame.joints(), rig.input().joints());
        assert_eq!(frame.events(), [input_event()]);
    }

    fn assert_pitch_blend(rig: &mut Rig, node: &AimOffsetNode) {
        rig.aim_at(0.0, 0.25);
        assert_aimed(rig, &rig.run(node), &blended(&level(), &high(), 0.5));

        rig.aim_at(0.0, -0.125);
        assert_aimed(rig, &rig.run(node), &blended(&level(), &low(), 0.25));

        rig.aim_at(0.0, 1.2);
        assert_aimed(rig, &rig.run(node), &high().pose());

        rig.aim_at(0.0, -1.2);
        assert_aimed(rig, &rig.run(node), &low().pose());
    }

    #[test]
    fn a_target_in_the_world_gives_the_deltas_of_its_points_blended_by_their_weights() {
        let mut rig = Rig::new();
        let clips = pitch_clips(&mut rig);
        rig.aim_at(0.0, 0.0);

        let level_frame = rig.run(&pitch_node(&clips));
        assert_aimed(&rig, &level_frame, &level().pose());
        assert_eq!(level_frame.events(), [input_event()]);
        assert_pitch_blend(&mut rig, &pitch_node(&clips));

        rig.model_to_world = Some(Affine3A::from_rotation_translation(
            Quat::from_rotation_y(1.1),
            Vec3::new(3.0, 0.0, -2.0),
        ));
        rig.aim_at(0.0, 0.25);
        assert_aimed(
            &rig,
            &rig.run(&pitch_node(&clips)),
            &blended(&level(), &high(), 0.5),
        );
    }

    #[test]
    fn a_target_crossing_behind_from_yaw_170_to_minus_170_degrees_turns_the_short_way() {
        let mut rig = Rig::new();
        let level = rig.add_aim("level", &level());
        let node = with_points(&[AimPoint::new(level, 0.0, 0.0)]).with_angle_halflife(0.1);
        rig.aim_at(170f32.to_radians(), 0.0);
        let mut state = rig.start(&node);
        rig.step(&node, &mut state, 0.0);
        assert!((state.angles.value.x - 170f32.to_radians()).abs() < 1e-4);

        rig.aim_at(-170f32.to_radians(), 0.0);
        for _ in 0..120 {
            rig.step(&node, &mut state, DT);
            let yaw = state.angles.value.x;
            assert!(
                yaw.abs() > 169.9f32.to_radians(),
                "yaw {}",
                yaw.to_degrees()
            );
        }

        assert!((state.angles.value.x + 170f32.to_radians()).abs() < 1e-3);
    }

    #[test]
    fn a_target_gone_missing_eases_the_weight_out_and_then_leaves_the_input_bit_for_bit() {
        let mut rig = Rig::new();
        let clips = pitch_clips(&mut rig);
        let node = pitch_node(&clips).with_weight_halflife(0.05);
        rig.aim_at(0.0, 0.25);
        let mut state = rig.start(&node);
        rig.step(&node, &mut state, 0.0);
        let mut frame = rig.step(&node, &mut state, DT);
        for _ in 0..60 {
            frame = rig.step(&node, &mut state, DT);
        }
        assert_ne!(frame.joints(), rig.input().joints());

        rig.forget_target();
        for _ in 0..300 {
            frame = rig.step(&node, &mut state, DT);
        }

        assert_eq!(state.weight.value, 0.0);
        assert_unchanged(&rig, &frame);
    }
}

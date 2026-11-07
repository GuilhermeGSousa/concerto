use std::f32::consts::{PI, TAU};

use concerto_ecs::Res;
use concerto_foundation::{
    assets::{asset_store::AssetStore, handle::AssetHandle},
    geometry::delauney::Triangulation2D,
    math::{Spring, SpringValue},
};
use concerto_mesh::{JointName, Skeleton};
use concerto_name::Name;
use concerto_reflect::Reflect;
use glam::{Vec2, Vec3};
use log::warn;
use uuid::Uuid;

use crate::{
    blackboard::AnimationBlackboard,
    clip::{Additive, AnimationClip},
    evaluation::AnimationGraphContext,
    look_at::{LookFrame, yaw_pitch_to},
    node::{
        AnimationNode, ReflectAnimationNode,
        model_space::{WorldToModel, bone_under_first, model_joint},
        pass_through,
    },
    pose::{AnimationFrame, EvaluatedFrame, FramePool, JointPose, apply_additive},
};

const DUPLICATE_TOLERANCE: f32 = 1e-4;
const LINE_TOLERANCE: f32 = 1e-3;
const SNAP_WEIGHT: f32 = 1.0e-4;

/// Layers additive aim clips, chosen by the target's yaw and pitch from a chosen bone, over its first input.
#[derive(Clone, Debug, PartialEq, Reflect, serde::Serialize, serde::Deserialize)]
#[reflect(AnimationNode)]
pub struct AimOffsetNode {
    pub points: Vec<AimPoint>,
    pub frame_bone: JointName,
    pub forward: Vec3,
    pub up: Vec3,
    pub target_param: Name,
    pub weight: f32,
    pub weight_param: Option<Name>,
    pub weight_halflife: f32,
    pub angle_halflife: f32,
}

impl Default for AimOffsetNode {
    fn default() -> Self {
        Self {
            points: Vec::new(),
            frame_bone: JointName::default(),
            forward: Vec3::Z,
            up: Vec3::Y,
            target_param: Name::default(),
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
    pub clip: AssetHandle<AnimationClip>,
    pub yaw: f32,
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
    pub fn new(target_param: Name, frame_bone: impl Into<JointName>) -> Self {
        Self {
            target_param,
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
    pub fn with_weight_param(mut self, param: Name) -> Self {
        self.weight_param = Some(param);
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
            .and_then(|param| blackboard.get_float(param))
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
        let target = *context.blackboard().get::<Vec3>(self.target_param)?;
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
                self.target_param.as_str(),
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
                        self.target_param.as_str()
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
    use uuid::Uuid;

    use super::{AimOffsetNode, AimOffsetState, AimPoint, AimWeights};
    use crate::{
        blackboard::AnimationBlackboard,
        clip::{Additive, AdditiveBase, AnimationChanelOutput, AnimationChannel, AnimationClip},
        evaluation::AnimationGraphContext,
        node::AnimationNode,
        pose::{
            AnimationFrame, EvaluatedFrame, FiredEvent, FramePool, JointPose, Pose, RootMotion,
            apply_additive,
        },
        test_walks::Walks,
    };

    fn weigh(points: &[Vec2], sample: Vec2) -> Vec<f32> {
        let mut weights = vec![f32::NAN; points.len()];
        AimWeights::new(points).weigh(sample, &mut weights);
        weights
    }

    fn assert_weights(actual: &[f32], expected: &[f32]) {
        assert_eq!(actual.len(), expected.len());
        for (index, (a, e)) in actual.iter().zip(expected).enumerate() {
            assert!(
                (a - e).abs() < 1e-5,
                "weight {index}: {actual:?} != {expected:?}"
            );
        }
    }

    fn assert_pitch_line(points: &[Vec2], low: usize, middle: usize, high: usize) {
        let mut halfway_up = vec![0.0; points.len()];
        halfway_up[middle] = 0.5;
        halfway_up[high] = 0.5;
        assert_weights(&weigh(points, Vec2::new(0.0, 0.25)), &halfway_up);

        let mut quarter_down = vec![0.0; points.len()];
        quarter_down[middle] = 0.5;
        quarter_down[low] = 0.5;
        assert_weights(&weigh(points, Vec2::new(0.3, -0.25)), &quarter_down);

        let mut top = vec![0.0; points.len()];
        top[high] = 1.0;
        assert_weights(&weigh(points, Vec2::new(0.0, 1.5)), &top);

        let mut bottom = vec![0.0; points.len()];
        bottom[low] = 1.0;
        assert_weights(&weigh(points, Vec2::new(-0.2, -2.0)), &bottom);

        for step in -20..=20 {
            let sample = Vec2::new(step as f32 * 0.07, step as f32 * 0.05);
            let weights = weigh(points, sample);
            let total: f32 = weights.iter().sum();
            assert!((total - 1.0).abs() < 1e-5, "{sample}: {weights:?}");
            assert!(weights.iter().all(|weight| *weight >= 0.0), "{weights:?}");
        }
        weigh(points, Vec2::NAN);
    }

    #[test]
    fn a_pitch_only_set_on_a_line_interpolates_by_pitch_and_clamps_at_the_ends() {
        let points = [
            Vec2::new(0.0, 0.0),
            Vec2::new(0.0, 0.5),
            Vec2::new(0.0, -0.5),
        ];

        assert_pitch_line(&points, 2, 0, 1);
    }

    #[test]
    fn a_point_a_thousandth_of_a_radian_off_the_line_still_counts_as_on_it() {
        let points = [
            Vec2::new(0.0, -0.5),
            Vec2::new(1e-3, 0.0),
            Vec2::new(0.0, 0.5),
        ];

        assert_pitch_line(&points, 0, 1, 2);
    }

    #[test]
    fn a_duplicated_point_is_dropped_and_keeps_a_zero_weight() {
        let points = [
            Vec2::new(0.0, -0.5),
            Vec2::new(0.0, 0.0),
            Vec2::new(0.0, 0.5),
            Vec2::new(0.0, 0.0),
            Vec2::new(5e-5, 0.0),
        ];

        assert_pitch_line(&points, 0, 1, 2);
        let weights = weigh(&points, Vec2::new(0.0, 0.0));
        assert_eq!(weights[3], 0.0);
        assert_eq!(weights[4], 0.0);
    }

    #[test]
    fn a_sample_inside_a_triangle_is_weighed_by_its_barycentric_coordinates() {
        let points = [
            Vec2::new(0.0, 0.0),
            Vec2::new(1.0, 0.0),
            Vec2::new(0.0, 1.0),
        ];

        assert_weights(&weigh(&points, Vec2::new(0.25, 0.25)), &[0.5, 0.25, 0.25]);
        assert_weights(&weigh(&points, Vec2::new(0.1, 0.6)), &[0.3, 0.1, 0.6]);
    }

    #[test]
    fn a_single_point_takes_the_whole_weight_wherever_the_sample_is() {
        let points = [Vec2::new(0.2, -0.1)];

        assert_weights(&weigh(&points, Vec2::new(0.2, -0.1)), &[1.0]);
        assert_weights(&weigh(&points, Vec2::new(-3.0, 2.0)), &[1.0]);
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

    fn input_motion() -> RootMotion {
        RootMotion {
            translation: Vec3::new(0.1, 0.0, 0.3),
            rotation: Quat::from_rotation_y(0.05),
        }
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

    fn wild() -> Delta {
        Delta {
            rotation: Quat::from_rotation_z(1.3),
            translation: Vec3::new(0.5, 0.5, 0.5),
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

    fn breathing() -> AnimationClip {
        let mut clip = AnimationClip::default();
        clip.add_channel(
            bone_id(CHEST),
            AnimationChannel::new(
                vec![0.0, 1.0],
                AnimationChanelOutput::Rotation(vec![Quat::IDENTITY, Quat::from_rotation_x(1.0)]),
            ),
        );
        additive(clip)
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
            frame.set_root_motion(input_motion());
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
            self.evaluate(node, state, true)
        }

        fn evaluate(
            &self,
            node: &AimOffsetNode,
            state: &mut AimOffsetState,
            with_skeleton: bool,
        ) -> AnimationFrame {
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
        assert_eq!(frame.root_motion(), input_motion());
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
    fn a_target_straight_along_forward_gives_the_full_delta_of_the_point_at_the_origin() {
        let mut rig = Rig::new();
        let clips = pitch_clips(&mut rig);
        rig.aim_at(0.0, 0.0);

        let frame = rig.run(&pitch_node(&clips));

        assert_aimed(&rig, &frame, &level().pose());
    }

    #[test]
    fn a_target_between_two_pitch_points_blends_their_deltas_and_clamps_past_the_ends() {
        let mut rig = Rig::new();
        let clips = pitch_clips(&mut rig);

        assert_pitch_blend(&mut rig, &pitch_node(&clips));
    }

    #[test]
    fn a_pitch_set_a_thousandth_of_a_radian_off_its_line_blends_as_on_it() {
        let mut rig = Rig::new();
        let clips = pitch_clips(&mut rig);
        let node = with_points(&[
            AimPoint::new(clips.low.clone(), 0.0, -0.5),
            AimPoint::new(clips.level.clone(), 1e-3, 0.0),
            AimPoint::new(clips.high.clone(), 0.0, 0.5),
        ]);

        assert_pitch_blend(&mut rig, &node);
    }

    #[test]
    fn a_duplicated_pitch_point_is_left_out_of_the_blend() {
        let mut rig = Rig::new();
        let clips = pitch_clips(&mut rig);
        let twin = rig.add_aim("twin", &wild());
        let node = with_points(&[
            AimPoint::new(clips.low.clone(), 0.0, -0.5),
            AimPoint::new(clips.level.clone(), 0.0, 0.0),
            AimPoint::new(clips.high.clone(), 0.0, 0.5),
            AimPoint::new(twin.clone(), 0.0, 0.0),
            AimPoint::new(twin, 5e-5, 0.0),
        ]);

        assert_pitch_blend(&mut rig, &node);
        rig.aim_at(0.0, 0.0);
        assert_aimed(&rig, &rig.run(&node), &level().pose());
    }

    #[test]
    fn a_target_inside_a_triangle_of_points_weighs_them_by_barycentric_coordinates() {
        let mut rig = Rig::new();
        let origin = rig.add_aim("origin", &level());
        let side = rig.add_aim("side", &low());
        let up = rig.add_aim("up", &high());
        let node = with_points(&[
            AimPoint::new(origin, 0.0, 0.0),
            AimPoint::new(side, 0.5, 0.0),
            AimPoint::new(up, 0.0, 0.5),
        ]);
        rig.aim_at(0.125, 0.125);

        let frame = rig.run(&node);

        let added =
            level().translation * 0.5 + low().translation * 0.25 + high().translation * 0.25;
        for bone in [HIPS, CHEST] {
            let rest = rig.skeleton.rest_pose[bone].translation;
            assert!(
                frame.joints()[bone]
                    .translation
                    .abs_diff_eq(rest + added, 1e-4),
                "bone {bone}: {:?}",
                frame.joints()[bone].translation
            );
        }
    }

    #[test]
    fn a_turned_model_to_world_maps_the_target_into_model_space_before_measuring() {
        let mut rig = Rig::new();
        let clips = pitch_clips(&mut rig);
        rig.model_to_world = Some(Affine3A::from_rotation_translation(
            Quat::from_rotation_y(1.1),
            Vec3::new(3.0, 0.0, -2.0),
        ));
        rig.aim_at(0.0, 0.25);

        let frame = rig.run(&pitch_node(&clips));

        assert_aimed(&rig, &frame, &blended(&level(), &high(), 0.5));
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
    fn the_first_evaluate_after_creation_or_a_reset_snaps_to_the_measured_angles() {
        let mut rig = Rig::new();
        let level = rig.add_aim("level", &level());
        let node = with_points(&[AimPoint::new(level, 0.0, 0.0)]).with_angle_halflife(0.5);
        rig.aim_at(0.3, 0.1);
        let mut state = rig.start(&node);

        rig.step(&node, &mut state, 0.0);
        assert!(state.angles.value.abs_diff_eq(Vec2::new(0.3, 0.1), 1e-4));

        rig.aim_at(-0.4, -0.2);
        rig.step(&node, &mut state, DT);
        assert!(state.angles.value.x > 0.2, "{}", state.angles.value);

        node.reset(&mut state);
        rig.step(&node, &mut state, DT);
        assert!(
            state.angles.value.abs_diff_eq(Vec2::new(-0.4, -0.2), 1e-4),
            "{}",
            state.angles.value
        );
    }

    #[test]
    fn an_angle_halflife_eases_toward_a_jumped_target_and_zero_snaps() {
        let mut rig = Rig::new();
        let level = rig.add_aim("level", &level());
        let snapping = with_points(&[AimPoint::new(level, 0.0, 0.0)]);
        let easing = snapping.clone().with_angle_halflife(0.1);
        rig.aim_at(0.0, 0.0);
        let mut eased = rig.start(&easing);
        let mut snapped = rig.start(&snapping);
        rig.step(&easing, &mut eased, 0.0);
        rig.step(&snapping, &mut snapped, 0.0);

        rig.aim_at(0.0, 0.4);
        rig.step(&snapping, &mut snapped, DT);
        for _ in 0..6 {
            rig.step(&easing, &mut eased, DT);
        }

        assert!((snapped.angles.value.y - 0.4).abs() < 1e-4);
        assert!(
            (eased.angles.value.y - 0.2).abs() < 1e-3,
            "{}",
            eased.angles.value
        );
    }

    #[test]
    fn a_looping_delta_is_sampled_at_the_clock_modulo_its_own_duration() {
        let mut rig = Rig::new();
        let long = rig.add_clip("long", additive(held(&unmoved(), 2.0)));
        let breath = rig.add_clip("breath", breathing());
        let node = with_points(&[
            AimPoint::new(long, 0.0, 0.5),
            AimPoint::new(breath, 0.0, 0.0),
        ]);
        rig.aim_at(0.0, 0.0);
        let mut state = rig.start(&node);
        rig.step(&node, &mut state, 0.0);
        let breathed = JointPose {
            rotation: Quat::from_rotation_x(0.25),
            ..JointPose::IDENTITY
        };

        let frame = rig.step(&node, &mut state, 1.25);
        assert_joint(&frame.joints()[CHEST], &rig.expected(CHEST, &breathed));

        let frame = rig.step(&node, &mut state, 1.0);
        assert!((state.clock - 0.25).abs() < 1e-4, "clock {}", state.clock);
        assert_joint(&frame.joints()[CHEST], &rig.expected(CHEST, &breathed));
    }

    #[test]
    fn root_motion_and_events_pass_through_and_the_root_bone_takes_its_delta() {
        let mut rig = Rig::new();
        let clips = pitch_clips(&mut rig);
        rig.aim_at(0.0, 0.0);

        let frame = rig.run(&pitch_node(&clips));

        assert_eq!(frame.root_motion(), input_motion());
        assert_eq!(frame.events(), [input_event()]);
        assert_joint(&frame.joints()[HIPS], &rig.expected(HIPS, &level().pose()));
        assert_ne!(frame.joints()[HIPS], rig.input().joints()[HIPS]);
    }

    #[test]
    fn weight_zero_leaves_the_input_bit_for_bit() {
        let mut rig = Rig::new();
        let clips = pitch_clips(&mut rig);
        rig.aim_at(0.0, 0.25);

        let frame = rig.run(&pitch_node(&clips).with_weight(0.0));

        assert_unchanged(&rig, &frame);
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

    #[test]
    fn a_clip_that_is_not_additive_leaves_the_input_bit_for_bit_and_warns_once() {
        let mut rig = Rig::new();
        let plain = rig.add_clip("plain", held(&level(), 1.0));
        let node = with_points(&[AimPoint::new(plain, 0.0, 0.0)]);
        rig.aim_at(0.0, 0.0);
        let mut state = rig.start(&node);

        let frame = rig.step(&node, &mut state, 0.0);

        assert_unchanged(&rig, &frame);
        assert!(state.warned_clip);
    }

    #[test]
    fn a_frame_the_node_cannot_resolve_leaves_the_input_and_warns_once() {
        let mut rig = Rig::new();
        let clips = pitch_clips(&mut rig);
        let aiming = pitch_node(&clips);
        rig.aim_at(0.0, 0.25);
        let unresolvable = [
            AimOffsetNode {
                frame_bone: "missing".into(),
                ..aiming.clone()
            },
            AimOffsetNode {
                frame_bone: "prop".into(),
                ..aiming.clone()
            },
            aiming.clone().with_forward(Vec3::Y),
            aiming.clone().with_forward(Vec3::ZERO),
        ];

        for node in &unresolvable {
            let mut state = rig.start(node);
            assert_unchanged(&rig, &rig.step(node, &mut state, 0.0));
            assert!(state.warned);
        }
        let mut state = rig.start(&aiming);
        assert_unchanged(&rig, &rig.evaluate(&aiming, &mut state, false));
        assert!(state.warned);
    }
}

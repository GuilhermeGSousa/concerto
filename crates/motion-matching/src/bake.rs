use std::f32::consts::{PI, TAU};

use concerto_animation::clip::AnimationClip;
use concerto_animation::mirror::PoseMirror;
use concerto_foundation::{math::spring_damper::angular_velocity, transform::Transform};
use concerto_mesh::{JointName, JointPose, Skeleton};
use glam::{Quat, Vec3};

use crate::{
    channel::{BodyMotion, FrameContext},
    database::{BakeError, BakedDatabase, MotionDatabase, Span},
    definition::{SimulationBoneSettings, TrajectorySettings},
    filter::savitzky_golay,
    simulation::TrajectorySample,
};

/// Where one entry's frames sit in the database and which clip time its first frame samples.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EntryPlan {
    /// The clip time, in seconds, the entry's first frame samples.
    pub start: f32,
    /// The index of the entry's first frame in the database.
    pub first_frame: usize,
    /// The number of frames the entry bakes.
    pub frames: usize,
}

impl EntryPlan {
    /// The clip time that frame `index` of this entry samples.
    pub fn frame_time(&self, index: usize, rate: f32, speed: f32) -> f32 {
        self.start + index as f32 * speed / rate
    }
}

impl MotionDatabase {
    /// Checks the rate, speed, clips and ranges of a database and lays out the frames of every entry.
    ///
    /// # Errors
    ///
    /// Returns [`BakeError::InvalidRate`], [`BakeError::InvalidSpeed`] or
    /// [`BakeError::InvalidSampleDt`] for a value that is not positive and
    /// finite, [`BakeError::ClipCountMismatch`] when `clips` does not hold one
    /// clip per entry, and for the first entry that fails,
    /// [`BakeError::ClipHasNoBones`], [`BakeError::InvalidRange`] or
    /// [`BakeError::RangeTooShort`].
    pub fn plan_entries(
        &self,
        skeleton: &Skeleton,
        clips: &[&AnimationClip],
    ) -> Result<Vec<EntryPlan>, BakeError> {
        if !(self.rate.is_finite() && self.rate > 0.0) {
            return Err(BakeError::InvalidRate(self.rate));
        }
        if !(self.speed.is_finite() && self.speed > 0.0) {
            return Err(BakeError::InvalidSpeed(self.speed));
        }
        if !(self.trajectory.sample_dt.is_finite() && self.trajectory.sample_dt > 0.0) {
            return Err(BakeError::InvalidSampleDt(self.trajectory.sample_dt));
        }
        if clips.len() != self.entries.len() {
            return Err(BakeError::ClipCountMismatch {
                expected: self.entries.len(),
                actual: clips.len(),
            });
        }

        let mut plans = Vec::with_capacity(clips.len());
        let mut first_frame = 0;
        for (entry, (spec, clip)) in self.entries.iter().zip(clips).enumerate() {
            if !skeleton
                .bone_ids
                .iter()
                .any(|id| clip.get_channels(id).is_some())
            {
                return Err(BakeError::ClipHasNoBones { entry });
            }
            let (start, end) = match spec.range {
                Some(range) => (range.start, range.end),
                None => (0.0, clip.duration()),
            };
            let valid = start.is_finite()
                && end.is_finite()
                && start >= 0.0
                && end <= clip.duration()
                && start < end;
            if !valid {
                return Err(BakeError::InvalidRange { entry });
            }
            let frames = ((end - start) * self.rate / self.speed).floor() as usize;
            if frames < 2 {
                return Err(BakeError::RangeTooShort { entry, frames });
            }
            plans.push(EntryPlan {
                start,
                first_frame,
                frames,
            });
            first_frame += frames;
        }
        Ok(plans)
    }

    /// [`bake`](Self::bake), calling `on_entry(finished, total)` after each entry, with `finished` counting from 1.
    ///
    /// # Errors
    ///
    /// Fails as [`bake`](Self::bake) does.
    pub fn bake_with_progress(
        &self,
        skeleton: &Skeleton,
        clips: &[&AnimationClip],
        mut on_entry: impl FnMut(usize, usize),
    ) -> Result<BakedDatabase, BakeError> {
        let plans = self.plan_entries(skeleton, clips)?;
        let mirror = self.mirror(skeleton)?;
        check_window("position_window", self.simulation.position_window)?;
        check_window("facing_window", self.simulation.facing_window)?;
        find_joint(skeleton, self.simulation.position_joint)?;
        find_joint(skeleton, self.simulation.facing_joint)?;
        let channels = self.live_channels()?;
        for (stored, channel) in self.channels.iter().zip(&channels) {
            channel
                .validate(skeleton, &self.trajectory)
                .map_err(|reason| BakeError::ChannelValue {
                    type_name: stored.type_name().to_owned(),
                    reason,
                })?;
        }
        let rate = self.rate;

        let bone_count = skeleton.bone_count();
        let frame_count: usize = plans.iter().map(|plan| plan.frames).sum();

        let mut spans = Vec::with_capacity(channels.len());
        let mut feature_dims = 0;
        for channel in &channels {
            let span = Span::new(feature_dims, channel.dims());
            feature_dims += span.dims();
            spans.push(span);
        }

        let mut features = vec![0.0; frame_count * feature_dims];
        let mut range_starts = Vec::with_capacity(plans.len());
        let mut range_stops = Vec::with_capacity(plans.len());
        let mut clip_starts = Vec::with_capacity(plans.len());
        let mut simulation_frames = Vec::with_capacity(frame_count);

        for (index, (plan, clip)) in plans.iter().zip(clips).enumerate() {
            let mut locals: Vec<Vec<JointPose>> = (0..plan.frames)
                .map(|index| sample_pose(skeleton, clip, plan.frame_time(index, rate, self.speed)))
                .collect();
            if let Some(mirror) = mirror.as_ref().filter(|_| self.entries[index].mirrored) {
                for local in &mut locals {
                    let sampled = local.clone();
                    mirror
                        .mirror_into(&sampled, local)
                        .map_err(BakeError::Mirror)?;
                }
            }
            let models: Vec<Vec<Transform>> = locals
                .iter()
                .map(|local| model_pose(skeleton, local))
                .collect();
            let simulation = self.simulation.transforms(skeleton, &models)?;

            let mut stored_locals = locals.clone();
            for ((local, model), simulation) in
                stored_locals.iter_mut().zip(&models).zip(&simulation)
            {
                reexpress_roots(skeleton, local, model, simulation.clone());
            }

            let motion = body_motion(&simulation, rate);
            let model_speeds = model_velocities(&models, rate);

            for frame in 0..plan.frames {
                simulation_frames.push((simulation[frame].translation, simulation[frame].rotation));

                let trajectory = self.trajectory.samples(&simulation, &motion, rate, frame);
                let context = FrameContext {
                    skeleton,
                    local_pose: &stored_locals[frame],
                    model_pose: &models[frame],
                    model_velocities: &model_speeds[frame],
                    simulation: simulation[frame].clone(),
                    body: motion[frame],
                    trajectory: &trajectory,
                };
                let row = (plan.first_frame + frame) * feature_dims;
                context.extract_features(&channels, &mut features[row..row + feature_dims]);
            }
            range_starts.push(plan.first_frame);
            range_stops.push(plan.first_frame + plan.frames);
            clip_starts.push(plan.start);
            on_entry(index + 1, plans.len());
        }

        let (offset, deviations) = feature_statistics(&features, frame_count, feature_dims, &spans);

        Ok(BakedDatabase::new(
            bone_count,
            frame_count,
            range_starts,
            range_stops,
            clip_starts,
            simulation_frames,
            features,
            feature_dims,
            spans,
            offset,
            deviations,
        ))
    }

    /// Bakes the computed part of a motion database whose channels are loaded, from the skeleton and one resolved clip per entry.
    ///
    /// # Errors
    ///
    /// Returns the first [`BakeError`] found: an invalid setting or entry as
    /// [`MotionDatabase::plan_entries`] reports it, an invalid smoothing window, a joint the
    /// skeleton lacks, a channel that is not loaded or fails its
    /// [`validate`](crate::channel::FeatureChannel::validate), or a pose that
    /// cannot be mirrored.
    pub fn bake(
        &self,
        skeleton: &Skeleton,
        clips: &[&AnimationClip],
    ) -> Result<BakedDatabase, BakeError> {
        self.bake_with_progress(skeleton, clips, |_, _| {})
    }
}

/// The skeleton's rest pose with every bone's channels in `clip` sampled at `time`.
pub fn sample_pose(skeleton: &Skeleton, clip: &AnimationClip, time: f32) -> Vec<JointPose> {
    let mut pose = skeleton.rest_pose.to_vec();
    for (joint, id) in pose.iter_mut().zip(skeleton.bone_ids.iter()) {
        if let Some(channels) = clip.get_channels(id) {
            for channel in channels {
                channel.sample_transform(time, joint);
            }
        }
    }
    pose
}

/// The model-space transform of every bone for the given local poses.
pub fn model_pose(skeleton: &Skeleton, local: &[JointPose]) -> Vec<Transform> {
    let mut model: Vec<Transform> = Vec::with_capacity(local.len());
    for (pose, parent) in local.iter().zip(skeleton.parents.iter()) {
        let transform =
            Transform::from_translation_rotation_scale(pose.translation, pose.rotation, pose.scale);
        let transform = match parent {
            Some(parent) => model[usize::from(*parent)].mul_transform(transform),
            None => transform,
        };
        model.push(transform);
    }
    model
}

fn check_window(name: &'static str, window: usize) -> Result<(), BakeError> {
    if window.is_multiple_of(2) || window <= 3 {
        return Err(BakeError::InvalidWindow { name, window });
    }
    Ok(())
}

fn find_joint(skeleton: &Skeleton, joint: JointName) -> Result<usize, BakeError> {
    skeleton
        .bone_index(joint.0)
        .ok_or_else(|| BakeError::MissingJoint(joint.0.to_string()))
}

fn smooth(values: &[f32], window: usize) -> Result<Vec<f32>, BakeError> {
    let mut window = window.min(values.len());
    if window.is_multiple_of(2) {
        window = window.saturating_sub(1);
    }
    if window <= 3 {
        return Ok(values.to_vec());
    }
    savitzky_golay(values, window, 3)
}

/// Rewrites each parentless bone's local pose as `inverse(simulation) * model`.
pub fn reexpress_roots(
    skeleton: &Skeleton,
    local: &mut [JointPose],
    model: &[Transform],
    simulation: Transform,
) {
    let inverse_rotation = simulation.rotation.inverse();
    let inverse = Transform::from_translation_rotation(
        inverse_rotation * -simulation.translation,
        inverse_rotation,
    );
    for ((pose, parent), model) in local.iter_mut().zip(skeleton.parents.iter()).zip(model) {
        if parent.is_none() {
            let relative = inverse.mul_transform(model.clone());
            *pose = JointPose {
                translation: relative.translation,
                rotation: relative.rotation,
                scale: relative.scale,
            };
        }
    }
}

fn difference_span(frame: usize, frames: usize) -> (usize, usize) {
    (frame.saturating_sub(1), (frame + 1).min(frames - 1))
}

fn difference_dt(previous: usize, next: usize, rate: f32) -> f32 {
    (next - previous) as f32 / rate
}

/// Model-space linear bone velocities per frame and bone, by central difference and one-sided at the ends.
pub fn model_velocities(model: &[Vec<Transform>], rate: f32) -> Vec<Vec<Vec3>> {
    let frames = model.len();
    (0..frames)
        .map(|frame| {
            let (previous, next) = difference_span(frame, frames);
            let bones = model[frame].len();
            if previous == next {
                return vec![Vec3::ZERO; bones];
            }
            let dt = difference_dt(previous, next, rate);
            (0..bones)
                .map(|bone| {
                    (model[next][bone].translation - model[previous][bone].translation) / dt
                })
                .collect()
        })
        .collect()
}

/// The simulation bone's linear and angular velocity per frame, by central difference and one-sided at the ends.
pub fn body_motion(simulation: &[Transform], rate: f32) -> Vec<BodyMotion> {
    let frames = simulation.len();
    (0..frames)
        .map(|frame| {
            let (previous, next) = difference_span(frame, frames);
            if previous == next {
                return BodyMotion::default();
            }
            let dt = difference_dt(previous, next, rate);
            BodyMotion {
                velocity: (simulation[next].translation - simulation[previous].translation) / dt,
                angular_velocity: angular_velocity(
                    simulation[next].rotation,
                    simulation[previous].rotation,
                    dt,
                ),
            }
        })
        .collect()
}

impl MotionDatabase {
    fn mirror(&self, skeleton: &Skeleton) -> Result<Option<PoseMirror>, BakeError> {
        if !self.entries.iter().any(|entry| entry.mirrored) {
            return Ok(None);
        }
        PoseMirror::new(skeleton, self.mirror_axis.normal())
            .map(Some)
            .map_err(BakeError::Mirror)
    }
}

// todo: dont return tuples its gross
fn feature_statistics(
    features: &[f32],
    frame_count: usize,
    feature_dims: usize,
    spans: &[Span],
) -> (Vec<f32>, Vec<f32>) {
    if frame_count == 0 {
        return (vec![0.0; feature_dims], vec![0.0; spans.len()]);
    }
    let count = frame_count as f64;
    let mut means = vec![0.0f64; feature_dims];
    for row in features.chunks_exact(feature_dims.max(1)) {
        for (mean, value) in means.iter_mut().zip(row) {
            *mean += f64::from(*value);
        }
    }
    for mean in &mut means {
        *mean /= count;
    }
    let mut variances = vec![0.0f64; feature_dims];
    for row in features.chunks_exact(feature_dims.max(1)) {
        for ((variance, mean), value) in variances.iter_mut().zip(&means).zip(row) {
            let delta = f64::from(*value) - mean;
            *variance += delta * delta;
        }
    }
    let deviations = spans
        .iter()
        .map(|span| {
            if span.dims() == 0 {
                return 0.0;
            }
            let sum: f64 = variances[span.range()]
                .iter()
                .map(|variance| (variance / count).sqrt())
                .sum();
            (sum / span.dims() as f64) as f32
        })
        .collect();
    (
        means.into_iter().map(|mean| mean as f32).collect(),
        deviations,
    )
}

impl SimulationBoneSettings {
    /// The simulation bone's model-space transform for every frame of one range, given each frame's model-space bone transforms.
    ///
    /// # Errors
    ///
    /// Returns [`BakeError::InvalidWindow`] for a smoothing window that is even
    /// or not greater than 3, and [`BakeError::MissingJoint`] when the skeleton
    /// lacks the position or facing joint.
    ///
    /// # Panics
    ///
    /// Panics if a pose in `model_poses` has fewer bones than `skeleton`.
    pub fn transforms(
        &self,
        skeleton: &Skeleton,
        model_poses: &[Vec<Transform>],
    ) -> Result<Vec<Transform>, BakeError> {
        check_window("position_window", self.position_window)?;
        check_window("facing_window", self.facing_window)?;
        let position_joint = find_joint(skeleton, self.position_joint)?;
        let facing_joint = find_joint(skeleton, self.facing_joint)?;
        if model_poses.is_empty() {
            return Ok(Vec::new());
        }

        let xs: Vec<f32> = model_poses
            .iter()
            .map(|pose| pose[position_joint].translation.x)
            .collect();
        let zs: Vec<f32> = model_poses
            .iter()
            .map(|pose| pose[position_joint].translation.z)
            .collect();
        let xs = smooth(&xs, self.position_window)?;
        let zs = smooth(&zs, self.position_window)?;

        let mut yaws: Vec<f32> = Vec::with_capacity(model_poses.len());
        for pose in model_poses {
            let direction = pose[facing_joint].rotation * self.facing_axis.vector();
            let flat = Vec3::new(direction.x, 0.0, direction.z);
            let previous = yaws.last().copied();
            let yaw = if flat.length() > 1e-6 {
                let mut yaw = flat.x.atan2(flat.z);
                if let Some(previous) = previous {
                    while yaw - previous > PI {
                        yaw -= TAU;
                    }
                    while yaw - previous < -PI {
                        yaw += TAU;
                    }
                }
                yaw
            } else {
                previous.unwrap_or(0.0)
            };
            yaws.push(yaw);
        }
        let yaws = smooth(&yaws, self.facing_window)?;

        Ok(xs
            .iter()
            .zip(&zs)
            .zip(&yaws)
            .map(|((x, z), yaw)| {
                Transform::from_translation_rotation(
                    Vec3::new(*x, 0.0, *z),
                    Quat::from_rotation_y(*yaw),
                )
            })
            .collect())
    }
}

impl TrajectorySettings {
    /// The trajectory samples ahead of `frame` of one range, clamped to the range's last frame.
    ///
    /// # Panics
    ///
    /// Panics if `simulation` is empty or `motion` is shorter than it.
    pub fn samples(
        &self,
        simulation: &[Transform],
        motion: &[BodyMotion],
        rate: f32,
        frame: usize,
    ) -> Vec<TrajectorySample> {
        let last = simulation.len() - 1;
        (0..self.sample_count)
            .map(|k| {
                let target = frame.saturating_add(self.frame_offset(k, rate)).min(last);
                TrajectorySample {
                    position: simulation[target].translation,
                    velocity: motion[target].velocity,
                    rotation: simulation[target].rotation,
                }
            })
            .collect()
    }
}

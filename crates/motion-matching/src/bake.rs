use std::f32::consts::{PI, TAU};

use concerto_animation::clip::AnimationClip;
use concerto_foundation::{math::spring_damper::angular_velocity, transform::Transform};
use concerto_mesh::{JointPose, Skeleton};
use glam::{Quat, Vec3};

use concerto_reflect::TypeRegistry;

use crate::{
    channel::{build_channels, extract_features, BodyMotion, FrameContext},
    database::{BakeError, ChannelSpan, MotionDatabase},
    definition::{MotionDatabaseDefinition, SimulationBoneSettings, TrajectorySettings},
    filter::savitzky_golay,
    simulation::TrajectorySample,
};

/// Where one entry's frames sit in the database and which clip time its first frame samples.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EntryPlan {
    pub start: f32,
    pub first_frame: usize,
    pub frames: usize,
}

impl EntryPlan {
    /// The clip time that frame `index` of this entry samples.
    pub fn frame_time(&self, index: usize, rate: f32, speed: f32) -> f32 {
        self.start + index as f32 * speed / rate
    }

    /// The database frames of this entry as a half-open range.
    pub fn range(&self) -> std::ops::Range<usize> {
        self.first_frame..self.first_frame + self.frames
    }
}

/// Checks the rate, speed, clips and ranges of a definition and lays out the frames of every entry.
pub fn plan_entries(
    definition: &MotionDatabaseDefinition,
    skeleton: &Skeleton,
    clips: &[&AnimationClip],
) -> Result<Vec<EntryPlan>, BakeError> {
    if !(definition.rate.is_finite() && definition.rate > 0.0) {
        return Err(BakeError::InvalidRate(definition.rate));
    }
    if !(definition.speed.is_finite() && definition.speed > 0.0) {
        return Err(BakeError::InvalidSpeed(definition.speed));
    }
    if !(definition.trajectory.sample_dt.is_finite() && definition.trajectory.sample_dt > 0.0) {
        return Err(BakeError::InvalidSampleDt(definition.trajectory.sample_dt));
    }
    if clips.len() != definition.entries.len() {
        return Err(BakeError::ClipCountMismatch {
            expected: definition.entries.len(),
            actual: clips.len(),
        });
    }

    let mut plans = Vec::with_capacity(clips.len());
    let mut first_frame = 0;
    for (entry, (spec, clip)) in definition.entries.iter().zip(clips).enumerate() {
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
        let frames = ((end - start) * definition.rate / definition.speed).floor() as usize;
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

fn find_joint(skeleton: &Skeleton, name: &str) -> Result<usize, BakeError> {
    skeleton
        .bone_index(name)
        .ok_or_else(|| BakeError::MissingJoint(name.to_string()))
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

/// The simulation bone's model-space transform for every frame of one range, given each frame's model-space bone transforms.
pub fn simulation_bone(
    settings: &SimulationBoneSettings,
    skeleton: &Skeleton,
    model_poses: &[Vec<Transform>],
) -> Result<Vec<Transform>, BakeError> {
    check_window("position_window", settings.position_window)?;
    check_window("facing_window", settings.facing_window)?;
    let position_joint = find_joint(skeleton, &settings.position_joint)?;
    let facing_joint = find_joint(skeleton, &settings.facing_joint)?;
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
    let xs = smooth(&xs, settings.position_window)?;
    let zs = smooth(&zs, settings.position_window)?;

    let mut yaws: Vec<f32> = Vec::with_capacity(model_poses.len());
    for pose in model_poses {
        let direction = pose[facing_joint].rotation * settings.facing_axis;
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
    let yaws = smooth(&yaws, settings.facing_window)?;

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

/// The trajectory samples ahead of `frame` of one range, clamped to the range's last frame.
pub fn trajectory_samples(
    settings: &TrajectorySettings,
    simulation: &[Transform],
    motion: &[BodyMotion],
    rate: f32,
    frame: usize,
) -> Vec<TrajectorySample> {
    let last = simulation.len() - 1;
    (0..settings.sample_count)
        .map(|k| {
            let offset = (settings.sample_dt * (k + 1) as f32 * rate).round() as usize;
            let target = frame.saturating_add(offset).min(last);
            TrajectorySample {
                position: simulation[target].translation,
                velocity: motion[target].velocity,
                rotation: simulation[target].rotation,
            }
        })
        .collect()
}

/// Bakes a motion database from its definition, the skeleton, one resolved clip per entry and the registry that knows the feature channel types.
pub fn bake(
    definition: &MotionDatabaseDefinition,
    skeleton: &Skeleton,
    clips: &[&AnimationClip],
    registry: &TypeRegistry,
) -> Result<MotionDatabase, BakeError> {
    let plans = plan_entries(definition, skeleton, clips)?;
    check_window("position_window", definition.simulation.position_window)?;
    check_window("facing_window", definition.simulation.facing_window)?;
    find_joint(skeleton, &definition.simulation.position_joint)?;
    find_joint(skeleton, &definition.simulation.facing_joint)?;
    let channels = build_channels(&definition.channels, registry)?;
    let rate = definition.rate;

    let bone_count = skeleton.bone_count();
    let frame_count: usize = plans.iter().map(|plan| plan.frames).sum();

    let mut spans = Vec::with_capacity(channels.len());
    let mut feature_dims = 0;
    for (stored, channel) in definition.channels.iter().zip(&channels) {
        let dims = channel.dims();
        spans.push(ChannelSpan {
            type_name: stored.type_name.clone(),
            offset: feature_dims,
            dims,
        });
        feature_dims += dims;
    }

    let mut features = vec![0.0; frame_count * feature_dims];
    let mut range_starts = Vec::with_capacity(plans.len());
    let mut range_stops = Vec::with_capacity(plans.len());
    let mut clip_starts = Vec::with_capacity(plans.len());
    let mut simulation_frames = Vec::with_capacity(frame_count);

    for (plan, clip) in plans.iter().zip(clips) {
        let locals: Vec<Vec<JointPose>> = (0..plan.frames)
            .map(|index| {
                sample_pose(
                    skeleton,
                    clip,
                    plan.frame_time(index, rate, definition.speed),
                )
            })
            .collect();
        let models: Vec<Vec<Transform>> = locals
            .iter()
            .map(|local| model_pose(skeleton, local))
            .collect();
        let simulation = simulation_bone(&definition.simulation, skeleton, &models)?;

        let mut stored_locals = locals.clone();
        for ((local, model), simulation) in stored_locals.iter_mut().zip(&models).zip(&simulation) {
            reexpress_roots(skeleton, local, model, simulation.clone());
        }

        let motion = body_motion(&simulation, rate);
        let model_speeds = model_velocities(&models, rate);

        for frame in 0..plan.frames {
            simulation_frames.push((simulation[frame].translation, simulation[frame].rotation));

            let trajectory =
                trajectory_samples(&definition.trajectory, &simulation, &motion, rate, frame);
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
            extract_features(&channels, &context, &mut features[row..row + feature_dims]);
        }
        range_starts.push(plan.first_frame);
        range_stops.push(plan.first_frame + plan.frames);
        clip_starts.push(plan.start);
    }

    Ok(MotionDatabase::new(
        definition.skeleton.clone(),
        bone_count,
        definition
            .entries
            .iter()
            .map(|entry| entry.clip.clone())
            .collect(),
        rate,
        definition.speed,
        definition.trajectory,
        frame_count,
        range_starts,
        range_stops,
        clip_starts,
        simulation_frames,
        features,
        feature_dims,
        spans,
    ))
}

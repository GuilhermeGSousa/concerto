use std::f32::consts::{PI, TAU};

use concerto_animation::clip::AnimationClip;
use concerto_foundation::math::spring_damper::angular_velocity;
use concerto_foundation::transform::Transform;
use concerto_mesh::{JointPose, Skeleton};
use glam::{Quat, Vec3};

use crate::{
    database::BakeError,
    definition::{ContactSettings, MotionDatabaseDefinition, SimulationBoneSettings},
    filter::{median_filter, savitzky_golay},
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
    if window % 2 == 0 || window <= 3 {
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
    if window % 2 == 0 {
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

/// Local-space linear and angular bone velocities per frame and bone, by central difference and one-sided at the ends.
pub fn local_velocities(local: &[Vec<JointPose>], rate: f32) -> (Vec<Vec<Vec3>>, Vec<Vec<Vec3>>) {
    let frames = local.len();
    let mut linear = Vec::with_capacity(frames);
    let mut angular = Vec::with_capacity(frames);
    for frame in 0..frames {
        let (previous, next) = difference_span(frame, frames);
        let bones = local[frame].len();
        if previous == next {
            linear.push(vec![Vec3::ZERO; bones]);
            angular.push(vec![Vec3::ZERO; bones]);
            continue;
        }
        let dt = difference_dt(previous, next, rate);
        linear.push(
            (0..bones)
                .map(|bone| {
                    (local[next][bone].translation - local[previous][bone].translation) / dt
                })
                .collect(),
        );
        angular.push(
            (0..bones)
                .map(|bone| {
                    angular_velocity(
                        local[next][bone].rotation,
                        local[previous][bone].rotation,
                        dt,
                    )
                })
                .collect(),
        );
    }
    (linear, angular)
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

/// Per frame and contact bone, whether the bone's model-space speed is below the threshold, after a median filter over the range.
pub fn contacts(
    settings: &ContactSettings,
    skeleton: &Skeleton,
    model_velocities: &[Vec<Vec3>],
) -> Result<Vec<Vec<bool>>, BakeError> {
    let joints = settings
        .bones
        .iter()
        .map(|name| find_joint(skeleton, name))
        .collect::<Result<Vec<_>, _>>()?;
    let mut result = vec![Vec::with_capacity(joints.len()); model_velocities.len()];
    for joint in joints {
        let touching: Vec<bool> = model_velocities
            .iter()
            .map(|velocities| velocities[joint].length() < settings.speed_threshold)
            .collect();
        for (frame, value) in median_filter(&touching, settings.median_window)
            .into_iter()
            .enumerate()
        {
            result[frame].push(value);
        }
    }
    Ok(result)
}

use std::collections::HashMap;

use concerto_animation::{
    blackboard::AnimationBlackboard,
    clip::AnimationClip,
    inertialization::{Inertializer, PoseVelocity},
    pose::{FiredEvent, Pose, RootMotion},
};
use concerto_foundation::{
    math::spring_damper::{angular_velocity, from_scaled_angle_axis},
    transform::Transform,
};
use concerto_mesh::skeleton::Skeleton;
use glam::{Quat, Vec3};
use log::error;
use uuid::Uuid;

use crate::{
    channel::BodyMotion,
    database::{BakeError, MotionDatabase},
    node::{trajectory_key, MotionMatchingNode, MotionMatchingTrajectory},
    query::FeatureQuery,
    search::{cost_of, search},
    simulation::TrajectorySample,
};

/// One search's raw query and what it was built from, so a drawn query is a single moment in time.
#[derive(Clone)]
pub struct LastQuery {
    pub features: Vec<f32>,
    pub frame: usize,
    pub simulation: Transform,
    pub trajectory: Vec<TrajectorySample>,
    pub best_cost: Option<f32>,
}

/// The per-player state of a motion matching node.
pub struct MotionMatchingState {
    initialised: bool,
    failed: bool,
    delta_time: f32,
    play_rate: f32,
    frame: usize,
    sub_frame_time: f32,
    search_timer: f32,
    forced_search_timer: f32,
    desired_velocity: Vec3,
    desired_rotation: Quat,
    velocity_change: f32,
    rotation_change: f32,
    pose: Pose,
    velocity: PoseVelocity,
    blended: Pose,
    previous_blended: Pose,
    last_delta: f32,
    inertializer: Option<Inertializer>,
    output: Pose,
    query: Option<FeatureQuery>,
    last_query: Option<LastQuery>,
    bone_indices: HashMap<Uuid, usize>,
    root_motion: BodyMotion,
    fired: Vec<FiredEvent>,
}

impl MotionMatchingState {
    pub(crate) fn new(search_period: f32) -> Self {
        Self {
            initialised: false,
            failed: false,
            delta_time: 0.0,
            play_rate: 1.0,
            frame: 0,
            sub_frame_time: 0.0,
            search_timer: search_period,
            forced_search_timer: 0.0,
            desired_velocity: Vec3::ZERO,
            desired_rotation: Quat::IDENTITY,
            velocity_change: 0.0,
            rotation_change: 0.0,
            pose: Pose::identity(0),
            velocity: PoseVelocity::zero(0),
            blended: Pose::identity(0),
            previous_blended: Pose::identity(0),
            last_delta: 0.0,
            inertializer: None,
            output: Pose::identity(0),
            query: None,
            last_query: None,
            bone_indices: HashMap::new(),
            root_motion: BodyMotion::default(),
            fired: Vec::new(),
        }
    }

    /// Whether the node has played its first frame.
    pub fn is_initialised(&self) -> bool {
        self.initialised
    }

    /// Whether the node stopped on an error; it stays stopped.
    pub fn has_failed(&self) -> bool {
        self.failed
    }

    /// The database frame the node plays.
    pub fn frame(&self) -> usize {
        self.frame
    }

    /// The local pose of the frame the node plays, before blending.
    pub fn pose(&self) -> &Pose {
        &self.pose
    }

    /// The live feature channels the node queries with, once initialised.
    pub fn query(&self) -> Option<&FeatureQuery> {
        self.query.as_ref()
    }

    /// The last search's raw query and what it was built from.
    pub fn last_query(&self) -> Option<&LastQuery> {
        self.last_query.as_ref()
    }

    pub(crate) fn clear_root_motion(&mut self) {
        self.output.set_root_motion(RootMotion::IDENTITY);
    }

    pub(crate) fn clear_fired(&mut self) {
        self.fired.clear();
    }

    pub(crate) fn fired(&self) -> &[FiredEvent] {
        &self.fired
    }

    fn fire(&mut self, sources: &Sources, from: usize, to: usize) {
        let database = sources.database;
        if to <= from {
            return;
        }
        let Some((entry, from_time)) = database.clip_time(from) else {
            return;
        };
        let Some((to_entry, to_time)) = database.clip_time(to) else {
            return;
        };
        if entry != to_entry {
            return;
        }
        let (Some(clip), Some(handle)) = (sources.clips.get(entry), database.clips().get(entry))
        else {
            return;
        };
        let mirrored = database.mirrored().get(entry).copied().unwrap_or(false);
        for key in clip.events_in(from_time, to_time, false) {
            self.fired.push(FiredEvent {
                clip: handle.id(),
                key: key as u32,
                time: clip.events()[key].time,
                weight: 1.0,
                mirrored,
            });
        }
    }

    /// Copies the node's pose into `output` and returns true, or returns false and leaves `output` untouched until the node is initialised.
    pub(crate) fn write_output(&self, output: &mut Pose) -> bool {
        if !self.initialised || self.output.len() != output.len() {
            return false;
        }
        output.copy_from(&self.output);
        true
    }

    fn root_delta(&self, sources: &Sources) -> RootMotion {
        let skeleton = sources.skeleton;
        let root = skeleton.parents.iter().position(Option::is_none);
        let Some((joint, rest)) = root.and_then(|root| {
            let played = skeleton
                .bone_ids
                .get(root)
                .and_then(|id| self.bone_indices.get(id))
                .and_then(|index| sources.player_skeleton.rest_pose.get(*index));
            self.pose
                .joints()
                .get(root)
                .zip(played.or(skeleton.rest_pose.get(root)))
        }) else {
            return RootMotion::IDENTITY;
        };
        let simulation = sources.database.simulation(self.frame).rotation;
        let turn_back = (simulation * joint.rotation * rest.rotation.inverse()).inverse();
        let dt = self.delta_time * self.play_rate;
        RootMotion {
            translation: turn_back * (self.root_motion.velocity * dt),
            rotation: from_scaled_angle_axis(turn_back * (self.root_motion.angular_velocity * dt)),
        }
    }

    fn scatter(&mut self, sources: &Sources) {
        for (bone, id) in sources.skeleton.bone_ids.iter().enumerate() {
            let Some(index) = self.bone_indices.get(id).copied() else {
                continue;
            };
            if let (Some(joint), Some(source)) = (
                self.output.get_joint_pose_mut(index),
                self.blended.joints().get(bone),
            ) {
                *joint = source.clone();
            }
        }
    }

    fn initialise(&mut self, sources: &Sources) -> Result<(), String> {
        let database = sources.database;
        let bone_count = database.bone_count();
        if sources.skeleton.bone_count() != bone_count {
            return Err(BakeError::BoneCountMismatch {
                expected: bone_count,
                actual: sources.skeleton.bone_count(),
            }
            .to_string());
        }
        let mut bone_indices = HashMap::new();
        for id in sources.skeleton.bone_ids.iter() {
            if let Some(index) = sources
                .player_skeleton
                .bone_ids
                .iter()
                .position(|player_id| player_id == id)
            {
                bone_indices.insert(*id, index);
            }
        }
        if bone_indices.is_empty() {
            return Err("no bone of the database skeleton is in the player's skeleton".into());
        }
        let query = FeatureQuery::new(database).map_err(|error| error.to_string())?;
        let frame = database
            .ranges()
            .next()
            .map(|range| range.start)
            .ok_or_else(|| "the database has no frames".to_string())?;
        let (pose, velocity) = sample(sources, frame).map_err(|error| error.to_string())?;

        self.output = Pose::from_joints(sources.player_skeleton.rest_pose.to_vec());
        self.inertializer = Some(Inertializer::new(bone_count));
        self.blended = copy_pose(&pose);
        self.previous_blended = copy_pose(&pose);
        self.pose = pose;
        self.velocity = velocity;
        self.frame = frame;
        self.sub_frame_time = 0.0;
        self.search_timer = 0.0;
        self.forced_search_timer = 0.0;
        self.last_delta = 0.0;
        self.velocity_change = 0.0;
        self.rotation_change = 0.0;
        self.query = Some(query);
        self.bone_indices = bone_indices;
        self.root_motion = database.body_motion(frame);
        self.scatter(sources);
        self.initialised = true;
        Ok(())
    }

    fn forced_search(
        &mut self,
        node: &MotionMatchingNode,
        first: &TrajectorySample,
        dt: f32,
    ) -> bool {
        let (velocity_change, rotation_change) = if dt > 0.0 {
            (
                (first.velocity - self.desired_velocity).length() / dt,
                angular_velocity(first.rotation, self.desired_rotation, dt).length(),
            )
        } else {
            (self.velocity_change, self.rotation_change)
        };
        let fired = self.forced_search_timer <= 0.0
            && ((self.velocity_change >= node.velocity_change_threshold
                && velocity_change < node.velocity_change_threshold)
                || (self.rotation_change >= node.rotation_change_threshold
                    && rotation_change < node.rotation_change_threshold));
        if fired {
            self.forced_search_timer = node.search_period;
        }
        self.velocity_change = velocity_change;
        self.rotation_change = rotation_change;
        self.desired_velocity = first.velocity;
        self.desired_rotation = first.rotation;
        fired
    }

    fn find_better_frame(
        &mut self,
        node: &MotionMatchingNode,
        trajectory: &MotionMatchingTrajectory,
        sources: &Sources,
    ) -> Result<Option<usize>, String> {
        let database = sources.database;
        let Some(query) = self.query.as_ref() else {
            return Ok(None);
        };
        let simulation =
            Transform::from_translation_rotation(trajectory.translation, trajectory.rotation);
        let samples = rescaled(trajectory, self.play_rate);
        let raw = match query.raw(
            database,
            sources.skeleton,
            self.frame,
            &simulation,
            &samples,
        ) {
            Ok(raw) => raw,
            Err(BakeError::TrajectoryTooShort { .. }) => {
                self.last_query = None;
                return Ok(None);
            }
            Err(error) => return Err(error.to_string()),
        };
        let mut features = raw.clone();
        database.normalise(&mut features);
        let end_of_range = database
            .range_of(self.frame)
            .is_some_and(|range| self.frame + 1 == range.end);
        let current = (!end_of_range).then_some(self.frame);
        let found = search(
            database,
            &features,
            current,
            node.ignore_surrounding,
            node.ignore_range_end,
        );
        self.last_query = Some(LastQuery {
            features: raw,
            frame: self.frame,
            simulation,
            trajectory: samples,
            best_cost: found.map(|(_, cost)| cost),
        });
        let Some((best, best_cost)) = found else {
            return Ok(None);
        };
        if end_of_range {
            return Ok((best != self.frame).then_some(best));
        }
        let differs = best.abs_diff(self.frame) > node.same_frame_tolerance;
        let Some(current_cost) = cost_of(database, &features, self.frame) else {
            return Ok(None);
        };
        Ok((best_cost < current_cost && differs).then_some(best))
    }

    fn run(
        &mut self,
        node: &MotionMatchingNode,
        blackboard: &AnimationBlackboard,
        sources: Sources,
    ) -> Result<(), String> {
        let started = self.initialised;
        if !started {
            self.initialise(&sources)?;
        }
        let database = sources.database;
        let dt = self.delta_time;
        let play_rate = self.play_rate;
        let rate = database.rate();
        let range = database
            .range_of(self.frame)
            .ok_or_else(|| "the state's frame left the database".to_string())?;

        self.search_timer -= dt;
        self.forced_search_timer -= dt;
        self.sub_frame_time += dt * play_rate;
        let whole = (self.sub_frame_time * rate).floor().max(0.0) as usize;
        self.sub_frame_time -= whole as f32 / rate;
        let previous_frame = self.frame;
        self.frame = (self.frame + whole).min(range.end - 1);
        let advanced = self.frame;
        let moved = self.frame != previous_frame;

        let trajectory = blackboard.get::<MotionMatchingTrajectory>(trajectory_key());
        let forced = trajectory
            .and_then(|trajectory| trajectory.samples.first())
            .is_some_and(|first| self.forced_search(node, first, dt));
        let timer_due = self.search_timer <= 0.0;
        if timer_due {
            self.search_timer = node.search_period;
        }
        let in_range_end = self.frame + node.ignore_range_end >= range.end;

        let mut better = None;
        if (timer_due || forced || in_range_end) && play_rate > 0.0 {
            if let Some(trajectory) = trajectory {
                better = self.find_better_frame(node, trajectory, &sources)?;
            }
        }

        if let Some(best) = better {
            let (pose, velocity) = sample(&sources, best).map_err(|error| error.to_string())?;
            let source_velocity = if self.last_delta > 0.0 {
                PoseVelocity::between(&self.previous_blended, &self.blended, self.last_delta)
            } else {
                self.velocity.scaled(play_rate)
            };
            if let Some(inertializer) = self.inertializer.as_mut() {
                inertializer.transition(
                    &self.blended,
                    &source_velocity,
                    &pose,
                    &velocity.scaled(play_rate),
                );
            }
            self.frame = best;
            self.pose = pose;
            self.velocity = velocity;
        } else if moved {
            let (pose, velocity) =
                sample(&sources, self.frame).map_err(|error| error.to_string())?;
            self.pose = pose;
            self.velocity = velocity;
        }

        self.previous_blended.copy_from(&self.blended);
        if let Some(inertializer) = self.inertializer.as_mut() {
            inertializer.update(
                &self.pose,
                &self.velocity,
                node.blend_halflife,
                dt,
                &mut self.blended,
            );
        }
        self.last_delta = dt;
        self.root_motion = database.body_motion(self.frame);
        let held = self.frame == previous_frame && self.frame + 1 == range.end;
        let delta = if held {
            RootMotion::IDENTITY
        } else {
            self.root_delta(&sources)
        };
        self.output.set_root_motion(delta);
        self.scatter(&sources);
        if started && better.is_none() {
            self.fire(&sources, previous_frame, advanced);
        }
        Ok(())
    }

    pub(crate) fn step(
        &mut self,
        node: &MotionMatchingNode,
        delta_time: f32,
        play_rate: f32,
        blackboard: &AnimationBlackboard,
        sources: Sources,
    ) {
        self.delta_time = delta_time;
        self.play_rate = play_rate;
        self.clear_root_motion();
        if self.failed {
            return;
        }
        if let Err(reason) = self.run(node, blackboard, sources) {
            error!("motion matching stopped: {reason}");
            self.failed = true;
            self.initialised = false;
        }
    }
}

pub(crate) struct Sources<'a> {
    pub database: &'a MotionDatabase,
    pub skeleton: &'a Skeleton,
    pub player_skeleton: &'a Skeleton,
    pub clips: Vec<&'a AnimationClip>,
}

fn rescaled(trajectory: &MotionMatchingTrajectory, play_rate: f32) -> Vec<TrajectorySample> {
    if play_rate == 1.0 {
        return trajectory.samples.clone();
    }
    trajectory
        .samples
        .iter()
        .map(|sample| TrajectorySample {
            position: trajectory.translation
                + (sample.position - trajectory.translation) / play_rate,
            velocity: sample.velocity / play_rate,
            rotation: sample.rotation,
        })
        .collect()
}

fn copy_pose(pose: &Pose) -> Pose {
    Pose::from_joints(pose.joints().to_vec())
}

fn sample(sources: &Sources, frame: usize) -> Result<(Pose, PoseVelocity), BakeError> {
    let database = sources.database;
    let range = database.range_of(frame).ok_or(BakeError::FrameOutOfRange {
        frame,
        frame_count: database.frame_count(),
    })?;
    let (first, second) = if frame + 1 < range.end {
        (frame, frame + 1)
    } else {
        (frame - 1, frame)
    };
    let first_pose = Pose::from_joints(database.pose(first, sources.skeleton, &sources.clips)?);
    let second_pose = Pose::from_joints(database.pose(second, sources.skeleton, &sources.clips)?);
    let velocity = PoseVelocity::between(&first_pose, &second_pose, 1.0 / database.rate());
    let pose = if first == frame {
        first_pose
    } else {
        second_pose
    };
    Ok((pose, velocity))
}

#[cfg(test)]
mod tests;

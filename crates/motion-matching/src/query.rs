use concerto_foundation::transform::Transform;
use concerto_mesh::Skeleton;

use crate::{
    channel::{BodyMotion, ChannelSource, FeatureChannel, FrameContext},
    database::{BakeError, MotionDatabase},
    simulation::TrajectorySample,
};

/// The live feature channels of a database, built once and used to make query vectors.
pub struct FeatureQuery {
    channels: Vec<Box<dyn FeatureChannel>>,
}

impl FeatureQuery {
    /// Copies the live channels of a loaded database; a channel not yet loaded is an error.
    pub fn new(database: &MotionDatabase) -> Result<Self, BakeError> {
        Ok(Self {
            channels: database
                .live_channels()?
                .into_iter()
                .map(|channel| channel.dyn_clone())
                .collect(),
        })
    }

    /// The live channels, in the database's order.
    pub fn channels(&self) -> &[Box<dyn FeatureChannel>] {
        &self.channels
    }

    /// The normalised feature vector for a character whose pose is database frame `pose_frame` and whose predicted path is `trajectory`, expressed in `simulation`.
    pub fn build(
        &self,
        database: &MotionDatabase,
        skeleton: &Skeleton,
        pose_frame: usize,
        simulation: &Transform,
        trajectory: &[TrajectorySample],
    ) -> Result<Vec<f32>, BakeError> {
        let mut features = self.raw(database, skeleton, pose_frame, simulation, trajectory)?;
        database.normalise(&mut features);
        Ok(features)
    }

    /// The vector `build` normalises, in the units `preview_features` reads: pose channels as baked for `pose_frame`, and trajectory channels from `trajectory` expressed in `simulation`.
    pub fn raw(
        &self,
        database: &MotionDatabase,
        skeleton: &Skeleton,
        pose_frame: usize,
        simulation: &Transform,
        trajectory: &[TrajectorySample],
    ) -> Result<Vec<f32>, BakeError> {
        if skeleton.bone_count() != database.bone_count() {
            return Err(BakeError::BoneCountMismatch {
                expected: database.bone_count(),
                actual: skeleton.bone_count(),
            });
        }
        if self.channels.len() != database.channels().len() {
            return Err(BakeError::ChannelCountMismatch {
                expected: self.channels.len(),
                actual: database.channels().len(),
            });
        }
        if pose_frame >= database.frame_count() {
            return Err(BakeError::FrameOutOfRange {
                frame: pose_frame,
                frame_count: database.frame_count(),
            });
        }
        let needed = database.trajectory().sample_count;
        let reads_trajectory = database
            .channels()
            .iter()
            .any(|span| span.source() == ChannelSource::Trajectory);
        if reads_trajectory && trajectory.len() < needed {
            return Err(BakeError::TrajectoryTooShort {
                needed,
                actual: trajectory.len(),
            });
        }

        let context = FrameContext {
            skeleton,
            local_pose: &[],
            model_pose: &[],
            model_velocities: &[],
            simulation: simulation.clone(),
            body: BodyMotion::default(),
            trajectory,
        };
        let mut features = database.features(pose_frame).to_vec();
        for (span, channel) in database.channels().iter().zip(&self.channels) {
            if span.source() == ChannelSource::Trajectory {
                channel.extract(&context, &mut features[span.range()]);
            }
        }
        Ok(features)
    }
}

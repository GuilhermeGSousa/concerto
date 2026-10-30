use concerto_foundation::transform::Transform;
use concerto_mesh::Skeleton;
use concerto_reflect::TypeRegistry;

use crate::{
    channel::{build_channels, BodyMotion, ChannelSource, FeatureChannel, FrameContext},
    database::{BakeError, MotionDatabase},
    definition::StoredChannel,
    simulation::TrajectorySample,
};

/// The live feature channels of a database, built once and used to make normalised query vectors.
pub struct FeatureQuery {
    channels: Vec<Box<dyn FeatureChannel>>,
}

impl FeatureQuery {
    /// Builds the live channels from the values stored in the database's spans; a type the registry lacks is an error.
    pub fn new(database: &MotionDatabase, registry: &TypeRegistry) -> Result<Self, BakeError> {
        let stored: Vec<StoredChannel> = database
            .channels()
            .iter()
            .map(|span| StoredChannel {
                type_name: span.type_name.clone(),
                value: span.value.clone(),
            })
            .collect();
        Ok(Self {
            channels: build_channels(&stored, registry)?,
        })
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
            .any(|span| span.source == ChannelSource::Trajectory);
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
            if span.source == ChannelSource::Trajectory {
                let range = span.offset..span.offset + span.dims;
                channel.extract(&context, &mut features[range]);
            }
        }
        database.normalise(&mut features);
        Ok(features)
    }
}

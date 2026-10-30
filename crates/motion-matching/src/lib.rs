//! A character simulation that drives the physics body and predicts its trajectory, and the bake of motion databases.

pub mod bake;
pub mod channel;
pub mod database;
pub mod definition;
pub mod features;
pub mod filter;
pub mod instance;
pub mod node;
pub mod plugin;
pub mod query;
pub mod search;
pub mod simulation;
pub mod systems;

pub use bake::bake;
pub use channel::{
    build_channels, extract_features, BodyMotion, ChannelSource, FeatureChannel, FrameContext,
    ReflectFeatureChannel,
};
pub use database::{BakeError, ChannelSpan, MotionDatabase};
pub use definition::{
    MotionDatabaseDefinition, MotionEntry, SimulationBoneSettings, StoredChannel, TimeRange,
    TrajectorySettings,
};
pub use features::{
    reference_channels, BonePositions, BoneVelocities, TrajectoryDirections, TrajectoryPositions,
};
pub use instance::MotionMatchingInstance;
pub use node::{MotionMatchingNode, MotionMatchingTrajectory, TRAJECTORY_KEY};
pub use plugin::MotionMatchingPlugin;
pub use query::FeatureQuery;
pub use search::{excluded_tail, search};
pub use simulation::{predict_trajectory, CharacterSimulation, DesiredMotion, TrajectorySample};

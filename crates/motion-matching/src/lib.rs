//! A character simulation that drives the physics body and predicts its trajectory, and the bake of motion databases.

pub mod bake;
pub mod channel;
pub mod database;
pub mod definition;
pub mod filter;
pub mod plugin;
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
pub use plugin::MotionMatchingPlugin;
pub use simulation::{predict_trajectory, CharacterSimulation, DesiredMotion, TrajectorySample};

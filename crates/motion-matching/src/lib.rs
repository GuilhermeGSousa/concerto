//! A character simulation that drives the physics body and predicts its trajectory.

pub mod bake;
pub mod channel;
pub mod database;
pub mod definition;
pub mod filter;
pub mod plugin;
pub mod simulation;
pub mod systems;

pub use bake::bake;
pub use channel::{build_channels, FeatureChannel, FrameContext, ReflectFeatureChannel};
pub use database::{BakeError, ChannelSpan, MotionDatabase, PoseView};
pub use definition::{
    ContactSettings, MotionDatabaseDefinition, MotionEntry, SimulationBoneSettings, StoredChannel,
    TimeRange,
};
pub use plugin::MotionMatchingPlugin;
pub use simulation::{predict_trajectory, CharacterSimulation, DesiredMotion, TrajectorySample};

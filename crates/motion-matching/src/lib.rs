//! A character simulation that drives the physics body and predicts its trajectory.

pub mod database;
pub mod definition;
pub mod plugin;
pub mod simulation;
pub mod systems;

pub use database::{BakeError, ChannelSpan, MotionDatabase, PoseView};
pub use definition::{
    ContactSettings, MotionDatabaseDefinition, MotionEntry, SimulationBoneSettings, StoredChannel,
    TimeRange,
};
pub use plugin::MotionMatchingPlugin;
pub use simulation::{predict_trajectory, CharacterSimulation, DesiredMotion, TrajectorySample};

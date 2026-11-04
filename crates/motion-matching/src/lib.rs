//! A character simulation that drives the physics body and predicts its trajectory, and the bake of motion databases.

pub mod bake;
pub mod channel;
pub mod database;
pub mod definition;
pub mod displacement;
pub mod features;
pub mod filter;
pub mod node;
pub mod plugin;
pub mod query;
pub mod search;
pub mod simulation;
pub mod staleness;
pub mod state;
pub mod systems;
#[cfg(test)]
mod test_support;

pub use bake::{bake, bake_with_progress};
pub use channel::{
    extract_features, preview_features, BodyMotion, ChannelSource, DynCloneChannel, FeatureChannel,
    FeatureChannelData, FeatureShape, FrameContext, ReflectFeatureChannel,
};
pub use database::{BakeError, ChannelSpan, MotionDatabase};
pub use definition::{
    MotionDatabaseDefinition, MotionEntry, SimulationBoneSettings, TimeRange, TrajectorySettings,
};
pub use displacement::{Adjustment, CharacterDisplacement, CharacterLink, Clamp, Synchronization};
pub use features::{
    reference_channels, BonePositions, BoneVelocities, TrajectoryDirections, TrajectoryPositions,
};
pub use node::MotionMatchingNode;
pub use plugin::MotionMatchingPlugin;
pub use query::FeatureQuery;
pub use search::{cost_of, search};
pub use simulation::{CharacterSimulation, DesiredMotion, TrajectorySample};
pub use staleness::{is_stale, weightless_hash, weights_changed_only, weights_of};
pub use state::{LastQuery, MotionMatchingState};

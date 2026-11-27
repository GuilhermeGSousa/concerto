//! A character simulation that drives the physics body and predicts its trajectory, and the bake of motion databases.

/// Baking a motion database: laying out its frames, the simulation bone and the trajectory.
pub mod bake;
/// Feature channels: the groups of values a database stores per frame and matches against.
pub mod channel;
/// The motion database asset, its baked frames and the errors of a bake.
pub mod database;
/// The settings a motion database is defined by: its clips, simulation bone and trajectory.
pub mod definition;
/// How the visible character is reconciled with its physics capsule each frame.
pub mod displacement;
/// The built-in feature channels: bone positions and velocities, trajectory positions and
/// directions.
pub mod features;
/// The Savitzky-Golay filter the bake smooths the simulation bone with.
pub mod filter;
/// The motion matching animation node.
pub mod node;
/// The plugin that registers motion matching.
pub mod plugin;
/// Building the feature query a search matches against.
pub mod query;
/// Searching a database for the frame nearest a query.
pub mod search;
/// The character simulation and the motion it is asked for.
pub mod simulation;
/// The hash of what a bake reads, which says whether a bake is stale, and the channel weights it
/// leaves out.
pub mod staleness;
/// The per-player state of the motion matching node.
pub mod state;
/// The systems that drive the capsule, publish its trajectory and place the character.
pub mod systems;
#[cfg(test)]
mod test_support;

pub use bake::{bake, bake_with_progress};
pub use channel::{
    extract_features, preview_features, BodyMotion, ChannelSource, DynCloneChannel, FeatureChannel,
    FeatureChannelData, FeatureShape, FrameContext, ReflectFeatureChannel,
};
pub use database::{BakeError, BakedDatabase, MotionDatabase};
pub use definition::{
    JointAxis, MirrorAxis, MotionEntry, SimulationBoneSettings, TimeRange, TrajectorySettings,
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
pub use staleness::{weightless_hash, weights_of};
pub use state::{LastQuery, MotionMatchingState};

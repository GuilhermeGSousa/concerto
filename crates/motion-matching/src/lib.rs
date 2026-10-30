//! A character simulation that drives the physics body and predicts its trajectory.

pub mod plugin;
pub mod simulation;
pub mod systems;

pub use plugin::MotionMatchingPlugin;
pub use simulation::{predict_trajectory, CharacterSimulation, DesiredMotion, TrajectorySample};

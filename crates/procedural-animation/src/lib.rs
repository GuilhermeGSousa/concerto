pub mod foot_placement;
pub mod look_at;
pub mod plugin;
pub mod procedural_motion;

pub use foot_placement::{FootPlacementLeg, FootPlacementNode, IgnoredBody};
pub use look_at::{LookAtBone, LookAtEye, LookAtNode, LookAtState};
pub use plugin::ProceduralAnimationPlugin;
pub use procedural_motion::{
    ProceduralMotionChannel, ProceduralMotionNode, ProceduralMotionState, Waveform,
};

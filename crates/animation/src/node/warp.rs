use concerto_name::Name;
use concerto_reflect::Reflect;
use glam::{Quat, Vec3};

use crate::clip::ClipRange;

/// Bends a clip's root motion over `window` so the root space ends on the `WarpTarget` under `target`.
#[derive(Clone, Debug, PartialEq, Reflect, serde::Serialize, serde::Deserialize)]
pub struct RootMotionWarp {
    pub target: Name,
    pub window: Option<ClipRange>,
    pub translation: bool,
    pub rotation: bool,
}

impl Default for RootMotionWarp {
    fn default() -> Self {
        Self::new(Name::default())
    }
}

impl RootMotionWarp {
    /// Warps translation and rotation over the whole clip toward the `WarpTarget` under `target`.
    pub fn new(target: Name) -> Self {
        Self {
            target,
            window: None,
            translation: true,
            rotation: true,
        }
    }

    /// Warps only over this span of the clip.
    pub fn with_window(mut self, window: ClipRange) -> Self {
        self.window = Some(window);
        self
    }

    /// Whether the horizontal translation is warped.
    pub fn with_translation(mut self, translation: bool) -> Self {
        self.translation = translation;
        self
    }

    /// Whether the yaw is warped.
    pub fn with_rotation(mut self, rotation: bool) -> Self {
        self.rotation = rotation;
        self
    }
}

/// Where a warped clip's root space ends, in world space: its ground position and its yaw.
#[derive(Clone, Copy, Debug, Default, PartialEq, Reflect)]
pub struct WarpTarget {
    pub position: Vec3,
    pub rotation: Quat,
}

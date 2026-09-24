use concerto_color::Color;
use concerto_ecs::resource::Resource;
use glam::Vec3;

use crate::vertex::GizmoVertex;

/// CPU-side buffer that accumulates every gizmo drawn during a frame.
///
/// [`DebugGizmos`](crate::gizmos::DebugGizmos) pushes line segments here as
/// systems run.  The render system uploads the whole buffer to the GPU once per
/// frame and then clears it, giving the classic *immediate-mode* behaviour:
/// gizmos only appear on frames where they are (re)drawn.
#[derive(Resource)]
pub struct GizmoStorage {
    /// Line-list vertices, two per segment.
    pub(crate) vertices: Vec<GizmoVertex>,
    /// When `false`, drawing calls are ignored and nothing is rendered.
    pub enabled: bool,
}

impl Default for GizmoStorage {
    fn default() -> Self {
        Self {
            vertices: Vec::new(),
            enabled: true,
        }
    }
}

impl GizmoStorage {
    /// Appends a single coloured line segment.
    #[inline]
    pub(crate) fn push_line(
        &mut self,
        start: Vec3,
        end: Vec3,
        start_color: Color,
        end_color: Color,
    ) {
        if !self.enabled {
            return;
        }
        self.vertices.push(GizmoVertex::new(start, start_color));
        self.vertices.push(GizmoVertex::new(end, end_color));
    }

    /// Number of line segments currently buffered.
    #[inline]
    pub fn segment_count(&self) -> usize {
        self.vertices.len() / 2
    }

    /// Drops all buffered geometry.  Called once per frame after upload.
    #[inline]
    pub(crate) fn clear(&mut self) {
        self.vertices.clear();
    }
}

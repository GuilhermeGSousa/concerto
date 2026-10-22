use concerto_color::Color;
use concerto_ecs::resource::Resource;
use glam::Vec3;

use crate::vertex::{GizmoVertex, WideGizmoVertex};

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
    /// Quad vertices for segments wider than a pixel, six per segment.
    pub(crate) wide: Vec<WideGizmoVertex>,
    /// When `false`, drawing calls are ignored and nothing is rendered.
    pub enabled: bool,
}

impl Default for GizmoStorage {
    fn default() -> Self {
        Self {
            vertices: Vec::new(),
            wide: Vec::new(),
            enabled: true,
        }
    }
}

impl GizmoStorage {
    /// Appends a coloured line segment `width` pixels wide.
    #[inline]
    pub(crate) fn push_line(
        &mut self,
        start: Vec3,
        end: Vec3,
        start_color: Color,
        end_color: Color,
        width: f32,
    ) {
        if !self.enabled {
            return;
        }
        if width <= 1.0 {
            self.vertices.push(GizmoVertex::new(start, start_color));
            self.vertices.push(GizmoVertex::new(end, end_color));
            return;
        }
        let corner = |along: f32, side: f32| WideGizmoVertex {
            start: start.to_array(),
            end: end.to_array(),
            color: if along < 0.5 { start_color } else { end_color }.to_array(),
            corner: [along, side],
            width,
        };
        self.wide.extend([
            corner(0.0, -1.0),
            corner(1.0, -1.0),
            corner(1.0, 1.0),
            corner(0.0, -1.0),
            corner(1.0, 1.0),
            corner(0.0, 1.0),
        ]);
    }

    /// Number of line segments currently buffered.
    #[inline]
    pub fn segment_count(&self) -> usize {
        self.vertices.len() / 2 + self.wide.len() / 6
    }

    /// Drops all buffered geometry.
    #[inline]
    pub(crate) fn clear(&mut self) {
        self.vertices.clear();
        self.wide.clear();
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.vertices.is_empty() && self.wide.is_empty()
    }
}

/// The render world's copy of the lines drawn in the main world last frame.
#[derive(Resource, Default)]
pub(crate) struct RenderGizmos {
    pub(crate) vertices: Vec<GizmoVertex>,
    pub(crate) wide: Vec<WideGizmoVertex>,
}

impl RenderGizmos {
    pub(crate) fn replace_with(&mut self, storage: &GizmoStorage) {
        self.vertices.clear();
        self.vertices.extend_from_slice(&storage.vertices);
        self.wide.clear();
        self.wide.extend_from_slice(&storage.wide);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use concerto_color::Color;

    #[test]
    fn the_render_copy_mirrors_the_latest_frame_only() {
        let mut storage = GizmoStorage::default();
        let mut render = RenderGizmos::default();
        storage.push_line(Vec3::ZERO, Vec3::X, Color::WHITE, Color::WHITE, 1.0);
        storage.push_line(Vec3::ZERO, Vec3::Y, Color::WHITE, Color::WHITE, 1.0);
        render.replace_with(&storage);
        assert_eq!(render.vertices.len(), 4);

        storage.clear();
        storage.push_line(Vec3::ZERO, Vec3::Z, Color::WHITE, Color::WHITE, 1.0);
        storage.push_line(Vec3::ZERO, Vec3::Z, Color::WHITE, Color::WHITE, 3.0);
        render.replace_with(&storage);
        assert_eq!(render.vertices.len(), 2);
        assert_eq!(render.wide.len(), 6);

        storage.clear();
        render.replace_with(&storage);
        assert!(render.vertices.is_empty());
        assert!(render.wide.is_empty());
    }

    #[test]
    fn a_wide_segment_is_a_quad_that_spans_both_endpoints_and_both_sides() {
        let mut storage = GizmoStorage::default();
        storage.push_line(Vec3::ZERO, Vec3::X, Color::WHITE, Color::BLACK, 4.0);
        assert!(storage.vertices.is_empty());
        assert_eq!(storage.segment_count(), 1);
        let corners: std::collections::HashSet<(i32, i32)> = storage
            .wide
            .iter()
            .map(|vertex| (vertex.corner[0] as i32, vertex.corner[1] as i32))
            .collect();
        assert_eq!(corners.len(), 4);
        for vertex in &storage.wide {
            assert_eq!(vertex.width, 4.0);
            assert_eq!(vertex.end, [1.0, 0.0, 0.0]);
            let expected = if vertex.corner[0] < 0.5 {
                Color::WHITE
            } else {
                Color::BLACK
            };
            assert_eq!(vertex.color, expected.to_array());
        }
    }
}

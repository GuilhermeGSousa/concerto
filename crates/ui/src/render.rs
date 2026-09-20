#![allow(clippy::items_after_test_module, clippy::too_many_arguments)]

use ecs::{
    query::Query,
    resource::{Res, ResMut},
};
use glyphon::{Resolution, TextArea, TextBounds};
use render::{
    MaterialPipeline, device::RenderDevice, queue::RenderQueue,
    render_asset::render_window::RenderWindow,
};
use wgpu::MultisampleState;

use crate::{
    material::UIMaterial,
    node::{RenderUIMaterial, RenderUINode},
    text::{
        RenderTextComponent,
        resources::{TextAtlas, TextFontSystem, TextRenderers, TextSwashCache, TextViewport},
    },
};

pub(crate) fn update_text_viewport(
    render_window: Res<RenderWindow>,
    queue: Res<RenderQueue>,
    mut text_viewport: ResMut<TextViewport>,
) {
    let (width, height) = render_window.size();
    text_viewport.update(&queue, Resolution { width, height });
}

/// Compute the screen-space scissor rectangle for a text node.
///
/// `TextBounds` is a clip rect in **absolute screen pixel coordinates** — all
/// four values are measured from the top-left corner of the window, not from
/// the node's own origin.  The text that falls outside this rect is discarded
/// by glyphon before it reaches the GPU.
///
/// The node's `location` is already in absolute screen pixels (set by
/// `write_absolute_positions` in `node.rs`), so we just map it straight
/// through.
#[cfg(test)]
fn node_text_bounds(location: glam::Vec2, size: glam::Vec2) -> TextBounds {
    TextBounds {
        left: location.x as i32,
        top: location.y as i32,
        right: (location.x + size.x) as i32,
        bottom: (location.y + size.y) as i32,
    }
}

pub(crate) fn prepare_text_renderer(
    mut text_renderers: ResMut<TextRenderers>,
    mut font_system: ResMut<TextFontSystem>,
    mut text_atlas: ResMut<TextAtlas>,
    mut text_swash_cache: ResMut<TextSwashCache>,
    text_viewport: Res<TextViewport>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    text_nodes: Query<&RenderTextComponent>,
) {
    let mut batched = text_nodes.iter().collect::<Vec<_>>();
    batched.sort_by_key(|render_text| render_text.layer);

    let renderers = &mut *text_renderers;
    renderers.layers.clear();
    for group in batched.chunk_by(|a, b| a.layer == b.layer) {
        let index = renderers.layers.len();
        if index == renderers.renderers.len() {
            renderers.renderers.push(glyphon::TextRenderer::new(
                &mut text_atlas,
                &device,
                MultisampleState::default(),
                None,
            ));
        }
        renderers.renderers[index]
            .prepare(
                &device,
                &queue,
                &mut font_system,
                &mut text_atlas,
                &text_viewport,
                group.iter().map(|render_text| TextArea {
                    buffer: &render_text.buffer,
                    left: render_text.location.x,
                    top: render_text.location.y,
                    scale: 1.0,
                    bounds: TextBounds {
                        left: render_text.clip_min.x.floor() as i32,
                        top: render_text.clip_min.y.floor() as i32,
                        right: render_text.clip_max.x.ceil() as i32,
                        bottom: render_text.clip_max.y.ceil() as i32,
                    },
                    default_color: render_text.color,
                    custom_glyphs: &[],
                }),
                &mut text_swash_cache,
            )
            .expect("Failed preparing for rendering text");
        renderers.layers.push(group[0].layer);
    }
}

/// Splits `layers` at the first batch belonging to `layer` or above.
///
/// Text for a layer is drawn after that layer's quads, so every batch strictly
/// below the quad about to be drawn must be flushed first.
fn batches_below(layers: &[i32], next: usize, layer: i32) -> usize {
    let mut end = next;
    while end < layers.len() && layers[end] < layer {
        end += 1;
    }
    end
}

#[cfg(test)]
mod tests {
    use super::node_text_bounds;
    use glam::Vec2;

    #[test]
    fn bounds_match_node_absolute_position() {
        let loc = Vec2::new(10.0, 800.0);
        let size = Vec2::new(120.0, 32.0);
        let b = node_text_bounds(loc, size);
        assert_eq!(b.left, 10);
        assert_eq!(b.top, 800);
        assert_eq!(b.right, 130); // 10 + 120
        assert_eq!(b.bottom, 832); // 800 + 32
    }

    #[test]
    fn bounds_at_origin() {
        let b = node_text_bounds(Vec2::ZERO, Vec2::new(800.0, 600.0));
        assert_eq!(b.left, 0);
        assert_eq!(b.top, 0);
        assert_eq!(b.right, 800);
        assert_eq!(b.bottom, 600);
    }
}

pub(crate) fn ui_renderpass(
    pipeline: Res<MaterialPipeline<UIMaterial>>,
    mut device: ResMut<RenderDevice>,
    render_window: Res<RenderWindow>,
    ui_nodes: Query<(&RenderUINode, Option<&RenderUIMaterial>)>,
    // Text
    text_renderers: Res<TextRenderers>,
    text_viewport: Res<TextViewport>,
    text_atlas: Res<TextAtlas>,
) {
    let encoder = device.command_encoder();

    if let Some(view) = render_window.get_view() {
        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("UI Render Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            occlusion_query_set: None,
            timestamp_writes: None,
        });

        let mut render_nodes = ui_nodes.iter().collect::<Vec<_>>();
        render_nodes.sort_by_key(|(render_node, _)| render_node.z_index);

        let layers = &text_renderers.layers;
        let mut batch = 0;

        for (render_node, render_material) in render_nodes {
            // Flush the text of every layer below this quad's before drawing it.
            let flush_to = batches_below(layers, batch, (render_node.z_index >> 32) as i32);
            while batch < flush_to {
                text_renderers.renderers[batch]
                    .render(&text_atlas, &text_viewport, &mut render_pass)
                    .expect("Error rendering text");
                batch += 1;
            }

            render_pass.set_index_buffer(
                render_node.index_buffer.slice(..),
                wgpu::IndexFormat::Uint16,
            );
            render_pass.set_vertex_buffer(0, render_node.vertex_buffer.slice(..));

            let Some(material) = render_material else {
                continue;
            };
            render_pass.set_pipeline(&pipeline.pipeline);
            render_pass.set_bind_group(0, &material.material_bind_group, &[]);

            render_pass.draw_indexed(0..render_node.index_count, 0, 0..1);
        }

        // Anything at or above the topmost quad's layer draws last.
        while batch < layers.len() {
            text_renderers.renderers[batch]
                .render(&text_atlas, &text_viewport, &mut render_pass)
                .expect("Error rendering text");
            batch += 1;
        }
    }
}

#[cfg(test)]
mod layer_tests {
    use super::batches_below;

    #[test]
    fn flushes_only_the_layers_below_the_quad_being_drawn() {
        let layers = [0, 1, 4];
        // A quad on layer 0 flushes nothing: its own text draws after it.
        assert_eq!(batches_below(&layers, 0, 0), 0);
        // A quad on layer 1 flushes layer 0's text first.
        assert_eq!(batches_below(&layers, 0, 1), 1);
        // A quad on layer 5 flushes everything still pending.
        assert_eq!(batches_below(&layers, 1, 5), 3);
    }

    #[test]
    fn layers_with_no_quads_of_their_own_still_flush_in_order() {
        // Text on layer 2 exists but no quad does; the next quad is on 7.
        let layers = [2];
        assert_eq!(batches_below(&layers, 0, 7), 1);
    }

    #[test]
    fn nothing_is_flushed_twice() {
        let layers = [0, 0, 3];
        let batch = batches_below(&layers, 0, 3);
        assert_eq!(batch, 2);
        assert_eq!(batches_below(&layers, batch, 3), 2);
    }
}

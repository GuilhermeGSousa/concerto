#![allow(clippy::items_after_test_module, clippy::too_many_arguments)]

use concerto_ecs::{
    Entity, Ref, World,
    query::{Query, change_detection::DetectChanges},
    resource::{Res, ResMut},
    system::input::SystemLocal,
    world::FromWorld,
};
use concerto_render::{
    MaterialPipeline, device::RenderDevice, queue::RenderQueue,
    render_asset::render_window::RenderWindow,
};
use glyphon::{Resolution, TextArea, TextBounds};
use std::collections::HashMap;
use wgpu::MultisampleState;

use crate::{
    material::UIMaterial,
    node::{RenderUIMaterial, RenderUINode, UIQuadIndices},
    resources::UIRenderDiagnostics,
    text::{
        RenderTextComponent,
        resources::{TextAtlas, TextFontSystem, TextRenderers, TextSwashCache, TextViewport},
    },
    vertex::QUAD_INDICES,
};

pub(crate) fn update_text_viewport(
    render_window: Res<RenderWindow>,
    queue: Res<RenderQueue>,
    mut text_viewport: ResMut<TextViewport>,
) {
    let (width, height) = render_window.size();
    text_viewport.update(&queue, Resolution { width, height });
}

pub(crate) struct PreparedText {
    resolution: Option<Resolution>,
    layers: HashMap<i32, Vec<Entity>>,
}

impl FromWorld for PreparedText {
    fn from_world(_: &World) -> Self {
        Self {
            resolution: None,
            layers: HashMap::new(),
        }
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
    diagnostics: Res<UIRenderDiagnostics>,
    text_nodes: Query<(Entity, Ref<RenderTextComponent>)>,
    mut prepared: SystemLocal<PreparedText>,
) {
    let resolution = text_viewport.resolution();
    let resized = prepared.resolution.replace(resolution) != Some(resolution);

    let mut batched = text_nodes.iter().collect::<Vec<_>>();
    batched.sort_by_key(|(entity, render_text)| {
        (render_text.layer, entity.index(), entity.generation())
    });

    let mut previous = std::mem::take(&mut prepared.layers);
    let renderers = &mut *text_renderers;
    let mut slot = 0;
    for group in batched.chunk_by(|a, b| a.1.layer == b.1.layer) {
        let layer = group[0].1.layer;
        while slot < renderers.layers.len() && renderers.layers[slot] < layer {
            renderers.layers.remove(slot);
            let renderer = renderers.renderers.remove(slot);
            renderers.spare.push(renderer);
        }
        if renderers.layers.get(slot) != Some(&layer) {
            let renderer = renderers.spare.pop().unwrap_or_else(|| {
                glyphon::TextRenderer::new(
                    &mut text_atlas,
                    &device,
                    MultisampleState::default(),
                    None,
                )
            });
            renderers.layers.insert(slot, layer);
            renderers.renderers.insert(slot, renderer);
        }

        let entities = group.iter().map(|(entity, _)| *entity).collect::<Vec<_>>();
        let regrouped = previous.remove(&layer).as_ref() != Some(&entities);
        let changed = group
            .iter()
            .any(|(_, render_text)| render_text.is_changed());
        if resized || regrouped || changed {
            renderers.renderers[slot]
                .prepare(
                    &device,
                    &queue,
                    &mut font_system,
                    &mut text_atlas,
                    &text_viewport,
                    group.iter().map(|(_, render_text)| TextArea {
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
            diagnostics.record_text_prepare();
        }
        prepared.layers.insert(layer, entities);
        slot += 1;
    }
    while slot < renderers.layers.len() {
        renderers.layers.remove(slot);
        let renderer = renderers.renderers.remove(slot);
        renderers.spare.push(renderer);
    }
}

fn batches_below(layers: &[i32], next: usize, layer: i32) -> usize {
    let mut end = next;
    while end < layers.len() && layers[end] < layer {
        end += 1;
    }
    end
}

pub(crate) fn ui_renderpass(
    pipeline: Res<MaterialPipeline<UIMaterial>>,
    mut device: ResMut<RenderDevice>,
    render_window: Res<RenderWindow>,
    ui_nodes: Query<(&RenderUINode, Option<&RenderUIMaterial>)>,
    quad_indices: Res<UIQuadIndices>,
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
            let flush_to = batches_below(layers, batch, (render_node.z_index >> 32) as i32);
            while batch < flush_to {
                text_renderers.renderers[batch]
                    .render(&text_atlas, &text_viewport, &mut render_pass)
                    .expect("Error rendering text");
                batch += 1;
            }

            render_pass.set_index_buffer(quad_indices.0.slice(..), wgpu::IndexFormat::Uint16);
            render_pass.set_vertex_buffer(0, render_node.vertex_buffer.slice(..));

            let Some(material) = render_material else {
                continue;
            };
            render_pass.set_pipeline(&pipeline.pipeline);
            render_pass.set_bind_group(0, &material.material_bind_group, &[]);

            render_pass.draw_indexed(0..QUAD_INDICES.len() as u32, 0, 0..1);
        }

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
        assert_eq!(batches_below(&layers, 0, 0), 0);
        assert_eq!(batches_below(&layers, 0, 1), 1);
        assert_eq!(batches_below(&layers, 1, 5), 3);
    }

    #[test]
    fn layers_with_no_quads_of_their_own_still_flush_in_order() {
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

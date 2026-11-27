use std::ops::Range;

use concerto_ecs::{
    entity::EntityHashMap,
    query::Query,
    resource::{Res, ResMut, Resource},
    Entity,
};

use crate::{
    components::{camera::RenderCamera, shadows::RenderLighting, skeleton::SkinUniforms},
    device::RenderDevice,
    resources::RenderContext,
};

/// The mesh one [`PhaseItem`] draws.
pub struct PhaseMesh {
    /// The vertex buffer, bound at slot 0.
    pub vertices: wgpu::Buffer,
    /// The `u32` index buffer.
    pub indices: wgpu::Buffer,
    /// The number of indices to draw.
    pub index_count: u32,
}

/// The instances one [`PhaseItem`] draws: a range of the vertex buffer bound at slot 1.
pub struct PhaseInstances {
    /// The instance buffer.
    pub buffer: wgpu::Buffer,
    /// The instances to draw, as indices into the buffer.
    pub range: Range<u32>,
}

/// Which of the engine's bind groups an item's pipeline takes, beside its own at group 0.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PhaseBindings {
    /// The camera's bind group, at group 1.
    pub camera: bool,
    /// The lights and shadow maps, at group 2.
    pub lighting: bool,
    /// The bone palettes at group 3, at this offset.
    pub skeleton: Option<u32>,
}

/// One draw into a camera's view target, holding everything the draw needs.
pub struct PhaseItem {
    /// A pipeline for the view target's format and sample count, with the view target's depth.
    pub pipeline: wgpu::RenderPipeline,
    /// The bind group at group 0.
    pub bind_group: wgpu::BindGroup,
    /// The engine bind groups the pipeline takes.
    pub bindings: PhaseBindings,
    /// The mesh to draw.
    pub mesh: PhaseMesh,
    /// The instances to draw it at.
    pub instances: PhaseInstances,
    /// How far the item is from the camera; the transparent phase draws the farthest first.
    pub view_distance: f32,
}

/// The draws queued for one camera this frame.
#[derive(Default)]
pub struct ViewPhase {
    /// Drawn in [`RenderSet::Draw`](crate::sets::RenderSet::Draw), in the order queued.
    pub opaque: Vec<PhaseItem>,
    /// Drawn in [`RenderSet::Transparent`](crate::sets::RenderSet::Transparent), farthest first, whoever queued them.
    pub transparent: Vec<PhaseItem>,
}

/// Every camera's [`ViewPhase`], emptied each frame at the start of [`RenderSet::Queue`](crate::sets::RenderSet::Queue) and filled by systems in it that run after [`clear_view_phases`].
#[derive(Resource, Default)]
pub struct ViewPhases {
    views: EntityHashMap<ViewPhase>,
}

impl ViewPhases {
    /// The phase of the render-world camera `camera`, created empty if it has none yet.
    pub fn view_mut(&mut self, camera: Entity) -> &mut ViewPhase {
        self.views.entry(camera).or_default()
    }

    /// The phase of the render-world camera `camera`, if anything was queued for it.
    pub fn view(&self, camera: Entity) -> Option<&ViewPhase> {
        self.views.get(&camera)
    }
}

/// Empties every camera's phase; a system that queues items runs after it.
pub fn clear_view_phases(mut phases: ResMut<ViewPhases>, render_cameras: Query<&RenderCamera>) {
    phases
        .views
        .retain(|camera, _| render_cameras.contains_entity(*camera));
    for phase in phases.views.values_mut() {
        phase.opaque.clear();
        phase.transparent.clear();
    }
}

struct PhasePass<'a> {
    label: &'static str,
    render_camera: &'a RenderCamera,
    context: &'a RenderContext,
    lighting: &'a RenderLighting,
    skins: &'a SkinUniforms,
}

impl PhasePass<'_> {
    fn draw(&self, encoder: &mut wgpu::CommandEncoder, items: &[PhaseItem]) {
        let rect = self
            .render_camera
            .viewport_rect(self.render_camera.target_size(self.context));
        if rect.is_empty() || items.is_empty() {
            return;
        }
        let view_target = &self.render_camera.view_target;
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            multiview_mask: None,
            label: Some(self.label),
            color_attachments: &[Some(view_target.color_attachment(wgpu::LoadOp::Load))],
            depth_stencil_attachment: Some(view_target.depth_attachment(wgpu::LoadOp::Load)),
            occlusion_query_set: None,
            timestamp_writes: None,
        });
        rect.apply(&mut pass);

        let mut pipeline = None;
        let mut bind_group = None;
        let mut vertices = None;
        let mut instances = None;
        for item in items {
            if pipeline != Some(&item.pipeline) {
                pass.set_pipeline(&item.pipeline);
                pipeline = Some(&item.pipeline);
                if item.bindings.camera {
                    pass.set_bind_group(1, &self.render_camera.camera_bind_group, &[]);
                }
                if item.bindings.lighting {
                    pass.set_bind_group(2, &self.lighting.bind_group, &[]);
                }
            }
            if bind_group != Some(&item.bind_group) {
                pass.set_bind_group(0, &item.bind_group, &[]);
                bind_group = Some(&item.bind_group);
            }
            if let Some(offset) = item.bindings.skeleton {
                pass.set_bind_group(3, self.skins.bind_group(), &[offset]);
            }
            if vertices != Some(&item.mesh.vertices) {
                pass.set_vertex_buffer(0, item.mesh.vertices.slice(..));
                pass.set_index_buffer(item.mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
                vertices = Some(&item.mesh.vertices);
            }
            if instances != Some(&item.instances.buffer) {
                pass.set_vertex_buffer(1, item.instances.buffer.slice(..));
                instances = Some(&item.instances.buffer);
            }
            pass.draw_indexed(0..item.mesh.index_count, 0, item.instances.range.clone());
        }
    }
}

pub(crate) fn render_opaque_phase(
    phases: Res<ViewPhases>,
    mut device: ResMut<RenderDevice>,
    render_cameras: Query<(Entity, &RenderCamera)>,
    lighting: Res<RenderLighting>,
    skins: Res<SkinUniforms>,
    context: Res<RenderContext>,
) {
    let encoder = device.command_encoder();
    for (camera, render_camera) in render_cameras.iter() {
        let Some(phase) = phases.view(camera) else {
            continue;
        };
        PhasePass {
            label: "Opaque Phase",
            render_camera,
            context: &context,
            lighting: &lighting,
            skins: &skins,
        }
        .draw(encoder, &phase.opaque);
    }
}

pub(crate) fn render_transparent_phase(
    mut phases: ResMut<ViewPhases>,
    mut device: ResMut<RenderDevice>,
    render_cameras: Query<(Entity, &RenderCamera)>,
    lighting: Res<RenderLighting>,
    skins: Res<SkinUniforms>,
    context: Res<RenderContext>,
) {
    let encoder = device.command_encoder();
    for (camera, render_camera) in render_cameras.iter() {
        let Some(phase) = phases.views.get_mut(&camera) else {
            continue;
        };
        phase
            .transparent
            .sort_by(|a, b| b.view_distance.total_cmp(&a.view_distance));
        PhasePass {
            label: "Transparent Phase",
            render_camera,
            context: &context,
            lighting: &lighting,
            skins: &skins,
        }
        .draw(encoder, &phase.transparent);
    }
}

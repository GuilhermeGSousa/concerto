use std::marker::PhantomData;

use concerto_ecs::{
    query::Query,
    resource::{Res, ResMut, Resource},
};
use concerto_foundation::{assets::AssetId, transform::GlobalTransformRaw};

use crate::{
    assets::{
        material::ShaderRef,
        vertex::{Vertex, VertexBufferLayout},
    },
    batching::{BatchedInstanceBuffers, BinCandidate, RenderDrawStats, ViewBatches},
    capabilities::RenderCapabilities,
    components::{
        camera::PixelRect,
        material::RenderMaterialComponent,
        mesh::{RenderMeshInstanceFlags, RenderMeshInstances},
        render_entity::MainEntity,
        skeleton::{RenderSkeletonComponent, SkinUniforms},
        visibility::RenderLayers,
    },
    device::RenderDevice,
    frustum::Frustum,
    layouts::SkeletonLayout,
    material_plugin::RenderMaterial,
    queue::RenderQueue,
    render_asset::{render_mesh::RenderMesh, RenderAssets},
    shader::create_shader_module,
    view_target::ViewTarget,
    Material,
};

const DEPTH_PASS_SHADER: &str = include_str!("shaders/shadow.wgsl");
const STANDARD_CUTOUT_SHADER: &str = include_str!("shaders/standard_cutout.wgsl");
const CUTOUT_DEF: &str = "CUTOUT";

#[derive(Resource)]
pub(crate) struct DepthPassLayout(pub(crate) wgpu::BindGroupLayout);

impl DepthPassLayout {
    pub(crate) fn new(device: &wgpu::Device) -> Self {
        Self(
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Depth Pass Layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            }),
        )
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum DepthPassKind {
    Shadow,
    Prepass,
}

impl DepthPassKind {
    fn depth_stencil(self, capabilities: &RenderCapabilities) -> wgpu::DepthStencilState {
        let bias = match self {
            Self::Shadow => wgpu::DepthBiasState {
                constant: 0,
                slope_scale: 1.0,
                clamp: if capabilities.depth_bias_clamp {
                    100.0
                } else {
                    0.0
                },
            },
            Self::Prepass => wgpu::DepthBiasState::default(),
        };
        wgpu::DepthStencilState {
            format: match self {
                Self::Shadow => wgpu::TextureFormat::Depth32Float,
                Self::Prepass => ViewTarget::DEPTH_FORMAT,
            },
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Less),
            stencil: wgpu::StencilState::default(),
            bias,
        }
    }
}

pub(crate) struct DepthPassPipeline<'a> {
    pub(crate) label: &'static str,
    pub(crate) kind: DepthPassKind,
    pub(crate) layout: &'a wgpu::PipelineLayout,
    pub(crate) vertex: &'a wgpu::ShaderModule,
    pub(crate) fragment: Option<&'a wgpu::ShaderModule>,
    pub(crate) cull_mode: Option<wgpu::Face>,
}

impl DepthPassPipeline<'_> {
    pub(crate) fn build(
        &self,
        device: &wgpu::Device,
        capabilities: &RenderCapabilities,
    ) -> wgpu::RenderPipeline {
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(self.label),
            layout: Some(self.layout),
            vertex: wgpu::VertexState {
                module: self.vertex,
                entry_point: Some("vs_main"),
                buffers: &[
                    Some(Vertex::describe()),
                    Some(GlobalTransformRaw::describe()),
                ],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: self.fragment.map(|module| wgpu::FragmentState {
                module,
                entry_point: Some("fs_cutout"),
                targets: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                cull_mode: self.cull_mode,
                ..Default::default()
            },
            depth_stencil: Some(self.kind.depth_stencil(capabilities)),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        })
    }
}

pub(crate) fn depth_pass_vertex_module(
    device: &wgpu::Device,
    capabilities: &RenderCapabilities,
    cutout: bool,
) -> wgpu::ShaderModule {
    let mut defs = capabilities.shader_defs();
    if cutout {
        defs.define(CUTOUT_DEF);
    }
    create_shader_module(device, "Depth Pass VS", DEPTH_PASS_SHADER, &defs)
}

pub(crate) struct DepthPassView {
    pub(crate) kind: DepthPassKind,
    pub(crate) target: wgpu::TextureView,
    pub(crate) view_proj: wgpu::BindGroup,
    pub(crate) frustum: Frustum,
    pub(crate) layers: Option<RenderLayers>,
    pub(crate) rect: Option<PixelRect>,
    pub(crate) shadow_layer: Option<usize>,
}

#[derive(Resource, Default)]
pub(crate) struct DepthPassViews(pub(crate) Vec<DepthPassView>);

pub(crate) fn clear_depth_pass_views(mut views: ResMut<DepthPassViews>) {
    views.0.clear();
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct CutoutBinKey {
    material: AssetId,
    mesh: AssetId,
}

pub(crate) struct CutoutPipelines<M> {
    shadow: wgpu::RenderPipeline,
    prepass: wgpu::RenderPipeline,
    _marker: PhantomData<fn() -> M>,
}

impl<M: Material> CutoutPipelines<M> {
    pub(crate) fn new(
        device: &wgpu::Device,
        capabilities: &RenderCapabilities,
        depth_pass_layout: &DepthPassLayout,
        skeleton_layout: &SkeletonLayout,
        material_layout: &wgpu::BindGroupLayout,
    ) -> Option<Self> {
        let source = match M::cutout_shader()? {
            ShaderRef::Default => STANDARD_CUTOUT_SHADER,
            ShaderRef::Source(source) => source,
        };
        let mut defs = capabilities.shader_defs();
        M::shader_defs(&mut defs);
        let fragment = create_shader_module(device, "Cutout FS", source, &defs);
        let vertex = depth_pass_vertex_module(device, capabilities, true);
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Cutout Pipeline Layout"),
            bind_group_layouts: &[
                Some(&depth_pass_layout.0),
                Some(skeleton_layout),
                Some(material_layout),
            ],
            immediate_size: 0,
        });
        let build = |label: &'static str, kind: DepthPassKind| {
            DepthPassPipeline {
                label,
                kind,
                layout: &layout,
                vertex: &vertex,
                fragment: Some(&fragment),
                cull_mode: M::cull_mode(),
            }
            .build(device, capabilities)
        };
        Some(Self {
            shadow: build("Cutout Shadow Pipeline", DepthPassKind::Shadow),
            prepass: build("Cutout Prepass Pipeline", DepthPassKind::Prepass),
            _marker: PhantomData,
        })
    }

    fn get(&self, kind: DepthPassKind) -> &wgpu::RenderPipeline {
        match kind {
            DepthPassKind::Shadow => &self.shadow,
            DepthPassKind::Prepass => &self.prepass,
        }
    }
}

impl<M: 'static> Resource for CutoutPipelines<M> {
    fn name() -> &'static str {
        std::any::type_name::<Self>()
    }
}

pub(crate) struct CutoutBatches<M> {
    views: Vec<ViewBatches<CutoutBinKey>>,
    _marker: PhantomData<fn() -> M>,
}

impl<M> Default for CutoutBatches<M> {
    fn default() -> Self {
        Self {
            views: Vec::new(),
            _marker: PhantomData,
        }
    }
}

impl<M: 'static> Resource for CutoutBatches<M> {
    fn name() -> &'static str {
        std::any::type_name::<Self>()
    }
}

pub(crate) fn queue_cutout_meshes<M: Material>(
    views: Res<DepthPassViews>,
    render_mesh_query: Query<(
        &MainEntity,
        Option<&RenderSkeletonComponent>,
        &RenderMaterialComponent<M>,
    )>,
    render_mesh_instances: Res<RenderMeshInstances>,
    buffers: Res<BatchedInstanceBuffers>,
    render_meshes: Res<RenderAssets<RenderMesh>>,
    render_materials: Res<RenderAssets<RenderMaterial<M>>>,
    mut batches: ResMut<CutoutBatches<M>>,
    mut draw_stats: ResMut<RenderDrawStats>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
) {
    batches.views.truncate(views.0.len());
    for (index, view) in views.0.iter().enumerate() {
        if batches.views.len() <= index {
            batches
                .views
                .push(ViewBatches::new(&device, "Cutout Instances Buffer"));
        }
        let candidates =
            render_mesh_query
                .iter()
                .filter_map(|(main_entity, skeleton, material)| {
                    let instance = render_mesh_instances.get(main_entity)?;
                    if !instance.flags.contains(
                        RenderMeshInstanceFlags::CUTOUT | RenderMeshInstanceFlags::SHADOW_CASTER,
                    ) || view
                        .layers
                        .is_some_and(|layers| !instance.layers.intersects(layers))
                        || !render_meshes.contains(&instance.mesh_asset_id)
                        || !render_materials.contains(&material.material_asset_id)
                    {
                        return None;
                    }
                    Some(BinCandidate {
                        key: CutoutBinKey {
                            material: material.material_asset_id,
                            mesh: instance.mesh_asset_id,
                        },
                        instance,
                        skeleton_offset: skeleton.map(|skeleton| skeleton.offset),
                    })
                });
        let view_batches = &mut batches.views[index];
        let stats =
            view_batches
                .bins
                .rebuild(candidates, &view.frustum, &buffers.current_input_buffer);
        view_batches.upload(&device, &queue);
        if let Some(total) = view
            .shadow_layer
            .and_then(|layer| draw_stats.shadow_layers.get_mut(layer))
        {
            total.draws += stats.draws;
            total.culled += stats.culled;
        }
    }
}

pub(crate) fn render_cutout_meshes<M: Material>(
    pipelines: Res<CutoutPipelines<M>>,
    views: Res<DepthPassViews>,
    batches: Res<CutoutBatches<M>>,
    render_meshes: Res<RenderAssets<RenderMesh>>,
    render_materials: Res<RenderAssets<RenderMaterial<M>>>,
    skins: Res<SkinUniforms>,
    mut device: ResMut<RenderDevice>,
) {
    let encoder = device.command_encoder();
    for (view, view_batches) in views.0.iter().zip(&batches.views) {
        if view_batches.bins.batches.is_empty() {
            continue;
        }
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            multiview_mask: None,
            label: Some("Cutout Depth Pass"),
            color_attachments: &[],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &view.target,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            occlusion_query_set: None,
            timestamp_writes: None,
        });
        if let Some(rect) = view.rect {
            rect.apply(&mut pass);
        }
        pass.set_pipeline(pipelines.get(view.kind));
        pass.set_bind_group(0, &view.view_proj, &[]);
        pass.set_vertex_buffer(1, view_batches.instances.buffer().slice(..));
        for batch in &view_batches.bins.batches {
            let (Some(mesh), Some(material)) = (
                render_meshes.get(&batch.key.mesh),
                render_materials.get(&batch.key.material),
            ) else {
                continue;
            };
            pass.set_bind_group(1, skins.bind_group(), &[batch.skeleton_offset]);
            pass.set_bind_group(2, &material.bind_group, &[]);
            pass.set_vertex_buffer(0, mesh.vertices.slice(..));
            pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..mesh.index_count, 0, batch.instances.clone());
        }
    }
}

use std::f32::consts::TAU;

use bytemuck::{Pod, Zeroable};
use concerto_app::{
    App,
    extractor::Extracted,
    plugins::Plugin,
    schedule_groups::{Extract, Render},
};
use concerto_ecs::{
    IntoSystemConfig,
    entity::{Entity, EntityHashSet},
    query::{Query, change_detection::DetectChanges, filter::With},
    resource::{Res, ResMut, Resource},
    system::input::SystemLocal,
    world::{FromWorld, World},
};
use concerto_render::{
    LightTint, SkyRadianceMap, ViewPipelines,
    capabilities::RenderCapabilities,
    components::{camera::RenderCamera, render_entity::RenderEntity},
    device::RenderDevice,
    layouts::CameraLayout,
    queue::RenderQueue,
    resources::RenderContext,
    sets::{BackgroundSet, RenderSet},
    shader::create_shader_module,
};
use glam::Vec3;
use half::f16;

use crate::{
    atmosphere::{
        Lut, MULTIPLE_SCATTERING_LUT_SIZE, TRANSMITTANCE_LUT_HEIGHT, TRANSMITTANCE_LUT_WIDTH,
        observer_radius, top_radius,
    },
    lighting::{AtmosphereLuts, SkyQuery, SunQuery, active_sun, rgb, toward_sun},
    sun::{Atmosphere, Sky},
};

const SKY_RADIANCE_MAP_WIDTH: u32 = 256;
const SKY_RADIANCE_MAP_HEIGHT: u32 = 128;
const SKY_RADIANCE_MAP_LEVELS: u32 = 7;

/// Renders the sky-radiance map and draws the sky behind every camera with a [`Sky`].
///
/// # Panics
///
/// Panics during [`finish`](Plugin::finish) if [`RenderPlugin`](concerto_render::plugin::RenderPlugin)
/// was not registered before it.
pub struct SkyRenderPlugin;

impl Plugin for SkyRenderPlugin {
    fn build(&self, app: &mut App) {
        app.render_mut()
            .insert_resource(ExtractedSky::default())
            .insert_resource(AtmosphereLuts::default());
        app.add_render_system(Extract, extract_sky)
            .add_render_system(
                Render,
                (upload_atmosphere_luts, render_sky_view)
                    .chain()
                    .before(RenderSet::Background),
            )
            .add_render_system(
                Render,
                draw_sky
                    .in_set(RenderSet::Background)
                    .in_set(BackgroundSet::Draw),
            );
    }

    fn finish(&self, app: &mut App) {
        let render = app.render();
        let device = render
            .get_resource::<RenderDevice>()
            .expect("SkyRenderPlugin needs RenderPlugin, registered before it");
        let capabilities = render
            .get_resource::<RenderCapabilities>()
            .expect("SkyRenderPlugin needs RenderPlugin, registered before it");
        let camera_layout = render
            .get_resource::<CameraLayout>()
            .expect("SkyRenderPlugin needs RenderPlugin, registered before it");
        let target_format = render
            .get_resource::<RenderContext>()
            .expect("SkyRenderPlugin needs RenderPlugin, registered before it")
            .view_target_format;

        let SkyRadiance { map, mips } = sky_radiance_map(device, capabilities);
        let sky_view = SkyViewPass::new(device, capabilities, map.format);
        let sky = SkyPass::new(device, capabilities, camera_layout, target_format);
        app.render_mut()
            .insert_resource(map)
            .insert_resource(mips)
            .insert_resource(sky_view)
            .insert_resource(sky);
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ExtractedSun {
    toward_sun: Vec3,
    illuminance: Vec3,
    tint: Vec3,
    disk_radius: f32,
}

#[derive(Resource, Default)]
pub(crate) struct ExtractedSky {
    sun: Option<ExtractedSun>,
    cameras: EntityHashSet,
}

pub(crate) fn extract_sky(
    suns: Extracted<SunQuery>,
    sky_suns: Extracted<SkyQuery>,
    tints: Extracted<Query<&LightTint>>,
    skies: Extracted<Query<&RenderEntity, With<Sky>>>,
    main_luts: Extracted<Res<AtmosphereLuts>>,
    mut luts: ResMut<AtmosphereLuts>,
    mut sky: ResMut<ExtractedSky>,
) {
    let sun = active_sun(&sky_suns, &suns)
        .and_then(|entity| suns.get_entity(entity))
        .map(|(entity, sun, light, transform)| ExtractedSun {
            toward_sun: toward_sun(transform),
            illuminance: rgb(light.color.to_linear()) * light.intensity,
            tint: tints
                .get_entity(entity)
                .map_or(Vec3::ZERO, |tint| rgb(tint.0)),
            disk_radius: sun.disk_radius,
        });
    *sky = ExtractedSky {
        sun,
        cameras: skies.iter().map(|render_entity| **render_entity).collect(),
    };
    if main_luts.is_changed() {
        *luts = main_luts.clone();
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct SkyViewUniform {
    rayleigh_scattering: [f32; 4],
    ozone_absorption: [f32; 4],
    mie: [f32; 4],
    planet: [f32; 4],
    toward_sun: [f32; 4],
    illuminance: [f32; 4],
    lut_sizes: [f32; 4],
}

impl SkyViewUniform {
    fn new(inputs: &SkyViewInputs) -> Self {
        let atmosphere = &inputs.atmosphere;
        Self {
            rayleigh_scattering: atmosphere
                .rayleigh_scattering
                .extend(atmosphere.rayleigh_scale_height)
                .to_array(),
            ozone_absorption: atmosphere
                .ozone_absorption
                .extend(atmosphere.ozone_center_height)
                .to_array(),
            mie: [
                atmosphere.mie_scattering,
                atmosphere.mie_absorption,
                atmosphere.mie_scale_height,
                atmosphere.mie_asymmetry,
            ],
            planet: [
                atmosphere.planet_radius,
                top_radius(atmosphere),
                observer_radius(atmosphere),
                atmosphere.ozone_width,
            ],
            toward_sun: inputs.toward_sun.extend(0.0).to_array(),
            illuminance: inputs.illuminance.extend(1.0).to_array(),
            lut_sizes: [
                TRANSMITTANCE_LUT_WIDTH as f32,
                TRANSMITTANCE_LUT_HEIGHT as f32,
                MULTIPLE_SCATTERING_LUT_SIZE as f32,
                MULTIPLE_SCATTERING_LUT_SIZE as f32,
            ],
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct SunUniform {
    toward_sun: [f32; 4],
    disk_radiance: [f32; 4],
}

impl SunUniform {
    fn new(sun: Option<&ExtractedSun>, atmosphere: &Atmosphere) -> Self {
        let Some(sun) = sun else {
            return Self::zeroed();
        };
        let radius = sun.disk_radius.to_radians();
        let solid_angle = (TAU * (1.0 - radius.cos())).max(1e-9);
        let ground = atmosphere.planet_radius / observer_radius(atmosphere);
        let horizon = -(1.0 - ground * ground).max(0.0).sqrt();
        Self {
            toward_sun: sun.toward_sun.extend(radius.cos()).to_array(),
            disk_radiance: (sun.tint * sun.illuminance / solid_angle)
                .extend(horizon)
                .to_array(),
        }
    }
}

#[derive(Resource)]
pub(crate) struct SkyRadianceMips {
    levels: Vec<wgpu::TextureView>,
    sources: Vec<wgpu::BindGroup>,
    pipeline: wgpu::RenderPipeline,
}

struct SkyRadiance {
    map: SkyRadianceMap,
    mips: SkyRadianceMips,
}

fn sky_radiance_map(device: &wgpu::Device, capabilities: &RenderCapabilities) -> SkyRadiance {
    let level_count = if capabilities.single_layer_texture_arrays {
        SKY_RADIANCE_MAP_LEVELS
    } else {
        1
    };
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("sky_radiance_map"),
        size: wgpu::Extent3d {
            width: SKY_RADIANCE_MAP_WIDTH,
            height: SKY_RADIANCE_MAP_HEIGHT,
            depth_or_array_layers: 1,
        },
        mip_level_count: level_count,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: SkyRadianceMap::format_for(capabilities),
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let levels: Vec<wgpu::TextureView> = (0..level_count)
        .map(|level| {
            texture.create_view(&wgpu::TextureViewDescriptor {
                base_mip_level: level,
                mip_level_count: Some(1),
                ..Default::default()
            })
        })
        .collect();
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("sky_radiance_mip_layout"),
        entries: &[texture_entry(0)],
    });
    let sources = levels
        .iter()
        .map(|level| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("sky_radiance_mip_bind_group"),
                layout: &layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(level),
                }],
            })
        })
        .collect();
    let module = create_shader_module(
        device,
        "Sky Radiance Mip",
        include_str!("shaders/sky_radiance_mip.wgsl"),
        &capabilities.shader_defs(),
    );
    let pipeline = fullscreen_pipeline(
        device,
        "Sky Radiance Mip Pipeline",
        &module,
        &[Some(&layout)],
        texture.format(),
        1,
    );
    SkyRadiance {
        map: SkyRadianceMap::new(device, &texture),
        mips: SkyRadianceMips {
            levels,
            sources,
            pipeline,
        },
    }
}

fn lut_texture(device: &wgpu::Device, label: &str, width: u32, height: u32) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba16Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

fn fragment_entry(binding: u32, ty: wgpu::BindingType) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty,
        count: None,
    }
}

fn uniform_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    fragment_entry(
        binding,
        wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
    )
}

fn texture_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    fragment_entry(
        binding,
        wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
    )
}

fn uniform_buffer(device: &wgpu::Device, label: &str, size: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: size as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn fullscreen_pipeline(
    device: &wgpu::Device,
    label: &str,
    module: &wgpu::ShaderModule,
    bind_group_layouts: &[Option<&wgpu::BindGroupLayout>],
    format: wgpu::TextureFormat,
    samples: u32,
) -> wgpu::RenderPipeline {
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(label),
        bind_group_layouts,
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module,
            entry_point: Some("vs_main"),
            buffers: &[],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module,
            entry_point: Some("fs_main"),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState {
            count: samples,
            ..Default::default()
        },
        multiview_mask: None,
        cache: None,
    })
}

#[derive(Resource)]
pub(crate) struct SkyViewPass {
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    uniform: wgpu::Buffer,
    transmittance: wgpu::Texture,
    multiple_scattering: wgpu::Texture,
}

impl SkyViewPass {
    fn new(
        device: &wgpu::Device,
        capabilities: &RenderCapabilities,
        format: wgpu::TextureFormat,
    ) -> Self {
        let transmittance = lut_texture(
            device,
            "sky_transmittance_lut",
            TRANSMITTANCE_LUT_WIDTH,
            TRANSMITTANCE_LUT_HEIGHT,
        );
        let multiple_scattering = lut_texture(
            device,
            "sky_multiple_scattering_lut",
            MULTIPLE_SCATTERING_LUT_SIZE,
            MULTIPLE_SCATTERING_LUT_SIZE,
        );
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("sky_lut_sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let uniform = uniform_buffer(device, "sky_view_uniform", size_of::<SkyViewUniform>());
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sky_view_layout"),
            entries: &[
                uniform_entry(0),
                texture_entry(1),
                texture_entry(2),
                fragment_entry(
                    3,
                    wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                ),
            ],
        });
        let transmittance_view = transmittance.create_view(&wgpu::TextureViewDescriptor::default());
        let multiple_scattering_view =
            multiple_scattering.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sky_view_bind_group"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&transmittance_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&multiple_scattering_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });
        let module = create_shader_module(
            device,
            "Sky View",
            include_str!("shaders/sky_view.wgsl"),
            &capabilities.shader_defs(),
        );
        Self {
            pipeline: fullscreen_pipeline(
                device,
                "Sky View Pipeline",
                &module,
                &[Some(&layout)],
                format,
                1,
            ),
            bind_group,
            uniform,
            transmittance,
            multiple_scattering,
        }
    }
}

#[derive(Resource)]
pub(crate) struct SkyPass {
    pipelines: ViewPipelines,
    bind_group: wgpu::BindGroup,
    uniform: wgpu::Buffer,
}

impl SkyPass {
    fn new(
        device: &wgpu::Device,
        capabilities: &RenderCapabilities,
        camera_layout: &CameraLayout,
        format: wgpu::TextureFormat,
    ) -> Self {
        let uniform = uniform_buffer(device, "sky_sun_uniform", size_of::<SunUniform>());
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sky_sun_layout"),
            entries: &[uniform_entry(0)],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sky_sun_bind_group"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            }],
        });
        let module = create_shader_module(
            device,
            "Sky",
            include_str!("shaders/sky.wgsl"),
            &capabilities.shader_defs(),
        );
        Self {
            pipelines: ViewPipelines::new(|samples| {
                fullscreen_pipeline(
                    device,
                    "Sky Pipeline",
                    &module,
                    &[Some(&camera_layout.camera_layout), Some(&layout)],
                    format,
                    samples,
                )
            }),
            bind_group,
            uniform,
        }
    }
}

fn write_lut(queue: &wgpu::Queue, texture: &wgpu::Texture, lut: &Lut) {
    let bytes: Vec<u8> = lut
        .texels
        .iter()
        .flat_map(|texel| texel.to_array())
        .flat_map(|channel| f16::from_f32(channel).to_le_bytes())
        .collect();
    queue.write_texture(
        texture.as_image_copy(),
        &bytes,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(lut.width * size_of::<[f16; 4]>() as u32),
            rows_per_image: Some(lut.height),
        },
        wgpu::Extent3d {
            width: lut.width,
            height: lut.height,
            depth_or_array_layers: 1,
        },
    );
}

pub(crate) fn upload_atmosphere_luts(
    luts: Res<AtmosphereLuts>,
    pass: Res<SkyViewPass>,
    queue: Res<RenderQueue>,
) {
    if !luts.is_changed() || luts.transmittance.texels.is_empty() {
        return;
    }
    write_lut(&queue, &pass.transmittance, &luts.transmittance);
    write_lut(&queue, &pass.multiple_scattering, &luts.multiple_scattering);
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SkyViewInputs {
    toward_sun: Vec3,
    illuminance: Vec3,
    atmosphere: Atmosphere,
}

pub(crate) struct LastSkyView {
    rendered: Option<SkyViewInputs>,
}

impl FromWorld for LastSkyView {
    fn from_world(_world: &World) -> Self {
        Self { rendered: None }
    }
}

pub(crate) fn render_sky_view(
    sky: Res<ExtractedSky>,
    luts: Res<AtmosphereLuts>,
    pass: Res<SkyViewPass>,
    mips: Res<SkyRadianceMips>,
    mut device: ResMut<RenderDevice>,
    queue: Res<RenderQueue>,
    mut last: SystemLocal<LastSkyView>,
) {
    if sky.cameras.is_empty() {
        return;
    }
    let inputs = sky
        .sun
        .filter(|_| !luts.transmittance.texels.is_empty())
        .map(|sun| SkyViewInputs {
            toward_sun: sun.toward_sun,
            illuminance: sun.illuminance,
            atmosphere: luts.atmosphere.clone(),
        });
    if last.rendered == inputs {
        return;
    }
    if let Some(inputs) = &inputs {
        queue.write_buffer(
            &pass.uniform,
            0,
            bytemuck::bytes_of(&SkyViewUniform::new(inputs)),
        );
    }
    let encoder = device.command_encoder();
    let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        multiview_mask: None,
        label: Some("Sky View Pass"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            depth_slice: None,
            view: &mips.levels[0],
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        occlusion_query_set: None,
        timestamp_writes: None,
    });
    if inputs.is_some() {
        render_pass.set_pipeline(&pass.pipeline);
        render_pass.set_bind_group(0, &pass.bind_group, &[]);
        render_pass.draw(0..3, 0..1);
    }
    drop(render_pass);
    for (source, level) in mips.sources.iter().zip(mips.levels.iter().skip(1)) {
        let mut mip_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            multiview_mask: None,
            label: Some("Sky Radiance Mip Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                depth_slice: None,
                view: level,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            occlusion_query_set: None,
            timestamp_writes: None,
        });
        mip_pass.set_pipeline(&mips.pipeline);
        mip_pass.set_bind_group(0, source, &[]);
        mip_pass.draw(0..3, 0..1);
    }
    last.rendered = inputs;
}

pub(crate) fn draw_sky(
    sky: Res<ExtractedSky>,
    luts: Res<AtmosphereLuts>,
    pass: Res<SkyPass>,
    render_cameras: Query<(Entity, &RenderCamera)>,
    context: Res<RenderContext>,
    mut device: ResMut<RenderDevice>,
    queue: Res<RenderQueue>,
) {
    if sky.cameras.is_empty() {
        return;
    }
    queue.write_buffer(
        &pass.uniform,
        0,
        bytemuck::bytes_of(&SunUniform::new(sky.sun.as_ref(), &luts.atmosphere)),
    );
    let encoder = device.command_encoder();
    for (entity, render_camera) in render_cameras.iter() {
        if !sky.cameras.contains(&entity) {
            continue;
        }
        let rect = render_camera.viewport_rect(render_camera.target_size(&context));
        if rect.is_empty() {
            continue;
        }
        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            multiview_mask: None,
            label: Some("Sky Pass"),
            color_attachments: &[Some(
                render_camera
                    .view_target
                    .color_attachment(wgpu::LoadOp::Load),
            )],
            depth_stencil_attachment: None,
            occlusion_query_set: None,
            timestamp_writes: None,
        });
        rect.apply(&mut render_pass);
        render_pass.set_pipeline(pass.pipelines.get(render_camera.view_target.sample_count()));
        render_pass.set_bind_group(0, &render_camera.camera_bind_group, &[]);
        render_pass.set_bind_group(1, &pass.bind_group, &[]);
        render_pass.draw(0..3, 0..1);
    }
}

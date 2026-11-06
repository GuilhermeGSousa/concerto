use crate::{
    assets::{mesh::Mesh, skeleton::Skeleton, texture::Texture},
    batching::{
        clear_draw_stats, BatchedInstanceBuffers, InstanceInputUniformBuffer, RenderDrawStats,
    },
    capabilities::RenderCapabilities,
    components::{
        camera::{extract_cameras, sync_camera_aspect, Camera},
        camera_environment::CameraEnvironment,
        light::{extract_lights, update_changed_lights, Light, RenderLights},
        material::MaterialComponent,
        mesh::{extract_meshes, RenderMeshInstances},
        render_entity::{extract, SyncWithRenderWorld},
        shadows::{
            assign_shadow_layers, resize_shadow_maps, update_shadow_view_proj, RenderLighting,
            RenderPointShadowMaps, RenderShadowViewProjs, RenderSpotDirectionalShadowMaps,
        },
        skeleton::{extract_skeletons, SkinUniforms},
        visibility::{NoFrustumCulling, NotShadowCaster},
    },
    device::RenderDevice,
    draw_target::DrawTarget,
    gpu::Gpu,
    layouts::{CameraLayout, LightingLayout, SkeletonLayout},
    material_plugin::clear_cameras,
    queue::RenderQueue,
    render_asset::{
        register_render_asset_budget,
        render_mesh::RenderMesh,
        render_texture::{DummyRenderTexture, RenderTexture},
        render_window::RenderWindow,
        RenderAssetPlugin,
    },
    resources::RenderContext,
    sets::RenderSet,
    systems::{
        render::{finish_render, present_window},
        update_window,
    },
};
use concerto_app::{
    edit_time::AtEditTime,
    plugins::Plugin,
    schedule_groups::{Extract, LateRender, LateUpdate, Render, RenderMain, Update},
};
use concerto_ecs::{IntoSetConfig, IntoSystemConfig, World};

fn offscreen_config(format: wgpu::TextureFormat) -> wgpu::SurfaceConfiguration {
    wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format,
        width: 0,
        height: 0,
        present_mode: wgpu::PresentMode::Fifo,
        alpha_mode: wgpu::CompositeAlphaMode::Opaque,
        view_formats: vec![],
        desired_maximum_frame_latency: 2,
    }
}

fn render_main(world: &mut World) {
    {
        profiling::scope!("schedule::render");
        world.run_schedule(Render);
    }

    {
        profiling::scope!("schedule::late_render");
        world.run_schedule(LateRender);
    }

    {
        profiling::scope!("world_tick");
        world.tick();
    }
}

/// Sets up the render world's schedules, extraction and presentation.
///
/// Builds on the [`Gpu`] and the [`DrawTarget`] its host inserted before the recipe ran.
///
/// # Panics
///
/// Panics during [`finish`](Plugin::finish) if the `App` has no [`Gpu`] resource.
pub struct RenderPlugin;

/// Makes meshes and skeletons require `SyncWithRenderWorld`.
pub fn register_sync_requirements(app: &mut concerto_app::App) {
    app.register_required_components::<concerto_mesh::MeshComponent, SyncWithRenderWorld>()
        .register_required_components::<concerto_mesh::SkeletonComponent, SyncWithRenderWorld>();
}

impl Plugin for RenderPlugin {
    fn build(&self, app: &mut concerto_app::App) {
        app.render_mut()
            .set_update_schedule(RenderMain)
            .add_system(RenderMain, render_main);

        let has_window = app
            .get_resource::<concerto_window::plugin::Window>()
            .is_some();

        register_render_asset_budget(app);
        app.register_plugin(RenderAssetPlugin::<RenderMesh>::new())
            .register_plugin(RenderAssetPlugin::<RenderTexture>::new());
        app.register_asset::<Mesh>()
            .register_asset::<Texture>()
            .register_asset::<Skeleton>();
        app.register_type::<Light>()
            .register_type::<Camera>()
            .register_type::<CameraEnvironment>()
            .register_type::<SyncWithRenderWorld>()
            .register_type::<NoFrustumCulling>()
            .register_type::<NotShadowCaster>()
            .register_type::<MaterialComponent>()
            .register_type::<concerto_mesh::MeshComponent>()
            .register_type::<concerto_mesh::SkeletonComponent>()
            .register_type::<concerto_mesh::SkeletonGizmo>();
        register_sync_requirements(app);
        app.track_removals::<concerto_mesh::MeshComponent>()
            .track_removals::<concerto_mesh::SkeletonComponent>()
            .track_removals::<NoFrustumCulling>()
            .track_removals::<NotShadowCaster>()
            .track_removals::<concerto_mesh::Aabb>();
        app.add_system(Update, concerto_mesh::update_mesh_bounds.at_edit_time());

        app.set_extract_fn(extract);
        app.add_render_system(Extract, extract_cameras)
            .add_render_system(Extract, extract_meshes)
            .add_render_system(Extract, extract_lights)
            .add_render_system(Extract, extract_skeletons);

        if has_window {
            app.add_system(LateUpdate, sync_camera_aspect.at_edit_time())
                .add_render_system(Extract, update_window::extract_window)
                .add_render_system(Render, update_window::update_render_window);
        }

        app.configure_render_sets(
            Render,
            (
                RenderSet::Lights,
                RenderSet::Queue,
                RenderSet::Shadows,
                RenderSet::Draw,
                RenderSet::Overlay,
            )
                .chain(),
        )
        .add_render_system(Render, clear_cameras)
        .add_render_system(Render, clear_draw_stats.in_set(RenderSet::Queue))
        .add_render_system(Render, assign_shadow_layers.in_set(RenderSet::Lights))
        .add_render_system(
            Render,
            update_changed_lights
                .in_set(RenderSet::Lights)
                .after(assign_shadow_layers),
        )
        .add_render_system(
            Render,
            (update_shadow_view_proj, resize_shadow_maps).in_set(RenderSet::Shadows),
        )
        .add_render_system(LateRender, finish_render)
        .add_render_system(
            LateRender,
            present_window
                .in_set(RenderSet::Present)
                .after(finish_render),
        );
    }

    fn finish(&self, app: &mut concerto_app::App) {
        let gpu = app
            .get_resource::<Gpu>()
            .cloned()
            .expect("RenderPlugin needs a Gpu resource");
        let target = app
            .get_resource::<DrawTarget>()
            .cloned()
            .unwrap_or(DrawTarget::None);

        let size = app
            .get_resource::<concerto_window::plugin::Window>()
            .map_or((0, 0), |window| window.size());

        let config = match &target {
            DrawTarget::Surface(surface) => {
                let surface_caps = surface.get_capabilities(&gpu.adapter);
                let surface_format = surface_caps
                    .formats
                    .iter()
                    .find(|f| f.is_srgb())
                    .copied()
                    .unwrap_or(surface_caps.formats[0]);
                let (width, height) = if size.0 == 0 || size.1 == 0 {
                    (0, 0)
                } else {
                    size
                };
                let config = wgpu::SurfaceConfiguration {
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                    format: surface_format,
                    width,
                    height,
                    present_mode: surface_caps.present_modes[0],
                    alpha_mode: surface_caps.alpha_modes[0],
                    view_formats: vec![],
                    desired_maximum_frame_latency: 2,
                };
                if width != 0 {
                    surface.configure(&gpu.device, &config);
                }
                config
            }
            DrawTarget::Texture(shared) => offscreen_config(shared.format()),
            DrawTarget::None => offscreen_config(wgpu::TextureFormat::Rgba8Unorm),
        };

        let Gpu {
            device,
            queue,
            adapter,
        } = gpu;

        let camera_layouts = CameraLayout::new(&device);

        let skeleton_layout = SkeletonLayout::new(&device);

        let capabilities = RenderCapabilities::from_adapter(&adapter);

        let lighting_layout = LightingLayout::new(&device, &capabilities);

        let render_lights = RenderLights::new(&device);
        let render_spot_directional_shadow_maps =
            RenderSpotDirectionalShadowMaps::new(&device, &capabilities);
        let render_point_shadow_maps = RenderPointShadowMaps::new(&device, &capabilities);
        let render_shadow_view_projs = RenderShadowViewProjs::new(&device);
        let render_lighting = RenderLighting::new(
            &device,
            &lighting_layout,
            &render_lights,
            &render_spot_directional_shadow_maps,
            &render_point_shadow_maps,
            &render_shadow_view_projs,
        );
        let skin_uniforms = SkinUniforms::new(&device, &skeleton_layout, &queue);
        let batched_instance_buffers = BatchedInstanceBuffers {
            current_input_buffer: InstanceInputUniformBuffer::new(),
        };

        app.render_mut()
            .insert_resource(DummyRenderTexture::new(&device))
            .insert_resource(RenderContext {
                target,
                surface_config: config,
            })
            .insert_resource(RenderDevice {
                device,
                encoder: None,
            })
            .insert_resource(RenderQueue { queue })
            .insert_resource(capabilities)
            .insert_resource(RenderWindow::new())
            .insert_resource(camera_layouts)
            .insert_resource(skeleton_layout)
            .insert_resource(lighting_layout)
            .insert_resource(render_lights)
            .insert_resource(render_spot_directional_shadow_maps)
            .insert_resource(render_point_shadow_maps)
            .insert_resource(render_shadow_view_projs)
            .insert_resource(render_lighting)
            .insert_resource(skin_uniforms)
            .insert_resource(batched_instance_buffers)
            .insert_resource(RenderDrawStats::default())
            .insert_resource(RenderMeshInstances::default());
    }
}

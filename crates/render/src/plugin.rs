use crate::{
    assets::{mesh::Mesh, skeleton::Skeleton, texture::Texture},
    capabilities::RenderCapabilities,
    components::{
        camera::{extract_cameras, sync_camera_aspect},
        light::{extract_lights, update_changed_lights, RenderLights},
        mesh::extract_meshes,
        render_entity::extract,
        shadows::{
            resize_shadow_maps, update_shadow_view_proj, RenderLighting, RenderPointShadowMaps,
            RenderShadowViewProjs, RenderSpotDirectionalShadowMaps,
        },
        skeleton::{extract_skeletons, SkinUniforms},
        world_environment::WorldEnvironment,
    },
    device::RenderDevice,
    layouts::{CameraLayout, LightingLayout, SkeletonLayout},
    material_plugin::clear_cameras,
    queue::RenderQueue,
    render_asset::{
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
    plugins::Plugin,
    schedule_groups::{Extract, LateRender, LateUpdate, Render, RenderMain, Update},
};
use concerto_color::Color;
use concerto_ecs::{resource::Resource, IntoSetConfig, IntoSystemConfig, World};
use std::sync::{Arc, Mutex};
use wgpu::{Adapter, Device, Instance, Limits, MemoryHints, Queue};

#[doc(hidden)]
pub struct RenderResources {
    pub device: Device,
    pub queue: Queue,
    pub adapter: Adapter,
    pub instance: Instance,
    pub surface: Option<Arc<wgpu::Surface<'static>>>,
}

#[derive(Resource)]
struct FutureRenderResources(Arc<Mutex<Option<RenderResources>>>);

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

/// Sets up the GPU, the render world's schedules, extraction and presentation.
///
/// Renders to the window if a [`Window`](concerto_window::plugin::Window) exists at build
/// time, and offscreen otherwise.
///
/// # Panics
///
/// Panics during [`build`](Plugin::build) if no GPU adapter or device is available.
pub struct RenderPlugin;

impl RenderPlugin {
    async fn initialize_renderer(
        window_handle: Option<Arc<winit::window::Window>>,
    ) -> RenderResources {
        // wasm renders through WebGL2 (see the `webgl` feature and the
        // downlevel_webgl2 limits below). `Instance::default()` enables every
        // backend and prefers WebGPU, which yields no adapter on browsers
        // without it, so pin wasm to GL and leave native on all backends.
        let backends = if cfg!(target_arch = "wasm32") {
            wgpu::Backends::GL
        } else {
            wgpu::Backends::all()
        };
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends,
            ..Default::default()
        });

        let surface = window_handle.as_ref().map(|handle| {
            Arc::new(
                instance
                    .create_surface(Arc::clone(handle))
                    .expect("Error creating surface."),
            )
        });

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::default(),
                compatible_surface: surface.as_deref(),
                force_fallback_adapter: false,
            })
            .await
            .unwrap();

        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    required_features: wgpu::Features::empty(),
                    required_limits: if cfg!(target_arch = "wasm32") {
                        // WebGL2 only guarantees 4 draw buffers, but
                        // `downlevel_webgl2_defaults` asks for 8, which some
                        // browsers/GPUs (e.g. SwiftShader, many mobiles)
                        // reject. The engine never renders to more than one
                        // color target, so take whatever the adapter offers.
                        Limits {
                            max_color_attachments: adapter.limits().max_color_attachments,
                            ..Limits::downlevel_webgl2_defaults()
                        }
                        .using_resolution(adapter.limits())
                    } else {
                        wgpu::Limits::default()
                    },
                    label: None,
                    memory_hints: MemoryHints::Performance,
                },
                None,
            )
            .await
            .unwrap();

        RenderResources {
            device,
            queue,
            adapter,
            instance,
            surface,
        }
    }
}

impl Plugin for RenderPlugin {
    fn build(&self, app: &mut concerto_app::App) {
        app.render_mut()
            .set_update_schedule(RenderMain)
            .add_system(RenderMain, render_main);

        let future_render_resources_wrapper = Arc::new(Mutex::new(None));
        app.insert_resource(FutureRenderResources(
            future_render_resources_wrapper.clone(),
        ));

        let is_windowed = app
            .get_resource::<concerto_window::plugin::Window>()
            .is_some();

        if is_windowed {
            let window = app
                .get_resource::<concerto_window::plugin::Window>()
                .unwrap();
            let window_handle = Arc::clone(&window.window_handle);

            let async_init = async move {
                let resources = RenderPlugin::initialize_renderer(Some(window_handle)).await;
                *future_render_resources_wrapper.lock().unwrap() = Some(resources);
            };

            #[cfg(target_arch = "wasm32")]
            wasm_bindgen_futures::spawn_local(async_init);
            #[cfg(not(target_arch = "wasm32"))]
            pollster::block_on(async_init);
        } else {
            let async_init = async move {
                let resources = RenderPlugin::initialize_renderer(None).await;
                *future_render_resources_wrapper.lock().unwrap() = Some(resources);
            };
            pollster::block_on(async_init);
        }

        app.register_plugin(RenderAssetPlugin::<RenderMesh>::new())
            .register_plugin(RenderAssetPlugin::<RenderTexture>::new());
        app.register_asset::<Mesh>()
            .register_asset::<Texture>()
            .register_asset::<Skeleton>();

        app.set_extract_fn(extract);
        app.add_render_system(Extract, extract_cameras)
            .add_render_system(Extract, extract_meshes)
            .add_render_system(Extract, extract_lights)
            .add_render_system(Extract, extract_skeletons);

        if is_windowed {
            app.add_system(Update, update_window::request_window_resize)
                .add_system(LateUpdate, sync_camera_aspect)
                .add_render_system(Extract, update_window::extract_window)
                .add_render_system(Render, update_window::update_render_window);
        }

        app.configure_render_sets(
            Render,
            (RenderSet::Lights, RenderSet::Shadows, RenderSet::Draw).chain(),
        )
        .add_render_system(Render, clear_cameras)
        .add_render_system(Render, update_changed_lights.in_set(RenderSet::Lights))
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

    fn ready(&self, app: &concerto_app::App) -> bool {
        app.get_resource::<FutureRenderResources>()
            .and_then(|future_render_resources| {
                future_render_resources
                    .0
                    .try_lock()
                    .map(|mutex| mutex.is_some())
                    .ok()
            })
            .unwrap_or(true)
    }

    fn finish(&self, app: &mut concerto_app::App) {
        let RenderResources {
            device,
            queue,
            adapter,
            instance: _instance,
            surface,
        } = app
            .remove_resource::<FutureRenderResources>()
            .unwrap()
            .0
            .lock()
            .unwrap()
            .take()
            .unwrap();

        let config = if let Some(ref surface) = surface {
            let surface_caps = surface.get_capabilities(&adapter);
            let surface_format = surface_caps
                .formats
                .iter()
                .find(|f| f.is_srgb())
                .copied()
                .unwrap_or(surface_caps.formats[0]);

            let window = app
                .get_resource::<concerto_window::plugin::Window>()
                .unwrap();
            let size = window.size();

            let config = wgpu::SurfaceConfiguration {
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                format: surface_format,
                width: size.0,
                height: size.1,
                present_mode: surface_caps.present_modes[0],
                alpha_mode: surface_caps.alpha_modes[0],
                view_formats: vec![],
                desired_maximum_frame_latency: 2,
            };

            surface.configure(&device, &config);
            config
        } else {
            let surface_format = wgpu::TextureFormat::Rgba8Unorm;
            wgpu::SurfaceConfiguration {
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                format: surface_format,
                width: 0,
                height: 0,
                present_mode: wgpu::PresentMode::Fifo,
                alpha_mode: wgpu::CompositeAlphaMode::Opaque,
                view_formats: vec![],
                desired_maximum_frame_latency: 2,
            }
        };

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

        app.render_mut()
            .insert_resource(DummyRenderTexture::new(&device))
            .insert_resource(RenderContext {
                surface,
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
            .insert_resource(WorldEnvironment::new(Color::rgba(0.1, 0.1, 0.1, 0.1)));
    }
}

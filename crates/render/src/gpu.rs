use concerto_ecs::resource::Resource;
use wgpu::{Limits, MemoryHints};

/// The GPU an `App` renders with. A host creates it and may give the same one to several `App`s.
#[derive(Resource, Clone)]
pub struct Gpu {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub adapter: wgpu::Adapter,
}

impl Gpu {
    /// The wgpu instance a host creates its surface from before requesting a `Gpu`.
    pub fn instance() -> wgpu::Instance {
        // wasm renders through WebGL2 (see the `webgl` feature and the
        // downlevel_webgl2 limits below). `Instance::default()` enables every
        // backend and prefers WebGPU, which yields no adapter on browsers
        // without it, so pin wasm to GL and leave native on all backends.
        let backends = if cfg!(target_arch = "wasm32") {
            wgpu::Backends::GL
        } else {
            wgpu::Backends::all()
        };
        wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends,
            ..Default::default()
        })
    }

    /// Requests a device, or returns `None` when the machine has no suitable adapter.
    pub async fn request(
        instance: &wgpu::Instance,
        compatible_surface: Option<&wgpu::Surface<'_>>,
    ) -> Option<Gpu> {
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::default(),
                compatible_surface,
                force_fallback_adapter: false,
            })
            .await?;

        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    required_features: wgpu::Features::empty(),
                    required_limits: if cfg!(target_arch = "wasm32") {
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
            .ok()?;

        Some(Gpu {
            device,
            queue,
            adapter,
        })
    }
}

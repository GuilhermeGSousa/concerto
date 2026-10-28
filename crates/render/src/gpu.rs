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

    /// Requests a `Gpu` for a test; with no adapter it prints a skip line and returns `None`, or panics when `CONCERTO_REQUIRE_GPU` is set.
    pub fn request_for_test() -> Option<Gpu> {
        let found = pollster::block_on(Gpu::request(&Gpu::instance(), None));
        skip_without_adapter(
            found,
            std::env::var_os("CONCERTO_REQUIRE_GPU").is_some_and(|value| !value.is_empty()),
        )
    }
}

fn skip_without_adapter<T>(found: Option<T>, require: bool) -> Option<T> {
    if found.is_none() {
        assert!(
            !require,
            "CONCERTO_REQUIRE_GPU is set but no GPU adapter is available"
        );
        eprintln!("SKIPPED: no GPU adapter is available");
    }
    found
}

#[cfg(test)]
mod tests {
    use super::skip_without_adapter;

    #[test]
    fn a_found_adapter_is_returned() {
        assert_eq!(skip_without_adapter(Some(1), true), Some(1));
    }

    #[test]
    fn a_missing_adapter_skips_when_not_required() {
        assert_eq!(skip_without_adapter::<u8>(None, false), None);
    }

    #[test]
    #[should_panic(expected = "CONCERTO_REQUIRE_GPU")]
    fn a_missing_adapter_panics_when_required() {
        skip_without_adapter::<u8>(None, true);
    }
}

use std::fmt;

use concerto_ecs::resource::Resource;
use wgpu::{Backend, DeviceType, Limits, MemoryHints};

/// The GPU an `App` renders with. A host creates it and may give the same one to several `App`s.
#[derive(Resource, Clone)]
pub struct Gpu {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub adapter: wgpu::Adapter,
}

impl Gpu {
    /// The wgpu instance a host creates its surface from before requesting a `Gpu`; a windowed host passes its event loop's display, which GLES needs to present.
    pub fn instance(display: Option<Box<dyn wgpu::wgt::WgpuHasDisplayHandle>>) -> wgpu::Instance {
        // wasm renders through WebGL2 (see the `webgl` feature and the
        // downlevel_webgl2 limits below). `Instance::default()` enables every
        // backend and prefers WebGPU, which yields no adapter on browsers
        // without it, so pin wasm to GL and leave native on all backends.
        let backends = if cfg!(target_arch = "wasm32") {
            wgpu::Backends::GL
        } else {
            wgpu::Backends::all()
        };
        wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends,
            display,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        })
    }

    /// Requests a device, or returns `None`, logging why, when the machine has no suitable adapter or the adapter gives no device; panics when `WGPU_ADAPTER_NAME` names no usable adapter.
    pub async fn request(
        instance: &wgpu::Instance,
        compatible_surface: Option<&wgpu::Surface<'_>>,
    ) -> Option<Gpu> {
        Gpu::try_request(instance, compatible_surface)
            .await
            .inspect_err(|error| log::warn!("No GPU: {error}"))
            .ok()
    }

    /// Requests a device as `request` does, saying why when there is none.
    pub async fn try_request(
        instance: &wgpu::Instance,
        compatible_surface: Option<&wgpu::Surface<'_>>,
    ) -> Result<Gpu, GpuRequestError> {
        let adapter =
            match wgpu::util::initialize_adapter_from_env(instance, compatible_surface).await {
                Ok(adapter) => adapter,
                Err(_) => instance
                    .request_adapter(&wgpu::RequestAdapterOptions {
                        power_preference: power_preference(wgpu::PowerPreference::from_env()),
                        compatible_surface,
                        force_fallback_adapter: false,
                        apply_limit_buckets: false,
                    })
                    .await
                    .map_err(|_| GpuRequestError::NoAdapter)?,
            };
        let info = adapter.get_info();
        log::info!(
            "Rendering on {} ({:?}, {:?})",
            info.name,
            info.device_type,
            info.backend
        );

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                required_features: adapter.features()
                    & wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES,
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
                experimental_features: wgpu::ExperimentalFeatures::disabled(),
                trace: wgpu::Trace::Off,
            })
            .await
            .map_err(|error| GpuRequestError::Device {
                error,
                name: info.name.clone(),
                device_type: info.device_type,
                backend: info.backend,
            })?;

        Ok(Gpu {
            device,
            queue,
            adapter,
        })
    }

    /// Requests a `Gpu` for a test; without one it prints a skip line naming why and the adapters seen, and returns `None`, or panics with that reason when `CONCERTO_REQUIRE_GPU` is set.
    pub fn request_for_test() -> Option<Gpu> {
        let instance = Gpu::instance(None);
        let found = pollster::block_on(Gpu::try_request(&instance, None))
            .map_err(|error| format!("{error}; adapters seen: {}", adapters_seen(&instance)));
        skip_without_gpu(
            found,
            std::env::var_os("CONCERTO_REQUIRE_GPU").is_some_and(|value| !value.is_empty()),
        )
    }
}

/// Why `Gpu::try_request` gave no `Gpu`.
#[derive(Debug)]
pub enum GpuRequestError {
    NoAdapter,
    Device {
        name: String,
        device_type: DeviceType,
        backend: Backend,
        error: wgpu::RequestDeviceError,
    },
}

impl fmt::Display for GpuRequestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GpuRequestError::NoAdapter => write!(f, "no adapter matched the request"),
            GpuRequestError::Device {
                error,
                name,
                device_type,
                backend,
            } => write!(
                f,
                "{} ({:?}, {:?}) gave no device: {error}",
                name, device_type, backend
            ),
        }
    }
}

impl std::error::Error for GpuRequestError {}

#[cfg(not(target_arch = "wasm32"))]
fn adapters_seen(instance: &wgpu::Instance) -> String {
    let seen: Vec<String> = pollster::block_on(instance.enumerate_adapters(wgpu::Backends::all()))
        .iter()
        .map(|adapter| {
            let info = adapter.get_info();
            format!("{} ({:?}, {:?})", info.name, info.device_type, info.backend)
        })
        .collect();
    if seen.is_empty() {
        "none".to_owned()
    } else {
        seen.join(", ")
    }
}

#[cfg(target_arch = "wasm32")]
fn adapters_seen(_instance: &wgpu::Instance) -> String {
    "not listed on the web".to_owned()
}

fn power_preference(from_env: Option<wgpu::PowerPreference>) -> wgpu::PowerPreference {
    from_env.unwrap_or(wgpu::PowerPreference::HighPerformance)
}

fn skip_without_gpu<T>(found: Result<T, String>, require: bool) -> Option<T> {
    match found {
        Ok(gpu) => Some(gpu),
        Err(reason) => {
            assert!(
                !require,
                "CONCERTO_REQUIRE_GPU is set but no GPU is available: {reason}"
            );
            eprintln!("SKIPPED: no GPU is available: {reason}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::skip_without_gpu;

    struct Lookup {
        found: Result<u8, String>,
        required: bool,
        returned: Option<u8>,
    }

    #[test]
    fn a_missing_gpu_skips_unless_required_and_then_panics() {
        let cases = [
            Lookup {
                found: Ok(1),
                required: true,
                returned: Some(1),
            },
            Lookup {
                found: Err("no adapter".into()),
                required: false,
                returned: None,
            },
        ];
        for case in cases {
            assert_eq!(skip_without_gpu(case.found, case.required), case.returned);
        }

        std::panic::catch_unwind(|| {
            skip_without_gpu::<u8>(Err("device lost".into()), true);
        })
        .expect_err("a required GPU that is missing panics");
    }
}

use concerto_ecs::{
    component::Component,
    reflect::{ReflectComponent, ReflectSceneComponent},
    resource::Resource,
};
use concerto_reflect::{Reflect, ReflectDefault};

/// How many samples per pixel a camera draws its scene with; edges are smoothed by resolving them.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Reflect)]
#[reflect(Component, Default, SceneComponent)]
pub enum Msaa {
    Off,
    Sample2,
    #[default]
    Sample4,
    Sample8,
}

impl Msaa {
    /// The number of samples per pixel this setting asks for.
    pub fn samples(self) -> u32 {
        match self {
            Msaa::Off => 1,
            Msaa::Sample2 => 2,
            Msaa::Sample4 => 4,
            Msaa::Sample8 => 8,
        }
    }
}

const COUNTS: [u32; 4] = [1, 2, 4, 8];

/// The sample counts the main pass can draw with on this device.
#[derive(Resource, Clone, Copy, Debug, PartialEq, Eq)]
pub struct MsaaSupport {
    counts: [bool; 4],
}

impl MsaaSupport {
    /// Support for the given sample counts; 1 is always supported.
    pub fn new(counts: &[u32]) -> Self {
        Self {
            counts: COUNTS.map(|count| count == 1 || counts.contains(&count)),
        }
    }

    pub(crate) fn from_device(
        adapter: &wgpu::Adapter,
        device: &wgpu::Device,
        color: wgpu::TextureFormat,
        depth: wgpu::TextureFormat,
    ) -> Self {
        let adapter_specific = device
            .features()
            .contains(wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES);
        let color = adapter.get_texture_format_features(color).flags;
        let depth = adapter.get_texture_format_features(depth).flags;
        let supported: Vec<u32> = COUNTS
            .into_iter()
            .filter(|&count| {
                (count == 4 || adapter_specific)
                    && color.sample_count_supported(count)
                    && depth.sample_count_supported(count)
                    && (count == 1
                        || color.contains(wgpu::TextureFormatFeatureFlags::MULTISAMPLE_RESOLVE))
            })
            .collect();
        Self::new(&supported)
    }

    /// The supported sample count nearest at or above what `msaa` asks for, else the highest.
    pub fn samples(&self, msaa: Msaa) -> u32 {
        let asked = msaa.samples();
        let supported = || {
            COUNTS
                .into_iter()
                .zip(self.counts)
                .filter_map(|(count, supported)| supported.then_some(count))
        };
        supported()
            .find(|&count| count >= asked)
            .or_else(|| supported().next_back())
            .unwrap_or(1)
    }
}

#[cfg(test)]
mod tests {
    use super::{Msaa, MsaaSupport};

    struct Case {
        supported: &'static [u32],
        msaa: Msaa,
        samples: u32,
    }

    #[test]
    fn a_camera_gets_the_nearest_supported_count_at_or_above_what_it_asks() {
        let case = |supported, msaa, samples| Case {
            supported,
            msaa,
            samples,
        };
        let cases = [
            case(&[4], Msaa::Off, 1),
            case(&[4], Msaa::Sample2, 4),
            case(&[4], Msaa::Sample4, 4),
            case(&[4], Msaa::Sample8, 4),
            case(&[2, 4, 8], Msaa::Sample2, 2),
            case(&[2, 4, 8], Msaa::Sample8, 8),
            case(&[], Msaa::Sample4, 1),
        ];
        for case in cases {
            assert_eq!(
                MsaaSupport::new(case.supported).samples(case.msaa),
                case.samples,
                "{:?} on {:?}",
                case.msaa,
                case.supported
            );
        }
    }
}

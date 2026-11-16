use concerto_foundation::assets::{asset_server::AssetLoadContext, Asset, LoadableAsset};

pub use wgpu_types::TextureFormat;

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum TextureKind {
    Sampled,
    RenderTarget,
}

#[derive(Asset, serde::Serialize, serde::Deserialize)]
pub struct Texture {
    pub width: u32,
    pub height: u32,
    pub format: wgpu_types::TextureFormat,
    pub kind: TextureKind,
    /// RGBA8 pixels matching `format`. Empty for `TextureKind::RenderTarget`.
    pub data: Vec<u8>,
    /// The mip levels below `data`, largest first, built from it when the texture loads.
    #[serde(skip)]
    pub mips: Vec<Vec<u8>>,
}

impl Texture {
    /// A GPU-only render target; the camera system allocates the wgpu texture.
    pub fn render_target(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            format: wgpu_types::TextureFormat::Rgba8UnormSrgb,
            kind: TextureKind::RenderTarget,
            data: Vec::new(),
            mips: Vec::new(),
        }
    }

    pub fn size(&self) -> wgpu::Extent3d {
        wgpu::Extent3d {
            width: self.width,
            height: self.height,
            depth_or_array_layers: 1,
        }
    }

    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// The number of mip levels, counting `data` as the first.
    pub fn mip_level_count(&self) -> u32 {
        1 + self.mips.len() as u32
    }
}

impl LoadableAsset for Texture {
    fn on_load(&mut self, _context: &AssetLoadContext) -> anyhow::Result<()> {
        if self.kind == TextureKind::Sampled && self.mips.is_empty() {
            self.mips = build_mips(self.width, self.height, self.format, &self.data);
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
struct MipExtent {
    width: u32,
    height: u32,
}

fn build_mips(width: u32, height: u32, format: TextureFormat, data: &[u8]) -> Vec<Vec<u8>> {
    if data.len() != width as usize * height as usize * 4 {
        return Vec::new();
    }
    let srgb = format.is_srgb();
    let decode: Vec<f32> = (0..=255u8)
        .map(|byte| {
            let value = byte as f32 / 255.0;
            if srgb {
                srgb_to_linear(value)
            } else {
                value
            }
        })
        .collect();

    let mut mips = Vec::new();
    let mut extent = MipExtent { width, height };
    let mut source = data;
    while extent.width > 1 || extent.height > 1 {
        let next = MipExtent {
            width: (extent.width / 2).max(1),
            height: (extent.height / 2).max(1),
        };
        mips.push(downsample(source, extent, next, &decode, srgb));
        extent = next;
        source = mips.last().unwrap();
    }
    mips
}

fn downsample(
    source: &[u8],
    extent: MipExtent,
    next: MipExtent,
    decode: &[f32],
    srgb: bool,
) -> Vec<u8> {
    let mut out = Vec::with_capacity(next.width as usize * next.height as usize * 4);
    for y in 0..next.height {
        let rows = [
            (y * 2).min(extent.height - 1),
            (y * 2 + 1).min(extent.height - 1),
        ];
        for x in 0..next.width {
            let columns = [
                (x * 2).min(extent.width - 1),
                (x * 2 + 1).min(extent.width - 1),
            ];
            for channel in 0..4 {
                let mut sum = 0.0;
                for row in rows {
                    for column in columns {
                        let index = ((row * extent.width + column) * 4) as usize + channel;
                        let byte = source[index];
                        sum += if channel == 3 {
                            byte as f32 / 255.0
                        } else {
                            decode[byte as usize]
                        };
                    }
                }
                let mean = sum / 4.0;
                let encoded = if srgb && channel != 3 {
                    linear_to_srgb(mean)
                } else {
                    mean
                };
                out.push((encoded * 255.0).round().clamp(0.0, 255.0) as u8);
            }
        }
    }
    out
}

fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb(c: f32) -> f32 {
    if c <= 0.0031308 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

#[cfg(test)]
mod tests {
    use super::{build_mips, TextureFormat};

    struct Case {
        width: u32,
        height: u32,
        format: TextureFormat,
        data: Vec<u8>,
        mips: Vec<Vec<u8>>,
    }

    #[test]
    fn mips_halve_down_to_one_texel_and_average_in_linear_space() {
        let cases = [
            Case {
                width: 4,
                height: 2,
                format: TextureFormat::Rgba8Unorm,
                data: [
                    [0, 0, 0, 0],
                    [100, 200, 40, 255],
                    [8, 8, 8, 8],
                    [8, 8, 8, 8],
                    [100, 200, 40, 255],
                    [0, 0, 0, 0],
                    [8, 8, 8, 8],
                    [8, 8, 8, 8],
                ]
                .concat(),
                mips: vec![
                    [[50, 100, 20, 128], [8, 8, 8, 8]].concat(),
                    vec![29, 54, 14, 68],
                ],
            },
            Case {
                width: 3,
                height: 1,
                format: TextureFormat::Rgba8Unorm,
                data: [[0, 0, 0, 0], [200, 200, 200, 200], [255, 255, 255, 255]].concat(),
                mips: vec![vec![100, 100, 100, 100]],
            },
            Case {
                width: 2,
                height: 1,
                format: TextureFormat::Rgba8UnormSrgb,
                data: [[0, 0, 0, 0], [255, 255, 255, 255]].concat(),
                mips: vec![vec![188, 188, 188, 128]],
            },
            Case {
                width: 1,
                height: 1,
                format: TextureFormat::Rgba8UnormSrgb,
                data: vec![1, 2, 3, 4],
                mips: vec![],
            },
            Case {
                width: 2,
                height: 2,
                format: TextureFormat::Rgba8UnormSrgb,
                data: vec![],
                mips: vec![],
            },
        ];
        for case in cases {
            assert_eq!(
                build_mips(case.width, case.height, case.format, &case.data),
                case.mips,
                "{}x{} {:?}",
                case.width,
                case.height,
                case.format,
            );
        }
    }
}

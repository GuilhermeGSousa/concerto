use anyhow::Context;
use image::{imageops::FilterType, ImageFormat, RgbaImage};
use wgpu_types::TextureFormat;

use super::blob::{block_extent, level_extent, TextureLevels};
use super::settings::{ColorSpace, Compression, TextureRole, TextureSettings};
use super::{SourceEncoding, TextureSource};

#[cfg(not(target_arch = "wasm32"))]
use link_cplusplus as _;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct ResolvedTexture {
    pub(crate) compression: Compression,
    pub(crate) format: TextureFormat,
}

impl ResolvedTexture {
    pub(crate) fn resolve(
        role: TextureRole,
        settings: &TextureSettings,
        has_alpha: bool,
    ) -> anyhow::Result<Self> {
        let compression = match (settings.compression, role) {
            (Compression::Auto, TextureRole::Color) if has_alpha => Compression::Bc7,
            (Compression::Auto, TextureRole::Color) => Compression::Bc1,
            (Compression::Auto, TextureRole::Normal) => Compression::Bc5,
            (Compression::Auto, TextureRole::Data) => Compression::Bc7,
            (compression, _) => compression,
        };
        let srgb = settings.color_space_for(role) == ColorSpace::Srgb;
        let format = match (compression, srgb) {
            (Compression::Bc7, true) => TextureFormat::Bc7RgbaUnormSrgb,
            (Compression::Bc7, false) => TextureFormat::Bc7RgbaUnorm,
            (Compression::Bc1, true) => TextureFormat::Bc1RgbaUnormSrgb,
            (Compression::Bc1, false) => TextureFormat::Bc1RgbaUnorm,
            (Compression::Uncompressed, true) => TextureFormat::Rgba8UnormSrgb,
            (Compression::Uncompressed, false) => TextureFormat::Rgba8Unorm,
            (Compression::Bc5 | Compression::Bc4, true) => anyhow::bail!(
                "{compression:?} has no sRGB form; set the texture's colour space to Linear"
            ),
            (Compression::Bc5, false) => TextureFormat::Bc5RgUnorm,
            (Compression::Bc4, false) => TextureFormat::Bc4RUnorm,
            (Compression::Auto, _) => unreachable!("Auto resolves above"),
        };
        Ok(Self {
            compression,
            format,
        })
    }
}

impl TextureSource {
    /// Derives the levels the GPU samples from `source`, as a texture of `role` with `settings`.
    ///
    /// # Errors
    ///
    /// Returns an error when `source` fails to decode, when `settings` ask for a
    /// block compression with no sRGB form (BC4 or BC5) in the sRGB colour space,
    /// or, on wasm, when `settings` ask for any block compression, which the web
    /// build cannot encode.
    pub fn derive_levels(
        &self,
        role: TextureRole,
        settings: &TextureSettings,
    ) -> anyhow::Result<TextureLevels> {
        let image = self.decode()?;
        let resolved = ResolvedTexture::resolve(role, settings, has_alpha(&image))?;
        let compressed = resolved.compression != Compression::Uncompressed;
        let image = fit(image, settings.max_size.pixels(), compressed);
        let (width, height) = image.dimensions();
        let texels = match (role, settings.color_space_for(role)) {
            (TextureRole::Normal, _) => Texels::Normals,
            (_, ColorSpace::Srgb) => Texels::Srgb,
            (_, ColorSpace::Linear) => Texels::Linear,
        };
        let base = image.into_raw();
        let mips = if settings.mips {
            build_mips(width, height, texels, &base)
        } else {
            Vec::new()
        };
        let levels = std::iter::once(base)
            .chain(mips)
            .enumerate()
            .map(|(level, texels)| {
                let extent = level_extent(width, height, level as u32);
                encode_level(resolved, extent, texels)
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        Ok(TextureLevels {
            format: resolved.format,
            width,
            height,
            levels,
        })
    }

    pub(crate) fn decode(&self) -> anyhow::Result<RgbaImage> {
        let format = match self.encoding {
            SourceEncoding::Png => ImageFormat::Png,
            SourceEncoding::Jpeg => ImageFormat::Jpeg,
        };
        Ok(image::load_from_memory_with_format(&self.bytes, format)
            .with_context(|| format!("failed to decode the texture's {:?} source", self.encoding))?
            .to_rgba8())
    }
}

pub(crate) fn has_alpha(image: &RgbaImage) -> bool {
    image.pixels().any(|pixel| pixel.0[3] < 255)
}

fn fit(image: RgbaImage, max_size: Option<u32>, blocks: bool) -> RgbaImage {
    let (width, height) = image.dimensions();
    let longest = width.max(height);
    let scale = max_size.map_or(1.0, |cap| (cap as f64 / longest as f64).min(1.0));
    let scaled = |side: u32| ((side as f64 * scale).round() as u32).max(1);
    let mut target_width = scaled(width);
    let mut target_height = scaled(height);
    if blocks {
        let to_blocks = |side: u32| ((side + 2) / 4 * 4).max(4);
        target_width = to_blocks(target_width);
        target_height = to_blocks(target_height);
    }
    if target_width == width && target_height == height {
        return image;
    }
    image::imageops::resize(&image, target_width, target_height, FilterType::Lanczos3)
}

fn encode_level(
    resolved: ResolvedTexture,
    extent: wgpu_types::Extent3d,
    texels: Vec<u8>,
) -> anyhow::Result<Vec<u8>> {
    if resolved.compression == Compression::Uncompressed {
        return Ok(texels);
    }
    let padded = block_extent(resolved.format, extent);
    let texels = pad(&texels, extent, padded);
    encode_blocks(resolved.compression, padded, &texels)
}

fn pad(texels: &[u8], extent: wgpu_types::Extent3d, padded: wgpu_types::Extent3d) -> Vec<u8> {
    if extent == padded {
        return texels.to_vec();
    }
    let mut out = Vec::with_capacity(padded.width as usize * padded.height as usize * 4);
    for y in 0..padded.height {
        let row = y.min(extent.height - 1);
        for x in 0..padded.width {
            let column = x.min(extent.width - 1);
            let at = ((row * extent.width + column) * 4) as usize;
            out.extend_from_slice(&texels[at..at + 4]);
        }
    }
    out
}

#[cfg(not(target_arch = "wasm32"))]
fn encode_blocks(
    compression: Compression,
    extent: wgpu_types::Extent3d,
    texels: &[u8],
) -> anyhow::Result<Vec<u8>> {
    use intel_tex_2::{bc1, bc4, bc5, bc7, RSurface, RgSurface, RgbaSurface};

    let width = extent.width;
    let height = extent.height;
    let rgba = RgbaSurface {
        data: texels,
        width,
        height,
        stride: width * 4,
    };
    let channels = |count: usize| -> Vec<u8> {
        texels
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|texel| texel[..count].iter().copied())
            .collect()
    };
    Ok(match compression {
        Compression::Bc1 => bc1::compress_blocks(&rgba),
        Compression::Bc7 if texels.as_chunks::<4>().0.iter().any(|texel| texel[3] < 255) => {
            bc7::compress_blocks(&bc7::alpha_fast_settings(), &rgba)
        }
        Compression::Bc7 => bc7::compress_blocks(&bc7::opaque_fast_settings(), &rgba),
        Compression::Bc4 => {
            let red = channels(1);
            bc4::compress_blocks(&RSurface {
                data: &red,
                width,
                height,
                stride: width,
            })
        }
        Compression::Bc5 => {
            let red_green = channels(2);
            bc5::compress_blocks(&RgSurface {
                data: &red_green,
                width,
                height,
                stride: width * 2,
            })
        }
        Compression::Auto | Compression::Uncompressed => {
            anyhow::bail!("{compression:?} is not a block compression")
        }
    })
}

#[cfg(target_arch = "wasm32")]
fn encode_blocks(
    compression: Compression,
    _extent: wgpu_types::Extent3d,
    _texels: &[u8],
) -> anyhow::Result<Vec<u8>> {
    anyhow::bail!("this build has no {compression:?} encoder; derive the texture Uncompressed")
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Texels {
    Linear,
    Srgb,
    Normals,
}

impl Texels {
    pub(crate) fn of(format: TextureFormat) -> Self {
        if format.is_srgb() {
            Texels::Srgb
        } else {
            Texels::Linear
        }
    }
}

#[derive(Clone, Copy)]
struct MipExtent {
    width: u32,
    height: u32,
}

pub(crate) fn build_mips(width: u32, height: u32, texels: Texels, data: &[u8]) -> Vec<Vec<u8>> {
    if width == 0 || height == 0 || data.len() != width as usize * height as usize * 4 {
        return Vec::new();
    }
    let srgb = texels == Texels::Srgb;
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

    let coverage = (texels == Texels::Srgb).then(|| alpha_coverage(data));
    let mut mips: Vec<Vec<u8>> = Vec::new();
    let mut unscaled: Option<Vec<u8>> = None;
    let mut extent = MipExtent { width, height };
    while extent.width > 1 || extent.height > 1 {
        let next = MipExtent {
            width: (extent.width / 2).max(1),
            height: (extent.height / 2).max(1),
        };
        let source = unscaled.as_deref().unwrap_or(data);
        let mut mip = downsample(source, extent, next, &decode, srgb);
        if texels == Texels::Normals {
            renormalize(&mut mip);
        }
        match coverage {
            Some(coverage) => {
                let mut scaled = mip.clone();
                keep_alpha_coverage(&mut scaled, coverage);
                mips.push(scaled);
            }
            None => mips.push(mip.clone()),
        }
        unscaled = Some(mip);
        extent = next;
    }
    mips
}

const ALPHA_COVERED: u8 = 128;
const ALPHA_SCALE_LIMIT: f32 = 4.0;

fn alpha_coverage(texels: &[u8]) -> f32 {
    let texels = texels.as_chunks::<4>().0;
    let covered = texels
        .iter()
        .filter(|texel| texel[3] >= ALPHA_COVERED)
        .count();
    covered as f32 / texels.len().max(1) as f32
}

fn keep_alpha_coverage(texels: &mut [u8], coverage: f32) {
    let mut alphas: Vec<u8> = texels
        .as_chunks::<4>()
        .0
        .iter()
        .map(|texel| texel[3])
        .collect();
    let count = alphas.len();
    let wanted = (coverage * count as f32).round() as usize;
    let covered = alphas
        .iter()
        .filter(|&&alpha| alpha >= ALPHA_COVERED)
        .count();
    if wanted == covered || wanted == 0 || wanted == count {
        return;
    }
    alphas.sort_unstable_by(|a, b| b.cmp(a));
    let edge = (alphas[wanted - 1] as f32 + alphas[wanted] as f32) * 0.5;
    if edge <= 0.0 {
        return;
    }
    let scale = (127.5 / edge).clamp(1.0 / ALPHA_SCALE_LIMIT, ALPHA_SCALE_LIMIT);
    for texel in texels.as_chunks_mut::<4>().0 {
        texel[3] = (texel[3] as f32 * scale).round().clamp(0.0, 255.0) as u8;
    }
}

fn renormalize(texels: &mut [u8]) {
    for texel in texels.as_chunks_mut::<4>().0 {
        let unpack = |byte: u8| byte as f32 / 255.0 * 2.0 - 1.0;
        let normal = [unpack(texel[0]), unpack(texel[1]), unpack(texel[2])];
        let length = normal.iter().map(|axis| axis * axis).sum::<f32>().sqrt();
        if length <= f32::EPSILON {
            continue;
        }
        for (byte, axis) in texel.iter_mut().zip(normal) {
            *byte = (((axis / length) * 0.5 + 0.5) * 255.0)
                .round()
                .clamp(0.0, 255.0) as u8;
        }
    }
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
        let rows = footprint(y, extent.height, next.height);
        for x in 0..next.width {
            let columns = footprint(x, extent.width, next.width);
            let count = (rows.len() * columns.len()) as f32;
            let mut sums = [0.0f32; 4];
            let mut weighted = [0.0f32; 3];
            for row in rows.clone() {
                for column in columns.clone() {
                    let texel = ((row * extent.width + column) * 4) as usize;
                    let alpha = source[texel + 3] as f32 / 255.0;
                    for channel in 0..3 {
                        let value = decode[source[texel + channel] as usize];
                        sums[channel] += value;
                        weighted[channel] += value * alpha;
                    }
                    sums[3] += alpha;
                }
            }
            for channel in 0..4 {
                let mean = if channel < 3 && srgb && sums[3] > 0.0 {
                    weighted[channel] / sums[3]
                } else {
                    sums[channel] / count
                };
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

fn footprint(index: u32, size: u32, next_size: u32) -> std::ops::Range<u32> {
    let start = index * size / next_size;
    let end = ((index + 1) * size / next_size).max(start + 1);
    start..end
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
    use super::{build_mips, Texels};

    struct Case {
        width: u32,
        height: u32,
        texels: Texels,
        data: Vec<u8>,
        mips: Vec<Vec<u8>>,
    }

    #[test]
    fn mips_halve_down_to_one_texel_average_in_linear_space_and_keep_a_colour_textures_coverage() {
        let cases = [
            Case {
                width: 4,
                height: 2,
                texels: Texels::Linear,
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
                texels: Texels::Linear,
                data: [[0, 0, 0, 0], [200, 200, 200, 200], [255, 255, 255, 255]].concat(),
                mips: vec![vec![152, 152, 152, 152]],
            },
            Case {
                width: 2,
                height: 1,
                texels: Texels::Srgb,
                data: [[0, 0, 0, 0], [255, 255, 255, 255]].concat(),
                mips: vec![vec![255, 255, 255, 128]],
            },
            Case {
                width: 4,
                height: 4,
                texels: Texels::Srgb,
                data: [
                    [[9, 9, 9, 255], [9, 9, 9, 100], [9, 9, 9, 255], [0, 0, 0, 0]],
                    [[0, 0, 0, 0], [0, 0, 0, 0], [0, 0, 0, 0], [0, 0, 0, 0]],
                    [[0, 0, 0, 0], [9, 9, 9, 255], [0, 0, 0, 0], [0, 0, 0, 0]],
                    [[0, 0, 0, 0], [0, 0, 0, 0], [0, 0, 0, 0], [0, 0, 0, 0]],
                ]
                .concat()
                .concat(),
                mips: vec![
                    [[9, 9, 9, 148], [9, 9, 9, 107], [9, 9, 9, 107], [0, 0, 0, 0]].concat(),
                    vec![9, 9, 9, 54],
                ],
            },
            Case {
                width: 2,
                height: 1,
                texels: Texels::Normals,
                data: [[255, 128, 128, 255], [128, 255, 128, 255]].concat(),
                mips: vec![vec![218, 218, 128, 255]],
            },
            Case {
                width: 1,
                height: 1,
                texels: Texels::Srgb,
                data: vec![1, 2, 3, 4],
                mips: vec![],
            },
            Case {
                width: 2,
                height: 2,
                texels: Texels::Srgb,
                data: vec![],
                mips: vec![],
            },
            Case {
                width: 0,
                height: 4,
                texels: Texels::Linear,
                data: vec![],
                mips: vec![],
            },
        ];
        for case in cases {
            assert_eq!(
                build_mips(case.width, case.height, case.texels, &case.data),
                case.mips,
                "{}x{} {:?}",
                case.width,
                case.height,
                case.texels,
            );
        }
    }
}

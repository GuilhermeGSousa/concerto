//! Textures painted in code at startup. Each covers a whole number of
//! real-world repeats so it tiles seamlessly; UVs elsewhere are in meters.
use concerto::render::assets::texture::{Texture, TextureFormat, TextureKind};

/// Tileable value noise in `[0, 1]` with `period` lattice cells per side.
fn value_noise(x: f32, y: f32, period: u32, seed: u32) -> f32 {
    let hash = |ix: i32, iy: i32| -> f32 {
        let ix = ix.rem_euclid(period as i32) as u32;
        let iy = iy.rem_euclid(period as i32) as u32;
        let mut h = ix.wrapping_mul(374_761_393)
            ^ iy.wrapping_mul(668_265_263)
            ^ seed.wrapping_mul(2_654_435_761);
        h = (h ^ (h >> 13)).wrapping_mul(1_274_126_177);
        (h ^ (h >> 16)) as f32 / u32::MAX as f32
    };
    let (x0, y0) = (x.floor() as i32, y.floor() as i32);
    let (fx, fy) = (x - x0 as f32, y - y0 as f32);
    let (sx, sy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
    let a = hash(x0, y0) + (hash(x0 + 1, y0) - hash(x0, y0)) * sx;
    let b = hash(x0, y0 + 1) + (hash(x0 + 1, y0 + 1) - hash(x0, y0 + 1)) * sx;
    a + (b - a) * sy
}

/// Fractal noise, tileable over the unit square.
fn fbm(u: f32, v: f32, base_period: u32, octaves: u32, seed: u32) -> f32 {
    let mut sum = 0.0;
    let mut amp = 0.5;
    let mut norm = 0.0;
    for o in 0..octaves {
        let period = base_period << o;
        sum += amp * value_noise(u * period as f32, v * period as f32, period, seed + o);
        norm += amp;
        amp *= 0.5;
    }
    sum / norm
}

fn paint(size: u32, f: impl Fn(f32, f32) -> [f32; 3]) -> Texture {
    let mut data = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let [r, g, b] = f(
                (x as f32 + 0.5) / size as f32,
                (y as f32 + 0.5) / size as f32,
            );
            for c in [r, g, b] {
                data.push((c.clamp(0.0, 1.0) * 255.0).round() as u8);
            }
            data.push(255);
        }
    }
    Texture {
        width: size,
        height: size,
        format: TextureFormat::Rgba8UnormSrgb,
        kind: TextureKind::Sampled,
        data,
    }
}

fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

fn scale(c: [f32; 3], s: f32) -> [f32; 3] {
    [c[0] * s, c[1] * s, c[2] * s]
}

/// Distance to the nearest grid line, in tile units, for `tiles` per side.
fn grid_distance(u: f32, v: f32, tiles: f32) -> f32 {
    let fu = (u * tiles).fract();
    let fv = (v * tiles).fract();
    fu.min(1.0 - fu).min(fv).min(1.0 - fv)
}

/// Worn vinyl floor tiles: 4×4 tiles per texture (one tile = 0.5 m when the
/// texture spans 2 m), with grout, per-tile tone and scuffs.
pub fn floor() -> Texture {
    paint(256, |u, v| {
        let tiles = 4.0;
        let tile = ((u * tiles).floor() + (v * tiles).floor() * 7.0) as u32;
        let tone = 0.9 + 0.1 * value_noise(tile as f32, 0.0, 64, 3);
        let checker = if ((u * tiles).floor() as i32 + (v * tiles).floor() as i32) % 2 == 0 {
            [0.62, 0.6, 0.55]
        } else {
            [0.5, 0.49, 0.46]
        };
        let grime = fbm(u, v, 4, 5, 11);
        let scuff = (fbm(u, v, 8, 3, 29) - 0.55).max(0.0) * 1.6;
        let mut color = scale(checker, tone * (0.8 + 0.35 * grime) - scuff * 0.25);
        let grout = grid_distance(u, v, tiles);
        if grout < 0.018 {
            color = scale(color, 0.55);
        }
        color
    })
}

/// Suspended ceiling panels with a dark grid and pinholes.
pub fn ceiling() -> Texture {
    paint(128, |u, v| {
        let base = [0.72, 0.71, 0.68];
        let stain = (fbm(u, v, 2, 4, 5) - 0.6).max(0.0) * 2.5;
        let pin = value_noise(u * 64.0, v * 64.0, 64, 17);
        let mut color = mix(base, [0.45, 0.38, 0.28], stain);
        if pin > 0.93 {
            color = scale(color, 0.7);
        }
        if grid_distance(u, v, 2.0) < 0.025 {
            color = [0.3, 0.3, 0.3];
        }
        color
    })
}

/// Drywall with grime creeping up from the floor; `v` runs down the wall
/// over one texture height (mapped to the wall's full height).
pub fn drywall() -> Texture {
    paint(128, |u, v| {
        let base = [0.58, 0.56, 0.5];
        let grime = fbm(u, v, 4, 5, 41);
        let low = (v - 0.7).max(0.0) * 2.5;
        let mut color = scale(base, 0.8 + 0.3 * grime);
        color = mix(
            color,
            [0.25, 0.22, 0.18],
            (low * (0.6 + 0.6 * grime)).min(0.8),
        );
        // Baseboard.
        if v > 0.94 {
            color = [0.2, 0.19, 0.18];
        }
        color
    })
}

/// Corrugated cardboard for the stock on the shelves.
pub fn cardboard() -> Texture {
    paint(64, |u, v| {
        let base = [0.55, 0.4, 0.24];
        let fiber = fbm(u, v, 4, 3, 71);
        let tape = (v - 0.5).abs() < 0.06;
        let color = scale(base, 0.8 + 0.3 * fiber);
        if tape {
            mix(color, [0.75, 0.62, 0.4], 0.6)
        } else {
            color
        }
    })
}

/// Painted steel for shelving, subtly mottled.
pub fn steel() -> Texture {
    paint(64, |u, v| {
        let n = fbm(u, v, 4, 4, 91);
        scale([0.42, 0.44, 0.47], 0.85 + 0.25 * n)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noise_tiles_seamlessly() {
        for i in 0..16 {
            let y = i as f32 / 16.0;
            let a = fbm(0.0, y, 4, 4, 1);
            let b = fbm(1.0, y, 4, 4, 1);
            assert!((a - b).abs() < 1e-4);
        }
    }

    #[test]
    fn textures_have_full_rgba_payloads() {
        for texture in [floor(), ceiling(), drywall(), cardboard(), steel()] {
            assert_eq!(
                texture.data.len(),
                (texture.width * texture.height * 4) as usize
            );
        }
    }
}

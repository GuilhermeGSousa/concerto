//! Textures painted in code at startup.
use concerto::render::assets::texture::{Texture, TextureFormat, TextureKind};

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

fn smooth(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Dark oak floorboards running along `u`, eight boards per texture.
pub fn floorboards() -> Texture {
    paint(256, |u, v| {
        let boards = 8.0;
        let row = (v * boards).floor();
        let shift = value_noise(row, 3.0, 64, 5) * 3.0;
        let along = u * 1.0 + shift / 3.0;
        let plank = (along * 2.0).floor();
        let tone = 0.75 + 0.35 * value_noise(row * 7.0 + plank, 1.0, 64, 9);
        let grain = fbm(along * 0.5, v * 4.0, 16, 3, 13);
        let streak = ((v * boards).fract() * 9.0 + grain * 6.0).sin() * 0.5 + 0.5;
        let base = mix([0.2, 0.12, 0.07], [0.3, 0.19, 0.11], streak * 0.6 + grain * 0.4);
        let mut color = scale(base, tone);
        let seam = (v * boards).fract();
        if !(0.04..=0.96).contains(&seam) {
            color = scale(color, 0.35);
        }
        if (along * 2.0).fract() < 0.012 {
            color = scale(color, 0.45);
        }
        let wear = (fbm(u, v, 4, 4, 77) - 0.5).max(0.0) * 0.6;
        mix(color, [0.34, 0.26, 0.18], wear)
    })
}

/// A damask wallpaper: staggered medallions on a ground, aged and stained.
/// One texture spans half a meter.
pub fn wallpaper(ground: [f32; 3], figure: [f32; 3], seed: u32) -> Texture {
    paint(128, |u, v| {
        let (gx, gy) = (u * 2.0, v * 2.0);
        let cell = |x: f32, y: f32| {
            let dx = x - x.round();
            let dy = y - y.round();
            (dx, dy)
        };
        let (ax, ay) = cell(gx, gy);
        let (bx, by) = cell(gx + 0.5, gy + 0.5);
        let shape = |dx: f32, dy: f32| {
            let r = (dx * dx * 1.6 + dy * dy).sqrt();
            let a = dy.atan2(dx);
            let petal = 0.17 + 0.06 * (a * 4.0).cos() + 0.03 * (a * 8.0).cos();
            let ring = (r - 0.26).abs() < 0.018;
            r < petal || ring
        };
        let motif = shape(ax, ay) || shape(bx, by);
        let mut color = if motif { figure } else { ground };
        let n = fbm(u, v, 4, 4, seed);
        color = scale(color, 0.85 + 0.3 * n);
        let stain = (fbm(u, v, 2, 4, seed + 7) - 0.62).max(0.0) * 2.2;
        mix(color, [0.12, 0.09, 0.05], stain)
    })
}

/// Raised panelling in dark wood: two panels across, one up.
pub fn wainscot() -> Texture {
    paint(128, |u, v| {
        let grain = fbm(u * 0.5, v * 3.0, 8, 3, 21);
        let base = mix([0.14, 0.08, 0.05], [0.24, 0.14, 0.08], grain);
        let pu = (u * 2.0).fract();
        let edge = pu.min(1.0 - pu).min(v.min(1.0 - v) * 0.5);
        let bevel = smooth(0.06, 0.1, edge);
        let groove = if (0.055..0.075).contains(&edge) { 0.5 } else { 1.0 };
        scale(base, (0.75 + 0.35 * bevel) * groove)
    })
}

/// Yellowed plaster with water stains and hairline cracks.
pub fn plaster() -> Texture {
    paint(128, |u, v| {
        let base = [0.52, 0.49, 0.43];
        let stain = (fbm(u, v, 2, 4, 5) - 0.55).max(0.0) * 2.5;
        let crack = (fbm(u, v, 4, 4, 17) - 0.5).abs() < 0.004;
        let color = scale(mix(base, [0.36, 0.3, 0.2], stain), 0.9 + 0.2 * fbm(u, v, 8, 2, 3));
        if crack { scale(color, 0.85) } else { color }
    })
}

/// Linen dust sheets: soft folds and settled dust.
pub fn linen() -> Texture {
    paint(128, |u, v| {
        let fold = fbm(u * 0.5, v * 2.0, 4, 3, 31);
        let weave = ((u * 128.0).sin() * (v * 128.0).sin()) * 0.03;
        let shade = 0.7 + 0.35 * smooth(0.3, 0.7, fold) + weave;
        let dust = fbm(u, v, 8, 3, 33) * 0.1;
        scale([0.74, 0.71, 0.64], shade - dust)
    })
}

/// Polished dark wood for furniture, frames and easels.
pub fn wood() -> Texture {
    paint(64, |u, v| {
        let grain = fbm(u * 0.5, v * 4.0, 8, 3, 43);
        let rings = ((u * 12.0 + grain * 5.0).sin() * 0.5 + 0.5) * 0.3;
        scale(mix([0.16, 0.09, 0.05], [0.28, 0.16, 0.09], grain), 0.85 + rings)
    })
}

/// Tarnished gilt for picture frames.
pub fn gilt() -> Texture {
    paint(64, |u, v| {
        let n = fbm(u, v, 8, 3, 51);
        let bead = ((u * 40.0).sin() * (v * 40.0).sin()).abs();
        let color = mix([0.33, 0.24, 0.08], [0.72, 0.56, 0.24], n * 0.7 + bead * 0.3);
        if n < 0.35 {
            scale(color, 0.5)
        } else {
            color
        }
    })
}

/// Worn leather, tinted per material for book spines.
pub fn leather() -> Texture {
    paint(64, |u, v| {
        let n = fbm(u, v, 8, 3, 61);
        let band = (v * 6.0).fract() < 0.06;
        let tone = 0.7 + 0.4 * n;
        if band {
            [0.55, 0.45, 0.2]
        } else {
            [tone, tone * 0.95, tone * 0.9]
        }
    })
}

/// Paper: lot tags, diary pages, the ledger.
pub fn paper() -> Texture {
    paint(64, |u, v| {
        let n = fbm(u, v, 4, 3, 71);
        let line = (v * 10.0).fract() < 0.06 && u > 0.15 && u < 0.85;
        let color = scale([0.82, 0.76, 0.6], 0.85 + 0.2 * n);
        if line {
            scale(color, 0.6)
        } else {
            color
        }
    })
}

/// A Turkey carpet: border bands and a central medallion.
pub fn rug(seed: u32) -> Texture {
    paint(128, |u, v| {
        let (x, y) = (u - 0.5, v - 0.5);
        let edge = (0.5 - x.abs()).min(0.5 - y.abs());
        let red = [0.34, 0.06, 0.05];
        let blue = [0.07, 0.08, 0.2];
        let gold = [0.5, 0.36, 0.14];
        let mut color = if edge < 0.025 {
            blue
        } else if edge < 0.07 {
            let t = (u.min(v) + u.max(v)) * 60.0;
            if t.sin() * (edge * 200.0).sin() > 0.4 { gold } else { red }
        } else if edge < 0.08 {
            gold
        } else {
            let r = (x * x + y * y * 2.0).sqrt();
            let lozenge = x.abs() + y.abs() * 1.4;
            if lozenge < 0.16 {
                gold
            } else if lozenge < 0.22 || (r * 30.0).sin() > 0.93 {
                blue
            } else {
                red
            }
        };
        let wear = fbm(u, v, 4, 4, seed);
        color = scale(color, 0.7 + 0.4 * wear);
        mix(color, [0.25, 0.2, 0.16], (wear - 0.62).max(0.0) * 2.0)
    })
}

/// Moonlit window glass: small panes and dark glazing bars.
pub fn window_glass() -> Texture {
    paint(64, |u, v| {
        let bar = (u * 3.0).fract() < 0.07 || (v * 4.0).fract() < 0.06;
        let sky = mix([0.55, 0.62, 0.8], [0.18, 0.22, 0.34], v);
        let grime = fbm(u, v, 4, 3, 141);
        if bar {
            [0.03, 0.03, 0.03]
        } else {
            scale(sky, 0.6 + 0.5 * grime)
        }
    })
}

/// The pictures in the house.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Picture {
    /// A lay figure's bust against a dark ground: smooth, faceless.
    Sitter,
    /// Figures standing about a dim room.
    Group,
    Landscape,
    Seascape,
    StillLife,
    /// Clara, in grey silk. The only face he ever painted.
    Clara,
}

pub const PICTURES: [Picture; 6] = [
    Picture::Sitter,
    Picture::Group,
    Picture::Landscape,
    Picture::Seascape,
    Picture::StillLife,
    Picture::Clara,
];

fn varnish(color: [f32; 3], u: f32, v: f32, seed: u32) -> [f32; 3] {
    let yellow = mix(color, [color[0] * 1.05, color[1] * 0.92, color[2] * 0.6], 0.5);
    let crack = (fbm(u, v, 8, 3, seed) - 0.5).abs() < 0.012;
    let vignette = 1.0 - ((u - 0.5).powi(2) + (v - 0.5).powi(2)) * 1.4;
    let c = scale(yellow, vignette.max(0.3) * (0.92 + 0.12 * fbm(u, v, 16, 2, seed + 1)));
    if crack { scale(c, 0.55) } else { c }
}

fn figure_bust(u: f32, v: f32, cx: f32, cy: f32, s: f32) -> f32 {
    let (x, y) = ((u - cx) / s, (v - cy) / s);
    let head = (x * x / 0.012 + (y + 0.22).powi(2) / 0.02) < 1.0;
    let neck = x.abs() < 0.04 && (-0.1..0.08).contains(&y);
    let width = 0.1 + 0.32 * ((y - 0.06) / 0.1).clamp(0.0, 1.0);
    let shoulders = (0.06..0.6).contains(&y) && x.abs() < width;
    if head || neck || shoulders { 1.0 } else { 0.0 }
}

pub fn picture(kind: Picture) -> Texture {
    match kind {
        Picture::Sitter => paint(128, |u, v| {
            let ground = mix([0.05, 0.04, 0.03], [0.16, 0.12, 0.08], fbm(u, v, 2, 4, 101));
            let shape = figure_bust(u, v, 0.5, 0.45, 0.8);
            let light = smooth(0.8, 0.2, u) * 0.6 + 0.4;
            let body = scale([0.62, 0.52, 0.4], light);
            let joint = ((v - 0.43).abs() < 0.012 && shape > 0.0) as u8 as f32;
            let c = mix(ground, scale(body, 1.0 - joint * 0.5), shape);
            varnish(c, u, v, 103)
        }),
        Picture::Group => paint(128, |u, v| {
            let wall = mix([0.07, 0.06, 0.05], [0.2, 0.15, 0.1], smooth(1.0, 0.0, v));
            let floor = v > 0.72;
            let mut c = if floor { [0.09, 0.06, 0.04] } else { wall };
            for (i, x) in [0.2, 0.38, 0.55, 0.8].into_iter().enumerate() {
                let s = 0.2 + 0.03 * i as f32;
                let top = 0.72 - s * 2.4;
                let dx = (u - x).abs();
                let body = dx < s * 0.18 && v > top + s * 0.35 && v < 0.74;
                let head = (dx * dx + (v - top - s * 0.18).powi(2)).sqrt() < s * 0.16;
                if body || head {
                    c = scale([0.55, 0.47, 0.36], 0.5 + 0.5 * smooth(0.0, 1.0, 1.0 - dx * 8.0));
                }
            }
            varnish(c, u, v, 107)
        }),
        Picture::Landscape => paint(128, |u, v| {
            let sky = mix([0.42, 0.33, 0.2], [0.12, 0.13, 0.15], smooth(0.55, 0.0, v));
            let hill = 0.55 + 0.08 * (u * 5.0).sin() + 0.04 * fbm(u, 0.0, 4, 3, 109);
            let c = if v > hill {
                mix([0.1, 0.1, 0.05], [0.05, 0.05, 0.03], fbm(u, v, 8, 3, 111))
            } else if v > hill - 0.2 * fbm(u * 3.0, v, 4, 3, 113) && (u * 9.0).fract() < 0.3 {
                [0.05, 0.06, 0.04]
            } else {
                scale(sky, 0.9 + 0.2 * fbm(u, v, 4, 3, 115))
            };
            varnish(c, u, v, 117)
        }),
        Picture::Seascape => paint(128, |u, v| {
            let sky = mix([0.3, 0.3, 0.28], [0.1, 0.11, 0.13], smooth(0.5, 0.0, v));
            let sea = mix([0.05, 0.08, 0.09], [0.15, 0.18, 0.17], fbm(u * 2.0, v * 8.0, 4, 3, 119));
            let mut c = if v > 0.5 { sea } else { sky };
            let ship = (u - 0.62).abs() < 0.01 && (0.35..0.5).contains(&v);
            let hull = (u - 0.62).abs() < 0.05 && (0.48..0.52).contains(&v);
            if ship || hull {
                c = [0.04, 0.03, 0.03];
            }
            varnish(c, u, v, 121)
        }),
        Picture::StillLife => paint(128, |u, v| {
            let ground = mix([0.04, 0.03, 0.02], [0.12, 0.08, 0.05], fbm(u, v, 2, 3, 123));
            let table = v > 0.68;
            let mut c = if table { [0.18, 0.1, 0.05] } else { ground };
            let vase = ((u - 0.4) / 0.12).powi(2) + ((v - 0.55) / 0.15).powi(2) < 1.0
                || ((u - 0.4).abs() < 0.04 && (0.32..0.45).contains(&v));
            let skull = ((u - 0.68) / 0.09).powi(2) + ((v - 0.62) / 0.08).powi(2) < 1.0;
            let socket = ((u - 0.65) / 0.02).powi(2) + ((v - 0.61) / 0.02).powi(2) < 1.0
                || ((u - 0.71) / 0.02).powi(2) + ((v - 0.61) / 0.02).powi(2) < 1.0;
            if vase {
                c = [0.25, 0.2, 0.12];
            }
            if skull {
                c = if socket { [0.02, 0.02, 0.02] } else { [0.62, 0.56, 0.44] };
            }
            varnish(c, u, v, 127)
        }),
        Picture::Clara => paint(128, |u, v| {
            let ground = mix([0.06, 0.05, 0.05], [0.18, 0.15, 0.14], fbm(u, v, 2, 4, 131));
            let shape = figure_bust(u, v, 0.5, 0.42, 0.75);
            let (x, y) = (u - 0.5, v - 0.28);
            let face = (x * x / 0.012 + y * y / 0.02) < 1.0;
            let hair = (x * x / 0.02 + (y + 0.03).powi(2) / 0.022) < 1.0 && !face;
            let eye = ((x.abs() - 0.035).powi(2) + (y + 0.01).powi(2)) < 0.00012;
            let mut c = mix(ground, [0.36, 0.38, 0.4], shape);
            if hair {
                c = [0.1, 0.07, 0.05];
            }
            if face {
                c = [0.72, 0.6, 0.52];
            }
            if eye {
                c = [0.05, 0.04, 0.04];
            }
            varnish(c, u, v, 133)
        }),
    }
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
        for texture in [floorboards(), plaster(), wainscot(), linen(), wood(), gilt(), paper(), rug(1), window_glass()]
            .into_iter()
            .chain(PICTURES.map(picture))
        {
            assert_eq!(
                texture.data.len(),
                (texture.width * texture.height * 4) as usize
            );
        }
    }
}

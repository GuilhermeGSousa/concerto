struct BloomUniform {
    threshold: f32,
    knee: f32,
    _padding: vec2<f32>,
};

@group(0) @binding(0)
var t_source: texture_2d<f32>;

@group(0) @binding(1)
var s_source: sampler;

@group(0) @binding(2)
var<uniform> bloom: BloomUniform;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    let uv = vec2<f32>(f32((vertex_index << 1u) & 2u), f32(vertex_index & 2u));
    var out: VertexOutput;
    out.position = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0);
    out.uv = uv;
    return out;
}

fn tap(uv: vec2<f32>, offset: vec2<f32>, texel: vec2<f32>) -> vec3<f32> {
    return max(textureSampleLevel(t_source, s_source, uv + offset * texel, 0.0).rgb, vec3<f32>(0.0));
}

struct Boxes {
    center: vec3<f32>,
    top_left: vec3<f32>,
    top_right: vec3<f32>,
    bottom_left: vec3<f32>,
    bottom_right: vec3<f32>,
};

fn boxes(uv: vec2<f32>) -> Boxes {
    let texel = 1.0 / vec2<f32>(textureDimensions(t_source));
    let a = tap(uv, vec2<f32>(-2.0, -2.0), texel);
    let b = tap(uv, vec2<f32>(0.0, -2.0), texel);
    let c = tap(uv, vec2<f32>(2.0, -2.0), texel);
    let d = tap(uv, vec2<f32>(-1.0, -1.0), texel);
    let e = tap(uv, vec2<f32>(1.0, -1.0), texel);
    let f = tap(uv, vec2<f32>(-2.0, 0.0), texel);
    let g = tap(uv, vec2<f32>(0.0, 0.0), texel);
    let h = tap(uv, vec2<f32>(2.0, 0.0), texel);
    let i = tap(uv, vec2<f32>(-1.0, 1.0), texel);
    let j = tap(uv, vec2<f32>(1.0, 1.0), texel);
    let k = tap(uv, vec2<f32>(-2.0, 2.0), texel);
    let l = tap(uv, vec2<f32>(0.0, 2.0), texel);
    let m = tap(uv, vec2<f32>(2.0, 2.0), texel);

    var out: Boxes;
    out.center = (d + e + i + j) * 0.25;
    out.top_left = (a + b + f + g) * 0.25;
    out.top_right = (b + c + g + h) * 0.25;
    out.bottom_left = (f + g + k + l) * 0.25;
    out.bottom_right = (g + h + l + m) * 0.25;
    return out;
}

fn luminance(color: vec3<f32>) -> f32 {
    return dot(color, vec3<f32>(0.2126, 0.7152, 0.0722));
}

fn above_threshold(color: vec3<f32>) -> vec3<f32> {
    if bloom.threshold <= 0.0 {
        return color;
    }
    let brightness = max(color.r, max(color.g, color.b));
    let soft = clamp(brightness - bloom.threshold + bloom.knee, 0.0, 2.0 * bloom.knee);
    let curve = soft * soft / (4.0 * bloom.knee + 1e-5);
    let kept = max(curve, brightness - bloom.threshold) / max(brightness, 1e-5);
    return color * kept;
}

@fragment
fn fs_downsample_first(in: VertexOutput) -> @location(0) vec4<f32> {
    let b = boxes(in.uv);
    let center = 0.5 / (1.0 + luminance(b.center));
    let top_left = 0.125 / (1.0 + luminance(b.top_left));
    let top_right = 0.125 / (1.0 + luminance(b.top_right));
    let bottom_left = 0.125 / (1.0 + luminance(b.bottom_left));
    let bottom_right = 0.125 / (1.0 + luminance(b.bottom_right));
    let total = center + top_left + top_right + bottom_left + bottom_right;
    let color = (b.center * center
        + b.top_left * top_left
        + b.top_right * top_right
        + b.bottom_left * bottom_left
        + b.bottom_right * bottom_right) / total;
    return vec4<f32>(above_threshold(color), 1.0);
}

@fragment
fn fs_downsample(in: VertexOutput) -> @location(0) vec4<f32> {
    let b = boxes(in.uv);
    let color = b.center * 0.5
        + (b.top_left + b.top_right + b.bottom_left + b.bottom_right) * 0.125;
    return vec4<f32>(color, 1.0);
}

@fragment
fn fs_upsample(in: VertexOutput) -> @location(0) vec4<f32> {
    let texel = 1.0 / vec2<f32>(textureDimensions(t_source));
    var color = tap(in.uv, vec2<f32>(0.0, 0.0), texel) * 4.0;
    color += tap(in.uv, vec2<f32>(-1.0, 0.0), texel) * 2.0;
    color += tap(in.uv, vec2<f32>(1.0, 0.0), texel) * 2.0;
    color += tap(in.uv, vec2<f32>(0.0, -1.0), texel) * 2.0;
    color += tap(in.uv, vec2<f32>(0.0, 1.0), texel) * 2.0;
    color += tap(in.uv, vec2<f32>(-1.0, -1.0), texel);
    color += tap(in.uv, vec2<f32>(1.0, -1.0), texel);
    color += tap(in.uv, vec2<f32>(-1.0, 1.0), texel);
    color += tap(in.uv, vec2<f32>(1.0, 1.0), texel);
    return vec4<f32>(color / 16.0, 1.0);
}

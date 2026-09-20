struct VertexInput {
    @location(0) position: vec2<f32>,
    /// Normalised coords: (0,0) = top-left, (1,1) = bottom-right.
    @location(1) uv: vec2<f32>,
};

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

/// Background fill colour.
@group(0) @binding(0) var<uniform> color: vec4<f32>;

/// Border outline colour.
@group(0) @binding(1) var<uniform> border_color: vec4<f32>;

/// Shape parameters: [border_width_px, node_width_px, node_height_px, corner_radius_px].
/// Filled by the engine each frame; users set border_width and corner_radius only.
@group(0) @binding(2) var<uniform> border_params: vec4<f32>;

/// Shape flags: [has_texture, rotation_radians, 0, 0].
@group(0) @binding(3) var<uniform> flags: vec4<f32>;

@group(0) @binding(4) var node_texture: texture_2d<f32>;
@group(0) @binding(5) var node_sampler: sampler;

@vertex
fn vs_main(model: VertexInput) -> VertexOutput {
    // Vertices are pre-transformed to NDC by extract_ui_nodes.
    var out: VertexOutput;
    out.position = vec4<f32>(model.position, 0.0, 1.0);
    out.uv = model.uv;
    return out;
}

/// Signed distance to a rounded rectangle centred on the origin. Negative
/// inside, positive outside, and the magnitude is in pixels — which is what
/// lets one smoothstep give a pixel-wide antialiased edge at any size.
fn rounded_box(point: vec2<f32>, half: vec2<f32>, radius: f32) -> f32 {
    let q = abs(point) - half + vec2<f32>(radius, radius);
    return length(max(q, vec2<f32>(0.0, 0.0))) + min(max(q.x, q.y), 0.0) - radius;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let border_w = border_params.x;
    let node_w = border_params.y;
    let node_h = border_params.z;

    var fill = color;
    if flags.x > 0.5 {
        fill = fill * textureSample(node_texture, node_sampler, in.uv);
    }

    // Before the layout pass has measured the node there is no shape to cut.
    if node_w <= 0.0 || node_h <= 0.0 {
        return fill;
    }

    let size = vec2<f32>(node_w, node_h);
    // `uv` spans the node's own box even when the quad is clipped, so pixel
    // coordinates stay relative to the whole node and the corners round where
    // the node's corners are rather than where it was cut.
    var point = in.uv * size - size * 0.5;
    // Rotating the sample point rotates the shape inside a node that has not
    // moved. The shape also has to shrink to stay inside that node: a box of
    // half-extent h spans h * (|cos| + |sin|) once turned.
    let angle = flags.y;
    let spread = abs(cos(angle)) + abs(sin(angle));
    let half = size * 0.5 / spread;
    if angle != 0.0 {
        let c = cos(angle);
        let s = sin(angle);
        point = vec2<f32>(point.x * c + point.y * s, -point.x * s + point.y * c);
    }
    let radius = clamp(border_params.w, 0.0, min(half.x, half.y));

    var out = fill;
    if border_w > 0.0 {
        let inner_half = max(half - vec2<f32>(border_w, border_w), vec2<f32>(0.0, 0.0));
        let inner = rounded_box(point, inner_half, max(radius - border_w, 0.0));
        out = mix(border_color, fill, 1.0 - smoothstep(-0.5, 0.5, inner));
    }

    let outer = rounded_box(point, half, radius);
    let coverage = 1.0 - smoothstep(-0.5, 0.5, outer);
    return vec4<f32>(out.rgb, out.a * coverage);
}

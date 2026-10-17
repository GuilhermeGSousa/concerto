struct CameraUniform {
    view_pos: vec3<f32>,
    view_proj: mat4x4<f32>,
};

@group(0) @binding(0)
var<uniform> camera: CameraUniform;

@group(1) @binding(0)
var<uniform> viewport: vec4<f32>;

struct VertexInput {
    @location(0) start: vec3<f32>,
    @location(1) end: vec3<f32>,
    @location(2) color: vec4<f32>,
    @location(3) corner: vec2<f32>,
    @location(4) width: f32,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
};

const NEAR_W: f32 = 1e-4;

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    var output: VertexOutput;
    output.color = input.color;

    var a = camera.view_proj * vec4<f32>(input.start, 1.0);
    var b = camera.view_proj * vec4<f32>(input.end, 1.0);
    if a.w < NEAR_W && b.w < NEAR_W {
        output.clip_position = vec4<f32>(0.0, 0.0, -1.0, 1.0);
        return output;
    }
    if a.w < NEAR_W {
        a = mix(a, b, (NEAR_W - a.w) / (b.w - a.w));
    }
    if b.w < NEAR_W {
        b = mix(b, a, (NEAR_W - b.w) / (a.w - b.w));
    }

    let half_size = viewport.xy * 0.5;
    let along = (b.xy / b.w - a.xy / a.w) * half_size;
    let length = length(along);
    var direction = vec2<f32>(1.0, 0.0);
    if length > 1e-6 {
        direction = along / length;
    }
    let normal = vec2<f32>(-direction.y, direction.x);

    var point = a;
    if input.corner.x > 0.5 {
        point = b;
    }
    let offset = normal * input.corner.y * input.width * 0.5 / half_size;
    output.clip_position = vec4<f32>(point.xy + offset * point.w, point.z, point.w);
    return output;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    return input.color;
}

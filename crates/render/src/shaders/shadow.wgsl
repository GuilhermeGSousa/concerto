const MAX_BONE_COUNT: i32 = 128;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) tex_coords: vec2<f32>,
    @location(2) normal: vec3<f32>,
    @location(3) tangent: vec3<f32>,
    @location(4) bitangent: vec3<f32>,
    @location(5) bone_indices: vec4<u32>,
    @location(6) bone_weights: vec4<f32>,
};

struct TransformInput {
    // Full transform
    @location(7) model_matrix_0: vec4<f32>,
    @location(8) model_matrix_1: vec4<f32>,
    @location(9) model_matrix_2: vec4<f32>,
    @location(10) model_matrix_3: vec4<f32>,
}

struct Skeleton {
    bones: array<mat4x4<f32>, MAX_BONE_COUNT>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
#ifdef CUTOUT
    @location(0) tex_coords: vec2<f32>,
#endif
}

@group(0) @binding(0)
var<uniform> view_proj: mat4x4<f32>;

@group(1) @binding(0)
var<uniform> bones: Skeleton;

@vertex
fn vs_main(
    model: VertexInput,
    instance: TransformInput,
) -> VertexOutput {

    let model_matrix = mat4x4<f32>(
        instance.model_matrix_0,
        instance.model_matrix_1,
        instance.model_matrix_2,
        instance.model_matrix_3,
    );

    let world_position = model_matrix * vec4<f32>(model.position, 1.0);

    var out: VertexOutput;
#ifdef CUTOUT
    out.tex_coords = model.tex_coords;
#endif

    let total_weight = model.bone_weights.x + model.bone_weights.y + model.bone_weights.z + model.bone_weights.w;
    if total_weight > 0 {
        var pose_transform = mat4x4<f32>();
        for (var i: i32 = 0; i < 4; i = i + 1) {
            pose_transform += bones.bones[model.bone_indices[i]] * model.bone_weights[i];
        }
        let skinned_pos = pose_transform * world_position;
        out.clip_position = view_proj * skinned_pos;
    } else {
        out.clip_position = view_proj * world_position;
    }

    return out;
}
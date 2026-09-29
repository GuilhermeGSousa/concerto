//! The engine's shaders, preprocessed with WebGL2's shader defs, must
//! translate to GLSL ES 3.00 — the only shading language WebGL2 accepts.
//! This is the translation wgpu's GL backend performs in the browser, so a
//! failure here is a shader that would fail to compile on wasm.

use concerto_render::{capabilities::RenderCapabilities, shader::compose_shader};
use naga::{
    back::glsl,
    proc::BoundsCheckPolicies,
    valid::{Capabilities, ValidationFlags, Validator},
    ShaderStage,
};

const WEBGL2: RenderCapabilities = RenderCapabilities {
    cube_array_textures: false,
    depth_bias_clamp: false,
    single_layer_texture_arrays: false,
};

fn assert_translates_to_glsl_es_300(
    label: &str,
    source: &str,
    entry_points: &[(&str, ShaderStage)],
) {
    let module = compose_shader(label, source, &WEBGL2.shader_defs()).unwrap();
    // WebGL2 can't do cube arrays, so don't let validation assume it can.
    let info = Validator::new(ValidationFlags::all(), Capabilities::empty())
        .validate(&module)
        .unwrap_or_else(|err| panic!("{label} failed validation: {err:?}"));

    for &(entry_point, shader_stage) in entry_points {
        let options = glsl::Options {
            version: glsl::Version::Embedded {
                version: 300,
                is_webgl: true,
            },
            ..Default::default()
        };
        let pipeline_options = glsl::PipelineOptions {
            shader_stage,
            entry_point: entry_point.to_string(),
            multiview: None,
        };

        let mut output = String::new();
        glsl::Writer::new(
            &mut output,
            &module,
            &info,
            &options,
            &pipeline_options,
            BoundsCheckPolicies::default(),
        )
        .and_then(|mut writer| writer.write())
        .unwrap_or_else(|err| {
            panic!("{label}::{entry_point} doesn't translate to GLSL ES 3.00: {err}")
        });
    }
}

#[test]
fn standard_material_shader_translates_for_webgl2() {
    assert_translates_to_glsl_es_300(
        "shader.wgsl",
        include_str!("../src/shaders/shader.wgsl"),
        &[
            ("vs_main", ShaderStage::Vertex),
            ("fs_main", ShaderStage::Fragment),
        ],
    );
}

#[test]
fn shadow_shader_translates_for_webgl2() {
    assert_translates_to_glsl_es_300(
        "shadow.wgsl",
        include_str!("../src/shaders/shadow.wgsl"),
        &[("vs_main", ShaderStage::Vertex)],
    );
}

#[test]
fn webgl2_defines_no_cube_array_support() {
    assert!(WEBGL2
        .shader_defs()
        .contains(concerto_render::shader::NO_CUBE_ARRAY_TEXTURES_SUPPORT));
}

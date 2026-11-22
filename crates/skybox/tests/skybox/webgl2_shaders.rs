use concerto_render::{
    capabilities::RenderCapabilities,
    shader::{SKY_RADIANCE_SQRT, ShaderDefs, compose_shader},
};
use naga::{
    ShaderStage,
    back::glsl,
    proc::BoundsCheckPolicies,
    valid::{Capabilities, ValidationFlags, Validator},
};

const WEBGL2: RenderCapabilities = RenderCapabilities {
    cube_array_textures: false,
    depth_bias_clamp: false,
    single_layer_texture_arrays: false,
    bc_textures: false,
    float_render_targets: true,
};

const WEBGL2_WITHOUT_FLOAT_TARGETS: RenderCapabilities = RenderCapabilities {
    float_render_targets: false,
    ..WEBGL2
};

struct Shader {
    label: &'static str,
    source: &'static str,
}

fn assert_translates_to_glsl_es_300(label: &str, source: &str, defs: &ShaderDefs) {
    let module = compose_shader(label, source, defs).unwrap();
    let info = Validator::new(ValidationFlags::all(), Capabilities::empty())
        .validate(&module)
        .unwrap_or_else(|err| panic!("{label} failed validation: {err:?}"));

    for stage in [ShaderStage::Vertex, ShaderStage::Fragment] {
        let entry_point = match stage {
            ShaderStage::Vertex => "vs_main",
            _ => "fs_main",
        };
        let options = glsl::Options {
            version: glsl::Version::Embedded {
                version: 300,
                is_webgl: true,
            },
            ..Default::default()
        };
        let pipeline_options = glsl::PipelineOptions {
            shader_stage: stage,
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
fn sky_shaders_compose_for_webgl2() {
    let def_sets = [
        WEBGL2.shader_defs(),
        WEBGL2_WITHOUT_FLOAT_TARGETS.shader_defs(),
    ];
    assert!(def_sets[1].contains(SKY_RADIANCE_SQRT));

    let shaders = [
        Shader {
            label: "sky_view.wgsl",
            source: include_str!("../../src/shaders/sky_view.wgsl"),
        },
        Shader {
            label: "sky.wgsl",
            source: include_str!("../../src/shaders/sky.wgsl"),
        },
        Shader {
            label: "sky_radiance_mip.wgsl",
            source: include_str!("../../src/shaders/sky_radiance_mip.wgsl"),
        },
    ];
    for defs in &def_sets {
        for shader in &shaders {
            assert_translates_to_glsl_es_300(shader.label, shader.source, defs);
        }
    }
}

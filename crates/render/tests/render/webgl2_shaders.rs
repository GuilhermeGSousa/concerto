use concerto_render::{
    capabilities::RenderCapabilities,
    shader::{compose_shader, ShaderDefs, NO_CUBE_ARRAY_TEXTURES_SUPPORT, SKY_RADIANCE_SQRT},
};
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
    bc_textures: false,
    float_render_targets: true,
};

const WEBGL2_WITHOUT_FLOAT_TARGETS: RenderCapabilities = RenderCapabilities {
    float_render_targets: false,
    ..WEBGL2
};

struct EntryPoint {
    name: &'static str,
    stage: ShaderStage,
}

const VERTEX: EntryPoint = EntryPoint {
    name: "vs_main",
    stage: ShaderStage::Vertex,
};

const FRAGMENT: EntryPoint = EntryPoint {
    name: "fs_main",
    stage: ShaderStage::Fragment,
};

const BLOOM: &[EntryPoint] = &[
    VERTEX,
    EntryPoint {
        name: "fs_downsample_first",
        stage: ShaderStage::Fragment,
    },
    EntryPoint {
        name: "fs_downsample",
        stage: ShaderStage::Fragment,
    },
    EntryPoint {
        name: "fs_upsample",
        stage: ShaderStage::Fragment,
    },
];

struct Shader {
    label: &'static str,
    source: &'static str,
    entry_points: &'static [EntryPoint],
}

fn assert_translates_to_glsl_es_300(
    label: &str,
    source: &str,
    entry_points: &[EntryPoint],
    defs: &ShaderDefs,
) {
    let module = compose_shader(label, source, defs).unwrap();
    let info = Validator::new(ValidationFlags::all(), Capabilities::empty())
        .validate(&module)
        .unwrap_or_else(|err| panic!("{label} failed validation: {err:?}"));

    for entry_point in entry_points {
        let options = glsl::Options {
            version: glsl::Version::Embedded {
                version: 300,
                is_webgl: true,
            },
            ..Default::default()
        };
        let pipeline_options = glsl::PipelineOptions {
            shader_stage: entry_point.stage,
            entry_point: entry_point.name.to_string(),
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
            panic!(
                "{label}::{} doesn't translate to GLSL ES 3.00: {err}",
                entry_point.name
            )
        });
    }
}

#[test]
fn every_shader_translates_to_glsl_es_300_under_the_webgl2_defines() {
    let def_sets = [
        WEBGL2.shader_defs(),
        WEBGL2_WITHOUT_FLOAT_TARGETS.shader_defs(),
    ];
    assert!(def_sets
        .iter()
        .all(|defs| defs.contains(NO_CUBE_ARRAY_TEXTURES_SUPPORT)));
    assert!(!def_sets[0].contains(SKY_RADIANCE_SQRT));
    assert!(def_sets[1].contains(SKY_RADIANCE_SQRT));

    let shaders = [
        Shader {
            label: "shader.wgsl",
            source: include_str!("../../src/shaders/shader.wgsl"),
            entry_points: &[VERTEX, FRAGMENT],
        },
        Shader {
            label: "shadow.wgsl",
            source: include_str!("../../src/shaders/shadow.wgsl"),
            entry_points: &[VERTEX],
        },
        Shader {
            label: "tonemapping.wgsl",
            source: include_str!("../../src/shaders/tonemapping.wgsl"),
            entry_points: &[VERTEX, FRAGMENT],
        },
        Shader {
            label: "ambient_occlusion.wgsl",
            source: include_str!("../../src/shaders/ambient_occlusion.wgsl"),
            entry_points: &[VERTEX, FRAGMENT],
        },
        Shader {
            label: "ambient_occlusion_blur.wgsl",
            source: include_str!("../../src/shaders/ambient_occlusion_blur.wgsl"),
            entry_points: &[VERTEX, FRAGMENT],
        },
        Shader {
            label: "bloom.wgsl",
            source: include_str!("../../src/shaders/bloom.wgsl"),
            entry_points: BLOOM,
        },
    ];
    for defs in &def_sets {
        for shader in &shaders {
            assert_translates_to_glsl_es_300(
                shader.label,
                shader.source,
                shader.entry_points,
                defs,
            );
        }
    }
}

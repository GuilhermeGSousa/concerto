//! WGSL preprocessing with naga_oil shader defs (`#ifdef`, `#if`, `#{NAME}`).

use std::{borrow::Cow, collections::HashMap};

use naga_oil::compose::{Composer, NagaModuleDescriptor, ShaderType};

pub use naga_oil::compose::ShaderDefValue;

/// Defined when cube-map arrays can't be sampled (WebGL2).
pub const NO_CUBE_ARRAY_TEXTURES_SUPPORT: &str = "NO_CUBE_ARRAY_TEXTURES_SUPPORT";

/// Named shader defs.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ShaderDefs(HashMap<String, ShaderDefValue>);

impl ShaderDefs {
    /// Defines `name` as `true`.
    pub fn define(&mut self, name: impl Into<String>) -> &mut Self {
        self.insert(name, ShaderDefValue::Bool(true))
    }

    /// Defines `name` with a value.
    pub fn insert(&mut self, name: impl Into<String>, value: ShaderDefValue) -> &mut Self {
        self.0.insert(name.into(), value);
        self
    }

    /// Undefines `name`.
    pub fn remove(&mut self, name: &str) -> Option<ShaderDefValue> {
        self.0.remove(name)
    }

    pub fn get(&self, name: &str) -> Option<&ShaderDefValue> {
        self.0.get(name)
    }

    pub fn contains(&self, name: &str) -> bool {
        self.0.contains_key(name)
    }
}

/// Preprocesses and validates `source`, returning the diagnostic on failure.
pub fn compose_shader(
    label: &str,
    source: &str,
    defs: &ShaderDefs,
) -> Result<wgpu::naga::Module, String> {
    let mut composer = Composer::default();
    composer
        .make_naga_module(NagaModuleDescriptor {
            source,
            file_path: label,
            shader_type: ShaderType::Wgsl,
            shader_defs: defs.0.clone(),
            additional_imports: &[],
        })
        .map_err(|err| err.emit_to_string(&composer))
}

/// Preprocesses `source` and creates a shader module. Panics if it fails to compose.
pub fn create_shader_module(
    device: &wgpu::Device,
    label: &str,
    source: &str,
    defs: &ShaderDefs,
) -> wgpu::ShaderModule {
    let module = compose_shader(label, source, defs)
        .unwrap_or_else(|err| panic!("failed to compose shader '{label}':\n{err}"));

    device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(label),
        source: wgpu::ShaderSource::Naga(Cow::Owned(module)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const BRANCHING: &str = r#"
#ifdef USE_B
fn picked() -> i32 { return 2; }
#else
fn picked() -> i32 { return 1; }
#endif

#if COUNT == 3
fn count_is_three() {}
#endif

@fragment
fn fs_main() -> @location(0) vec4<f32> {
    return vec4<f32>(f32(picked()));
}
"#;

    fn returned_literal(module: &wgpu::naga::Module) -> Option<i32> {
        let (_, function) = module
            .functions
            .iter()
            .find(|(_, f)| f.name.as_deref() == Some("picked"))?;
        function
            .expressions
            .iter()
            .find_map(|(_, expr)| match expr {
                wgpu::naga::Expression::Literal(wgpu::naga::Literal::I32(value)) => Some(*value),
                _ => None,
            })
    }

    fn has_function(module: &wgpu::naga::Module, name: &str) -> bool {
        module
            .functions
            .iter()
            .any(|(_, f)| f.name.as_deref() == Some(name))
    }

    #[test]
    fn ifdef_picks_the_else_branch_when_undefined() {
        let mut defs = ShaderDefs::default();
        defs.insert("COUNT", ShaderDefValue::Int(1));

        let module = compose_shader("branching", BRANCHING, &defs).unwrap();

        assert_eq!(returned_literal(&module), Some(1));
        assert!(!has_function(&module, "count_is_three"));
    }

    #[test]
    fn ifdef_and_if_follow_the_defs() {
        let mut defs = ShaderDefs::default();
        defs.define("USE_B").insert("COUNT", ShaderDefValue::Int(3));

        let module = compose_shader("branching", BRANCHING, &defs).unwrap();

        assert_eq!(returned_literal(&module), Some(2));
        assert!(has_function(&module, "count_is_three"));
    }

    #[test]
    fn invalid_shader_reports_the_label() {
        let err = compose_shader("broken.wgsl", "fn oops( {", &ShaderDefs::default()).unwrap_err();

        assert!(err.contains("broken.wgsl"), "{err}");
    }
}

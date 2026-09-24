extern crate proc_macro;

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{parse_macro_input, DeriveInput, Expr, Field, Ident, Lit, Meta, Type};

/// Texture dimension for the `#[texture(N)]` attribute.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TextureViewDimension {
    D2,
    Cube,
}

/// Represents the different types of shader resource bindings supported by the
/// `#[derive(AsBindGroup)]` macro.  Each variant corresponds to one
/// `@group(0)` entry in the generated `wgpu::BindGroupLayout`.
#[derive(Debug)]
enum BindingKind {
    /// A texture binding.  `dimension` controls whether it is a 2-D texture
    /// (the default) or a cube-map texture (`#[texture(N, dimension = "cube")]`).
    Texture {
        index: u32,
        dimension: TextureViewDimension,
    },
    /// A sampler linked to the texture at `texture_index`.
    Sampler { index: u32 },
    /// A plain `bytemuck::Pod + bytemuck::Zeroable` value uploaded as a uniform buffer.
    Uniform { index: u32 },
}

/// Associates a struct field with its shader binding metadata extracted from
/// the `#[texture(N)]`, `#[sampler(N)]`, or `#[uniform(N)]` attributes.
struct BindingField<'a> {
    field: &'a Field,
    kind: BindingKind,
}

/// Parsed contents of the struct-level `#[material(...)]` attribute.
struct MaterialAttr {
    vertex_shader: Option<Expr>,
    fragment_shader: Option<Expr>,
    /// `needs_camera()` override.  Defaults to `true` in the trait.
    camera: Option<bool>,
    /// `needs_lighting()` override.  Defaults to `false` in the trait.
    lighting: Option<bool>,
    /// `needs_skeleton()` override.  Defaults to `false` in the trait.
    skeleton: Option<bool>,
    /// `cull_mode()` override: `"back"`, `"front"`, or `"none"`.
    cull_mode: Option<String>,
    /// `topology()` override: `"triangle_list"` or `"line_list"`.
    topology: Option<String>,
    /// `clear_depth()` override.  Defaults to `true` in the trait.
    clear_depth: Option<bool>,
    /// `depth_stencil()` override: `"none"`, `"default"`, or `"read_only"`.
    ///
    /// - `"none"` → returns `None` (no depth/stencil)
    /// - `"default"` → depth write + `Less` compare (same as trait default)
    /// - `"read_only"` → depth test without write, `LessEqual` compare (skybox)
    depth_stencil: Option<String>,
    /// `vertex_layouts()` override: an arbitrary Rust expression.
    vertex_layouts: Option<Expr>,
    /// `blend_state()` override: `"replace"`, `"alpha"`, or `"none"`.
    blend: Option<String>,
}

/// Parse the integer literal inside an attribute like `#[texture(0)]`.
fn parse_index_from_attr(attr: &syn::Attribute) -> Option<u32> {
    match &attr.meta {
        Meta::List(list) => {
            let lit: Lit = syn::parse2(list.tokens.clone()).ok()?;
            if let Lit::Int(int) = lit {
                int.base10_parse::<u32>().ok()
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Parse `#[texture(N)]` or `#[texture(N, dimension = "cube")]`.
///
/// Returns `(index, dimension)` on success, or `None` if the attribute does
/// not match the expected syntax.
fn parse_texture_attr(attr: &syn::Attribute) -> Option<(u32, TextureViewDimension)> {
    let Meta::List(ref list) = attr.meta else {
        return None;
    };

    // Use a manual syn parser that handles both forms:
    //   #[texture(N)]
    //   #[texture(N, dimension = "cube")]
    use syn::parse::Parser;
    let parser = |input: syn::parse::ParseStream<'_>| -> syn::Result<(u32, TextureViewDimension)> {
        let idx_lit: syn::LitInt = input.parse()?;
        let idx = idx_lit.base10_parse::<u32>()?;
        let mut dim = TextureViewDimension::D2;
        if input.peek(syn::Token![,]) {
            let _: syn::Token![,] = input.parse()?;
            let key: syn::Ident = input.parse()?;
            let _: syn::Token![=] = input.parse()?;
            let val: syn::LitStr = input.parse()?;
            if key == "dimension" {
                dim = match val.value().as_str() {
                    "cube" => TextureViewDimension::Cube,
                    _ => TextureViewDimension::D2,
                };
            }
        }
        Ok((idx, dim))
    };
    parser.parse2(list.tokens.clone()).ok()
}

/// Parse a `key = true/false` boolean from a nested-meta value.
fn parse_bool_value(meta: &syn::meta::ParseNestedMeta<'_>) -> syn::Result<bool> {
    let value = meta.value()?;
    let lit: syn::LitBool = value.parse()?;
    Ok(lit.value)
}

/// Parse a `key = "string"` string literal from a nested-meta value.
fn parse_str_value(meta: &syn::meta::ParseNestedMeta<'_>) -> syn::Result<String> {
    let value = meta.value()?;
    let lit: syn::LitStr = value.parse()?;
    Ok(lit.value())
}

/// Extract binding fields from a struct's named fields.
fn collect_binding_fields(fields: &syn::FieldsNamed) -> Vec<BindingField<'_>> {
    let mut bindings = Vec::new();
    for field in &fields.named {
        for attr in &field.attrs {
            if attr.path().is_ident("texture") {
                if let Some((index, dimension)) = parse_texture_attr(attr) {
                    bindings.push(BindingField {
                        field,
                        kind: BindingKind::Texture { index, dimension },
                    });
                }
            } else if attr.path().is_ident("sampler") {
                if let Some(index) = parse_index_from_attr(attr) {
                    bindings.push(BindingField {
                        field,
                        kind: BindingKind::Sampler { index },
                    });
                }
            } else if attr.path().is_ident("uniform") {
                if let Some(index) = parse_index_from_attr(attr) {
                    bindings.push(BindingField {
                        field,
                        kind: BindingKind::Uniform { index },
                    });
                }
            }
        }
    }
    bindings
}

/// Parse the struct-level `#[material(...)]` attribute into a [`MaterialAttr`].
///
/// Supported keys:
/// - `vertex_shader = <expr>` — WGSL source for the vertex stage
/// - `fragment_shader = <expr>` — WGSL source for the fragment stage
/// - `camera = true|false` — override `needs_camera()` (trait default: `true`)
/// - `lighting = true|false` — override `needs_lighting()` (trait default: `false`)
/// - `skeleton = true|false` — override `needs_skeleton()` (trait default: `false`)
/// - `cull_mode = "back"|"front"|"none"` — override `cull_mode()`
/// - `topology = "triangle_list"|"line_list"` — override `topology()`
/// - `clear_depth = true|false` — override `clear_depth()` (trait default: `true`)
/// - `depth_stencil = "none"|"default"|"read_only"` — override `depth_stencil()`
/// - `vertex_layouts = <expr>` — override `vertex_layouts()`
///
/// `derive_as_bind_group` always emits `impl Material for YourStruct { … }`.
/// Methods whose keys are absent use the trait's default implementations.
fn parse_material_attr(attrs: &[syn::Attribute]) -> MaterialAttr {
    let mut result = MaterialAttr {
        vertex_shader: None,
        fragment_shader: None,
        camera: None,
        lighting: None,
        skeleton: None,
        cull_mode: None,
        topology: None,
        clear_depth: None,
        depth_stencil: None,
        vertex_layouts: None,
        blend: None,
    };
    for attr in attrs {
        if !attr.path().is_ident("material") {
            continue;
        }
        let _ = attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("vertex_shader") {
                let value = meta.value()?;
                result.vertex_shader = Some(value.parse()?);
            } else if meta.path.is_ident("fragment_shader") {
                let value = meta.value()?;
                result.fragment_shader = Some(value.parse()?);
            } else if meta.path.is_ident("camera") {
                result.camera = Some(parse_bool_value(&meta)?);
            } else if meta.path.is_ident("lighting") {
                result.lighting = Some(parse_bool_value(&meta)?);
            } else if meta.path.is_ident("skeleton") {
                result.skeleton = Some(parse_bool_value(&meta)?);
            } else if meta.path.is_ident("cull_mode") {
                result.cull_mode = Some(parse_str_value(&meta)?);
            } else if meta.path.is_ident("topology") {
                result.topology = Some(parse_str_value(&meta)?);
            } else if meta.path.is_ident("clear_depth") {
                result.clear_depth = Some(parse_bool_value(&meta)?);
            } else if meta.path.is_ident("depth_stencil") {
                result.depth_stencil = Some(parse_str_value(&meta)?);
            } else if meta.path.is_ident("vertex_layouts") {
                let value = meta.value()?;
                result.vertex_layouts = Some(value.parse()?);
            } else if meta.path.is_ident("blend") {
                result.blend = Some(parse_str_value(&meta)?);
            }
            Ok(())
        });
    }
    result
}

/// Generate the `bind_group_layout` method body.
fn gen_bind_group_layout(bindings: &[BindingField<'_>], struct_name: &Ident) -> TokenStream2 {
    let struct_label = struct_name.to_string() + "_layout";
    let entries: Vec<TokenStream2> = bindings
        .iter()
        .map(|b| match &b.kind {
            BindingKind::Texture { index, dimension } => {
                let view_dim = match dimension {
                    TextureViewDimension::D2 => quote! { wgpu::TextureViewDimension::D2 },
                    TextureViewDimension::Cube => quote! { wgpu::TextureViewDimension::Cube },
                };
                quote! {
                    wgpu::BindGroupLayoutEntry {
                        binding: #index,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            multisampled: false,
                            view_dimension: #view_dim,
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        },
                        count: None,
                    }
                }
            }
            BindingKind::Sampler { index } => {
                quote! {
                    wgpu::BindGroupLayoutEntry {
                        binding: #index,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    }
                }
            }
            BindingKind::Uniform { index } => {
                quote! {
                    wgpu::BindGroupLayoutEntry {
                        binding: #index,
                        visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    }
                }
            }
        })
        .collect();

    quote! {
        fn bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some(#struct_label),
                entries: &[ #(#entries),* ],
            })
        }
    }
}

/// Check whether a type looks like `Option<AssetHandle<…>>`.
fn is_option_asset_handle(ty: &Type) -> bool {
    if let Type::Path(type_path) = ty {
        if let Some(seg) = type_path.path.segments.last() {
            if seg.ident == "Option" {
                if let syn::PathArguments::AngleBracketed(args) = &seg.arguments {
                    if let Some(syn::GenericArgument::Type(Type::Path(inner))) = args.args.first() {
                        if let Some(inner_seg) = inner.path.segments.last() {
                            return inner_seg.ident == "AssetHandle";
                        }
                    }
                }
            }
        }
    }
    false
}

/// Generate the `create_bind_group` method body.
fn gen_create_bind_group(bindings: &[BindingField<'_>], struct_name: &Ident) -> TokenStream2 {
    let bind_group_label = struct_name.to_string() + "_bind_group";
    let mut entry_stmts: Vec<TokenStream2> = Vec::new();

    for b in bindings {
        let field_name = b.field.ident.as_ref().unwrap();
        match &b.kind {
            BindingKind::Texture { index, .. } => {
                // If the type is Option<AssetHandle<…>> we do a lookup in render_textures.
                if is_option_asset_handle(&b.field.ty) {
                    entry_stmts.push(quote! {
                        if let Some(tex_handle) = &self.#field_name {
                            if let Some(tex) = render_textures.get(&tex_handle.id()) {
                                entries.push(wgpu::BindGroupEntry {
                                    binding: #index,
                                    resource: wgpu::BindingResource::TextureView(&tex.view),
                                });
                            } else {
                                return Err(concerto_render::render_asset::AssetPreparationError::NotReady);
                            }
                        } else {
                            entries.push(wgpu::BindGroupEntry {
                                binding: #index,
                                resource: wgpu::BindingResource::TextureView(&dummy_texture.view),
                            });
                        }
                    });
                }
            }
            BindingKind::Sampler { index } => {
                // Find the texture field that was declared before this sampler. We look for the
                // most-recently-seen texture binding whose index is `index - 1`.
                // For robustness we emit code that re-queries the same Option field used for the
                // paired texture.  We track which texture field to pair with by searching backwards.
                let paired_texture: Option<(&Field, u32)> = bindings
                    .iter()
                    .filter_map(|b2| {
                        if let BindingKind::Texture { index: ti, .. } = b2.kind {
                            Some((b2.field, ti))
                        } else {
                            None
                        }
                    })
                    .rfind(|(_, ti)| *ti < *index);

                if let Some((tex_field, _)) = paired_texture {
                    let tex_field_name = tex_field.ident.as_ref().unwrap();
                    if is_option_asset_handle(&tex_field.ty) {
                        entry_stmts.push(quote! {
                            if let Some(tex_handle) = &self.#tex_field_name {
                                if let Some(tex) = render_textures.get(&tex_handle.id()) {
                                    entries.push(wgpu::BindGroupEntry {
                                        binding: #index,
                                        resource: wgpu::BindingResource::Sampler(&tex.sampler),
                                    });
                                } else {
                                    return Err(concerto_render::render_asset::AssetPreparationError::NotReady);
                                }
                            } else {
                                entries.push(wgpu::BindGroupEntry {
                                    binding: #index,
                                    resource: wgpu::BindingResource::Sampler(&dummy_texture.sampler),
                                });
                            }
                        });
                    }
                } else {
                    // Standalone sampler — always use dummy.
                    entry_stmts.push(quote! {
                        entries.push(wgpu::BindGroupEntry {
                            binding: #index,
                            resource: wgpu::BindingResource::Sampler(&dummy_texture.sampler),
                        });
                    });
                }
            }
            BindingKind::Uniform { index } => {
                let buf_label = format!("{}_{}_uniform", struct_name, field_name);
                entry_stmts.push(quote! {
                    let uniform_buf = device.create_buffer_init(
                        &wgpu::util::BufferInitDescriptor {
                            label: Some(#buf_label),
                            contents: bytemuck::cast_slice(&[self.#field_name]),
                            usage: wgpu::BufferUsages::UNIFORM,
                        },
                    );
                    entries.push(wgpu::BindGroupEntry {
                        binding: #index,
                        resource: wgpu::BindingResource::Buffer(
                            uniform_buf.as_entire_buffer_binding(),
                        ),
                    });
                });
            }
        }
    }

    quote! {
        fn create_bind_group(
            &self,
            device: &wgpu::Device,
            render_textures: &concerto_render::render_asset::RenderAssets<concerto_render::render_asset::render_texture::RenderTexture>,
            dummy_texture: &concerto_render::render_asset::render_texture::DummyRenderTexture,
            layout: &wgpu::BindGroupLayout,
        ) -> Result<wgpu::BindGroup, concerto_render::render_asset::AssetPreparationError> {
            use wgpu::util::DeviceExt;
            let mut entries: Vec<wgpu::BindGroupEntry<'_>> = Vec::new();
            #(#entry_stmts)*
            Ok(device.create_bind_group(&wgpu::BindGroupDescriptor {
                layout,
                entries: &entries,
                label: Some(#bind_group_label),
            }))
        }
    }
}

/// Generate the body of `impl Material for <Name>` from the parsed attribute.
fn gen_material_impl(name: &Ident, m: &MaterialAttr) -> TokenStream2 {
    let camera_fn = m
        .camera
        .map(|val| {
            quote! {
                fn needs_camera() -> bool { #val }
            }
        })
        .unwrap_or_default();

    let lighting_fn = m
        .lighting
        .map(|val| {
            quote! {
                fn needs_lighting() -> bool { #val }
            }
        })
        .unwrap_or_default();

    let skeleton_fn = m
        .skeleton
        .map(|val| {
            quote! {
                fn needs_skeleton() -> bool { #val }
            }
        })
        .unwrap_or_default();

    let cull_mode_fn = m
        .cull_mode
        .as_deref()
        .map(|cm| {
            let expr = match cm {
                "front" => quote! { Some(wgpu::Face::Front) },
                "none" => quote! { None },
                _ => quote! { Some(wgpu::Face::Back) },
            };
            quote! { fn cull_mode() -> Option<wgpu::Face> { #expr } }
        })
        .unwrap_or_default();

    let topology_fn = m
        .topology
        .as_deref()
        .map(|topo| {
            let expr = match topo {
                "line_list" => quote! { wgpu::PrimitiveTopology::LineList },
                _ => quote! { wgpu::PrimitiveTopology::TriangleList },
            };
            quote! { fn topology() -> wgpu::PrimitiveTopology { #expr } }
        })
        .unwrap_or_default();

    let clear_depth_fn = m
        .clear_depth
        .map(|val| {
            quote! {
                fn clear_depth() -> bool { #val }
            }
        })
        .unwrap_or_default();

    let depth_stencil_fn = m
        .depth_stencil
        .as_deref()
        .map(|ds| {
            let expr = match ds {
                "none" => quote! { None },
                "read_only" => quote! {
                    Some(wgpu::DepthStencilState {
                        format: wgpu::TextureFormat::Depth32Float,
                        depth_write_enabled: false,
                        depth_compare: wgpu::CompareFunction::LessEqual,
                        stencil: wgpu::StencilState::default(),
                        bias: wgpu::DepthBiasState::default(),
                    })
                },
                _ => quote! {
                    Some(wgpu::DepthStencilState {
                        format: wgpu::TextureFormat::Depth32Float,
                        depth_write_enabled: true,
                        depth_compare: wgpu::CompareFunction::Less,
                        stencil: wgpu::StencilState::default(),
                        bias: wgpu::DepthBiasState::default(),
                    })
                },
            };
            quote! { fn depth_stencil() -> Option<wgpu::DepthStencilState> { #expr } }
        })
        .unwrap_or_default();

    let vertex_layouts_fn = m
        .vertex_layouts
        .as_ref()
        .map(|expr| {
            quote! {
                fn vertex_layouts() -> Vec<wgpu::VertexBufferLayout<'static>> { #expr }
            }
        })
        .unwrap_or_default();

    let blend_state_fn = m
        .blend
        .as_deref()
        .map(|b| {
            let expr = match b {
                "alpha" => quote! { Some(wgpu::BlendState::ALPHA_BLENDING) },
                "none" => quote! { None },
                _ => quote! { Some(wgpu::BlendState::REPLACE) },
            };
            quote! { fn blend_state() -> Option<wgpu::BlendState> { #expr } }
        })
        .unwrap_or_default();

    quote! {
        impl concerto_render::assets::material::Material for #name {
            #camera_fn
            #lighting_fn
            #skeleton_fn
            #cull_mode_fn
            #topology_fn
            #clear_depth_fn
            #depth_stencil_fn
            #vertex_layouts_fn
            #blend_state_fn
        }
    }
}

/// `#[derive(AsBindGroup)]` — automatically implement [`AsBindGroup`] for a struct,
/// and optionally [`Material`] when any Material-related keys are present in
/// `#[material(...)]`.
///
/// # Field attributes
///
/// | Attribute       | Field type                          | Description                                  |
/// |-----------------|-------------------------------------|----------------------------------------------|
/// | `#[texture(N)]` | `Option<AssetHandle<Texture>>`      | Texture binding at slot *N* (FRAGMENT stage) |
/// | `#[sampler(N)]` | *(any)*                             | Sampler binding at slot *N* (FRAGMENT stage) |
/// | `#[uniform(N)]` | `T: bytemuck::Pod + bytemuck::Zeroable` | Uniform buffer at slot *N*               |
///
/// # Struct attributes (`#[material(...)]`)
///
/// | Key              | Values                                    | Trait method overridden   |
/// |------------------|-------------------------------------------|---------------------------|
/// | `vertex_shader`  | `<expr>`                                  | `AsBindGroup::vertex_shader()` |
/// | `fragment_shader`| `<expr>`                                  | `AsBindGroup::fragment_shader()` |
/// | `camera`         | `true \| false`                           | `needs_camera()` (default `true`) |
/// | `lighting`       | `true \| false`                           | `needs_lighting()` (default `false`) |
/// | `skeleton`       | `true \| false`                           | `needs_skeleton()` (default `false`) |
/// | `cull_mode`      | `"back" \| "front" \| "none"`            | `cull_mode()` (default `Back`) |
/// | `topology`       | `"triangle_list" \| "line_list"`          | `topology()` |
/// | `clear_depth`    | `true \| false`                           | `clear_depth()` (default `true`) |
/// | `depth_stencil`  | `"none" \| "default" \| "read_only"`     | `depth_stencil()` |
/// | `vertex_layouts` | `<expr>`                                  | `vertex_layouts()` |
///
/// The macro always emits `impl Material for YourStruct { … }`.  Methods whose
/// keys are absent fall back to the trait's default implementations.
#[proc_macro_derive(AsBindGroup, attributes(texture, sampler, uniform, material))]
pub fn derive_as_bind_group(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = &input.ident;

    let mat_attr = parse_material_attr(&input.attrs);

    let vertex_shader_expr = match mat_attr.vertex_shader {
        Some(ref expr) => quote! { concerto_render::assets::material::ShaderRef::Source(#expr) },
        None => quote! { concerto_render::assets::material::ShaderRef::Default },
    };
    let fragment_shader_expr = match mat_attr.fragment_shader {
        Some(ref expr) => quote! { concerto_render::assets::material::ShaderRef::Source(#expr) },
        None => quote! { concerto_render::assets::material::ShaderRef::Default },
    };

    let named_fields = match &input.data {
        syn::Data::Struct(s) => match &s.fields {
            syn::Fields::Named(f) => f,
            _ => {
                return syn::Error::new_spanned(
                    &input,
                    "AsBindGroup only supports structs with named fields",
                )
                .to_compile_error()
                .into()
            }
        },
        _ => {
            return syn::Error::new_spanned(&input, "AsBindGroup only supports structs")
                .to_compile_error()
                .into()
        }
    };

    let bindings = collect_binding_fields(named_fields);

    let layout_fn = gen_bind_group_layout(&bindings, name);
    let bind_group_fn = gen_create_bind_group(&bindings, name);

    let material_impl = gen_material_impl(name, &mat_attr);

    let expanded = quote! {
        impl concerto_render::assets::material::AsBindGroup for #name {
            fn vertex_shader() -> concerto_render::assets::material::ShaderRef {
                #vertex_shader_expr
            }

            fn fragment_shader() -> concerto_render::assets::material::ShaderRef {
                #fragment_shader_expr
            }

            #layout_fn

            #bind_group_fn
        }

        #material_impl
    };

    expanded.into()
}

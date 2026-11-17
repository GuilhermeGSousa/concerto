extern crate proc_macro;
extern crate syn;

#[macro_use]
extern crate quote;

use proc_macro::TokenStream;
use syn::{parse_macro_input, Data, DeriveInput, Fields};

/// Implementation detail of concerto-foundation's hygienic `asset_id!` wrapper.
#[proc_macro]
pub fn asset_id_bytes(input: TokenStream) -> TokenStream {
    let path = parse_macro_input!(input as syn::LitStr);
    match read_asset_id(&path) {
        Ok(AssetIdLookup::Found(bytes)) => quote!({
            // Keep the configured root visible to rustc's environment tracking.
            const _: &str = env!("CONCERTO_ASSET_ROOT");
            [#(#bytes),*]
        })
        .into(),
        Ok(AssetIdLookup::Missing(message)) => quote_spanned!(path.span()=> {
            const _: &str = env!("CONCERTO_ASSET_ROOT");
            #[deprecated(note = #message)]
            const fn missing_asset() {}
            const _: () = missing_asset();
            [0u8; 16]
        })
        .into(),
        Err(message) => syn::Error::new(path.span(), message)
            .to_compile_error()
            .into(),
    }
}

enum AssetIdLookup {
    Found([u8; 16]),
    Missing(String),
}

fn read_asset_id(literal: &syn::LitStr) -> Result<AssetIdLookup, String> {
    use std::path::Path;

    let tracking_root = std::env::var_os("CONCERTO_ASSET_ROOT").ok_or_else(|| {
        "asset_id! requires Cargo dependency tracking: add asset-build to [build-dependencies] and call asset_build::track_assets(\"content\") in build.rs".to_string()
    })?;
    let manifest = std::env::var_os("CARGO_MANIFEST_DIR")
        .ok_or_else(|| "asset_id! must be compiled by Cargo".to_string())?;
    let value = literal.value().replace('\\', "/");
    let relative = Path::new(&value);
    if relative.is_absolute() {
        return Err("asset_id! expects a path relative to the calling package".into());
    }
    let path = Path::new(&manifest).join(relative);
    let mut found = existing_file(&path)?.map(|canonical| AssetFile {
        path: path.clone(),
        canonical,
        embedded_as: None,
    });
    for (at, _) in value.match_indices('#') {
        if found.is_some() {
            break;
        }
        let file = Path::new(&manifest).join(&value[..at]);
        found = existing_file(&file)?
            .filter(|canonical| canonical.is_file())
            .map(|canonical| AssetFile {
                path: file,
                canonical,
                embedded_as: Some(&value[at + 1..]),
            });
    }
    let Some(file) = found else {
        return Ok(AssetIdLookup::Missing(format!(
            "asset_id!: no asset at '{}'; using a nil id",
            path.display()
        )));
    };
    if !file.canonical.starts_with(Path::new(&tracking_root)) {
        return Err(format!(
            "asset_id!: '{}' is outside the directory registered by asset_build::track_assets",
            file.path.display()
        ));
    }
    let header = concerto_asset_format::read_content_asset_header(&file.canonical)
        .map_err(|error| format!("asset_id!: {error:#}"))?;
    let Some(name) = file.embedded_as else {
        return Ok(AssetIdLookup::Found(header.asset_id.to_bytes()));
    };
    match header.embedded.iter().find(|asset| asset.name() == name) {
        Some(asset) => Ok(AssetIdLookup::Found(asset.asset_id.to_bytes())),
        None => Err(format!(
            "asset_id!: '{}' holds no embedded asset named '{name}'; it holds [{}]",
            file.path.display(),
            header
                .embedded
                .iter()
                .map(|asset| asset.name())
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

struct AssetFile<'a> {
    path: std::path::PathBuf,
    canonical: std::path::PathBuf,
    embedded_as: Option<&'a str>,
}

fn existing_file(path: &std::path::Path) -> Result<Option<std::path::PathBuf>, String> {
    match path.canonicalize() {
        Ok(canonical) => Ok(Some(canonical)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!(
            "asset_id!: cannot open '{}': {error}",
            path.display()
        )),
    }
}

#[proc_macro_derive(Asset, attributes(asset))]
pub fn asset(input: TokenStream) -> TokenStream {
    let ast = syn::parse(input).unwrap();
    impl_asset(&ast)
}

#[derive(Default)]
struct AssetAttributes {
    version: Option<u32>,
    editor_only: Option<syn::Type>,
}

fn asset_attributes(ast: &syn::DeriveInput) -> syn::Result<AssetAttributes> {
    let mut attributes = AssetAttributes::default();
    for attr in ast
        .attrs
        .iter()
        .filter(|attr| attr.path().is_ident("asset"))
    {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("editor_only") {
                attributes.editor_only = Some(meta.value()?.parse::<syn::Type>().map_err(|_| {
                    meta.error("expected #[asset(editor_only = …)] with a type")
                })?);
                return Ok(());
            }
            let expected = || {
                meta.error(
                    "expected #[asset(version = …)] with an integer literal or #[asset(editor_only = …)] with a type",
                )
            };
            if !meta.path.is_ident("version") {
                return Err(expected());
            }
            match meta.value().and_then(|value| value.parse::<syn::Lit>()) {
                Ok(syn::Lit::Int(int)) => {
                    attributes.version = Some(int.base10_parse::<u32>()?);
                    Ok(())
                }
                _ => Err(expected()),
            }
        })?;
    }
    Ok(attributes)
}

fn impl_asset(ast: &syn::DeriveInput) -> TokenStream {
    let name = &ast.ident;
    let attributes = match asset_attributes(ast) {
        Ok(attributes) => attributes,
        Err(error) => return error.to_compile_error().into(),
    };
    let version = attributes
        .version
        .map(|version| quote! { const FORMAT_VERSION: u32 = #version; });
    let editor_only = match attributes.editor_only {
        Some(editor_only) => quote! { #editor_only },
        None => quote! { () },
    };
    let gen = quote! {
        impl Asset for #name {
            type EditorOnly = #editor_only;

            #version

            fn name() -> &'static str {
                stringify!(#name)
            }
        }

    };
    gen.into()
}

#[proc_macro_derive(Blendable)]
pub fn lerp_derive(input: TokenStream) -> TokenStream {
    // 1. Parse the input Rust code (the struct) into a syntax tree
    let ast = parse_macro_input!(input as DeriveInput);
    let name = &ast.ident;

    // 2. Check that we are deriving on a Struct
    let fields_processing = match ast.data {
        Data::Struct(data_struct) => match data_struct.fields {
            // Case A: Named fields (e.g., struct Point { x: f32, y: f32 })
            Fields::Named(fields) => {
                let recurse = fields.named.iter().map(|f| {
                    let field_name = &f.ident;
                    let field_type = &f.ty;
                    // Generate: field_name: self.field_name.lerp(other.field_name, t)
                    quote! {
                        #field_name: #field_type::interpolate(from.#field_name, other.#field_name, t)
                    }
                });
                quote! {
                    { #(#recurse),* }
                }
            }
            // Case B: Unnamed/Tuple fields (e.g., struct Point(f32, f32))
            Fields::Unnamed(fields) => {
                let recurse = fields.unnamed.iter().enumerate().map(|(i, field)| {
                    let index = syn::Index::from(i);
                    let field_type = &field.ty;
                    quote! {
                        #field_type::interpolate(from.#index, other.#index, t)
                    }
                });
                quote! {
                    ( #(#recurse),* )
                }
            }
            // Case C: Unit structs (e.g., struct Unit;)
            Fields::Unit => quote! { {} },
        },
        _ => panic!("Lerp derive macro only supports Structs"),
    };

    // 3. Generate the implementation
    let gen = quote! {
        impl Blendable for #name {
            fn interpolate(from: Self, other: Self, t: f32) -> Self {
                #name #fields_processing
            }
        }
    };

    // 4. Return the generated code
    gen.into()
}

#[proc_macro_derive(AsAny)]
pub fn any_derive(input: TokenStream) -> TokenStream {
    let ast: DeriveInput = syn::parse(input).unwrap();
    let name = &ast.ident;
    let gen = quote! {

        impl AsAny for #name {
            fn as_any(&self) -> &dyn Any
            {
                self
            }

            fn as_any_mut(&mut self) -> &mut dyn Any
            {
                self
            }
        }

    };
    gen.into()
}

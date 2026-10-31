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
    let canonical = match path.canonicalize() {
        Ok(canonical) => canonical,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(AssetIdLookup::Missing(format!(
                "asset_id!: no asset at '{}'; using a nil id",
                path.display()
            )));
        }
        Err(error) => {
            return Err(format!(
                "asset_id!: cannot open '{}': {error}",
                path.display()
            ))
        }
    };
    if !canonical.starts_with(Path::new(&tracking_root)) {
        return Err(format!(
            "asset_id!: '{}' is outside the directory registered by asset_build::track_assets",
            path.display()
        ));
    }
    concerto_asset_format::read_content_asset_header(&canonical)
        .map(|header| AssetIdLookup::Found(header.asset_id.to_bytes()))
        .map_err(|error| format!("asset_id!: {error:#}"))
}

#[proc_macro_derive(Asset, attributes(asset))]
pub fn asset(input: TokenStream) -> TokenStream {
    let ast = syn::parse(input).unwrap();
    impl_asset(&ast)
}

fn asset_version(ast: &syn::DeriveInput) -> syn::Result<Option<u32>> {
    let mut version = None;
    for attr in ast
        .attrs
        .iter()
        .filter(|attr| attr.path().is_ident("asset"))
    {
        attr.parse_nested_meta(|meta| {
            let expected = || meta.error("expected #[asset(version = …)] with an integer literal");
            if !meta.path.is_ident("version") {
                return Err(expected());
            }
            match meta.value().and_then(|value| value.parse::<syn::Lit>()) {
                Ok(syn::Lit::Int(int)) => {
                    version = Some(int.base10_parse::<u32>()?);
                    Ok(())
                }
                _ => Err(expected()),
            }
        })?;
    }
    Ok(version)
}

fn impl_asset(ast: &syn::DeriveInput) -> TokenStream {
    let name = &ast.ident;
    let version = match asset_version(ast) {
        Ok(version) => version.map(|version| quote! { const FORMAT_VERSION: u32 = #version; }),
        Err(error) => return error.to_compile_error().into(),
    };
    let gen = quote! {
        impl Asset for #name {
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

//! Derive macros for `concerto-ecs`; use them through its re-exports.

extern crate proc_macro;
extern crate syn;

#[macro_use]
extern crate quote;

use proc_macro::TokenStream;

#[proc_macro_derive(Component, attributes(require))]
pub fn component(input: TokenStream) -> TokenStream {
    let ast = syn::parse(input).unwrap();
    impl_component(&ast)
}

struct Requirement {
    ty: syn::Type,
    constructor: Option<syn::Expr>,
}

impl syn::parse::Parse for Requirement {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let ty = input.parse()?;
        let constructor = if input.peek(syn::Token![=]) {
            input.parse::<syn::Token![=]>()?;
            Some(input.parse()?)
        } else {
            None
        };
        Ok(Self { ty, constructor })
    }
}

fn ecs_root() -> proc_macro2::TokenStream {
    use proc_macro_crate::{crate_name, FoundCrate};
    let named = |name: &str| {
        let ident =
            proc_macro2::Ident::new(&name.replace('-', "_"), proc_macro2::Span::call_site());
        quote!(::#ident)
    };
    match crate_name("concerto-ecs") {
        Ok(FoundCrate::Itself) => {
            if std::env::var("CARGO_CRATE_NAME").is_ok_and(|name| name == "concerto_ecs") {
                quote!(crate)
            } else {
                quote!(::concerto_ecs)
            }
        }
        Ok(FoundCrate::Name(name)) => named(&name),
        Err(_) => match crate_name("concerto") {
            Ok(FoundCrate::Name(name)) => {
                let root = named(&name);
                quote!(#root::ecs)
            }
            _ => quote!(::concerto_ecs),
        },
    }
}

fn impl_component(ast: &syn::DeriveInput) -> TokenStream {
    let name = &ast.ident;
    let (impl_generics, type_generics, where_clause) = ast.generics.split_for_impl();
    let mut requirements = Vec::new();
    for attr in ast
        .attrs
        .iter()
        .filter(|attr| attr.path().is_ident("require"))
    {
        match attr.parse_args_with(
            syn::punctuated::Punctuated::<Requirement, syn::Token![,]>::parse_terminated,
        ) {
            Ok(parsed) => requirements.extend(parsed),
            Err(error) => return error.to_compile_error().into(),
        }
    }
    let register_required = if requirements.is_empty() {
        quote!()
    } else {
        let root = ecs_root();
        let registrations = requirements.iter().map(|requirement| {
            let ty = &requirement.ty;
            match &requirement.constructor {
                Some(expr) => quote! { required.register::<#ty>(|| #expr); },
                None => quote! {
                    required.register::<#ty>(<#ty as ::std::default::Default>::default);
                },
            }
        });
        quote! {
            fn register_required(required: &mut #root::component::RequiredComponents) {
                #(#registrations)*
            }
        }
    };
    let gen = quote! {
        impl #impl_generics Component for #name #type_generics #where_clause {
            #register_required
        }
    };
    gen.into()
}

#[proc_macro_derive(Resource)]
pub fn resource(input: TokenStream) -> TokenStream {
    let ast = syn::parse(input).unwrap();
    impl_resource(&ast)
}

fn impl_resource(ast: &syn::DeriveInput) -> TokenStream {
    let name = &ast.ident;
    let (impl_generics, type_generics, where_clause) = ast.generics.split_for_impl();
    let gen = quote! {
        impl #impl_generics Resource for #name #type_generics #where_clause  {
            fn name() -> &'static str {
                stringify!(#name)
            }
        }

    };
    gen.into()
}

#[proc_macro_derive(Event)]
pub fn event(input: TokenStream) -> TokenStream {
    let ast = syn::parse(input).unwrap();
    impl_event(&ast)
}

fn impl_event(ast: &syn::DeriveInput) -> TokenStream {
    let name = &ast.ident;
    let (impl_generics, type_generics, where_clause) = ast.generics.split_for_impl();
    let gen = quote! {
        impl #impl_generics Event for #name #type_generics #where_clause  {
        }

    };
    gen.into()
}

#[proc_macro_derive(SystemSet)]
pub fn system_set(input: TokenStream) -> TokenStream {
    let ast = syn::parse(input).unwrap();
    impl_label(&ast, quote!(SystemSet))
}

#[proc_macro_derive(SystemTag)]
pub fn system_tag(input: TokenStream) -> TokenStream {
    let ast = syn::parse(input).unwrap();
    impl_label(&ast, quote!(SystemTag))
}

#[proc_macro_derive(ScheduleLabel)]
pub fn schedule_label(input: TokenStream) -> TokenStream {
    let ast = syn::parse(input).unwrap();
    impl_label(&ast, quote!(ScheduleLabel))
}

fn impl_label(ast: &syn::DeriveInput, trait_name: proc_macro2::TokenStream) -> TokenStream {
    let name = &ast.ident;
    let (impl_generics, type_generics, where_clause) = ast.generics.split_for_impl();
    let gen = quote! {
        impl #impl_generics #trait_name for #name #type_generics #where_clause {
            fn dyn_clone(&self) -> ::std::boxed::Box<dyn #trait_name> {
                ::std::boxed::Box::new(::std::clone::Clone::clone(self))
            }
        }
    };
    gen.into()
}

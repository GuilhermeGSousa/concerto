extern crate proc_macro;
extern crate syn;

#[macro_use]
extern crate quote;

use proc_macro::TokenStream;

#[proc_macro_derive(Component)]
pub fn component(input: TokenStream) -> TokenStream {
    let ast = syn::parse(input).unwrap();
    impl_component(&ast)
}

fn impl_component(ast: &syn::DeriveInput) -> TokenStream {
    let name = &ast.ident;
    let (impl_generics, type_generics, where_clause) = ast.generics.split_for_impl();
    let gen = quote! {
        impl #impl_generics Component for #name #type_generics #where_clause {}
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

use proc_macro::TokenStream;
use quote::{quote, quote_spanned};
use syn::{Data, DataStruct, DeriveInput, Fields, spanned::Spanned};

#[proc_macro_derive(Editable)]
pub fn derive_editable(input: TokenStream) -> TokenStream {
    let ast = syn::parse_macro_input!(input as DeriveInput);
    let name = &ast.ident;
    let Data::Struct(DataStruct {
        fields: Fields::Named(fields),
        ..
    }) = &ast.data
    else {
        return syn::Error::new_spanned(
            name,
            "#[derive(Editable)] supports structs with named fields only",
        )
        .to_compile_error()
        .into();
    };

    // Spanned on each field's type so a non-`Editable` field is reported there.
    let visits = fields.named.iter().map(|field| {
        let ident = field.ident.as_ref().expect("named field");
        let label = ident.to_string();
        quote_spanned!(field.ty.span()=> visitor.field(#label, &self.#ident);)
    });
    let visits_mut = fields.named.iter().map(|field| {
        let ident = field.ident.as_ref().expect("named field");
        let label = ident.to_string();
        quote_spanned!(field.ty.span()=> visitor.field(#label, &mut self.#ident);)
    });

    let (impl_generics, type_generics, where_clause) = ast.generics.split_for_impl();
    quote! {
        impl #impl_generics ::editable::Editable for #name #type_generics #where_clause {
            fn visit(&self, visitor: &mut dyn ::editable::PropertyVisitor) {
                #(#visits)*
            }

            fn visit_mut(&mut self, visitor: &mut dyn ::editable::PropertyVisitorMut) {
                #(#visits_mut)*
            }
        }
    }
    .into()
}

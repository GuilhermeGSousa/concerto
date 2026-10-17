use proc_macro::TokenStream;
use proc_macro_crate::{FoundCrate, crate_name};
use quote::{quote, quote_spanned};
use syn::{Data, DataStruct, DeriveInput, Fields, spanned::Spanned};

fn reflect_root(dependencies: &[&str]) -> proc_macro2::TokenStream {
    if dependencies.contains(&"concerto-reflect") {
        quote!(::concerto_reflect)
    } else if dependencies.contains(&"concerto") {
        quote!(::concerto::reflect)
    } else {
        quote!(::concerto_reflect)
    }
}

fn caller_reflect_root() -> proc_macro2::TokenStream {
    let mut dependencies = Vec::new();
    match crate_name("concerto-reflect") {
        Ok(FoundCrate::Name(name)) if name != "concerto_reflect" => {
            let ident = proc_macro2::Ident::new(&name, proc_macro2::Span::call_site());
            return quote!(::#ident);
        }
        Ok(_) => dependencies.push("concerto-reflect"),
        Err(_) => {}
    }
    match crate_name("concerto") {
        Ok(FoundCrate::Name(name)) if name != "concerto" => {
            let ident = proc_macro2::Ident::new(&name, proc_macro2::Span::call_site());
            if dependencies.is_empty() {
                return quote!(::#ident::reflect);
            }
        }
        Ok(_) => dependencies.push("concerto"),
        Err(_) => {}
    }
    reflect_root(&dependencies)
}

#[proc_macro_derive(Reflect)]
pub fn derive_reflect(input: TokenStream) -> TokenStream {
    let ast = syn::parse_macro_input!(input as DeriveInput);
    let name = &ast.ident;
    let Data::Struct(DataStruct {
        fields: Fields::Named(fields),
        ..
    }) = &ast.data
    else {
        return syn::Error::new_spanned(
            name,
            "#[derive(Reflect)] supports structs with named fields only",
        )
        .to_compile_error()
        .into();
    };

    // Spanned on each field's type so a non-`Reflect` field is reported there.
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

    let root = caller_reflect_root();
    let (impl_generics, type_generics, where_clause) = ast.generics.split_for_impl();
    quote! {
        impl #impl_generics #root::Reflect for #name #type_generics #where_clause {
            fn visit(&self, visitor: &mut dyn #root::PropertyVisitor) {
                #(#visits)*
            }

            fn visit_mut(&mut self, visitor: &mut dyn #root::PropertyVisitorMut) {
                #(#visits_mut)*
            }
        }
    }
    .into()
}

#[cfg(test)]
mod tests {
    use super::reflect_root;

    #[test]
    fn a_crate_depending_on_concerto_reaches_reflect_through_it() {
        assert_eq!(
            reflect_root(&["concerto"]).to_string(),
            ":: concerto :: reflect"
        );
    }

    #[test]
    fn a_crate_depending_on_concerto_reflect_uses_it_directly() {
        assert_eq!(
            reflect_root(&["concerto-reflect"]).to_string(),
            ":: concerto_reflect"
        );
        assert_eq!(
            reflect_root(&["concerto-reflect", "concerto"]).to_string(),
            ":: concerto_reflect"
        );
    }
}

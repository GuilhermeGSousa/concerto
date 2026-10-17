use proc_macro::TokenStream;
use proc_macro_crate::{FoundCrate, crate_name};
use quote::{quote, quote_spanned};
use syn::{Data, DataStruct, DeriveInput, Fields, spanned::Spanned};

fn editable_root(dependencies: &[&str]) -> proc_macro2::TokenStream {
    if dependencies.contains(&"concerto-editable") {
        quote!(::concerto_editable)
    } else if dependencies.contains(&"concerto") {
        quote!(::concerto::editable)
    } else {
        quote!(::concerto_editable)
    }
}

fn caller_editable_root() -> proc_macro2::TokenStream {
    let mut dependencies = Vec::new();
    match crate_name("concerto-editable") {
        Ok(FoundCrate::Name(name)) if name != "concerto_editable" => {
            let ident = proc_macro2::Ident::new(&name, proc_macro2::Span::call_site());
            return quote!(::#ident);
        }
        Ok(_) => dependencies.push("concerto-editable"),
        Err(_) => {}
    }
    match crate_name("concerto") {
        Ok(FoundCrate::Name(name)) if name != "concerto" => {
            let ident = proc_macro2::Ident::new(&name, proc_macro2::Span::call_site());
            if dependencies.is_empty() {
                return quote!(::#ident::editable);
            }
        }
        Ok(_) => dependencies.push("concerto"),
        Err(_) => {}
    }
    editable_root(&dependencies)
}

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

    let root = caller_editable_root();
    let (impl_generics, type_generics, where_clause) = ast.generics.split_for_impl();
    quote! {
        impl #impl_generics #root::Editable for #name #type_generics #where_clause {
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
    use super::editable_root;

    #[test]
    fn a_crate_depending_on_concerto_reaches_editable_through_it() {
        assert_eq!(
            editable_root(&["concerto"]).to_string(),
            ":: concerto :: editable"
        );
    }

    #[test]
    fn a_crate_depending_on_concerto_editable_uses_it_directly() {
        assert_eq!(
            editable_root(&["concerto-editable"]).to_string(),
            ":: concerto_editable"
        );
        assert_eq!(
            editable_root(&["concerto-editable", "concerto"]).to_string(),
            ":: concerto_editable"
        );
    }
}

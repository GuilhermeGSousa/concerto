use proc_macro::TokenStream;
use proc_macro_crate::{FoundCrate, crate_name};
use quote::{quote, quote_spanned};
use syn::{Data, DataEnum, DeriveInput, Fields, Member, spanned::Spanned};

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

#[proc_macro_derive(Reflect, attributes(reflect))]
pub fn derive_reflect(input: TokenStream) -> TokenStream {
    let ast = syn::parse_macro_input!(input as DeriveInput);
    let name = &ast.ident;
    let (opaque, declared) = match reflect_attributes(&ast) {
        Ok(found) => found,
        Err(error) => return error.to_compile_error().into(),
    };

    let root = caller_reflect_root();
    let (impl_generics, type_generics, where_clause) = ast.generics.split_for_impl();
    let reflect = if opaque {
        quote! {
            impl #impl_generics #root::Leaf for #name #type_generics #where_clause {
                fn encode(&self) -> ::std::result::Result<::std::vec::Vec<u8>, #root::LeafError> {
                    #root::encode_leaf(self)
                }

                fn decode(&mut self, bytes: &[u8]) -> ::std::result::Result<(), #root::LeafError> {
                    #root::decode_leaf(self, bytes)
                }
            }

            impl #impl_generics #root::Reflect for #name #type_generics #where_clause {
                fn reflect_ref(&self) -> #root::ReflectRef<'_> {
                    #root::ReflectRef::Leaf(self)
                }

                fn reflect_mut(&mut self) -> #root::ReflectMut<'_> {
                    #root::ReflectMut::Leaf(self)
                }
            }
        }
    } else {
        match &ast.data {
            Data::Struct(data) => {
                let fields = struct_fields(&root, &data.fields);
                quote! {
                    impl #impl_generics #root::Fields for #name #type_generics #where_clause {
                        #fields
                    }

                    impl #impl_generics #root::Reflect for #name #type_generics #where_clause {
                        fn reflect_ref(&self) -> #root::ReflectRef<'_> {
                            #root::ReflectRef::Struct(self)
                        }

                        fn reflect_mut(&mut self) -> #root::ReflectMut<'_> {
                            #root::ReflectMut::Struct(self)
                        }
                    }
                }
            }
            Data::Enum(data) => {
                let fields = enum_fields(&root, data);
                let names = data.variants.iter().map(|variant| {
                    let ident = &variant.ident;
                    let label = ident.to_string();
                    let pattern = ignore_pattern(&variant.fields);
                    quote!(Self::#ident #pattern => #label)
                });
                let labels = data
                    .variants
                    .iter()
                    .map(|variant| variant.ident.to_string());
                let builds = data.variants.iter().map(|variant| {
                    let ident = &variant.ident;
                    let label = ident.to_string();
                    let build = match &variant.fields {
                        Fields::Named(named) => {
                            let fields = named.named.iter().map(|field| {
                                let ident = field.ident.as_ref().expect("named field");
                                quote_spanned!(field.ty.span()=> #ident: ::std::default::Default::default())
                            });
                            quote!(Self::#ident { #(#fields),* })
                        }
                        Fields::Unnamed(unnamed) => {
                            let fields = unnamed.unnamed.iter().map(|field| {
                                quote_spanned!(field.ty.span()=> ::std::default::Default::default())
                            });
                            quote!(Self::#ident(#(#fields),*))
                        }
                        Fields::Unit => quote!(Self::#ident),
                    };
                    quote!(#label => *self = #build,)
                });
                quote! {
                    impl #impl_generics #root::Fields for #name #type_generics #where_clause {
                        #fields
                    }

                    impl #impl_generics #root::Enum for #name #type_generics #where_clause {
                        fn variant_name(&self) -> &'static str {
                            match self {
                                #(#names,)*
                            }
                        }

                        fn variant_names(&self) -> &'static [&'static str] {
                            &[#(#labels),*]
                        }

                        fn set_variant(&mut self, name: &str) -> bool {
                            match name {
                                #(#builds)*
                                _ => return false,
                            }
                            true
                        }
                    }

                    impl #impl_generics #root::Reflect for #name #type_generics #where_clause {
                        fn reflect_ref(&self) -> #root::ReflectRef<'_> {
                            #root::ReflectRef::Enum(self)
                        }

                        fn reflect_mut(&mut self) -> #root::ReflectMut<'_> {
                            #root::ReflectMut::Enum(self)
                        }
                    }
                }
            }
            Data::Union(_) => {
                return syn::Error::new_spanned(
                    name,
                    "#[derive(Reflect)] does not support unions; add #[reflect(opaque)] to reflect it as a serde leaf",
                )
                .to_compile_error()
                .into();
            }
        }
    };

    let field_types: Vec<&syn::Type> = if opaque {
        Vec::new()
    } else {
        match &ast.data {
            Data::Struct(data) => data.fields.iter().map(|field| &field.ty).collect(),
            Data::Enum(data) => data
                .variants
                .iter()
                .flat_map(|variant| variant.fields.iter().map(|field| &field.ty))
                .collect(),
            Data::Union(_) => Vec::new(),
        }
    };
    let dependencies = field_types
        .iter()
        .map(|ty| quote_spanned!(ty.span()=> registry.register::<#ty>();));
    let data_types: Vec<_> = declared
        .iter()
        .map(|declared| syn::Ident::new(&format!("Reflect{declared}"), declared.span()))
        .collect();
    let data = data_types.iter().map(|data| {
        quote_spanned!(data.span()=> registration.insert(<#data as #root::FromType<Self>>::from_type());)
    });
    let mut registration_generics = ast.generics.clone();
    let registration_where = registration_generics.make_where_clause();
    for data in &data_types {
        registration_where.predicates.push(
            syn::parse_quote_spanned!(data.span()=> #data: #root::FromType<#name #type_generics>),
        );
    }
    let type_params: Vec<_> = ast
        .generics
        .type_params()
        .map(|param| param.ident.clone())
        .collect();
    for ty in field_types
        .iter()
        .filter(|ty| mentions_any(&quote!(#ty), &type_params))
    {
        registration_where
            .predicates
            .push(syn::parse_quote_spanned!(ty.span()=> #ty: #root::GetTypeRegistration));
    }
    let registration_where = &registration_generics.where_clause;
    quote! {
        #reflect

        impl #impl_generics #root::GetTypeRegistration for #name #type_generics #registration_where {
            fn get_type_registration() -> #root::TypeRegistration {
                let mut registration = #root::TypeRegistration::of::<Self>();
                #(#data)*
                registration
            }

            fn register_dependencies(registry: &mut #root::TypeRegistry) {
                #(#dependencies)*
            }
        }
    }
    .into()
}

fn mentions_any(tokens: &proc_macro2::TokenStream, idents: &[syn::Ident]) -> bool {
    tokens.clone().into_iter().any(|token| match token {
        proc_macro2::TokenTree::Ident(ident) => idents.contains(&ident),
        proc_macro2::TokenTree::Group(group) => mentions_any(&group.stream(), idents),
        _ => false,
    })
}

fn field_labels(fields: &Fields) -> Vec<(String, Member, proc_macro2::Span)> {
    fields
        .members()
        .zip(fields.iter())
        .map(|(member, field)| {
            let label = match &member {
                Member::Named(ident) => ident.to_string(),
                Member::Unnamed(index) => index.index.to_string(),
            };
            (label, member, field.ty.span())
        })
        .collect()
}

fn struct_fields(root: &proc_macro2::TokenStream, fields: &Fields) -> proc_macro2::TokenStream {
    let labels = field_labels(fields);
    let len = labels.len();
    let names = labels
        .iter()
        .enumerate()
        .map(|(index, (label, _, _))| quote!(#index => ::std::option::Option::Some(#label)));
    let refs = labels.iter().enumerate().map(|(index, (_, member, span))| {
        quote_spanned!(*span=> #index => ::std::option::Option::Some(&self.#member as &dyn #root::Reflect))
    });
    let muts = labels.iter().enumerate().map(|(index, (_, member, span))| {
        quote_spanned!(*span=> #index => ::std::option::Option::Some(&mut self.#member as &mut dyn #root::Reflect))
    });
    quote! {
        fn field_len(&self) -> usize {
            #len
        }

        fn name_at(&self, index: usize) -> ::std::option::Option<&'static str> {
            match index {
                #(#names,)*
                _ => ::std::option::Option::None,
            }
        }

        fn field_at(&self, index: usize) -> ::std::option::Option<&dyn #root::Reflect> {
            match index {
                #(#refs,)*
                _ => ::std::option::Option::None,
            }
        }

        fn field_at_mut(&mut self, index: usize) -> ::std::option::Option<&mut dyn #root::Reflect> {
            match index {
                #(#muts,)*
                _ => ::std::option::Option::None,
            }
        }
    }
}

fn ignore_pattern(fields: &Fields) -> proc_macro2::TokenStream {
    match fields {
        Fields::Named(_) => quote!({ .. }),
        Fields::Unnamed(_) => quote!((..)),
        Fields::Unit => quote!(),
    }
}

fn binding(index: usize) -> syn::Ident {
    syn::Ident::new(&format!("field_{index}"), proc_macro2::Span::call_site())
}

fn bind_pattern(fields: &Fields) -> proc_macro2::TokenStream {
    let bindings = (0..fields.len()).map(binding);
    match fields {
        Fields::Named(named) => {
            let pairs = named.named.iter().zip(bindings).map(|(field, binding)| {
                let ident = field.ident.as_ref().expect("named field");
                quote!(#ident: #binding)
            });
            quote!({ #(#pairs),* })
        }
        Fields::Unnamed(_) => quote!((#(#bindings),*)),
        Fields::Unit => quote!(),
    }
}

fn enum_fields(root: &proc_macro2::TokenStream, data: &DataEnum) -> proc_macro2::TokenStream {
    let lens = data.variants.iter().map(|variant| {
        let ident = &variant.ident;
        let pattern = ignore_pattern(&variant.fields);
        let len = variant.fields.len();
        quote!(Self::#ident #pattern => #len)
    });
    let names = data.variants.iter().map(|variant| {
        let ident = &variant.ident;
        let pattern = ignore_pattern(&variant.fields);
        let arms = field_labels(&variant.fields)
            .into_iter()
            .enumerate()
            .map(|(index, (label, _, _))| quote!(#index => ::std::option::Option::Some(#label)));
        quote!(Self::#ident #pattern => match index { #(#arms,)* _ => ::std::option::Option::None })
    });
    let access = |mutable: bool| {
        data.variants
            .iter()
            .map(|variant| {
                let ident = &variant.ident;
                let pattern = bind_pattern(&variant.fields);
                let arms = variant.fields.iter().enumerate().map(|(index, field)| {
                    let binding = binding(index);
                    if mutable {
                        quote_spanned!(field.ty.span()=> #index => ::std::option::Option::Some(#binding as &mut dyn #root::Reflect))
                    } else {
                        quote_spanned!(field.ty.span()=> #index => ::std::option::Option::Some(#binding as &dyn #root::Reflect))
                    }
                });
                quote! {
                    #[allow(unused_variables)]
                    Self::#ident #pattern => match index { #(#arms,)* _ => ::std::option::Option::None }
                }
            })
            .collect::<Vec<_>>()
    };
    let refs = access(false);
    let muts = access(true);
    quote! {
        fn field_len(&self) -> usize {
            match self {
                #(#lens,)*
            }
        }

        fn name_at(&self, index: usize) -> ::std::option::Option<&'static str> {
            match self {
                #(#names,)*
            }
        }

        fn field_at(&self, index: usize) -> ::std::option::Option<&dyn #root::Reflect> {
            match self {
                #(#refs,)*
            }
        }

        fn field_at_mut(&mut self, index: usize) -> ::std::option::Option<&mut dyn #root::Reflect> {
            match self {
                #(#muts,)*
            }
        }
    }
}

fn reflect_attributes(ast: &DeriveInput) -> syn::Result<(bool, Vec<syn::Ident>)> {
    let mut opaque = false;
    let mut declared = Vec::new();
    for attribute in ast
        .attrs
        .iter()
        .filter(|attribute| attribute.path().is_ident("reflect"))
    {
        attribute.parse_nested_meta(|meta| {
            let ident = meta.path.require_ident()?.clone();
            if ident == "opaque" {
                opaque = true;
            } else {
                declared.push(ident);
            }
            Ok(())
        })?;
    }
    Ok((opaque, declared))
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

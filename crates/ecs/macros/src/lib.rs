//! Derive macros for `concerto-ecs`; use them through its re-exports.

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

/// Makes a trait queryable with `All<&dyn Trait>`.
///
/// Emits the trait unchanged, plus the glue that lets any component implementing it be
/// registered with `World::register_component_as::<dyn Trait, Component>()`. The trait must
/// be dyn-compatible (object safe) and may not have lifetime parameters.
///
/// ```ignore
/// #[queryable]
/// trait Interactable {
///     fn prompt(&self) -> &str;
/// }
/// ```
#[proc_macro_attribute]
pub fn queryable(attr: TokenStream, item: TokenStream) -> TokenStream {
    if !attr.is_empty() {
        return syn::Error::new(
            proc_macro2::TokenStream::from(attr)
                .into_iter()
                .next()
                .unwrap()
                .span(),
            "`#[queryable]` takes no arguments",
        )
        .to_compile_error()
        .into();
    }

    let item_trait = syn::parse_macro_input!(item as syn::ItemTrait);
    impl_queryable(&item_trait)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

fn impl_queryable(item_trait: &syn::ItemTrait) -> syn::Result<proc_macro2::TokenStream> {
    if let Some(lifetime) = item_trait.generics.lifetimes().next() {
        return Err(syn::Error::new_spanned(
            lifetime,
            "a `#[queryable]` trait cannot have lifetime parameters: its trait object must be 'static",
        ));
    }

    let name = &item_trait.ident;
    let trait_query = quote!(::concerto_ecs::query::trait_query);

    // `dyn Trait<..>` must be 'static to have a `TypeId`, so every type parameter must be too.
    let mut generics = item_trait.generics.clone();
    let static_bounds: Vec<syn::WherePredicate> = generics
        .type_params()
        .map(|param| {
            let ident = &param.ident;
            syn::parse_quote!(#ident: 'static)
        })
        .collect();
    generics
        .make_where_clause()
        .predicates
        .extend(static_bounds);

    let (_, type_generics, where_clause) = generics.split_for_impl();
    let dyn_trait = quote!(dyn #name #type_generics);

    let mut cast_generics = generics.clone();
    cast_generics
        .params
        .push(syn::parse_quote!(__QueryableComponent));
    cast_generics
        .make_where_clause()
        .predicates
        .push(syn::parse_quote!(
            __QueryableComponent: #name #type_generics + ::concerto_ecs::Component
        ));
    let (cast_impl_generics, _, cast_where_clause) = cast_generics.split_for_impl();

    // `QueryableTrait` covers every object lifetime, so it gets an extra lifetime parameter.
    let mut lifetime_generics = generics.clone();
    lifetime_generics
        .params
        .insert(0, syn::parse_quote!('__queryable));
    let (lifetime_impl_generics, _, _) = lifetime_generics.split_for_impl();

    Ok(quote! {
        #item_trait

        // SAFETY: `Static` is this trait object at `'static`, and `from_static` only
        // shortens the object lifetime.
        unsafe impl #lifetime_impl_generics #trait_query::QueryableTrait
            for #dyn_trait + '__queryable #where_clause
        {
            type Static = #dyn_trait;

            fn from_static(ptr: *const Self::Static) -> *const Self {
                ptr
            }
        }

        impl #cast_impl_generics #trait_query::ImplementedBy<__QueryableComponent>
            for #dyn_trait #cast_where_clause
        {
            fn cast(component: &__QueryableComponent) -> &Self::Static {
                component
            }
        }
    })
}

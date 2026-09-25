//! Derive macros for `concerto-ecs`.
//!
//! Use them through their re-exports in `concerto_ecs`, which also bring the
//! matching trait into scope: every derive names its trait unqualified. The
//! examples here are not compiled, since this crate cannot depend on
//! `concerto_ecs`.
//!
//! # Examples
//!
//! ```ignore
//! use concerto_ecs::{Component, Resource};
//!
//! #[derive(Component)]
//! struct Health(u32);
//!
//! #[derive(Resource)]
//! struct Score(u64);
//! ```

extern crate proc_macro;
extern crate syn;

#[macro_use]
extern crate quote;

use proc_macro::TokenStream;

/// Implements `Component`, so the type can be attached to entities.
///
/// # Examples
///
/// ```ignore
/// use concerto_ecs::{Component, World};
///
/// #[derive(Component)]
/// struct Health(u32);
///
/// let mut world = World::new();
/// world.spawn(Health(100));
/// ```
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

/// Implements `Resource`, so the type can be stored once per world.
///
/// # Examples
///
/// ```ignore
/// use concerto_ecs::{Resource, World};
///
/// #[derive(Resource)]
/// struct Score(u64);
///
/// let mut world = World::new();
/// world.insert_resource(Score(0));
/// ```
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

/// Implements `Event`, so the type can be sent between systems.
///
/// # Examples
///
/// ```ignore
/// use concerto_ecs::{Event, events::event_writer::EventWriter};
///
/// #[derive(Event)]
/// struct Jumped;
///
/// fn jump(mut jumped: EventWriter<Jumped>) {
///     jumped.write(Jumped);
/// }
/// ```
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

/// Implements `SystemSet`, so the type can name a group of systems.
///
/// The type must also implement `Clone`, `Eq`, `Hash` and `Debug`.
///
/// # Examples
///
/// ```ignore
/// use concerto_ecs::{IntoSystemConfig, Schedule, SystemSet};
///
/// #[derive(SystemSet, Clone, PartialEq, Eq, Hash, Debug)]
/// enum Frame {
///     Input,
///     Simulate,
/// }
///
/// fn integrate() {}
///
/// let mut schedule = Schedule::new();
/// schedule.add_system(integrate.in_set(Frame::Simulate));
/// ```
#[proc_macro_derive(SystemSet)]
pub fn system_set(input: TokenStream) -> TokenStream {
    let ast = syn::parse(input).unwrap();
    impl_label(&ast, quote!(SystemSet))
}

/// Implements `ScheduleLabel`, so the type can name a schedule.
///
/// The type must also implement `Clone`, `Eq`, `Hash` and `Debug`.
///
/// # Examples
///
/// ```ignore
/// use concerto_ecs::system::schedule::{ScheduleLabel, Schedules};
///
/// #[derive(ScheduleLabel, Clone, PartialEq, Eq, Hash, Debug)]
/// struct Update;
///
/// fn tick() {}
///
/// let mut schedules = Schedules::default();
/// schedules.add_system(Update, tick);
/// ```
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

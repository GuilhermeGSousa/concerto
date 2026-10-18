extern crate self as concerto_reflect;

mod leaves;
mod property;
mod registry;

use std::any::Any;

/// Implements [`Reflect`] for a struct with named fields by visiting each
/// field in declaration order. Every field must itself be `Reflect`.
///
/// ```compile_fail
/// #[derive(concerto_reflect::Reflect)]
/// struct Tuple(f32);
/// ```
///
/// ```compile_fail
/// #[derive(concerto_reflect::Reflect)]
/// enum Choice { A, B }
/// ```
///
/// ```compile_fail
/// struct Opaque;
///
/// #[derive(concerto_reflect::Reflect)]
/// struct HasOpaque { inner: Opaque }
/// ```
pub use concerto_reflect_macros::Reflect;
pub use property::{PathError, PropertyPath, with_property, with_property_mut};
pub use registry::{
    FromType, GetTypeRegistration, ReflectDefault, TypeData, TypeRegistration, TypeRegistry,
};

/// Structural access to a value. Opaque values use the default empty visitors;
/// named structs usually implement traversal through `#[derive(Reflect)]`.
/// Presentation, snapshots and validation belong to the caller.
pub trait Reflect: Any {
    fn visit(&self, _visitor: &mut dyn PropertyVisitor) {}

    fn visit_mut(&mut self, _visitor: &mut dyn PropertyVisitorMut) {}
}

pub trait PropertyVisitor {
    fn field(&mut self, name: &'static str, value: &dyn Reflect);
}

pub trait PropertyVisitorMut {
    fn field(&mut self, name: &'static str, value: &mut dyn Reflect);
}

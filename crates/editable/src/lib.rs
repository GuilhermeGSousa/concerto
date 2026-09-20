extern crate self as editable;

mod leaves;
mod property;

use std::any::Any;

/// Implements [`Editable`] for a struct with named fields by visiting each
/// field in declaration order. Every field must itself be `Editable`.
///
/// ```compile_fail
/// #[derive(editable::Editable)]
/// struct Tuple(f32);
/// ```
///
/// ```compile_fail
/// #[derive(editable::Editable)]
/// enum Choice { A, B }
/// ```
///
/// ```compile_fail
/// struct Opaque;
///
/// #[derive(editable::Editable)]
/// struct HasOpaque { inner: Opaque }
/// ```
pub use editable_macros::Editable;
pub use property::{PathError, PropertyPath, with_property, with_property_mut};

/// Structural access to a value. Opaque values use the default empty visitors;
/// named structs usually implement traversal through `#[derive(Editable)]`.
/// Presentation, snapshots and validation belong to the caller.
pub trait Editable: Any {
    fn visit(&self, _visitor: &mut dyn PropertyVisitor) {}

    fn visit_mut(&mut self, _visitor: &mut dyn PropertyVisitorMut) {}
}

pub trait PropertyVisitor {
    fn field(&mut self, name: &'static str, value: &dyn Editable);
}

pub trait PropertyVisitorMut {
    fn field(&mut self, name: &'static str, value: &mut dyn Editable);
}

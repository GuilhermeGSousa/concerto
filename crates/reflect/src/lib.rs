extern crate self as concerto_reflect;

mod json;
mod kinds;
mod leaves;
mod property;
mod registry;
mod value;

use std::any::Any;

/// Implements [`Reflect`] for a struct or an enum, whose fields must all be `Reflect`;
/// `#[reflect(opaque)]` makes the type a serde [`Leaf`] instead.
///
/// ```compile_fail
/// struct Opaque;
///
/// #[derive(concerto_reflect::Reflect)]
/// struct HasOpaque { inner: Opaque }
/// ```
pub use concerto_reflect_macros::Reflect;
pub use json::{JsonLeaf, ReflectJson, json_to_value, json_to_value_with, to_json};
pub use kinds::{
    Enum, Fields, Leaf, LeafError, List, ReflectMut, ReflectRef, decode_leaf, encode_leaf,
};
pub use property::{PathError, PropertyPath, with_property, with_property_mut};
pub use registry::{
    FromType, GetTypeRegistration, ReflectDefault, ReflectEnumInfo, ReflectOption, TypeData,
    TypeRegistration, TypeRegistry,
};
pub use value::{
    EncodeError, LeafHook, ReflectedValue, ValueWarning, apply_value, apply_value_with, to_value,
    to_value_with,
};

/// Structural access to a value: what kind it is, and its parts.
/// Presentation, snapshots and validation belong to the caller.
pub trait Reflect: Any + Send + Sync {
    fn reflect_ref(&self) -> ReflectRef<'_>;

    fn reflect_mut(&mut self) -> ReflectMut<'_>;

    /// The concrete type's name, as [`std::any::type_name`] writes it.
    fn type_name(&self) -> &'static str {
        std::any::type_name::<Self>()
    }

    /// Visits a struct's fields, or an enum's active variant's fields; lists and leaves have none.
    fn visit(&self, visitor: &mut dyn PropertyVisitor) {
        let fields: &dyn Fields = match self.reflect_ref() {
            ReflectRef::Struct(fields) => fields,
            ReflectRef::Enum(variant) => variant,
            ReflectRef::List(_) | ReflectRef::Leaf(_) => return,
        };
        for index in 0..fields.field_len() {
            if let (Some(name), Some(value)) = (fields.name_at(index), fields.field_at(index)) {
                visitor.field(name, value);
            }
        }
    }

    fn visit_mut(&mut self, visitor: &mut dyn PropertyVisitorMut) {
        let fields: &mut dyn Fields = match self.reflect_mut() {
            ReflectMut::Struct(fields) => fields,
            ReflectMut::Enum(variant) => variant,
            ReflectMut::List(_) | ReflectMut::Leaf(_) => return,
        };
        for index in 0..fields.field_len() {
            let Some(name) = fields.name_at(index) else {
                continue;
            };
            if let Some(value) = fields.field_at_mut(index) {
                visitor.field(name, value);
            }
        }
    }
}

pub trait PropertyVisitor {
    fn field(&mut self, name: &'static str, value: &dyn Reflect);
}

pub trait PropertyVisitorMut {
    fn field(&mut self, name: &'static str, value: &mut dyn Reflect);
}

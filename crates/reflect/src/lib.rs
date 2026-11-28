//! Runtime reflection: reading and writing a value's fields by name, storing it as a
//! self-describing [`ReflectedValue`], and looking types up by name in a [`TypeRegistry`].
//!
//! Derive [`Reflect`] on a type to make it inspectable in the editor, saveable in scenes and
//! assets, and addressable by a [`PropertyPath`].

extern crate self as concerto_reflect;

mod json;
mod kinds;
mod leaves;
mod property;
mod registry;
mod value;

use std::any::Any;

/// Implements [`Reflect`] for a struct or an enum, whose fields must all be `Reflect`.
///
/// `#[reflect(opaque)]` makes the type a serde [`Leaf`] instead, and `#[reflect(skip)]` on a
/// struct field leaves that field out, so nothing reflective reads, writes or stores it.
///
/// ```compile_fail
/// struct Opaque;
///
/// #[derive(concerto_reflect::Reflect)]
/// struct HasOpaque { inner: Opaque }
/// ```
pub use concerto_reflect_macros::Reflect;
pub use json::{JsonLeaf, ReflectJson, to_json};
pub use kinds::{
    Enum, Fields, Leaf, LeafError, List, ReflectMut, ReflectRef, decode_leaf, encode_leaf,
};
pub use property::{PathError, PathSegment, PropertyPath};
pub use registry::{
    FromType, GetTypeRegistration, ReflectDefault, ReflectEnumInfo, ReflectList, ReflectOption,
    TypeData, TypeRegistration, TypeRegistry,
};
pub use value::{EncodeError, LeafHook, ReflectedValue, ValueWarning};

/// Structural access to a value: what kind it is, and its parts.
///
/// Presentation, snapshots and validation belong to the caller.
pub trait Reflect: Any + Send + Sync {
    /// Returns the value's kind and its parts, for reading.
    fn reflect_ref(&self) -> ReflectRef<'_>;

    /// Returns the value's kind and its parts, for writing.
    fn reflect_mut(&mut self) -> ReflectMut<'_>;

    /// The concrete type's name, as [`std::any::type_name`] writes it.
    fn type_name(&self) -> &'static str {
        std::any::type_name::<Self>()
    }

    /// Visits a struct's fields, an enum's active variant's fields, or a list's elements; leaves have none.
    fn visit(&self, visitor: &mut dyn PropertyVisitor) {
        let fields: &dyn Fields = match self.reflect_ref() {
            ReflectRef::Struct(fields) => fields,
            ReflectRef::Enum(variant) => variant,
            ReflectRef::List(list) => {
                for index in 0..list.len() {
                    if let Some(value) = list.get(index) {
                        visitor.element(index, value);
                    }
                }
                return;
            }
            ReflectRef::Leaf(_) => return,
        };
        for index in 0..fields.field_len() {
            if let (Some(name), Some(value)) = (fields.name_at(index), fields.field_at(index)) {
                visitor.field(name, value);
            }
        }
    }

    /// Visits a struct's fields, an enum's active variant's fields, or a list's elements, mutably;
    /// leaves have none.
    fn visit_mut(&mut self, visitor: &mut dyn PropertyVisitorMut) {
        let fields: &mut dyn Fields = match self.reflect_mut() {
            ReflectMut::Struct(fields) => fields,
            ReflectMut::Enum(variant) => variant,
            ReflectMut::List(list) => {
                for index in 0..list.len() {
                    if let Some(value) = list.get_mut(index) {
                        visitor.element(index, value);
                    }
                }
                return;
            }
            ReflectMut::Leaf(_) => return,
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

impl dyn Reflect {
    /// The value as `T`, or `None` if it is another type.
    pub fn downcast_ref<T: Reflect>(&self) -> Option<&T> {
        (self as &dyn Any).downcast_ref::<T>()
    }

    /// The value as `T`, mutably, or `None` if it is another type.
    pub fn downcast_mut<T: Reflect>(&mut self) -> Option<&mut T> {
        (self as &mut dyn Any).downcast_mut::<T>()
    }
}

/// Receives each field or list element that [`Reflect::visit`] walks.
pub trait PropertyVisitor {
    /// Receives the field `name` and its value.
    fn field(&mut self, name: &'static str, value: &dyn Reflect);

    /// Receives a list's element; ignored by default.
    fn element(&mut self, _index: usize, _value: &dyn Reflect) {}
}

/// Receives each field or list element that [`Reflect::visit_mut`] walks, mutably.
pub trait PropertyVisitorMut {
    /// Receives the field `name` and its value.
    fn field(&mut self, name: &'static str, value: &mut dyn Reflect);

    /// Receives a list's element; ignored by default.
    fn element(&mut self, _index: usize, _value: &mut dyn Reflect) {}
}

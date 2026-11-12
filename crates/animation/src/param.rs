use std::{any::TypeId, fmt, marker::PhantomData};

use concerto_name::Name;
use concerto_reflect::{FromType, JsonLeaf, Reflect, ReflectDefault, ReflectJson, TypeData};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The name of a blackboard parameter a node reads as a `T`.
#[derive(Reflect, Serialize, Deserialize)]
#[serde(transparent)]
#[reflect(opaque, Default, Json, Param)]
pub struct Param<T: Reflect>(pub Name, PhantomData<fn() -> T>);

impl<T: Reflect> Param<T> {
    /// The parameter named `name`.
    pub fn new(name: Name) -> Self {
        Self(name, PhantomData)
    }
}

impl<T: Reflect> From<Name> for Param<T> {
    fn from(name: Name) -> Self {
        Self::new(name)
    }
}

impl<T: Reflect> Clone for Param<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: Reflect> Copy for Param<T> {}

impl<T: Reflect> Default for Param<T> {
    fn default() -> Self {
        Self::new(Name::default())
    }
}

impl<T: Reflect> PartialEq for Param<T> {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl<T: Reflect> fmt::Debug for Param<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.0, f)
    }
}

impl<T: Reflect> JsonLeaf for Param<T> {
    fn to_json(&self) -> Value {
        self.0.to_json()
    }

    fn from_json(value: &Value) -> Result<Self, String> {
        Name::from_json(value).map(Self::new)
    }
}

/// Reads the name a reflected [`Param`] holds, and names and builds the value it names.
#[derive(Clone, Copy)]
pub struct ReflectParam {
    name: fn(&dyn Reflect) -> Option<Name>,
    value_type: TypeId,
    value_type_name: &'static str,
    default_value: fn() -> Box<dyn Reflect>,
}

impl ReflectParam {
    /// The parameter's name, or `None` when `value` is not the `Param` this was built for.
    pub fn name(&self, value: &dyn Reflect) -> Option<Name> {
        (self.name)(value)
    }

    /// The type of the value the parameter names on the blackboard.
    pub fn value_type(&self) -> TypeId {
        self.value_type
    }

    /// That type's name, as [`std::any::type_name`] writes it.
    pub fn value_type_name(&self) -> &'static str {
        self.value_type_name
    }

    /// A `Default` value of that type, for an editor to edit and put on a blackboard.
    pub fn default_value(&self) -> Box<dyn Reflect> {
        (self.default_value)()
    }
}

impl TypeData for ReflectParam {}

impl<T: Reflect + Default> FromType<Param<T>> for ReflectParam {
    fn from_type() -> Self {
        Self {
            name: |value| value.downcast_ref::<Param<T>>().map(|param| param.0),
            value_type: TypeId::of::<T>(),
            value_type_name: std::any::type_name::<T>(),
            default_value: || Box::new(T::default()),
        }
    }
}

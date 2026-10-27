use std::{
    any::{Any, TypeId},
    collections::HashMap,
};

use crate::Reflect;

/// Data about a type that is stored on its registration, keyed by its own type.
pub trait TypeData: Send + Sync + 'static {}

/// Builds `Self` for the concrete type `T`.
pub trait FromType<T> {
    fn from_type() -> Self;
}

/// Implemented by the derive; produces `T`'s registration with its declared type data.
pub trait GetTypeRegistration: Reflect {
    fn get_type_registration() -> TypeRegistration;

    /// Registers the types this one holds.
    fn register_dependencies(_registry: &mut TypeRegistry) {}
}

/// One registered type: its names and the type data it was declared to support.
pub struct TypeRegistration {
    type_id: TypeId,
    type_path: &'static str,
    short_name: &'static str,
    data: HashMap<TypeId, Box<dyn Any + Send + Sync>>,
}

impl TypeRegistration {
    /// A registration for `T` with no type data yet.
    pub fn of<T: Reflect>() -> Self {
        let type_path = std::any::type_name::<T>();
        Self {
            type_id: TypeId::of::<T>(),
            type_path,
            short_name: short_name(type_path),
            data: HashMap::new(),
        }
    }

    pub fn type_id(&self) -> TypeId {
        self.type_id
    }

    /// The canonical full path, e.g. `concerto_foundation::transform::Transform`.
    pub fn type_path(&self) -> &'static str {
        self.type_path
    }

    /// The final `::` segment, e.g. `Transform`.
    pub fn short_name(&self) -> &'static str {
        self.short_name
    }

    pub fn data<D: TypeData>(&self) -> Option<&D> {
        self.data
            .get(&TypeId::of::<D>())
            .and_then(|data| data.downcast_ref::<D>())
    }

    pub fn insert<D: TypeData>(&mut self, data: D) {
        self.data.insert(TypeId::of::<D>(), Box::new(data));
    }
}

fn short_name(type_path: &'static str) -> &'static str {
    let without_generics = type_path.split('<').next().unwrap_or(type_path);
    let start = without_generics.rfind("::").map_or(0, |index| index + 2);
    &type_path[start..]
}

/// Every registered type, by `TypeId`, with what it was declared to support.
#[derive(Default)]
pub struct TypeRegistry {
    registrations: HashMap<TypeId, TypeRegistration>,
    by_path: HashMap<&'static str, TypeId>,
    by_short_name: HashMap<&'static str, TypeId>,
}

impl TypeRegistry {
    /// Adds `T`, every type data its derive declared, and the types it holds; registering twice changes nothing.
    pub fn register<T: GetTypeRegistration>(&mut self) {
        if self.registrations.contains_key(&TypeId::of::<T>()) {
            return;
        }
        self.add(T::get_type_registration());
        T::register_dependencies(self);
    }

    /// Adds a registration built by hand; one for a type already registered is ignored.
    pub fn add(&mut self, registration: TypeRegistration) {
        let type_id = registration.type_id;
        if self.registrations.contains_key(&type_id) {
            return;
        }
        self.by_path.insert(registration.type_path, type_id);
        if registration.short_name != registration.type_path {
            if let Some(previous) = self.by_short_name.get(registration.short_name)
                && let Some(previous) = self.registrations.get(previous)
            {
                log::warn!(
                    "Two types share the short name '{}': {} and {}; the short name now resolves to {}. Use the full path to name the other.",
                    registration.short_name,
                    previous.type_path,
                    registration.type_path,
                    registration.type_path
                );
            }
            self.by_short_name.insert(registration.short_name, type_id);
        }
        self.registrations.insert(type_id, registration);
    }

    pub fn get(&self, id: TypeId) -> Option<&TypeRegistration> {
        self.registrations.get(&id)
    }

    /// Looks a type up by full path, or by short name when no full path matches.
    pub fn get_by_name(&self, name: &str) -> Option<&TypeRegistration> {
        self.by_path
            .get(name)
            .or_else(|| self.by_short_name.get(name))
            .and_then(|id| self.registrations.get(id))
    }

    /// The type data `D` of the type `id`, if it was declared.
    pub fn get_type_data<D: TypeData>(&self, id: TypeId) -> Option<&D> {
        self.get(id).and_then(TypeRegistration::data)
    }

    pub fn iter(&self) -> impl Iterator<Item = &TypeRegistration> {
        self.registrations.values()
    }
}

/// Builds a `Default` value of the type.
pub struct ReflectDefault {
    default: fn() -> Box<dyn Reflect>,
}

impl ReflectDefault {
    pub fn default(&self) -> Box<dyn Reflect> {
        (self.default)()
    }
}

impl TypeData for ReflectDefault {}

impl<T: Reflect + Default> FromType<T> for ReflectDefault {
    fn from_type() -> Self {
        Self {
            default: || Box::new(T::default()),
        }
    }
}

/// An enum's variant names, in declaration order; the derive registers it for every non-opaque enum.
pub struct ReflectEnumInfo {
    pub variants: &'static [&'static str],
}

impl TypeData for ReflectEnumInfo {}

/// Marks `Option<T>`, which also carries a [`ReflectEnumInfo`] of `None` and `Some`.
pub struct ReflectOption;

impl TypeData for ReflectOption {}

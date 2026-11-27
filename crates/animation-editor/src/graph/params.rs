use std::any::{Any, TypeId};

use concerto_animation::param::ReflectParam;
use concerto_name::Name;
use concerto_reflect::{
    Fields, GetTypeRegistration, PropertyVisitor, Reflect, ReflectMut, ReflectRef,
    TypeRegistration, TypeRegistry,
};

/// A parameter a node names: its name and what its `Param<T>` says of the value.
#[derive(Clone, Copy)]
pub struct FoundParam {
    /// The parameter's name.
    pub name: Name,
    /// The `Param<T>`'s type data, which names and builds a value of `T`.
    pub data: ReflectParam,
}

struct Finder<'a> {
    types: &'a TypeRegistry,
    found: &'a mut Vec<FoundParam>,
}

impl Finder<'_> {
    fn value(&mut self, value: &dyn Reflect) {
        let Some(data) = self
            .types
            .get_type_data::<ReflectParam>((value as &dyn Any).type_id())
        else {
            value.visit(self);
            return;
        };
        let Some(name) = data.name(value).filter(|name| !name.as_str().is_empty()) else {
            return;
        };
        let known = self
            .found
            .iter()
            .any(|found| found.name == name && found.data.value_type() == data.value_type());
        if !known {
            self.found.push(FoundParam { name, data: *data });
        }
    }
}

impl PropertyVisitor for Finder<'_> {
    fn field(&mut self, _name: &'static str, value: &dyn Reflect) {
        self.value(value);
    }

    fn element(&mut self, _index: usize, value: &dyn Reflect) {
        self.value(value);
    }
}

/// Adds to `found` every named `Param<T>` inside `value`, nested values, lists, options and enum variants included.
pub fn find_params(value: &dyn Reflect, types: &TypeRegistry, found: &mut Vec<FoundParam>) {
    Finder { types, found }.value(value);
}

/// The last segment of a type's name, as the Parameters section shows it.
pub fn short_type(type_name: &str) -> &str {
    let name = type_name.split('<').next().unwrap_or(type_name);
    name.rsplit("::").next().unwrap_or(name)
}

/// One parameter of a graph tab and the value its preview plays it at.
pub struct GraphParam {
    /// The parameter's name.
    pub name: Name,
    /// The type name of the parameter's value.
    pub type_name: &'static str,
    /// Whether another parameter in view has this name and another type.
    pub shared: bool,
    value_type: TypeId,
    label: Name,
    shown: bool,
    value: Box<dyn Reflect>,
}

impl GraphParam {
    /// The value the preview plays the parameter at.
    pub fn value(&self) -> &dyn Reflect {
        self.value.as_ref()
    }

    /// The field of the `Parameters` value that edits it: its name, with its type when the name is shared.
    pub fn label(&self) -> &'static str {
        self.label.as_str()
    }

    /// The type of the value, as its `Param<T>` names it.
    pub fn value_type(&self) -> TypeId {
        self.value_type
    }

    /// The last segment of its value's type name.
    pub fn short_type(&self) -> &str {
        short_type(self.type_name)
    }

    fn is(&self, found: &FoundParam) -> bool {
        self.name == found.name && self.value_type == found.data.value_type()
    }
}

/// The parameters the graph in view names, the document value `Parameters`: one field per parameter, kept for the tab's life and never saved.
#[derive(Default)]
pub struct GraphParameters {
    params: Vec<GraphParam>,
}

impl GraphParameters {
    /// The parameters the graph in view names, by name and then by type.
    pub fn shown(&self) -> impl Iterator<Item = &GraphParam> {
        self.params.iter().filter(|param| param.shown)
    }

    /// The parameter `found` names, in view or seen earlier.
    pub fn held(&self, found: &FoundParam) -> Option<&GraphParam> {
        self.params.iter().find(|param| param.is(found))
    }

    /// Whether the parameters in view are exactly `found`.
    pub fn shows(&self, found: &[FoundParam]) -> bool {
        self.shown().count() == found.len()
            && found
                .iter()
                .all(|found| self.shown().any(|param| param.is(found)))
    }

    /// Puts `found` in view, each at the value it had when last in view or at its type's `Default`.
    pub fn show(&mut self, found: &[FoundParam]) {
        for param in &mut self.params {
            param.shown = false;
        }
        for found in found {
            if let Some(param) = self.params.iter_mut().find(|param| param.is(found)) {
                param.shown = true;
                continue;
            }
            self.params.push(GraphParam {
                name: found.name,
                type_name: found.data.value_type_name(),
                shared: false,
                value_type: found.data.value_type(),
                label: found.name,
                shown: true,
                value: found.data.default_value(),
            });
        }
        self.params.sort_by(|a, b| {
            a.name
                .as_str()
                .cmp(b.name.as_str())
                .then_with(|| a.type_name.cmp(b.type_name))
        });
        let names: Vec<Name> = self.shown().map(|param| param.name).collect();
        for param in self.params.iter_mut().filter(|param| param.shown) {
            param.shared = names.iter().filter(|name| **name == param.name).count() > 1;
            param.label = if param.shared {
                Name::new(&format!("{} ({})", param.name, short_type(param.type_name)))
            } else {
                param.name
            };
        }
    }

    fn at(&self, index: usize) -> Option<&GraphParam> {
        self.shown().nth(index)
    }
}

impl Fields for GraphParameters {
    fn field_len(&self) -> usize {
        self.shown().count()
    }

    fn name_at(&self, index: usize) -> Option<&'static str> {
        self.at(index).map(GraphParam::label)
    }

    fn field_at(&self, index: usize) -> Option<&dyn Reflect> {
        self.at(index).map(GraphParam::value)
    }

    fn field_at_mut(&mut self, index: usize) -> Option<&mut dyn Reflect> {
        self.params
            .iter_mut()
            .filter(|param| param.shown)
            .nth(index)
            .map(|param| param.value.as_mut())
    }
}

impl Reflect for GraphParameters {
    fn reflect_ref(&self) -> ReflectRef<'_> {
        ReflectRef::Struct(self)
    }

    fn reflect_mut(&mut self) -> ReflectMut<'_> {
        ReflectMut::Struct(self)
    }
}

impl GetTypeRegistration for GraphParameters {
    fn get_type_registration() -> TypeRegistration {
        TypeRegistration::of::<Self>()
    }
}

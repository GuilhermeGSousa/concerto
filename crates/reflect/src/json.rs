use std::any::{Any, TypeId};

use concerto_color::Color;
use glam::{Quat, Vec2, Vec3, Vec4};
use serde_json::{Map, Number, Value};

use crate::{
    Enum, Fields, FromType, LeafError, LeafHook, List, Reflect, ReflectDefault, ReflectMut,
    ReflectRef, ReflectedValue, TypeData, TypeRegistry,
    value::{NoHook, Path},
};

/// A leaf type's JSON form, from which its [`ReflectJson`] is built.
pub trait JsonLeaf: Reflect + Sized {
    fn to_json(&self) -> Value;
    /// Reads the value back, or says what shape was expected.
    fn from_json(value: &Value) -> Result<Self, String>;
}

/// Writes a leaf as JSON and reads it back.
#[derive(Clone, Copy)]
pub struct ReflectJson {
    pub to_json: fn(&dyn Reflect) -> Value,
    pub from_json: fn(&Value) -> Result<Box<dyn Reflect>, String>,
}

impl TypeData for ReflectJson {}

impl<T: JsonLeaf> FromType<T> for ReflectJson {
    fn from_type() -> Self {
        Self {
            to_json: |value| {
                (value as &dyn Any)
                    .downcast_ref::<T>()
                    .map_or(Value::Null, T::to_json)
            },
            from_json: |value| T::from_json(value).map(|value| Box::new(value) as Box<dyn Reflect>),
        }
    }
}

/// Writes `value` as JSON, each leaf through its [`ReflectJson`] or as `{ "opaque": "<type name>" }`.
pub fn to_json(value: &dyn Reflect, registry: &TypeRegistry) -> Value {
    match value.reflect_ref() {
        ReflectRef::Struct(fields) => fields_to_json(fields, registry),
        ReflectRef::Enum(variant) if variant.field_len() == 0 => {
            Value::String(variant.variant_name().to_owned())
        }
        ReflectRef::Enum(variant) => {
            let mut object = Map::new();
            object.insert(
                variant.variant_name().to_owned(),
                fields_to_json(variant, registry),
            );
            Value::Object(object)
        }
        ReflectRef::List(list) => Value::Array(
            (0..list.len())
                .filter_map(|index| list.get(index))
                .map(|element| to_json(element, registry))
                .collect(),
        ),
        ReflectRef::Leaf(_) => {
            match registry.get_type_data::<ReflectJson>((value as &dyn Any).type_id()) {
                Some(codec) => (codec.to_json)(value),
                None => {
                    let mut object = Map::new();
                    object.insert(
                        "opaque".to_owned(),
                        Value::String(value.type_name().to_owned()),
                    );
                    Value::Object(object)
                }
            }
        }
    }
}

fn fields_to_json(fields: &dyn Fields, registry: &TypeRegistry) -> Value {
    let mut object = Map::new();
    for index in 0..fields.field_len() {
        if let (Some(name), Some(field)) = (fields.name_at(index), fields.field_at(index)) {
            object.insert(name.to_owned(), to_json(field, registry));
        }
    }
    Value::Object(object)
}

/// Builds the stored value of the type `type_id` from JSON shaped as [`to_json`] writes it, leaving out fields it omits or marks opaque.
pub fn json_to_value(
    value: &Value,
    type_id: TypeId,
    registry: &TypeRegistry,
) -> Result<ReflectedValue, String> {
    json_to_value_with(value, type_id, registry, &mut NoHook)
}

/// [`json_to_value`], letting `hook` encode any leaf first.
pub fn json_to_value_with(
    value: &Value,
    type_id: TypeId,
    registry: &TypeRegistry,
    hook: &mut dyn LeafHook,
) -> Result<ReflectedValue, String> {
    let mut decoder = Decoder {
        registry,
        hook,
        path: Path::default(),
    };
    if let Some(codec) = registry.get_type_data::<ReflectJson>(type_id) {
        return decoder.leaf(codec, value);
    }
    let Some(registration) = registry.get(type_id) else {
        return Err("the type is not registered, so JSON cannot build it".to_owned());
    };
    let Some(default) = registration.data::<ReflectDefault>() else {
        return Err(format!(
            "{} has no JSON form and no registered Default, so JSON cannot build it",
            registration.type_path()
        ));
    };
    let mut scratch = default.default();
    decoder.value(value, scratch.as_mut())
}

struct Decoder<'a> {
    registry: &'a TypeRegistry,
    hook: &'a mut dyn LeafHook,
    path: Path,
}

impl Decoder<'_> {
    fn error(&self, reason: impl std::fmt::Display) -> String {
        let path = self.path.render();
        if path.is_empty() {
            reason.to_string()
        } else {
            format!("{path}: {reason}")
        }
    }

    fn codec(&self, value: &dyn Reflect) -> Option<&ReflectJson> {
        self.registry
            .get_type_data::<ReflectJson>((value as &dyn Any).type_id())
    }

    fn value(
        &mut self,
        value: &Value,
        scratch: &mut dyn Reflect,
    ) -> Result<ReflectedValue, String> {
        if matches!(scratch.reflect_ref(), ReflectRef::Leaf(_)) {
            let Some(codec) = self.codec(scratch).copied() else {
                return Err(self.error(format!(
                    "{} has no JSON form, so it cannot be written",
                    scratch.type_name()
                )));
            };
            return self.leaf(&codec, value);
        }
        match scratch.reflect_mut() {
            ReflectMut::Struct(fields) => self.fields(value, fields).map(ReflectedValue::Struct),
            ReflectMut::Enum(variant) => self.variant(value, variant),
            ReflectMut::List(list) => self.list(value, list).map(ReflectedValue::List),
            ReflectMut::Leaf(_) => unreachable!("checked not to be a leaf"),
        }
    }

    fn leaf(&mut self, codec: &ReflectJson, value: &Value) -> Result<ReflectedValue, String> {
        let decoded = (codec.from_json)(value).map_err(|reason| self.error(reason))?;
        let encoded = match decoded.reflect_ref() {
            ReflectRef::Leaf(leaf) => self
                .hook
                .encode(decoded.as_ref())
                .unwrap_or_else(|| leaf.encode()),
            _ => Err(LeafError("its JSON codec does not build a leaf".into())),
        };
        encoded
            .map(ReflectedValue::Leaf)
            .map_err(|error| self.error(error))
    }

    fn fields(
        &mut self,
        value: &Value,
        fields: &mut dyn Fields,
    ) -> Result<Vec<(String, ReflectedValue)>, String> {
        let Some(object) = value.as_object() else {
            return Err(self.error(format!(
                "expected an object with the fields {}",
                field_names(fields)
            )));
        };
        let mut stored = Vec::with_capacity(object.len());
        for (name, value) in object {
            self.path.field(name);
            let field = self.field(name, value, fields);
            self.path.pop();
            if let Some(field) = field? {
                stored.push((name.clone(), field));
            }
        }
        Ok(stored)
    }

    fn field(
        &mut self,
        name: &str,
        value: &Value,
        fields: &mut dyn Fields,
    ) -> Result<Option<ReflectedValue>, String> {
        let Some(index) = fields.index_of(name) else {
            return Err(self.error(format!(
                "no such field; the fields are {}",
                field_names(fields)
            )));
        };
        let Some(field) = fields.field_at_mut(index) else {
            return Err(self.error("the field cannot be written"));
        };
        if self.is_opaque_marker(value, field) {
            return Ok(None);
        }
        self.value(value, field).map(Some)
    }

    fn is_opaque_marker(&self, value: &Value, field: &dyn Reflect) -> bool {
        matches!(field.reflect_ref(), ReflectRef::Leaf(_))
            && self.codec(field).is_none()
            && value.as_object().is_some_and(|object| {
                object.len() == 1
                    && object.get("opaque").and_then(Value::as_str) == Some(field.type_name())
            })
    }

    fn variant(&mut self, value: &Value, variant: &mut dyn Enum) -> Result<ReflectedValue, String> {
        let named = match value {
            Value::String(name) => Some((name, None)),
            Value::Object(object) if object.len() == 1 => object
                .iter()
                .next()
                .map(|(name, fields)| (name, Some(fields))),
            _ => None,
        };
        let Some((name, fields)) = named else {
            return Err(
                self.error("expected a variant name, or an object with one key naming the variant")
            );
        };
        if variant.variant_name() != name && !variant.set_variant(name) {
            return Err(self.error(format!("no variant '{name}'")));
        }
        let fields = match fields {
            Some(fields) => self.fields(fields, variant)?,
            None => Vec::new(),
        };
        Ok(ReflectedValue::Variant {
            name: name.clone(),
            fields,
        })
    }

    fn list(&mut self, value: &Value, list: &mut dyn List) -> Result<Vec<ReflectedValue>, String> {
        let Some(elements) = value.as_array() else {
            return Err(self.error("expected an array"));
        };
        let mut stored = Vec::with_capacity(elements.len());
        for (index, element) in elements.iter().enumerate() {
            if list.len() <= index {
                list.push_default();
            }
            self.path.index(index);
            let decoded = match list.get_mut(index) {
                Some(scratch) => self.value(element, scratch),
                None => Err(self.error("the list cannot grow to hold this element")),
            };
            self.path.pop();
            stored.push(decoded?);
        }
        Ok(stored)
    }
}

fn field_names(fields: &dyn Fields) -> String {
    (0..fields.field_len())
        .filter_map(|index| fields.name_at(index))
        .collect::<Vec<_>>()
        .join(", ")
}

impl JsonLeaf for bool {
    fn to_json(&self) -> Value {
        Value::Bool(*self)
    }

    fn from_json(value: &Value) -> Result<Self, String> {
        value
            .as_bool()
            .ok_or_else(|| "expected true or false".to_owned())
    }
}

impl JsonLeaf for String {
    fn to_json(&self) -> Value {
        Value::String(self.clone())
    }

    fn from_json(value: &Value) -> Result<Self, String> {
        value
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| "expected a string".to_owned())
    }
}

macro_rules! json_integers {
    ($read:ident: $($integer:ty),* $(,)?) => {
        $(
            impl JsonLeaf for $integer {
                fn to_json(&self) -> Value {
                    Value::from(*self)
                }

                fn from_json(value: &Value) -> Result<Self, String> {
                    value
                        .$read()
                        .and_then(|value| <$integer>::try_from(value).ok())
                        .ok_or_else(|| {
                            format!(
                                "expected an integer from {} to {}",
                                <$integer>::MIN,
                                <$integer>::MAX
                            )
                        })
                }
            }
        )*
    };
}

json_integers!(as_u64: u8, u16, u32, u64, usize);
json_integers!(as_i64: i8, i16, i32, i64, isize);

const FLOAT_SHAPE: &str = "expected a number, or \"NaN\", \"inf\" or \"-inf\"";

fn float_to_json(shortest: String, finite: bool) -> Value {
    if finite {
        shortest
            .parse::<f64>()
            .ok()
            .and_then(Number::from_f64)
            .map_or(Value::Null, Value::Number)
    } else {
        Value::String(shortest)
    }
}

fn float_from_json<F: std::str::FromStr + Copy>(
    value: &Value,
    is_finite: fn(F) -> bool,
) -> Result<F, String> {
    let parsed = match value {
        Value::Number(number) => number
            .to_string()
            .parse::<F>()
            .ok()
            .filter(|&parsed| is_finite(parsed)),
        Value::String(text) => text.parse::<F>().ok().filter(|&parsed| !is_finite(parsed)),
        _ => None,
    };
    parsed.ok_or_else(|| FLOAT_SHAPE.to_owned())
}

impl JsonLeaf for f32 {
    fn to_json(&self) -> Value {
        float_to_json(self.to_string(), self.is_finite())
    }

    fn from_json(value: &Value) -> Result<Self, String> {
        float_from_json(value, f32::is_finite)
    }
}

impl JsonLeaf for f64 {
    fn to_json(&self) -> Value {
        float_to_json(self.to_string(), self.is_finite())
    }

    fn from_json(value: &Value) -> Result<Self, String> {
        float_from_json(value, f64::is_finite)
    }
}

fn components_to_json(components: &[f32]) -> Value {
    Value::Array(components.iter().map(JsonLeaf::to_json).collect())
}

fn components_from_json<const N: usize>(value: &Value) -> Result<[f32; N], String> {
    let shape = || format!("expected an array of {N} numbers");
    let elements = value.as_array().filter(|elements| elements.len() == N);
    let mut components = [0.0; N];
    for (component, element) in components.iter_mut().zip(elements.ok_or_else(shape)?) {
        *component = f32::from_json(element).map_err(|_| shape())?;
    }
    Ok(components)
}

macro_rules! json_vectors {
    ($($vector:ty: $len:literal),* $(,)?) => {
        $(
            impl JsonLeaf for $vector {
                fn to_json(&self) -> Value {
                    components_to_json(&self.to_array())
                }

                fn from_json(value: &Value) -> Result<Self, String> {
                    components_from_json::<$len>(value).map(<$vector>::from_array)
                }
            }
        )*
    };
}

json_vectors!(Vec2: 2, Vec3: 3, Vec4: 4, Quat: 4);

impl JsonLeaf for Color {
    /// The linear components `[r, g, b, a]`; reading them back gives a linear color.
    fn to_json(&self) -> Value {
        components_to_json(&self.to_array())
    }

    fn from_json(value: &Value) -> Result<Self, String> {
        components_from_json::<4>(value).map(|[r, g, b, a]| Color::rgba(r, g, b, a))
    }
}

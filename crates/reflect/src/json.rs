use std::any::Any;

use glam::{Quat, Vec2, Vec3, Vec4};
use serde_json::{Map, Number, Value};

use crate::{Fields, FromType, Reflect, ReflectRef, TypeData, TypeRegistry};

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

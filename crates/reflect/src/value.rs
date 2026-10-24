use serde::{Deserialize, Serialize};

use crate::{Fields, LeafError, Reflect, ReflectMut, ReflectRef};

/// A reflected value as stored data: self-describing, with field and variant names.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ReflectedValue {
    Leaf(Vec<u8>),
    Struct(Vec<(String, ReflectedValue)>),
    Variant {
        name: String,
        fields: Vec<(String, ReflectedValue)>,
    },
    List(Vec<ReflectedValue>),
}

/// Sees each leaf before its own encoding does; `None` leaves the leaf to encode itself.
pub trait LeafHook {
    fn encode(&mut self, leaf: &dyn Reflect) -> Option<Result<Vec<u8>, LeafError>>;
    fn decode(&mut self, leaf: &mut dyn Reflect, bytes: &[u8]) -> Option<Result<(), LeafError>>;
}

pub(crate) struct NoHook;

impl LeafHook for NoHook {
    fn encode(&mut self, _: &dyn Reflect) -> Option<Result<Vec<u8>, LeafError>> {
        None
    }

    fn decode(&mut self, _: &mut dyn Reflect, _: &[u8]) -> Option<Result<(), LeafError>> {
        None
    }
}

/// A leaf that could not be encoded, and where it is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EncodeError {
    pub path: String,
    pub error: LeafError,
}

impl std::fmt::Display for EncodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.path, self.error)
    }
}

impl std::error::Error for EncodeError {}

/// A part of the stored value that was not applied, and where it is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValueWarning {
    pub path: String,
    pub reason: String,
}

impl std::fmt::Display for ValueWarning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.path.is_empty() {
            f.write_str(&self.reason)
        } else {
            write!(f, "{}: {}", self.path, self.reason)
        }
    }
}

/// Stores `value` as a tree, each leaf encoding itself.
pub fn to_value(value: &dyn Reflect) -> Result<ReflectedValue, EncodeError> {
    to_value_with(value, &mut NoHook)
}

/// Stores `value` as a tree, letting `hook` encode any leaf first.
pub fn to_value_with(
    value: &dyn Reflect,
    hook: &mut dyn LeafHook,
) -> Result<ReflectedValue, EncodeError> {
    Encoder {
        hook,
        path: Path::default(),
    }
    .value(value)
}

/// Writes `stored` into `value` in place, returning every part that was not applied.
pub fn apply_value(value: &mut dyn Reflect, stored: &ReflectedValue) -> Vec<ValueWarning> {
    apply_value_with(value, stored, &mut NoHook)
}

/// [`apply_value`], letting `hook` decode any leaf first.
pub fn apply_value_with(
    value: &mut dyn Reflect,
    stored: &ReflectedValue,
    hook: &mut dyn LeafHook,
) -> Vec<ValueWarning> {
    let mut applier = Applier {
        hook,
        path: Path::default(),
        warnings: Vec::new(),
    };
    applier.value(value, stored);
    applier.warnings
}

#[derive(Default)]
pub(crate) struct Path(Vec<String>);

impl Path {
    pub(crate) fn field(&mut self, name: &str) {
        self.0.push(if self.0.is_empty() {
            name.to_owned()
        } else {
            format!(".{name}")
        });
    }

    pub(crate) fn index(&mut self, index: usize) {
        self.0.push(format!("[{index}]"));
    }

    pub(crate) fn pop(&mut self) {
        self.0.pop();
    }

    pub(crate) fn render(&self) -> String {
        self.0.concat()
    }
}

struct Encoder<'h> {
    hook: &'h mut dyn LeafHook,
    path: Path,
}

impl Encoder<'_> {
    fn value(&mut self, value: &dyn Reflect) -> Result<ReflectedValue, EncodeError> {
        match value.reflect_ref() {
            ReflectRef::Struct(fields) => Ok(ReflectedValue::Struct(self.fields(fields)?)),
            ReflectRef::Enum(variant) => Ok(ReflectedValue::Variant {
                name: variant.variant_name().to_owned(),
                fields: self.fields(variant)?,
            }),
            ReflectRef::List(list) => (0..list.len())
                .filter_map(|index| list.get(index).map(|element| (index, element)))
                .map(|(index, element)| {
                    self.path.index(index);
                    let encoded = self.value(element);
                    self.path.pop();
                    encoded
                })
                .collect::<Result<_, _>>()
                .map(ReflectedValue::List),
            ReflectRef::Leaf(leaf) => self
                .hook
                .encode(value)
                .unwrap_or_else(|| leaf.encode())
                .map(ReflectedValue::Leaf)
                .map_err(|error| EncodeError {
                    path: self.path.render(),
                    error,
                }),
        }
    }

    fn fields(
        &mut self,
        fields: &dyn Fields,
    ) -> Result<Vec<(String, ReflectedValue)>, EncodeError> {
        let mut encoded = Vec::with_capacity(fields.field_len());
        for index in 0..fields.field_len() {
            let (Some(name), Some(field)) = (fields.name_at(index), fields.field_at(index)) else {
                continue;
            };
            self.path.field(name);
            let value = self.value(field);
            self.path.pop();
            encoded.push((name.to_owned(), value?));
        }
        Ok(encoded)
    }
}

struct Applier<'h> {
    hook: &'h mut dyn LeafHook,
    path: Path,
    warnings: Vec<ValueWarning>,
}

impl Applier<'_> {
    fn warn(&mut self, reason: impl Into<String>) {
        self.warnings.push(ValueWarning {
            path: self.path.render(),
            reason: reason.into(),
        });
    }

    fn value(&mut self, value: &mut dyn Reflect, stored: &ReflectedValue) {
        let is_leaf = matches!(value.reflect_ref(), ReflectRef::Leaf(_));
        if is_leaf {
            let ReflectedValue::Leaf(bytes) = stored else {
                self.warn("stored data is not a leaf value");
                return;
            };
            let decoded = match self.hook.decode(value, bytes) {
                Some(decoded) => decoded,
                None => match value.reflect_mut() {
                    ReflectMut::Leaf(leaf) => leaf.decode(bytes),
                    _ => unreachable!("checked to be a leaf"),
                },
            };
            if let Err(error) = decoded {
                self.warn(format!("could not decode the stored value: {error}"));
            }
            return;
        }
        match (value.reflect_mut(), stored) {
            (ReflectMut::Struct(fields), ReflectedValue::Struct(stored)) => {
                self.fields(fields, stored)
            }
            (ReflectMut::Enum(variant), ReflectedValue::Variant { name, fields }) => {
                if variant.variant_name() != name && !variant.set_variant(name) {
                    self.warn(format!("unknown variant '{name}'"));
                    return;
                }
                self.fields(variant, fields);
            }
            (ReflectMut::List(list), ReflectedValue::List(stored)) => {
                list.truncate(stored.len());
                while list.len() < stored.len() {
                    list.push_default();
                }
                for (index, element) in stored.iter().enumerate() {
                    if let Some(value) = list.get_mut(index) {
                        self.path.index(index);
                        self.value(value, element);
                        self.path.pop();
                    }
                }
            }
            _ => self.warn("stored data is a different kind of value"),
        }
    }

    fn fields(&mut self, fields: &mut dyn Fields, stored: &[(String, ReflectedValue)]) {
        for (name, value) in stored {
            self.path.field(name);
            match fields.field_mut(name) {
                Some(field) => self.value(field, value),
                None => self.warn("no such field; ignored"),
            }
            self.path.pop();
        }
    }
}

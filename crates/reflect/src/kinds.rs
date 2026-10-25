use crate::Reflect;

/// What kind of value a [`Reflect`] is, for reading.
pub enum ReflectRef<'a> {
    Struct(&'a dyn Fields),
    Enum(&'a dyn Enum),
    List(&'a dyn List),
    Leaf(&'a dyn Leaf),
}

/// What kind of value a [`Reflect`] is, for writing.
pub enum ReflectMut<'a> {
    Struct(&'a mut dyn Fields),
    Enum(&'a mut dyn Enum),
    List(&'a mut dyn List),
    Leaf(&'a mut dyn Leaf),
}

/// Named fields: a struct's, or the active variant's of an enum. Tuple fields are named `"0"`, `"1"`, ….
pub trait Fields {
    fn field_len(&self) -> usize;
    fn name_at(&self, index: usize) -> Option<&'static str>;
    fn field_at(&self, index: usize) -> Option<&dyn Reflect>;
    fn field_at_mut(&mut self, index: usize) -> Option<&mut dyn Reflect>;

    fn index_of(&self, name: &str) -> Option<usize> {
        (0..self.field_len()).find(|&index| self.name_at(index) == Some(name))
    }

    fn field(&self, name: &str) -> Option<&dyn Reflect> {
        self.field_at(self.index_of(name)?)
    }

    fn field_mut(&mut self, name: &str) -> Option<&mut dyn Reflect> {
        let index = self.index_of(name)?;
        self.field_at_mut(index)
    }
}

/// An enum: its active variant's name, and that variant's fields.
pub trait Enum: Fields {
    fn variant_name(&self) -> &'static str;
    /// Every variant's name, in declaration order.
    fn variant_names(&self) -> &'static [&'static str];
    /// Switches to the variant `name` built from defaults; false when there is no such variant.
    fn set_variant(&mut self, name: &str) -> bool;
}

/// A growable sequence of reflected elements.
pub trait List {
    fn len(&self) -> usize;
    fn get(&self, index: usize) -> Option<&dyn Reflect>;
    fn get_mut(&mut self, index: usize) -> Option<&mut dyn Reflect>;
    fn push_default(&mut self);
    fn truncate(&mut self, len: usize);

    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// A value with no reflected inside, which encodes itself as bytes.
pub trait Leaf {
    fn encode(&self) -> Result<Vec<u8>, LeafError>;
    fn decode(&mut self, bytes: &[u8]) -> Result<(), LeafError>;
}

/// A leaf could not be encoded or decoded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LeafError(pub String);

impl std::fmt::Display for LeafError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for LeafError {}

/// Encodes a serde value as a leaf's bytes.
pub fn encode_leaf<T: serde::Serialize>(value: &T) -> Result<Vec<u8>, LeafError> {
    bincode::serialize(value).map_err(|error| LeafError(error.to_string()))
}

/// Decodes a leaf's bytes into a serde value in place.
pub fn decode_leaf<T: serde::de::DeserializeOwned>(
    value: &mut T,
    bytes: &[u8],
) -> Result<(), LeafError> {
    *value = bincode::deserialize(bytes).map_err(|error| LeafError(error.to_string()))?;
    Ok(())
}

impl<T: Reflect + Default> Fields for Option<T> {
    fn field_len(&self) -> usize {
        usize::from(self.is_some())
    }

    fn name_at(&self, index: usize) -> Option<&'static str> {
        (self.is_some() && index == 0).then_some("0")
    }

    fn field_at(&self, index: usize) -> Option<&dyn Reflect> {
        self.as_ref()
            .filter(|_| index == 0)
            .map(|value| value as &dyn Reflect)
    }

    fn field_at_mut(&mut self, index: usize) -> Option<&mut dyn Reflect> {
        self.as_mut()
            .filter(|_| index == 0)
            .map(|value| value as &mut dyn Reflect)
    }
}

impl<T: Reflect + Default> Enum for Option<T> {
    fn variant_name(&self) -> &'static str {
        if self.is_some() { "Some" } else { "None" }
    }

    fn variant_names(&self) -> &'static [&'static str] {
        &["None", "Some"]
    }

    fn set_variant(&mut self, name: &str) -> bool {
        match name {
            "None" => *self = None,
            "Some" => *self = Some(T::default()),
            _ => return false,
        }
        true
    }
}

impl<T: Reflect + Default> Reflect for Option<T> {
    fn reflect_ref(&self) -> ReflectRef<'_> {
        ReflectRef::Enum(self)
    }

    fn reflect_mut(&mut self) -> ReflectMut<'_> {
        ReflectMut::Enum(self)
    }
}

impl<T: Reflect + Default> List for Vec<T> {
    fn len(&self) -> usize {
        Vec::len(self)
    }

    fn get(&self, index: usize) -> Option<&dyn Reflect> {
        self.as_slice()
            .get(index)
            .map(|value| value as &dyn Reflect)
    }

    fn get_mut(&mut self, index: usize) -> Option<&mut dyn Reflect> {
        self.as_mut_slice()
            .get_mut(index)
            .map(|value| value as &mut dyn Reflect)
    }

    fn push_default(&mut self) {
        self.push(T::default());
    }

    fn truncate(&mut self, len: usize) {
        Vec::truncate(self, len);
    }
}

impl<T: Reflect + Default> Reflect for Vec<T> {
    fn reflect_ref(&self) -> ReflectRef<'_> {
        ReflectRef::List(self)
    }

    fn reflect_mut(&mut self) -> ReflectMut<'_> {
        ReflectMut::List(self)
    }
}

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::component::Component;

/// A human-readable label for an entity.
///
/// Authoring tools name things — a glTF node, a Blender object — and that name
/// is the only handle a person has on an entity once it is spawned. Without a
/// component to carry it, the name lives only in the scene file and a tree view
/// can show nothing but entity indices.
///
/// Names are not identifiers: nothing enforces uniqueness, and code should
/// never look an entity up by one.
#[derive(Component, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Name(String);

impl Name {
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn set(&mut self, name: impl Into<String>) {
        self.0 = name.into();
    }
}

impl fmt::Debug for Name {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.0, f)
    }
}

impl fmt::Display for Name {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl<T: Into<String>> From<T> for Name {
    fn from(value: T) -> Self {
        Self::new(value)
    }
}

impl AsRef<str> for Name {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

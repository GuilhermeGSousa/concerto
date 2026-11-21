use std::{
    collections::HashMap,
    fmt,
    hash::{Hash, Hasher},
    ops::Deref,
    sync::{LazyLock, RwLock},
};

use concerto_reflect::{JsonLeaf, Reflect, ReflectDefault, ReflectJson};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

struct Entry {
    index: u32,
    text: &'static str,
}

static EMPTY: Entry = Entry { index: 0, text: "" };

static TABLE: LazyLock<RwLock<HashMap<&'static str, &'static Entry>>> =
    LazyLock::new(|| RwLock::new(HashMap::from([("", &EMPTY)])));

/// An interned string: copying, comparing and hashing it never touch the string.
#[derive(Clone, Copy, Reflect)]
#[reflect(opaque, Default, Json)]
pub struct Name(&'static Entry);

impl Name {
    /// The name of `text`, interning it on first use.
    pub fn new(text: &str) -> Name {
        if let Some(entry) = TABLE.read().unwrap().get(text) {
            return Name(entry);
        }
        let mut table = TABLE.write().unwrap();
        if let Some(entry) = table.get(text) {
            return Name(entry);
        }
        let text: &'static str = Box::leak(text.into());
        let index = u32::try_from(table.len()).expect("the name table is full");
        let entry: &'static Entry = Box::leak(Box::new(Entry { index, text }));
        table.insert(text, entry);
        Name(entry)
    }

    /// The text this name was interned from.
    pub fn as_str(&self) -> &'static str {
        self.0.text
    }
}

impl Default for Name {
    fn default() -> Self {
        Name(&EMPTY)
    }
}

impl PartialEq for Name {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self.0, other.0)
    }
}

impl Eq for Name {}

impl Hash for Name {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.index.hash(state);
    }
}

impl PartialEq<str> for Name {
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}

impl PartialEq<&str> for Name {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

impl PartialEq<String> for Name {
    fn eq(&self, other: &String) -> bool {
        self.as_str() == other
    }
}

impl Deref for Name {
    type Target = str;

    fn deref(&self) -> &str {
        self.as_str()
    }
}

impl AsRef<str> for Name {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Display for Name {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self.as_str(), f)
    }
}

impl fmt::Debug for Name {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.as_str(), f)
    }
}

impl From<&str> for Name {
    fn from(text: &str) -> Self {
        Name::new(text)
    }
}

impl From<String> for Name {
    fn from(text: String) -> Self {
        Name::new(&text)
    }
}

impl Serialize for Name {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for Name {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(Name::from)
    }
}

impl JsonLeaf for Name {
    fn to_json(&self) -> Value {
        Value::String(self.as_str().to_owned())
    }

    fn from_json(value: &Value) -> Result<Self, String> {
        value
            .as_str()
            .map(Name::new)
            .ok_or_else(|| "expected a string".to_owned())
    }
}

/// Interns a string literal once per call site.
#[macro_export]
macro_rules! name {
    ($text:literal) => {{
        static NAME: ::std::sync::OnceLock<$crate::Name> = ::std::sync::OnceLock::new();
        *NAME.get_or_init(|| $crate::Name::new($text))
    }};
}

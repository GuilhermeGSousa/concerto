use std::collections::HashMap;
use std::fmt;
use std::marker::PhantomData;

use crate::CaseError;

/// A dense index into one table of a case or of the catalog.
pub trait Id: Copy + Eq {
    const KIND: &'static str;
    fn from_index(index: usize) -> Self;
    fn index(self) -> usize;
}

pub(crate) fn is_id(name: &str) -> bool {
    let allowed = |c: char| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-');
    !name.is_empty() && name.len() <= 64 && name.chars().all(allowed)
}

macro_rules! ids {
    ($($(#[$doc:meta])* $name:ident => $kind:literal,)*) => {
        $(
            $(#[$doc])*
            #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
            pub struct $name(u32);

            impl Id for $name {
                const KIND: &'static str = $kind;

                fn from_index(index: usize) -> Self {
                    Self(index as u32)
                }

                fn index(self) -> usize {
                    self.0 as usize
                }
            }
        )*
    };
}

ids! {
    /// A string of the case's string table.
    StrId => "string",
    /// A character of the case.
    NpcId => "character",
    /// An overheard conversation.
    ConvId => "conversation",
    /// A fact.
    FactId => "fact",
    /// A deduction term.
    TermId => "term",
    /// A bank of terms a blank picks from.
    TermBankId => "term bank",
    /// A deduction.
    DeductionId => "deduction",
    /// A prop the case places in the level.
    PlacementId => "placement",
    /// A room of the level.
    RoomId => "room",
    /// An anchor of the level.
    AnchorId => "anchor",
    /// A door of the level.
    DoorId => "door",
    /// A special feature of the level that a case turns on or off.
    FeatureId => "feature",
    /// A key.
    KeyId => "key",
    /// A prop of the props library.
    PropId => "prop",
    /// A portrait.
    PortraitId => "portrait",
    /// An idle activity the game can play.
    ActivityId => "activity",
}

/// The names of one kind of thing, each with its id.
pub struct Names<I> {
    names: Vec<String>,
    lookup: HashMap<String, u32>,
    marker: PhantomData<fn() -> I>,
}

impl<I> Default for Names<I> {
    fn default() -> Self {
        Self {
            names: Vec::new(),
            lookup: HashMap::new(),
            marker: PhantomData,
        }
    }
}

impl<I> fmt::Debug for Names<I> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(&self.names).finish()
    }
}

impl<I: Id> Names<I> {
    pub(crate) fn build<'a>(
        file: &'static str,
        names: impl Iterator<Item = &'a str>,
    ) -> Result<Self, CaseError> {
        let mut built = Self::default();
        for (index, name) in names.enumerate() {
            if !is_id(name) {
                return Err(CaseError::NotAnId {
                    file,
                    owner: format!("{} number {}", I::KIND, index + 1),
                    kind: I::KIND,
                });
            }
            if built.lookup.contains_key(name) {
                return Err(CaseError::Duplicate {
                    file,
                    kind: I::KIND,
                    id: name.to_string(),
                });
            }
            built.insert(name);
        }
        Ok(built)
    }

    pub(crate) fn insert(&mut self, name: &str) -> I {
        if let Some(index) = self.lookup.get(name) {
            return I::from_index(*index as usize);
        }
        let index = self.names.len();
        self.names.push(name.to_string());
        self.lookup.insert(name.to_string(), index as u32);
        I::from_index(index)
    }

    pub fn id(&self, name: &str) -> Option<I> {
        self.lookup
            .get(name)
            .map(|index| I::from_index(*index as usize))
    }

    pub fn name(&self, id: I) -> &str {
        &self.names[id.index()]
    }

    pub fn len(&self) -> usize {
        self.names.len()
    }

    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    pub fn ids(&self) -> impl Iterator<Item = I> + use<I> {
        (0..self.names.len()).map(I::from_index)
    }
}

/// The rows of one case file, addressed by id or by the name the file gave them.
pub struct Table<I, T> {
    names: Names<I>,
    rows: Vec<T>,
}

impl<I, T: fmt::Debug> fmt::Debug for Table<I, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_map()
            .entries(self.names.names.iter().zip(&self.rows))
            .finish()
    }
}

impl<I: Id, T> Table<I, T> {
    pub(crate) fn new(names: Names<I>, rows: Vec<T>) -> Self {
        Self { names, rows }
    }

    pub fn get(&self, id: I) -> &T {
        &self.rows[id.index()]
    }

    pub fn id(&self, name: &str) -> Option<I> {
        self.names.id(name)
    }

    pub fn name(&self, id: I) -> &str {
        self.names.name(id)
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub fn ids(&self) -> impl Iterator<Item = I> + use<I, T> {
        self.names.ids()
    }

    pub fn rows(&self) -> &[T] {
        &self.rows
    }
}

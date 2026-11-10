use serde::Deserialize;

use crate::{ActivityId, AnchorId, CaseError, DoorId, KeyId, Names, PortraitId, PropId, RoomId};

/// What a case may refer to without defining it: the level's ids and the game's libraries.
#[derive(Debug, Default)]
pub struct Catalog {
    pub rooms: Names<RoomId>,
    pub anchors: Names<AnchorId>,
    pub doors: Names<DoorId>,
    pub keys: Names<KeyId>,
    pub props: Names<PropId>,
    pub portraits: Names<PortraitId>,
    pub activities: Names<ActivityId>,
}

#[derive(Deserialize)]
struct LevelManifest {
    rooms: Vec<Room>,
    doors: Vec<Door>,
    anchors: Vec<Anchor>,
}

#[derive(Deserialize)]
struct Room {
    id: String,
}

#[derive(Deserialize)]
struct Door {
    id: String,
    key: Option<String>,
}

#[derive(Deserialize)]
struct Anchor {
    id: String,
}

impl Catalog {
    pub const LEVEL_MANIFEST: &'static str = "level_manifest";

    pub fn new() -> Self {
        Self::default()
    }

    /// Reads the rooms, anchors, doors and keys of an exported `level_manifest.ron`.
    pub fn from_level_manifest(text: &str) -> Result<Self, CaseError> {
        let file = Self::LEVEL_MANIFEST;
        let manifest: LevelManifest = ron::from_str(text).map_err(|error| CaseError::Syntax {
            file,
            line: error.span.start.line,
            column: error.span.start.col,
        })?;
        let mut catalog = Self {
            rooms: Names::build(file, manifest.rooms.iter().map(|room| room.id.as_str()))?,
            anchors: Names::build(
                file,
                manifest.anchors.iter().map(|anchor| anchor.id.as_str()),
            )?,
            doors: Names::build(file, manifest.doors.iter().map(|door| door.id.as_str()))?,
            ..Self::default()
        };
        for key in manifest.doors.iter().filter_map(|door| door.key.as_deref()) {
            catalog.keys.insert(key);
        }
        Ok(catalog)
    }

    pub fn with_room(mut self, name: &str) -> Self {
        self.rooms.insert(name);
        self
    }

    pub fn with_anchor(mut self, name: &str) -> Self {
        self.anchors.insert(name);
        self
    }

    pub fn with_door(mut self, name: &str) -> Self {
        self.doors.insert(name);
        self
    }

    pub fn with_key(mut self, name: &str) -> Self {
        self.keys.insert(name);
        self
    }

    pub fn with_prop(mut self, name: &str) -> Self {
        self.props.insert(name);
        self
    }

    pub fn with_portrait(mut self, name: &str) -> Self {
        self.portraits.insert(name);
        self
    }

    pub fn with_activity(mut self, name: &str) -> Self {
        self.activities.insert(name);
        self
    }
}

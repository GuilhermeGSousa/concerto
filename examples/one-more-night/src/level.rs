use std::collections::{HashMap, VecDeque, hash_map::Entry};

use concerto::ecs::Resource;
use glam::{Vec2, Vec3};
use serde::Deserialize;

use crate::case::{AnchorId, CaseError, Catalog, DoorId, DoorState, Id, RoomId, Zone};

#[derive(Deserialize)]
#[serde(rename = "LevelManifest")]
struct RawManifest {
    rooms: Vec<RawRoom>,
    doors: Vec<RawDoor>,
    anchors: Vec<RawAnchor>,
}

#[derive(Deserialize)]
struct RawCorner(f32, f32);

#[derive(Deserialize)]
struct RawSpan(RawCorner, RawCorner);

#[derive(Deserialize)]
struct RawPoint(f32, f32, f32);

#[derive(Deserialize)]
#[serde(rename = "Room")]
struct RawRoom {
    zone: Zone,
    bounds: Vec<RawSpan>,
}

#[derive(Deserialize)]
#[serde(rename = "Door")]
struct RawDoor {
    a: String,
    b: String,
    zone: Zone,
    default: DoorState,
    key: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename = "Anchor")]
struct RawAnchor {
    room: String,
    pos: RawPoint,
    facing: f32,
}

/// A rectangle of floor, in the engine's x and z.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FloorRect {
    pub min: Vec2,
    pub max: Vec2,
}

impl FloorRect {
    pub fn contains(&self, position: Vec3) -> bool {
        position.x >= self.min.x
            && position.x <= self.max.x
            && position.z >= self.min.y
            && position.z <= self.max.y
    }
}

/// A room of the level.
#[derive(Clone, Debug)]
pub struct Room {
    pub zone: Zone,
    pub bounds: Vec<FloorRect>,
}

/// A named place of the level: where a character stands or a prop sits.
#[derive(Clone, Copy, Debug)]
pub struct Anchor {
    pub room: RoomId,
    pub position: Vec3,
    /// Radians about +Y; 0 looks along -Z.
    pub yaw: f32,
}

/// A connection between two rooms, as the level ships it.
#[derive(Clone, Debug)]
pub struct LevelDoor {
    pub a: RoomId,
    pub b: RoomId,
    pub zone: Zone,
    pub state: DoorState,
    pub key: Option<String>,
}

/// The level's rooms, anchors and doors, in the engine's space: a manifest point `(x, y, z)` is `(x, z, -y)` here.
#[derive(Resource, Clone, Debug, Default)]
pub struct Level {
    rooms: Vec<Room>,
    anchors: Vec<Anchor>,
    doors: Vec<LevelDoor>,
    door_positions: HashMap<DoorId, Vec3>,
}

/// Turns a manifest point, z up, into the engine's, y up.
pub fn from_manifest(x: f32, y: f32, z: f32) -> Vec3 {
    Vec3::new(x, z, -y)
}

/// Turns a manifest facing, degrees clockwise from +y, into a yaw about the engine's +Y.
pub fn yaw_from_facing(degrees: f32) -> f32 {
    -degrees.to_radians()
}

impl Level {
    /// Reads a `level_manifest.ron`; `catalog` is the one built from the same text, so ids line up.
    pub fn from_manifest(text: &str, catalog: &Catalog) -> Result<Self, CaseError> {
        let raw: RawManifest = ron::from_str(text).map_err(|error| CaseError::Syntax {
            file: Catalog::LEVEL_MANIFEST,
            line: error.span.start.line,
            column: error.span.start.col,
        })?;
        let room = |name: &str| catalog.rooms.id(name).unwrap_or(RoomId::from_index(0));
        Ok(Self {
            rooms: raw
                .rooms
                .iter()
                .map(|room| Room {
                    zone: room.zone,
                    bounds: room
                        .bounds
                        .iter()
                        .map(|span| {
                            let a = from_manifest(span.0.0, span.0.1, 0.0);
                            let b = from_manifest(span.1.0, span.1.1, 0.0);
                            FloorRect {
                                min: Vec2::new(a.x.min(b.x), a.z.min(b.z)),
                                max: Vec2::new(a.x.max(b.x), a.z.max(b.z)),
                            }
                        })
                        .collect(),
                })
                .collect(),
            anchors: raw
                .anchors
                .iter()
                .map(|anchor| Anchor {
                    room: room(&anchor.room),
                    position: from_manifest(anchor.pos.0, anchor.pos.1, anchor.pos.2),
                    yaw: yaw_from_facing(anchor.facing),
                })
                .collect(),
            doors: raw
                .doors
                .iter()
                .map(|door| LevelDoor {
                    a: room(&door.a),
                    b: room(&door.b),
                    zone: door.zone,
                    state: door.default,
                    key: door.key.clone(),
                })
                .collect(),
            door_positions: HashMap::new(),
        })
    }

    pub fn room(&self, id: RoomId) -> &Room {
        &self.rooms[id.index()]
    }

    pub fn anchor(&self, id: AnchorId) -> &Anchor {
        &self.anchors[id.index()]
    }

    pub fn door(&self, id: DoorId) -> &LevelDoor {
        &self.doors[id.index()]
    }

    pub fn doors(&self) -> impl Iterator<Item = DoorId> + '_ {
        (0..self.doors.len()).map(DoorId::from_index)
    }

    /// The room whose floor `position` is over.
    pub fn room_at(&self, position: Vec3) -> Option<RoomId> {
        self.rooms
            .iter()
            .position(|room| room.bounds.iter().any(|rect| rect.contains(position)))
            .map(RoomId::from_index)
    }

    /// Records where the scene put `door`, which routes then pass through.
    pub fn place_door(&mut self, door: DoorId, position: Vec3) {
        self.door_positions.insert(door, position);
    }

    pub fn door_position(&self, door: DoorId) -> Option<Vec3> {
        self.door_positions.get(&door).copied()
    }

    /// The doors that join `a` and `b` directly.
    pub fn doors_between(&self, a: RoomId, b: RoomId) -> impl Iterator<Item = DoorId> + '_ {
        self.doors().filter(move |door| {
            let door = self.door(*door);
            (door.a == a && door.b == b) || (door.a == b && door.b == a)
        })
    }

    /// The doors to walk through from `from` to `to`, fewest first; empty when they are one room or unconnected.
    pub fn doors_on_the_way(&self, from: RoomId, to: RoomId) -> Vec<DoorId> {
        let mut came_by: HashMap<RoomId, Option<DoorId>> = HashMap::from([(from, None)]);
        let mut queue = VecDeque::from([from]);
        while let Some(room) = queue.pop_front() {
            if room == to {
                break;
            }
            for id in self.doors() {
                let door = self.door(id);
                let next = if door.a == room {
                    door.b
                } else if door.b == room {
                    door.a
                } else {
                    continue;
                };
                if let Entry::Vacant(unseen) = came_by.entry(next) {
                    unseen.insert(Some(id));
                    queue.push_back(next);
                }
            }
        }
        let mut doors = Vec::new();
        let mut room = to;
        while let Some(Some(id)) = came_by.get(&room) {
            doors.push(*id);
            let door = self.door(*id);
            room = if door.a == room { door.b } else { door.a };
        }
        doors.reverse();
        doors
    }

    /// The points to walk from one anchor to another: both ends, and between them each door on the way that the scene has placed.
    pub fn route(&self, from: AnchorId, to: AnchorId) -> Vec<Vec3> {
        let (start, end) = (self.anchor(from), self.anchor(to));
        let mut points = vec![start.position.with_y(0.0)];
        points.extend(
            self.doors_on_the_way(start.room, end.room)
                .into_iter()
                .filter_map(|door| self.door_position(door))
                .map(|position| position.with_y(0.0)),
        );
        points.push(end.position.with_y(0.0));
        points
    }
}

/// The length of a path through `points`.
pub fn path_length(points: &[Vec3]) -> f32 {
    points
        .windows(2)
        .map(|pair| pair[0].distance(pair[1]))
        .sum()
}

/// The point `distance` along a path through `points`, held at its ends.
pub fn point_along(points: &[Vec3], distance: f32) -> Vec3 {
    let mut left = distance.max(0.0);
    for pair in points.windows(2) {
        let length = pair[0].distance(pair[1]);
        if left <= length && length > 0.0 {
            return pair[0].lerp(pair[1], left / length);
        }
        left -= length;
    }
    points.last().copied().unwrap_or(Vec3::ZERO)
}

#[cfg(test)]
mod tests {
    use glam::Vec3;

    use super::{path_length, point_along};
    use crate::casefile::CaseData;

    #[test]
    fn the_test_level_reads_rooms_anchors_and_routes_through_placed_doors() {
        let data = CaseData::test();
        let mut level = data.level.clone();
        let catalog = &data.catalog;
        let room = |name: &str| catalog.rooms.id(name).unwrap();
        let anchor = |name: &str| catalog.anchors.id(name).unwrap();
        let door = |name: &str| catalog.doors.id(name).unwrap();

        assert_eq!(
            level.room_at(Vec3::new(3.0, 0.0, -3.0)),
            Some(room("room_a"))
        );
        assert_eq!(
            level.room_at(Vec3::new(9.0, 1.0, -3.0)),
            Some(room("room_b"))
        );
        assert_eq!(
            level.room_at(Vec3::new(9.0, 0.0, -7.5)),
            Some(room("corridor"))
        );
        assert_eq!(level.room_at(Vec3::new(-1.0, 0.0, 0.0)), None);

        let seat = level.anchor(anchor("a_seat_1"));
        assert_eq!(seat.position, Vec3::new(1.5, 0.0, -1.5));
        assert!(
            (seat.yaw + std::f32::consts::FRAC_PI_2).abs() < 1e-5,
            "it faces +X"
        );

        assert_eq!(
            level.doors_on_the_way(room("room_a"), room("room_b")),
            [door("d_ab")]
        );
        level.place_door(door("d_ab"), Vec3::new(6.0, 1.0, -3.0));
        let route = level.route(anchor("a_seat_1"), anchor("b_stand_1"));
        assert_eq!(
            route,
            [
                Vec3::new(1.5, 0.0, -1.5),
                Vec3::new(6.0, 0.0, -3.0),
                Vec3::new(8.4, 0.0, -3.0)
            ]
        );
        let length = path_length(&route);
        assert!((point_along(&route, length + 5.0) - route[2]).length() < 1e-5);
        assert!((point_along(&route, length - 1.0) - Vec3::new(7.4, 0.0, -3.0)).length() < 1e-4);
        assert_eq!(point_along(&route, -1.0), route[0]);
    }
}

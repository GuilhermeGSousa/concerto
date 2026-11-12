use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, VecDeque, hash_map::Entry};
use std::sync::{Arc, Mutex};

use concerto::ecs::Resource;
use glam::{Vec2, Vec3};
use serde::Deserialize;

use crate::case::{AnchorId, CaseError, Catalog, DoorId, DoorState, FeatureId, Id, RoomId, Zone};

const STEP_UP: f32 = 0.6;
const STOREY: f32 = 3.4;
const LAMP_HEIGHT: f32 = 2.1;
const LAMP_CLEARANCE: f32 = 0.9;
const LAMP_SPACING: f32 = 6.5;
const LAMP_RANGE: f32 = 7.0;
const LAMP_STOREY: f32 = 3.6;

#[derive(Deserialize)]
#[serde(rename = "LevelManifest")]
struct RawManifest {
    rooms: Vec<RawRoom>,
    doors: Vec<RawDoor>,
    anchors: Vec<RawAnchor>,
    #[serde(default)]
    features: Vec<RawFeature>,
    #[serde(default)]
    ways: Vec<RawWay>,
    #[serde(default)]
    paths: Vec<RawPath>,
}

#[derive(Deserialize)]
#[serde(rename = "Way")]
struct RawWay {
    id: String,
    pos: RawPoint,
}

#[derive(Deserialize)]
#[serde(rename = "Path")]
struct RawPath {
    a: String,
    b: String,
}

#[derive(Deserialize)]
struct RawCorner(f32, f32);

#[derive(Deserialize)]
struct RawSpan(RawCorner, RawCorner);

#[derive(Deserialize)]
struct RawPoint(f32, f32, f32);

#[derive(Deserialize, Clone, Copy)]
enum RawZone {
    Public,
    Service,
    Private,
    Open,
}

impl RawZone {
    fn zone(self) -> Zone {
        match self {
            RawZone::Public | RawZone::Open => Zone::Public,
            RawZone::Service => Zone::Service,
            RawZone::Private => Zone::Private,
        }
    }
}

#[derive(Deserialize, Clone, Copy, PartialEq)]
enum RawDoorState {
    Open,
    Closed,
    Locked,
    AlwaysOpen,
}

#[derive(Deserialize, Clone, Copy, PartialEq)]
enum RawStorey {
    Cellar,
    Ground,
    First,
    Attic,
    Grounds,
}

impl RawStorey {
    fn headroom(self) -> f32 {
        match self {
            RawStorey::Cellar | RawStorey::Attic => 2.4,
            RawStorey::First => 3.0,
            RawStorey::Ground | RawStorey::Grounds => 3.4,
        }
    }
}

#[derive(Deserialize)]
#[serde(rename = "Room")]
struct RawRoom {
    id: String,
    level: RawStorey,
    zone: RawZone,
    bounds: Vec<RawSpan>,
}

#[derive(Deserialize)]
#[serde(rename = "Door")]
struct RawDoor {
    a: String,
    b: String,
    zone: RawZone,
    default: RawDoorState,
    key: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename = "Feature")]
struct RawFeature {
    #[serde(default)]
    on: Vec<String>,
    #[serde(default)]
    off: Vec<String>,
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
    fn holds(&self, other: &FloorRect) -> bool {
        self.min.cmple(other.min).all() && self.max.cmpge(other.max).all()
    }

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
    /// The height of its floor, taken from its lowest anchor.
    pub floor: f32,
    /// How high it reaches: a storey above its floor, or a step above its highest anchor when it runs through several storeys.
    pub top: f32,
    /// Under a roof, so lit by lamps and not by the sky.
    pub indoors: bool,
    /// From its floor to its ceiling.
    pub headroom: f32,
}

/// A ceiling light of a room.
#[derive(Clone, Copy, Debug)]
pub struct Lamp {
    pub room: RoomId,
    pub position: Vec3,
    /// How far its light reaches.
    pub range: f32,
    /// How bright it is beside a lamp at full height: one hung lower under a low ceiling is dimmer, so the floor below is lit alike.
    pub glow: f32,
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
    /// An opening with nothing to close, such as an archway.
    pub always_open: bool,
}

/// The scene nodes a feature of the level shows when it is on and when it is off.
#[derive(Clone, Debug, Default)]
pub struct FeatureNodes {
    pub on: Vec<String>,
    pub off: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct RouteEnds {
    from: u32,
    to: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Reached {
    millimetres: u32,
    node: u32,
}

#[derive(Clone, Debug, Default)]
struct WalkGraph {
    points: Vec<Vec3>,
    links: Vec<Vec<u32>>,
    anchors: HashMap<AnchorId, u32>,
    found: Arc<Mutex<HashMap<RouteEnds, Vec<Vec3>>>>,
}

impl WalkGraph {
    fn read(raw: &RawManifest, catalog: &Catalog) -> Self {
        let index: HashMap<&str, u32> = raw
            .ways
            .iter()
            .enumerate()
            .map(|(at, way)| (way.id.as_str(), at as u32))
            .collect();
        let mut links = vec![Vec::new(); raw.ways.len()];
        for path in &raw.paths {
            if let (Some(a), Some(b)) = (index.get(path.a.as_str()), index.get(path.b.as_str())) {
                links[*a as usize].push(*b);
                links[*b as usize].push(*a);
            }
        }
        Self {
            points: raw
                .ways
                .iter()
                .map(|way| from_manifest(way.pos.0, way.pos.1, way.pos.2))
                .collect(),
            links,
            anchors: raw
                .ways
                .iter()
                .enumerate()
                .filter_map(|(at, way)| Some((catalog.anchors.id(&way.id)?, at as u32)))
                .collect(),
            found: Arc::default(),
        }
    }

    fn shortest(&self, ends: RouteEnds) -> Option<Vec<Vec3>> {
        let mut found = self.found.lock().ok()?;
        if let Some(route) = found.get(&ends) {
            return Some(route.clone());
        }
        let mut best = vec![u32::MAX; self.points.len()];
        let mut came_from = vec![u32::MAX; self.points.len()];
        let mut open = BinaryHeap::from([Reverse(Reached {
            millimetres: 0,
            node: ends.from,
        })]);
        best[ends.from as usize] = 0;
        while let Some(Reverse(reached)) = open.pop() {
            if reached.node == ends.to {
                break;
            }
            if reached.millimetres > best[reached.node as usize] {
                continue;
            }
            let here = self.points[reached.node as usize];
            for next in &self.links[reached.node as usize] {
                let step = (here.distance(self.points[*next as usize]) * 1000.0) as u32;
                let millimetres = reached.millimetres.saturating_add(step);
                if millimetres < best[*next as usize] {
                    best[*next as usize] = millimetres;
                    came_from[*next as usize] = reached.node;
                    open.push(Reverse(Reached {
                        millimetres,
                        node: *next,
                    }));
                }
            }
        }
        if best[ends.to as usize] == u32::MAX {
            return None;
        }
        let mut route = vec![self.points[ends.to as usize]];
        let mut node = ends.to;
        while node != ends.from {
            node = came_from[node as usize];
            route.push(self.points[node as usize]);
        }
        route.reverse();
        found.insert(ends, route.clone());
        Some(route)
    }
}

/// The level's rooms, anchors and doors, in the engine's space: a manifest point `(x, y, z)` is `(x, z, -y)` here.
#[derive(Resource, Clone, Debug, Default)]
pub struct Level {
    rooms: Vec<Room>,
    anchors: Vec<Anchor>,
    doors: Vec<LevelDoor>,
    features: Vec<FeatureNodes>,
    door_positions: HashMap<DoorId, Vec3>,
    homes: HashMap<u8, RoomId>,
    walk: WalkGraph,
}

/// Turns a manifest point, z up, into the engine's, y up.
pub fn from_manifest(x: f32, y: f32, z: f32) -> Vec3 {
    Vec3::new(x, z, -y)
}

/// Turns a manifest facing, degrees counter-clockwise from north (+y), into a yaw about the engine's +Y, within half a turn of zero.
pub fn yaw_from_facing(degrees: f32) -> f32 {
    let turn = std::f32::consts::TAU;
    let yaw = degrees.to_radians().rem_euclid(turn);
    if yaw > turn / 2.0 { yaw - turn } else { yaw }
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
                .map(|room| {
                    let heights = || {
                        raw.anchors
                            .iter()
                            .filter(|anchor| anchor.room == room.id)
                            .map(|anchor| anchor.pos.2)
                    };
                    let floor = heights().min_by(f32::total_cmp).unwrap_or(0.0);
                    let highest = heights().max_by(f32::total_cmp).unwrap_or(0.0);
                    Room {
                        floor,
                        top: (floor + STOREY).max(highest + STEP_UP),
                        zone: room.zone.zone(),
                        indoors: room.level != RawStorey::Grounds,
                        headroom: room.level.headroom(),
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
                    }
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
                    zone: door.zone.zone(),
                    state: match door.default {
                        RawDoorState::Open | RawDoorState::AlwaysOpen => DoorState::Open,
                        RawDoorState::Closed => DoorState::Closed,
                        RawDoorState::Locked => DoorState::Locked,
                    },
                    key: door.key.clone(),
                    always_open: door.default == RawDoorState::AlwaysOpen,
                })
                .collect(),
            features: raw
                .features
                .iter()
                .map(|feature| FeatureNodes {
                    on: feature.on.clone(),
                    off: feature.off.clone(),
                })
                .collect(),
            door_positions: HashMap::new(),
            homes: HashMap::new(),
            walk: WalkGraph::read(&raw, catalog),
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

    /// The room `position` stands in: of the rooms that reach its height and whose floor plan holds it, the one with the highest floor at or below it.
    pub fn room_at(&self, position: Vec3) -> Option<RoomId> {
        self.rooms
            .iter()
            .enumerate()
            .filter(|(_, room)| {
                room.floor <= position.y + STEP_UP
                    && position.y <= room.top
                    && room.bounds.iter().any(|rect| rect.contains(position))
            })
            .max_by(|a, b| a.1.floor.total_cmp(&b.1.floor))
            .map(|(index, _)| RoomId::from_index(index))
    }

    /// A ceiling lamp over every stretch of indoor floor, one storey after another in a room that runs through several.
    pub fn lamps(&self) -> Vec<Lamp> {
        let mut lamps = Vec::new();
        for (index, room) in self.rooms.iter().enumerate() {
            if !room.indoors {
                continue;
            }
            let storeys = 1
                + ((room.top - STOREY - room.floor) / LAMP_STOREY)
                    .ceil()
                    .max(0.0) as usize;
            let height = LAMP_HEIGHT.min(room.headroom - LAMP_CLEARANCE);
            for (at, rect) in room.bounds.iter().enumerate() {
                let covered = room.bounds.iter().enumerate().any(|(other, bigger)| {
                    other != at && bigger.holds(rect) && (bigger != rect || other < at)
                });
                if covered {
                    continue;
                }
                let size = rect.max - rect.min;
                let across = (size.x / LAMP_SPACING).ceil().max(1.0) as usize;
                let along = (size.y / LAMP_SPACING).ceil().max(1.0) as usize;
                for storey in 0..storeys {
                    for column in 0..across {
                        for row in 0..along {
                            let x = rect.min.x + size.x * (column as f32 + 0.5) / across as f32;
                            let z = rect.min.y + size.y * (row as f32 + 0.5) / along as f32;
                            let y = room.floor + height + LAMP_STOREY * storey as f32;
                            lamps.push(Lamp {
                                room: RoomId::from_index(index),
                                position: Vec3::new(x, y, z),
                                range: LAMP_RANGE,
                                glow: (height / LAMP_HEIGHT).powi(2),
                            });
                        }
                    }
                }
            }
        }
        lamps
    }

    /// The scene nodes to hide given which features are on: the `on` nodes of a feature that is off, and the `off` nodes of one that is on.
    pub fn hidden_nodes(&self, is_on: impl Fn(FeatureId) -> bool) -> Vec<&str> {
        self.features
            .iter()
            .enumerate()
            .flat_map(|(index, feature)| {
                if is_on(FeatureId::from_index(index)) {
                    &feature.off
                } else {
                    &feature.on
                }
            })
            .map(String::as_str)
            .collect()
    }

    /// Records where the scene put `door`, which routes then pass through.
    pub fn place_door(&mut self, door: DoorId, position: Vec3) {
        self.door_positions.insert(door, position);
    }

    /// Records the room `player` starts the night in as their own.
    pub fn place_home(&mut self, player: u8, room: RoomId) {
        self.homes.insert(player, room);
    }

    /// The room that is `player`'s own, once the scene has placed their start.
    pub fn home(&self, player: u8) -> Option<RoomId> {
        self.homes.get(&player).copied()
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

    /// Where a character at `anchor` puts their feet: its place on the level's walk graph, or under it on the floor of its room.
    pub fn footing(&self, anchor: AnchorId) -> Vec3 {
        if let Some(node) = self.walk.anchors.get(&anchor) {
            return self.walk.points[*node as usize];
        }
        let anchor = self.anchor(anchor);
        anchor.position.with_y(self.room(anchor.room).floor)
    }

    /// The points to walk from one anchor to another: the shortest way over the level's walk graph, round the furniture and by the stairs. A level without one gives both ends, each on its floor, and between them each door on the way that the scene has placed, at the higher of the floors it joins.
    pub fn route(&self, from: AnchorId, to: AnchorId) -> Vec<Vec3> {
        if from != to
            && let (Some(a), Some(b)) = (self.walk.anchors.get(&from), self.walk.anchors.get(&to))
            && let Some(route) = self.walk.shortest(RouteEnds { from: *a, to: *b })
        {
            return route;
        }
        let (start, end) = (self.anchor(from), self.anchor(to));
        let mut points = vec![self.footing(from)];
        points.extend(
            self.doors_on_the_way(start.room, end.room)
                .into_iter()
                .filter_map(|door| {
                    let sides = self.door(door);
                    let floor = self.room(sides.a).floor.max(self.room(sides.b).floor);
                    self.door_position(door)
                        .map(|position| position.with_y(floor))
                }),
        );
        points.push(self.footing(to));
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
    fn the_manor_reads_its_floors_archways_and_feature_nodes() {
        let data = CaseData::manor_walk();
        let room = |name: &str| data.catalog.rooms.id(name).unwrap();
        let door = |name: &str| data.catalog.doors.id(name).unwrap();
        let level = &data.level;

        assert_eq!(data.catalog.rooms.len(), 40);
        assert_eq!(level.doors().count(), 53);
        assert!(level.door(door("a_hall_e")).always_open);
        assert!(!level.door(door("d_back")).always_open);

        let bedroom = level.room(room("g1"));
        let middle = (bedroom.bounds[0].min + bedroom.bounds[0].max) / 2.0;
        let upstairs = Vec3::new(middle.x, bedroom.floor + 0.1, middle.y);
        assert!(bedroom.floor > 2.0, "a bedroom is a floor up");
        assert_eq!(level.room_at(upstairs), Some(room("g1")));
        assert_ne!(
            level.room_at(upstairs.with_y(0.0)),
            Some(room("g1")),
            "the same spot on the ground floor is another room, or none"
        );

        let lobby = |height: f32| level.room_at(Vec3::new(32.2, height, -9.0));
        assert_eq!(lobby(0.0), Some(room("passage")));
        assert_eq!(
            lobby(3.8),
            Some(room("backstairs")),
            "the back stairs' lobby a floor above the passage"
        );
        assert_eq!(lobby(7.2), Some(room("backstairs")));
        assert_eq!(lobby(-3.0), Some(room("cellar_pass")));
        assert_eq!(
            level.room_at(Vec3::new(18.2, 3.0, -13.0)),
            Some(room("hall")),
            "the main stair climbs inside the hall"
        );

        let lamps = level.lamps();
        assert!(
            lamps.len() <= 100,
            "{} lamps leave no room under the renderer's 128 lights",
            lamps.len()
        );
        for anchor in &level.anchors {
            let home = level.room(anchor.room);
            let lit = lamps.iter().any(|lamp| {
                lamp.room == anchor.room
                    && lamp.position.y > anchor.position.y
                    && lamp.position.distance(anchor.position) < lamp.range
            });
            assert_eq!(
                lit, home.indoors,
                "the anchor at {} has a lamp of its room over it only indoors",
                anchor.position
            );
        }
        let garden = Vec3::new(9.0, -0.6, -28.0);
        assert!(
            lamps
                .iter()
                .all(|lamp| lamp.position.distance(garden) > lamp.range),
            "no lamp reaches the middle of the garden"
        );

        let anchor = |name: &str| data.catalog.anchors.id(name).unwrap();
        let upstairs_walk = level.route(anchor("g1_p01_a"), anchor("gallery_p01_a"));
        assert!(
            upstairs_walk
                .iter()
                .all(|point| (point.y - bedroom.floor).abs() < 1e-4),
            "a walk between two rooms upstairs stays on their floor"
        );

        let upstairs = level.route(anchor("drawing_p03_a"), anchor("gallery_p01_a"));
        assert_eq!(upstairs.first().map(|point| point.y), Some(0.0));
        assert!(
            upstairs.iter().any(|point| (point.y - 1.99).abs() < 0.05),
            "the way up from the drawing room crosses the main stair's landing"
        );
        assert!(
            upstairs
                .windows(2)
                .all(|pair| (pair[1].y - pair[0].y).abs() <= 0.5 * pair[0].distance(pair[1]) + 0.3),
            "and never climbs steeper than a stair"
        );
        let to_bed = path_length(&level.route(anchor("servhall_p02_b"), anchor("attic_staff_09")));
        assert!(
            (38.0..50.0).contains(&to_bed),
            "the Staff's walk to bed is the back stairs' length, not {to_bed} m"
        );

        let hidden = level.hidden_nodes(|_| false);
        assert!(hidden.contains(&"acoustic.F01.src"));
        assert!(
            !hidden.contains(&"F01.bolt"),
            "a feature that is off shows its off nodes"
        );
    }

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

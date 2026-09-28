//! Procedural house plans: rooms carved from a square grid, joined by doors.
use std::collections::VecDeque;

use glam::{Vec2, Vec3};

use crate::game::Rand;

/// Side of one grid cell in meters.
pub const CELL: f32 = 4.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Cell {
    pub x: i32,
    pub y: i32,
}

impl Cell {
    pub const fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }

    /// World-space center of the cell on the floor.
    pub fn center(self) -> Vec3 {
        Vec3::new(
            (self.x as f32 + 0.5) * CELL,
            0.0,
            (self.y as f32 + 0.5) * CELL,
        )
    }

    pub fn from_world(p: Vec3) -> Self {
        Self::new((p.x / CELL).floor() as i32, (p.z / CELL).floor() as i32)
    }

    pub fn manhattan(self, other: Cell) -> i32 {
        (self.x - other.x).abs() + (self.y - other.y).abs()
    }

    pub fn step(self, dir: Dir) -> Cell {
        let (dx, dy) = dir.offset();
        Cell::new(self.x + dx, self.y + dy)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Dir {
    North,
    East,
    South,
    West,
}

impl Dir {
    pub const ALL: [Dir; 4] = [Dir::North, Dir::East, Dir::South, Dir::West];

    pub fn offset(self) -> (i32, i32) {
        match self {
            Dir::North => (0, -1),
            Dir::East => (1, 0),
            Dir::South => (0, 1),
            Dir::West => (-1, 0),
        }
    }

    /// Unit vector on the XZ plane.
    pub fn vector(self) -> Vec3 {
        let (x, y) = self.offset();
        Vec3::new(x as f32, 0.0, y as f32)
    }

}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoomKind {
    /// The entrance hall: the front door and the ledger.
    Hall,
    Corridor,
    /// Marrow's studio: easels and most of the figures.
    Studio,
    Gallery,
    Parlour,
    Library,
    Dining,
    /// Clara's room.
    Bedroom,
}

#[derive(Debug, Clone, Copy)]
pub struct Room {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub kind: RoomKind,
}

impl Room {
    pub fn cells(&self) -> impl Iterator<Item = Cell> + '_ {
        (self.y..self.y + self.h).flat_map(move |y| (self.x..self.x + self.w).map(move |x| Cell::new(x, y)))
    }

    pub fn area(&self) -> i32 {
        self.w * self.h
    }
}

/// A spot on a wall, seen from inside `cell`: the wall is on its `dir` side.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WallSpot {
    pub cell: Cell,
    pub dir: Dir,
}


/// Parameters for one house.
#[derive(Debug, Clone)]
pub struct LevelSpec {
    pub size: i32,
    pub lots: usize,
    pub oil: usize,
    pub figures: usize,
    /// Fraction of walls spots holding a lit candle sconce.
    pub lit_fraction: f32,
    /// Hard cap on candles, for fragment-shader cost on weak GPUs.
    pub max_lights: usize,
    /// Extra doors beyond a spanning tree, as a fraction of room pairs.
    pub loop_doors: f32,
    pub bedroom: bool,
    pub seed: u64,
}

#[derive(Debug, Clone)]
pub struct Level {
    pub size: i32,
    room_of: Vec<usize>,
    pub rooms: Vec<Room>,
    open_east: Vec<bool>,
    open_south: Vec<bool>,
    pub start: Cell,
    /// The front door: the hall cell and which outer wall it is in.
    pub exit: WallSpot,
    /// Paintings to catalogue.
    pub lots: Vec<WallSpot>,
    pub oil: Vec<Cell>,
    pub figures: Vec<Cell>,
    /// Where a diary page lies.
    pub page: Option<Cell>,
    /// Lit candle sconces.
    pub lights: Vec<WallSpot>,
}

impl Level {
    fn index(&self, c: Cell) -> usize {
        (c.y * self.size + c.x) as usize
    }

    pub fn contains(&self, c: Cell) -> bool {
        c.x >= 0 && c.y >= 0 && c.x < self.size && c.y < self.size
    }

    pub fn cells(&self) -> impl Iterator<Item = Cell> + '_ {
        (0..self.size).flat_map(move |y| (0..self.size).map(move |x| Cell::new(x, y)))
    }

    pub fn room_index(&self, c: Cell) -> Option<usize> {
        self.contains(c).then(|| self.room_of[self.index(c)])
    }

    pub fn room(&self, c: Cell) -> Option<&Room> {
        self.room_index(c).map(|i| &self.rooms[i])
    }

    /// Whether you can walk from `c` one step in `dir`.
    pub fn is_open(&self, c: Cell, dir: Dir) -> bool {
        let n = c.step(dir);
        if !self.contains(c) || !self.contains(n) {
            return false;
        }
        match dir {
            Dir::East => self.open_east[self.index(c)],
            Dir::West => self.open_east[self.index(n)],
            Dir::South => self.open_south[self.index(c)],
            Dir::North => self.open_south[self.index(n)],
        }
    }

    /// An open edge between two different rooms.
    pub fn is_door(&self, c: Cell, dir: Dir) -> bool {
        self.is_open(c, dir) && self.room_index(c) != self.room_index(c.step(dir))
    }

    /// A wall stands on this side of the cell (outer walls included).
    pub fn is_wall(&self, c: Cell, dir: Dir) -> bool {
        !self.is_open(c, dir) && self.exit != (WallSpot { cell: c, dir })
    }

    fn set_open(&mut self, c: Cell, dir: Dir, open: bool) {
        let n = c.step(dir);
        if !self.contains(c) || !self.contains(n) {
            return;
        }
        let i = match dir {
            Dir::East | Dir::South => self.index(c),
            Dir::West | Dir::North => self.index(n),
        };
        match dir {
            Dir::East | Dir::West => self.open_east[i] = open,
            Dir::South | Dir::North => self.open_south[i] = open,
        }
    }

    pub fn neighbors(&self, c: Cell) -> impl Iterator<Item = Cell> + '_ {
        Dir::ALL
            .into_iter()
            .filter(move |d| self.is_open(c, *d))
            .map(move |d| c.step(d))
    }

    /// Steps from `from` to every cell (`i32::MAX` if unreachable).
    pub fn distances(&self, from: Cell) -> Vec<i32> {
        let mut dist = vec![i32::MAX; (self.size * self.size) as usize];
        if !self.contains(from) {
            return dist;
        }
        let mut queue = VecDeque::new();
        dist[self.index(from)] = 0;
        queue.push_back(from);
        while let Some(c) = queue.pop_front() {
            let d = dist[self.index(c)];
            for n in self.neighbors(c) {
                let i = self.index(n);
                if dist[i] == i32::MAX {
                    dist[i] = d + 1;
                    queue.push_back(n);
                }
            }
        }
        dist
    }

    pub fn distance(&self, dist: &[i32], c: Cell) -> i32 {
        if self.contains(c) {
            dist[self.index(c)]
        } else {
            i32::MAX
        }
    }

    /// Every wall spot facing into the house, doors and the exit excluded.
    pub fn wall_spots(&self) -> impl Iterator<Item = WallSpot> + '_ {
        self.cells().flat_map(move |cell| {
            Dir::ALL
                .into_iter()
                .filter(move |d| self.is_wall(cell, *d))
                .map(move |dir| WallSpot { cell, dir })
        })
    }

    /// Wall segments one cell long, each listed once.
    pub fn walls(&self) -> Vec<Wall> {
        let mut walls = Vec::new();
        for c in self.cells() {
            for dir in Dir::ALL {
                let n = c.step(dir);
                let outer = !self.contains(n);
                if !outer && matches!(dir, Dir::North | Dir::West) {
                    continue;
                }
                let door = if (WallSpot { cell: c, dir }) == self.exit {
                    true
                } else if outer {
                    false
                } else if !self.is_open(c, dir) {
                    false
                } else if self.is_door(c, dir) {
                    true
                } else {
                    continue;
                };
                let along_x = matches!(dir, Dir::North | Dir::South);
                walls.push(Wall {
                    center: c.center() + dir.vector() * CELL * 0.5,
                    along_x,
                    outer,
                    door,
                });
            }
        }
        walls
    }

    pub fn extent(&self) -> f32 {
        self.size as f32 * CELL
    }
}

/// A wall segment one cell long, centered on a cell edge.
#[derive(Debug, Clone, Copy)]
pub struct Wall {
    pub center: Vec3,
    /// True when the wall runs along X (separates cells differing in y).
    pub along_x: bool,
    pub outer: bool,
    /// Has a doorway in its middle.
    pub door: bool,
}

impl Wall {
    /// Half extents (x, z) of the wall's footprint for a given thickness.
    pub fn half_extents(&self, thickness: f32) -> Vec2 {
        let long = CELL * 0.5 + thickness * 0.5;
        if self.along_x {
            Vec2::new(long, thickness * 0.5)
        } else {
            Vec2::new(thickness * 0.5, long)
        }
    }
}

fn split_at(rand: &mut Rand, length: i32) -> i32 {
    if length >= 4 && rand.unit() < 0.75 {
        2 + rand.index((length - 3) as usize) as i32
    } else if rand.unit() < 0.5 {
        1
    } else {
        length - 1
    }
}

fn split_rooms(rand: &mut Rand, x: i32, y: i32, w: i32, h: i32, out: &mut Vec<Room>) {
    let area = w * h;
    let small_enough = w <= 3 && h <= 3;
    let stop = small_enough && (area <= 3 || rand.unit() < 0.6);
    if stop || (w == 1 && h <= 4) || (h == 1 && w <= 4) {
        out.push(Room {
            x,
            y,
            w,
            h,
            kind: RoomKind::Parlour,
        });
        return;
    }
    let split_x = if w == h { rand.unit() < 0.5 } else { w > h };
    if split_x {
        let at = split_at(rand, w);
        split_rooms(rand, x, y, at, h, out);
        split_rooms(rand, x + at, y, w - at, h, out);
    } else {
        let at = split_at(rand, h);
        split_rooms(rand, x, y, w, at, out);
        split_rooms(rand, x, y + at, w, h - at, out);
    }
}

pub fn generate(spec: &LevelSpec) -> Level {
    let size = spec.size.max(3);
    let count = (size * size) as usize;
    let mut rand = Rand::new(spec.seed);

    let mut rooms = Vec::new();
    split_rooms(&mut rand, 0, 0, size, size, &mut rooms);
    let mut room_of = vec![0; count];
    for (i, room) in rooms.iter().enumerate() {
        for c in room.cells() {
            room_of[(c.y * size + c.x) as usize] = i;
        }
    }

    let mut level = Level {
        size,
        room_of,
        rooms,
        open_east: vec![false; count],
        open_south: vec![false; count],
        start: Cell::new(0, 0),
        exit: WallSpot {
            cell: Cell::new(0, 0),
            dir: Dir::North,
        },
        lots: Vec::new(),
        oil: Vec::new(),
        figures: Vec::new(),
        page: None,
        lights: Vec::new(),
    };

    let cells: Vec<Cell> = level.cells().collect();
    for &c in &cells {
        for dir in [Dir::East, Dir::South] {
            let n = c.step(dir);
            if level.contains(n) && level.room_index(c) == level.room_index(n) {
                level.set_open(c, dir, true);
            }
        }
    }

    // Candidate doors between each pair of adjacent rooms.
    let mut pairs: Vec<((usize, usize), Vec<(Cell, Dir)>)> = Vec::new();
    for &c in &cells {
        for dir in [Dir::East, Dir::South] {
            let n = c.step(dir);
            let (Some(a), Some(b)) = (level.room_index(c), level.room_index(n)) else {
                continue;
            };
            if a == b {
                continue;
            }
            let key = (a.min(b), a.max(b));
            match pairs.iter_mut().find(|(k, _)| *k == key) {
                Some((_, edges)) => edges.push((c, dir)),
                None => pairs.push((key, vec![(c, dir)])),
            }
        }
    }

    // A random spanning tree of rooms, then a few extra doors for loops.
    let room_count = level.rooms.len();
    let mut joined = vec![false; room_count];
    joined[0] = true;
    let mut order: Vec<usize> = (0..pairs.len()).collect();
    for i in (1..order.len()).rev() {
        order.swap(i, rand.index(i + 1));
    }
    let mut used = vec![false; pairs.len()];
    loop {
        let next = order.iter().copied().find(|&p| {
            let (a, b) = pairs[p].0;
            joined[a] != joined[b]
        });
        let Some(p) = next else {
            break;
        };
        let ((a, b), edges) = &pairs[p];
        let (c, dir) = edges[rand.index(edges.len())];
        level.set_open(c, dir, true);
        joined[*a] = true;
        joined[*b] = true;
        used[p] = true;
    }
    for p in order {
        if !used[p] && rand.unit() < spec.loop_doors {
            let edges = &pairs[p].1;
            let (c, dir) = edges[rand.index(edges.len())];
            level.set_open(c, dir, true);
        }
    }

    // The hall: a small room on the outside, holding the front door.
    let outer_spots: Vec<WallSpot> = cells
        .iter()
        .flat_map(|&cell| Dir::ALL.into_iter().map(move |dir| WallSpot { cell, dir }))
        .filter(|s| !level.contains(s.cell.step(s.dir)))
        .collect();
    let hall_spot = outer_spots
        .iter()
        .copied()
        .filter(|s| {
            let room = &level.rooms[level.room_index(s.cell).unwrap()];
            room.area() >= 2 && room.area() <= 4
        })
        .max_by_key(|s| {
            // Prefer the middle of a side, like a real front door.
            let mid = (size - 1) as f32 * 0.5;
            let off = (s.cell.x as f32 - mid).abs().min((s.cell.y as f32 - mid).abs());
            (-(off * 10.0) as i32) * 100 + rand.index(50) as i32
        })
        .unwrap_or(outer_spots[0]);
    level.exit = hall_spot;
    level.start = hall_spot.cell;
    let hall = level.room_index(hall_spot.cell).unwrap();
    level.rooms[hall].kind = RoomKind::Hall;

    let from_start = level.distances(level.start);

    // Room kinds: corridors by shape, the studio is the biggest room left.
    for room in level.rooms.iter_mut() {
        if room.kind != RoomKind::Hall && (room.w == 1 || room.h == 1) && room.area() >= 3 {
            room.kind = RoomKind::Corridor;
        }
    }
    let biggest = |level: &Level, exclude: &[RoomKind]| {
        (0..level.rooms.len())
            .filter(|&i| !exclude.contains(&level.rooms[i].kind))
            .max_by_key(|&i| level.rooms[i].area())
    };
    if let Some(i) = biggest(&level, &[RoomKind::Hall]) {
        level.rooms[i].kind = RoomKind::Studio;
    }
    if let Some(i) = biggest(&level, &[RoomKind::Hall, RoomKind::Studio]) {
        level.rooms[i].kind = RoomKind::Gallery;
    }
    if spec.bedroom {
        let far = (0..level.rooms.len())
            .filter(|&i| level.rooms[i].kind == RoomKind::Parlour)
            .max_by_key(|&i| {
                let r = level.rooms[i];
                level.distance(&from_start, Cell::new(r.x, r.y))
            });
        if let Some(i) = far {
            level.rooms[i].kind = RoomKind::Bedroom;
        }
    }
    let mut flavours = [RoomKind::Library, RoomKind::Dining, RoomKind::Parlour];
    for i in (1..flavours.len()).rev() {
        flavours.swap(i, rand.index(i + 1));
    }
    let mut next_flavour = 0;
    for room in level.rooms.iter_mut() {
        if room.kind == RoomKind::Parlour {
            room.kind = flavours[next_flavour % flavours.len()];
            next_flavour += 1;
        }
    }

    // Lots: paintings far from the door, at most one per room where possible.
    let max_dist = cells
        .iter()
        .map(|c| level.distance(&from_start, *c))
        .max()
        .unwrap_or(0);
    let hall_room = level.room_index(level.start);
    let spots: Vec<WallSpot> = level
        .wall_spots()
        .filter(|s| level.room_index(s.cell) != hall_room)
        .collect();
    let min_lot_dist = (max_dist / 3).max(2);
    let mut lot_rooms: Vec<usize> = Vec::new();
    for _ in 0..spec.lots {
        let best = spots
            .iter()
            .filter(|s| !level.lots.contains(s))
            .filter(|s| level.distance(&from_start, s.cell) >= min_lot_dist)
            .map(|&s| {
                let room = level.room_index(s.cell).unwrap();
                let spread = level
                    .lots
                    .iter()
                    .map(|l| l.cell.manhattan(s.cell))
                    .min()
                    .unwrap_or(size) as f32;
                let fresh_room = if lot_rooms.contains(&room) { -4.0 } else { 0.0 };
                let gallery = match level.rooms[room].kind {
                    RoomKind::Gallery => 1.0,
                    RoomKind::Bedroom => 3.0,
                    _ => 0.0,
                };
                (s, spread.min(5.0) + fresh_room + gallery + rand.unit() * 2.0)
            })
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(s, _)| s);
        if let Some(s) = best {
            lot_rooms.push(level.room_index(s.cell).unwrap());
            level.lots.push(s);
        }
    }

    let mut taken = vec![level.start];
    let pick = |level: &Level, taken: &mut Vec<Cell>, rand: &mut Rand, filter: &dyn Fn(Cell) -> bool| {
        let options: Vec<Cell> = cells
            .iter()
            .copied()
            .filter(|c| !taken.contains(c) && filter(*c))
            .collect();
        let _ = level;
        (!options.is_empty()).then(|| {
            let c = options[rand.index(options.len())];
            taken.push(c);
            c
        })
    };

    // Figures: the studio first, then spread through the house, never in the
    // hall.
    let studio: Vec<Cell> = level
        .rooms
        .iter()
        .filter(|r| r.kind == RoomKind::Studio)
        .flat_map(|r| r.cells().collect::<Vec<_>>())
        .collect();
    let mut figures = Vec::new();
    for _ in 0..spec.figures {
        let in_studio = figures.iter().filter(|c| studio.contains(c)).count();
        let prefer_studio = in_studio < studio.len().min(3);
        let got = if prefer_studio {
            pick(&level, &mut taken, &mut rand, &|c| studio.contains(&c) && level.distance(&from_start, c) >= 2)
        } else {
            None
        };
        let got = got.or_else(|| {
            pick(&level, &mut taken, &mut rand, &|c| {
                level.room_index(c) != hall_room && level.distance(&from_start, c) >= 2
            })
        });
        figures.extend(got);
    }
    level.figures = figures;

    let mut oil = Vec::new();
    for _ in 0..spec.oil {
        oil.extend(pick(&level, &mut taken, &mut rand, &|c| level.distance(&from_start, c) >= 2));
    }
    level.oil = oil;
    level.page = pick(&level, &mut taken, &mut rand, &|c| {
        let d = level.distance(&from_start, c);
        d >= 2 && d <= (max_dist * 2 / 3).max(2)
    });

    // Candles: one by the front door, then scattered, at most one per cell.
    let mut lights = Vec::new();
    if let Some(s) = level
        .wall_spots()
        .filter(|s| s.cell == level.start)
        .find(|s| s.dir != level.exit.dir)
    {
        lights.push(s);
    }
    let mut rest: Vec<WallSpot> = level
        .wall_spots()
        .filter(|s| s.cell != level.start && !level.lots.contains(s))
        .collect();
    let wanted = ((count as f32 * spec.lit_fraction) as usize).clamp(1, spec.max_lights);
    while lights.len() < wanted && !rest.is_empty() {
        let s = rest.swap_remove(rand.index(rest.len()));
        if lights.iter().all(|l: &WallSpot| l.cell != s.cell) {
            lights.push(s);
        }
    }
    level.lights = lights;

    level
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(seed: u64, size: i32) -> LevelSpec {
        LevelSpec {
            size,
            lots: 4,
            oil: 3,
            figures: 8,
            lit_fraction: 0.3,
            max_lights: 12,
            loop_doors: 0.35,
            bedroom: true,
            seed,
        }
    }

    #[test]
    fn every_cell_is_reachable() {
        for seed in 0..100 {
            let level = generate(&spec(seed, 7));
            let dist = level.distances(level.start);
            assert!(level.cells().all(|c| level.distance(&dist, c) < i32::MAX));
        }
    }

    #[test]
    fn rooms_tile_the_house() {
        for seed in 0..50 {
            let level = generate(&spec(seed, 7));
            let total: i32 = level.rooms.iter().map(Room::area).sum();
            assert_eq!(total, 49);
            assert!(level.rooms.iter().all(|r| r.w <= 4 && r.h <= 4));
            assert_eq!(
                level.rooms.iter().filter(|r| r.kind == RoomKind::Hall).count(),
                1
            );
            assert!(level.rooms.len() >= 6, "{} rooms", level.rooms.len());
        }
    }

    #[test]
    fn placements_are_sound() {
        for seed in 0..50 {
            let s = spec(seed, 7);
            let level = generate(&s);
            assert_eq!(level.lots.len(), s.lots);
            for lot in &level.lots {
                assert!(level.is_wall(lot.cell, lot.dir), "lot on an open edge");
                assert_ne!(level.room(lot.cell).unwrap().kind, RoomKind::Hall);
            }
            let dist = level.distances(level.start);
            for f in &level.figures {
                assert!(level.distance(&dist, *f) >= 2, "figure next to the door");
            }
            let mut all = vec![level.start];
            all.extend(&level.figures);
            all.extend(&level.oil);
            all.extend(level.page);
            let n = all.len();
            all.sort_by_key(|c| (c.x, c.y));
            all.dedup();
            assert_eq!(all.len(), n, "two things share a cell");
            assert!(level.lights.len() <= s.max_lights);
            assert!(level.lights.iter().all(|l| level.is_wall(l.cell, l.dir)));
        }
    }

    #[test]
    fn front_door_is_in_an_outer_wall_of_the_hall() {
        for seed in 0..50 {
            let level = generate(&spec(seed, 6));
            let exit = level.exit;
            assert!(!level.contains(exit.cell.step(exit.dir)));
            assert_eq!(level.room(exit.cell).unwrap().kind, RoomKind::Hall);
            assert!(!level.is_wall(exit.cell, exit.dir));
            assert!(level.walls().iter().any(|w| w.door && w.outer));
        }
    }

    #[test]
    fn doors_only_join_different_rooms() {
        let level = generate(&spec(5, 8));
        for w in level.walls() {
            if w.door && !w.outer {
                let inside = Cell::from_world(w.center - if w.along_x { Vec3::Z } else { Vec3::X });
                let dir = if w.along_x { Dir::South } else { Dir::East };
                assert!(level.is_door(inside, dir));
            }
        }
    }

    #[test]
    fn generation_is_deterministic() {
        let a = generate(&spec(42, 8));
        let b = generate(&spec(42, 8));
        assert_eq!(a.lots, b.lots);
        assert_eq!(a.figures, b.figures);
        assert_eq!(a.walls().len(), b.walls().len());
    }
}

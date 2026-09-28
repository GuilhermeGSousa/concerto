//! Procedural store floors: a braided maze of shelving on a square grid.
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

/// Parameters for one floor.
#[derive(Debug, Clone)]
pub struct LevelSpec {
    pub size: i32,
    pub keys: usize,
    pub batteries: usize,
    pub hunters: usize,
    pub decoys: usize,
    /// Fraction of cells whose ceiling light works.
    pub lit_fraction: f32,
    /// Hard cap on working lights, for fragment-shader cost on weak GPUs.
    pub max_lights: usize,
    pub seed: u64,
}

#[derive(Debug, Clone)]
pub struct Level {
    pub size: i32,
    open_east: Vec<bool>,
    open_south: Vec<bool>,
    pub start: Cell,
    /// The exit cell and which outer wall the door is in.
    pub exit: (Cell, Dir),
    pub keys: Vec<Cell>,
    pub batteries: Vec<Cell>,
    pub hunters: Vec<Cell>,
    pub decoys: Vec<Cell>,
    /// Cells whose ceiling light works.
    pub lights: Vec<Cell>,
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

    /// Whether you can walk from `c` one step in `dir`.
    pub fn is_open(&self, c: Cell, dir: Dir) -> bool {
        let (dx, dy) = dir.offset();
        let n = Cell::new(c.x + dx, c.y + dy);
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

    fn set_open(&mut self, c: Cell, dir: Dir, open: bool) {
        let (dx, dy) = dir.offset();
        let n = Cell::new(c.x + dx, c.y + dy);
        if !self.contains(c) || !self.contains(n) {
            return;
        }
        match dir {
            Dir::East => {
                let i = self.index(c);
                self.open_east[i] = open
            }
            Dir::West => {
                let i = self.index(n);
                self.open_east[i] = open
            }
            Dir::South => {
                let i = self.index(c);
                self.open_south[i] = open
            }
            Dir::North => {
                let i = self.index(n);
                self.open_south[i] = open
            }
        }
    }

    pub fn neighbors(&self, c: Cell) -> impl Iterator<Item = Cell> + '_ {
        Dir::ALL
            .into_iter()
            .filter(move |d| self.is_open(c, *d))
            .map(move |d| {
                let (dx, dy) = d.offset();
                Cell::new(c.x + dx, c.y + dy)
            })
    }

    pub fn wall_count(&self, c: Cell) -> usize {
        Dir::ALL
            .into_iter()
            .filter(|d| !self.is_open(c, *d))
            .count()
    }

    /// Steps from `from` to every cell (`i32::MAX` if unreachable).
    pub fn distances(&self, from: Cell) -> Vec<i32> {
        let mut dist = vec![i32::MAX; (self.size * self.size) as usize];
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
        dist[self.index(c)]
    }

    /// The first step on a shortest path from `from` toward `to`, or `None` if
    /// already there or unreachable.
    #[cfg(test)]
    pub fn next_step(&self, from: Cell, to: Cell) -> Option<Cell> {
        if from == to || !self.contains(from) || !self.contains(to) {
            return None;
        }
        let dist = self.distances(to);
        self.neighbors(from)
            .min_by_key(|n| dist[self.index(*n)])
            .filter(|n| dist[self.index(*n)] < dist[self.index(from)])
    }

    /// Wall segments between cells and around the outside, each as (center,
    /// along-axis is X?).
    pub fn walls(&self) -> Vec<Wall> {
        let mut walls = Vec::new();
        for c in self.cells() {
            let center = c.center();
            if c.x + 1 < self.size && !self.is_open(c, Dir::East) {
                walls.push(Wall::new(center + Vec3::X * CELL * 0.5, false, false));
            }
            if c.y + 1 < self.size && !self.is_open(c, Dir::South) {
                walls.push(Wall::new(center + Vec3::Z * CELL * 0.5, true, false));
            }
            for dir in Dir::ALL {
                let (dx, dy) = dir.offset();
                if self.contains(Cell::new(c.x + dx, c.y + dy)) {
                    continue;
                }
                if self.exit == (c, dir) {
                    continue;
                }
                let along_x = matches!(dir, Dir::North | Dir::South);
                walls.push(Wall::new(center + dir.vector() * CELL * 0.5, along_x, true));
            }
        }
        walls
    }

    /// Extent of the floor in world space.
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
    /// Outer store walls are drywall, inner ones are shelving.
    pub outer: bool,
}

impl Wall {
    fn new(center: Vec3, along_x: bool, outer: bool) -> Self {
        Self {
            center,
            along_x,
            outer,
        }
    }

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

pub fn generate(spec: &LevelSpec) -> Level {
    let size = spec.size.max(3);
    let count = (size * size) as usize;
    let mut rand = Rand::new(spec.seed);
    let mut level = Level {
        size,
        open_east: vec![false; count],
        open_south: vec![false; count],
        start: Cell::new(0, 0),
        exit: (Cell::new(size - 1, size - 1), Dir::East),
        keys: Vec::new(),
        batteries: Vec::new(),
        hunters: Vec::new(),
        decoys: Vec::new(),
        lights: Vec::new(),
    };

    let mut visited = vec![false; count];
    let mut stack = vec![level.start];
    visited[level.index(level.start)] = true;
    while let Some(&c) = stack.last() {
        let mut options: Vec<Dir> = Dir::ALL
            .into_iter()
            .filter(|d| {
                let (dx, dy) = d.offset();
                let n = Cell::new(c.x + dx, c.y + dy);
                level.contains(n) && !visited[level.index(n)]
            })
            .collect();
        if options.is_empty() {
            stack.pop();
            continue;
        }
        let dir = options.swap_remove(rand.index(options.len()));
        let (dx, dy) = dir.offset();
        let n = Cell::new(c.x + dx, c.y + dy);
        level.set_open(c, dir, true);
        visited[level.index(n)] = true;
        stack.push(n);
    }

    let cells: Vec<Cell> = level.cells().collect();
    for &c in &cells {
        if level.wall_count(c) >= 3 && rand.unit() < 0.7 {
            let closed: Vec<Dir> = Dir::ALL
                .into_iter()
                .filter(|d| {
                    let (dx, dy) = d.offset();
                    !level.is_open(c, *d) && level.contains(Cell::new(c.x + dx, c.y + dy))
                })
                .collect();
            if !closed.is_empty() {
                let dir = closed[rand.index(closed.len())];
                level.set_open(c, dir, true);
            }
        } else if rand.unit() < 0.06 {
            let dir = Dir::ALL[rand.index(4)];
            level.set_open(c, dir, true);
        }
    }

    let from_start = level.distances(level.start);
    let max_dist = cells
        .iter()
        .map(|c| level.distance(&from_start, *c))
        .max()
        .unwrap_or(0);

    let edge: Vec<(Cell, Dir)> = cells
        .iter()
        .flat_map(|&c| {
            Dir::ALL.into_iter().filter_map(move |d| {
                let (dx, dy) = d.offset();
                let outside = Cell::new(c.x + dx, c.y + dy);
                let on_edge =
                    outside.x < 0 || outside.y < 0 || outside.x >= size || outside.y >= size;
                on_edge.then_some((c, d))
            })
        })
        .collect();
    let far = edge
        .iter()
        .filter(|(c, _)| level.distance(&from_start, *c) as f32 >= max_dist as f32 * 0.75)
        .copied()
        .collect::<Vec<_>>();
    level.exit = if far.is_empty() {
        *edge
            .iter()
            .max_by_key(|(c, _)| level.distance(&from_start, *c))
            .unwrap()
    } else {
        far[rand.index(far.len())]
    };

    let mut taken = vec![level.start, level.exit.0];

    let min_key_dist = (max_dist / 3).max(2);
    for _ in 0..spec.keys {
        let best = cells
            .iter()
            .filter(|c| !taken.contains(c))
            .filter(|c| level.distance(&from_start, **c) >= min_key_dist)
            .map(|&c| {
                let spread = taken.iter().map(|t| t.manhattan(c)).min().unwrap_or(0) as f32;
                let dead_end = if level.wall_count(c) >= 3 { 1.5 } else { 0.0 };
                (c, spread + dead_end + rand.unit())
            })
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(c, _)| c);
        if let Some(c) = best {
            level.keys.push(c);
            taken.push(c);
        }
    }

    let pick = |taken: &mut Vec<Cell>, rand: &mut Rand, min_dist: i32| -> Option<Cell> {
        let options: Vec<Cell> = cells
            .iter()
            .copied()
            .filter(|c| !taken.contains(c) && level.distance(&from_start, *c) >= min_dist)
            .collect();
        (!options.is_empty()).then(|| {
            let c = options[rand.index(options.len())];
            taken.push(c);
            c
        })
    };

    let mut batteries = Vec::new();
    for _ in 0..spec.batteries {
        batteries.extend(pick(&mut taken, &mut rand, 2));
    }
    let hunter_min = (max_dist / 2).clamp(3, 6);
    let mut hunters = Vec::new();
    for _ in 0..spec.hunters {
        hunters.extend(
            pick(&mut taken, &mut rand, hunter_min).or_else(|| pick(&mut taken, &mut rand, 2)),
        );
    }
    let mut decoys = Vec::new();
    for _ in 0..spec.decoys {
        decoys.extend(pick(&mut taken, &mut rand, 2));
    }
    level.batteries = batteries;
    level.hunters = hunters;
    level.decoys = decoys;

    let mut lights = vec![level.start, level.exit.0];
    let mut rest: Vec<Cell> = cells
        .iter()
        .copied()
        .filter(|c| !lights.contains(c))
        .collect();
    let wanted = ((count as f32 * spec.lit_fraction) as usize).min(spec.max_lights);
    while lights.len() < wanted && !rest.is_empty() {
        lights.push(rest.swap_remove(rand.index(rest.len())));
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
            keys: 4,
            batteries: 3,
            hunters: 4,
            decoys: 8,
            lit_fraction: 0.5,
            max_lights: 16,
            seed,
        }
    }

    #[test]
    fn every_cell_is_reachable() {
        for seed in 0..50 {
            let level = generate(&spec(seed, 8));
            let dist = level.distances(level.start);
            assert!(level.cells().all(|c| level.distance(&dist, c) < i32::MAX));
        }
    }

    #[test]
    fn placements_are_distinct_and_rules_hold() {
        for seed in 0..50 {
            let s = spec(seed, 8);
            let level = generate(&s);
            assert_eq!(level.keys.len(), s.keys);
            assert_eq!(level.hunters.len(), s.hunters);
            let mut all = vec![level.start, level.exit.0];
            all.extend(&level.keys);
            all.extend(&level.batteries);
            all.extend(&level.hunters);
            all.extend(&level.decoys);
            let mut dedup = all.clone();
            dedup.sort_by_key(|c| (c.x, c.y));
            dedup.dedup();
            assert_eq!(dedup.len(), all.len(), "two things share a cell");

            let dist = level.distances(level.start);
            for h in &level.hunters {
                assert!(
                    level.distance(&dist, *h) >= 2,
                    "hunter spawns next to the player"
                );
            }
            assert!(level.lights.contains(&level.start));
            assert!(level.lights.len() <= s.max_lights.max(2));
        }
    }

    #[test]
    fn exit_door_is_in_an_outer_wall() {
        for seed in 0..50 {
            let level = generate(&spec(seed, 7));
            let (cell, dir) = level.exit;
            let (dx, dy) = dir.offset();
            assert!(!level.contains(Cell::new(cell.x + dx, cell.y + dy)));
            let door = cell.center() + dir.vector() * CELL * 0.5;
            assert!(level.walls().iter().all(|w| w.center.distance(door) > 0.1));
        }
    }

    #[test]
    fn braiding_leaves_few_dead_ends() {
        let level = generate(&spec(3, 10));
        let dead_ends = level.cells().filter(|c| level.wall_count(*c) >= 3).count();
        assert!(dead_ends < 25, "{dead_ends} dead ends");
    }

    #[test]
    fn next_step_walks_toward_the_target() {
        let level = generate(&spec(9, 8));
        let target = level.exit.0;
        let dist = level.distances(target);
        let mut at = level.start;
        for _ in 0..200 {
            let Some(next) = level.next_step(at, target) else {
                break;
            };
            assert!(level.distance(&dist, next) < level.distance(&dist, at));
            at = next;
        }
        assert_eq!(at, target);
    }

    #[test]
    fn generation_is_deterministic() {
        let a = generate(&spec(42, 8));
        let b = generate(&spec(42, 8));
        assert_eq!(a.keys, b.keys);
        assert_eq!(a.hunters, b.hunters);
        assert_eq!(a.walls().len(), b.walls().len());
    }
}

use std::{
    cmp::Ordering,
    collections::{BinaryHeap, HashMap, hash_map::Entry},
    time::Duration,
};

use concerto_ecs::{Entity, Resource};
use concerto_name::Name;
use glam::{Quat, Vec2, Vec3};
use log::warn;

use crate::point::NavLink;

/// One point as the graph is built from it: its entity, world position and rotation, anchor, `enabled` flag and links.
#[derive(Clone, Debug, PartialEq)]
pub struct NavPointRecord {
    pub entity: Entity,
    pub position: Vec3,
    pub rotation: Quat,
    pub anchor: Name,
    pub enabled: bool,
    pub links: Vec<NavLink>,
}

impl NavPointRecord {
    /// An enabled, unnamed, unrotated point at `position` with no links.
    pub fn new(entity: Entity, position: Vec3) -> Self {
        Self {
            entity,
            position,
            rotation: Quat::IDENTITY,
            anchor: Name::default(),
            enabled: true,
            links: Vec::new(),
        }
    }

    /// Sets the world rotation the point's facing is read from.
    pub fn with_rotation(mut self, rotation: Quat) -> Self {
        self.rotation = rotation;
        self
    }

    /// Sets the anchor.
    pub fn with_anchor(mut self, anchor: Name) -> Self {
        self.anchor = anchor;
        self
    }

    /// Sets whether the point can be entered.
    pub fn with_enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Adds a link.
    pub fn with_link(mut self, link: NavLink) -> Self {
        self.links.push(link);
        self
    }
}

/// A point of the graph: where it is, which way it faces, its anchor and whether it can be entered.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NavGraphPoint {
    pub entity: Entity,
    pub position: Vec3,
    pub facing: Quat,
    pub anchor: Name,
    pub enabled: bool,
}

/// An authored link between two points of the graph.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NavEdge {
    pub from: Entity,
    pub to: Entity,
    pub one_way: bool,
}

/// A route: the point it ends at, the positions it passes, and its length.
#[derive(Clone, Debug, PartialEq)]
pub struct NavPath {
    pub destination: Entity,
    pub positions: Vec<Vec3>,
    pub length: f32,
}

impl NavPath {
    /// The time to walk the path at `speed`, or `None` unless `speed` is finite and above zero.
    pub fn travel_time(&self, speed: f32) -> Option<Duration> {
        if !speed.is_finite() || speed <= 0.0 {
            return None;
        }
        Duration::try_from_secs_f32(self.length / speed).ok()
    }
}

#[derive(Clone, Copy, Debug)]
struct Step {
    to: usize,
    cost: f32,
}

#[derive(Clone, Copy, Debug)]
struct Frontier {
    estimate: f32,
    point: usize,
}

impl PartialEq for Frontier {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Frontier {}

impl PartialOrd for Frontier {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Frontier {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .estimate
            .total_cmp(&self.estimate)
            .then_with(|| other.point.cmp(&self.point))
    }
}

/// The graph of every `NavPoint` and its links, rebuilt from the components.
#[derive(Resource, Clone, Debug, Default)]
pub struct NavGraph {
    points: Vec<NavGraphPoint>,
    indices: HashMap<Entity, usize>,
    anchors: HashMap<Name, usize>,
    adjacency: Vec<Vec<Step>>,
    edges: Vec<NavEdge>,
}

impl NavGraph {
    /// Builds the graph from the points, skipping links to unknown points and to the point itself.
    pub fn build(records: impl IntoIterator<Item = NavPointRecord>) -> Self {
        let mut records: Vec<NavPointRecord> = records.into_iter().collect();
        records.sort_by_key(|record| record.entity.index());
        let mut graph = NavGraph::default();
        for record in &records {
            let index = graph.points.len();
            graph.points.push(NavGraphPoint {
                entity: record.entity,
                position: record.position,
                facing: yaw_facing(record.rotation),
                anchor: record.anchor,
                enabled: record.enabled,
            });
            graph.indices.insert(record.entity, index);
            if record.anchor != Name::default() {
                match graph.anchors.entry(record.anchor) {
                    Entry::Occupied(_) => {
                        warn!(
                            "two nav points share the anchor `{}`; the one with the lower entity index is kept",
                            record.anchor.as_str()
                        );
                        graph.points[index].anchor = Name::default();
                    }
                    Entry::Vacant(entry) => {
                        entry.insert(index);
                    }
                }
            }
        }
        graph.adjacency = vec![Vec::new(); graph.points.len()];
        for record in &records {
            let from = graph.indices[&record.entity];
            for link in &record.links {
                let Some(to) = link
                    .to
                    .entity()
                    .and_then(|to| graph.indices.get(&to).copied())
                else {
                    continue;
                };
                if to == from {
                    continue;
                }
                let cost = graph.points[from]
                    .position
                    .distance(graph.points[to].position);
                graph.adjacency[from].push(Step { to, cost });
                if !link.one_way {
                    graph.adjacency[to].push(Step { to: from, cost });
                }
                graph.edges.push(NavEdge {
                    from: record.entity,
                    to: graph.points[to].entity,
                    one_way: link.one_way,
                });
            }
        }
        graph
    }

    /// The point at `entity`.
    pub fn point(&self, entity: Entity) -> Option<&NavGraphPoint> {
        self.indices.get(&entity).map(|&index| &self.points[index])
    }

    /// The point named `anchor`, or `None` for the empty name.
    pub fn anchor(&self, anchor: Name) -> Option<Entity> {
        self.anchors
            .get(&anchor)
            .map(|&index| self.points[index].entity)
    }

    /// The enabled point nearest `position` on the horizontal plane.
    pub fn nearest(&self, position: Vec3) -> Option<Entity> {
        self.points
            .iter()
            .filter(|point| point.enabled)
            .min_by(|a, b| {
                horizontal_distance(a.position, position)
                    .total_cmp(&horizontal_distance(b.position, position))
            })
            .map(|point| point.entity)
    }

    /// The shortest route from `from` to `to` through enabled points.
    pub fn path(&self, from: Entity, to: Entity) -> Option<NavPath> {
        let start = *self.indices.get(&from)?;
        let goal = *self.indices.get(&to)?;
        if !self.points[start].enabled || !self.points[goal].enabled {
            return None;
        }
        let goal_position = self.points[goal].position;
        let mut cost = vec![f32::INFINITY; self.points.len()];
        let mut previous: Vec<Option<usize>> = vec![None; self.points.len()];
        let mut frontier = BinaryHeap::new();
        cost[start] = 0.0;
        frontier.push(Frontier {
            estimate: self.points[start].position.distance(goal_position),
            point: start,
        });
        while let Some(Frontier { estimate, point }) = frontier.pop() {
            if point == goal {
                break;
            }
            if estimate > cost[point] + self.points[point].position.distance(goal_position) {
                continue;
            }
            for step in &self.adjacency[point] {
                if !self.points[step.to].enabled {
                    continue;
                }
                let reached = cost[point] + step.cost;
                if reached < cost[step.to] {
                    cost[step.to] = reached;
                    previous[step.to] = Some(point);
                    frontier.push(Frontier {
                        estimate: reached + self.points[step.to].position.distance(goal_position),
                        point: step.to,
                    });
                }
            }
        }
        if !cost[goal].is_finite() {
            return None;
        }
        let mut route = vec![goal];
        while let Some(point) = previous[*route.last().unwrap()] {
            route.push(point);
        }
        route.reverse();
        Some(NavPath {
            destination: self.points[goal].entity,
            positions: route
                .iter()
                .map(|&index| self.points[index].position)
                .collect(),
            length: cost[goal],
        })
    }

    /// The route from the enabled point nearest `position` to `to`, its length counting the leg to that point.
    pub fn path_from(&self, position: Vec3, to: Entity) -> Option<NavPath> {
        let start = self.nearest(position)?;
        let mut path = self.path(start, to)?;
        path.length += position.distance(path.positions[0]);
        Some(path)
    }

    /// Every point.
    pub fn points(&self) -> impl Iterator<Item = &NavGraphPoint> {
        self.points.iter()
    }

    /// One edge per authored link.
    pub fn edges(&self) -> impl Iterator<Item = NavEdge> {
        self.edges.iter().copied()
    }
}

pub(crate) fn yaw_facing(rotation: Quat) -> Quat {
    let forward = rotation * Vec3::Z;
    Quat::from_rotation_y(forward.x.atan2(forward.z))
}

pub(crate) fn horizontal_distance(a: Vec3, b: Vec3) -> f32 {
    Vec2::new(a.x - b.x, a.z - b.z).length()
}

#[cfg(test)]
mod tests {
    use concerto_ecs::{Entity, component::scene::SceneEntityRef};
    use glam::Vec3;

    use super::{NavGraph, NavPointRecord};
    use crate::point::NavLink;

    fn entity(index: u32) -> Entity {
        Entity::parse_id(&format!("{index}v1")).unwrap()
    }

    fn link(to: Entity) -> NavLink {
        NavLink {
            to: SceneEntityRef::Entity(to),
            one_way: false,
        }
    }

    fn one_way(to: Entity) -> NavLink {
        NavLink {
            to: SceneEntityRef::Entity(to),
            one_way: true,
        }
    }

    struct Case {
        name: &'static str,
        points: Vec<NavPointRecord>,
        from: Entity,
        to: Entity,
        route: Option<Vec<Vec3>>,
        length: f32,
    }

    #[test]
    fn the_path_is_the_shortest_route_through_enabled_points_taking_one_way_links_only_forwards() {
        let [a, b, c, d] = [entity(1), entity(2), entity(3), entity(4)];
        let short = Vec3::new(1.0, 0.0, 1.0);
        let long = Vec3::new(1.0, 0.0, -5.0);
        let goal = Vec3::new(2.0, 0.0, 0.0);
        let shortcut = Vec3::new(1.0, 0.0, 0.0);
        let two_routes_and_a_disabled_shortcut = || {
            vec![
                NavPointRecord::new(a, Vec3::ZERO)
                    .with_link(link(b))
                    .with_link(link(c))
                    .with_link(link(d)),
                NavPointRecord::new(b, short).with_link(link(entity(5))),
                NavPointRecord::new(c, long).with_link(link(entity(5))),
                NavPointRecord::new(d, shortcut)
                    .with_enabled(false)
                    .with_link(link(entity(5))),
                NavPointRecord::new(entity(5), goal),
            ]
        };
        let far = Vec3::new(0.0, 0.0, 10.0);
        let one_way_shortcut = || {
            vec![
                NavPointRecord::new(a, Vec3::ZERO)
                    .with_link(one_way(c))
                    .with_link(link(b)),
                NavPointRecord::new(b, far).with_link(link(c)),
                NavPointRecord::new(c, Vec3::X),
            ]
        };
        let cases = [
            Case {
                name: "the shorter route, past a disabled shortcut",
                points: two_routes_and_a_disabled_shortcut(),
                from: a,
                to: entity(5),
                route: Some(vec![Vec3::ZERO, short, goal]),
                length: 2.0 * 2f32.sqrt(),
            },
            Case {
                name: "into a disabled point",
                points: two_routes_and_a_disabled_shortcut(),
                from: a,
                to: d,
                route: None,
                length: 0.0,
            },
            Case {
                name: "along a one-way link",
                points: one_way_shortcut(),
                from: a,
                to: c,
                route: Some(vec![Vec3::ZERO, Vec3::X]),
                length: 1.0,
            },
            Case {
                name: "against a one-way link",
                points: one_way_shortcut(),
                from: c,
                to: a,
                route: Some(vec![Vec3::X, far, Vec3::ZERO]),
                length: 101f32.sqrt() + 10.0,
            },
            Case {
                name: "up a height",
                points: vec![
                    NavPointRecord::new(a, Vec3::ZERO).with_link(link(b)),
                    NavPointRecord::new(b, Vec3::new(3.0, 4.0, 0.0)),
                ],
                from: a,
                to: b,
                route: Some(vec![Vec3::ZERO, Vec3::new(3.0, 4.0, 0.0)]),
                length: 5.0,
            },
            Case {
                name: "where the costs round",
                points: vec![
                    NavPointRecord::new(a, Vec3::ZERO).with_link(link(b)),
                    NavPointRecord::new(b, Vec3::new(4.0, 0.0, -1.0)).with_link(link(c)),
                    NavPointRecord::new(c, Vec3::new(4.0, 0.0, 4.0)),
                ],
                from: a,
                to: c,
                route: Some(vec![
                    Vec3::ZERO,
                    Vec3::new(4.0, 0.0, -1.0),
                    Vec3::new(4.0, 0.0, 4.0),
                ]),
                length: 17f32.sqrt() + 5.0,
            },
        ];
        for case in cases {
            let path = NavGraph::build(case.points).path(case.from, case.to);
            assert_eq!(
                path.as_ref().map(|path| path.positions.clone()),
                case.route,
                "{}",
                case.name
            );
            if let Some(path) = path {
                assert_eq!(path.destination, case.to, "{}", case.name);
                assert!((path.length - case.length).abs() < 1e-4, "{}", case.name);
            }
        }
    }
}

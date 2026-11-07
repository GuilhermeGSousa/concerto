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

/// A route over the graph: the points it passes, their positions, and its length.
#[derive(Clone, Debug, PartialEq)]
pub struct NavPath {
    pub points: Vec<Entity>,
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
            points: route
                .iter()
                .map(|&index| self.points[index].entity)
                .collect(),
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
    use std::time::Duration;

    use concerto_ecs::{Entity, component::scene::SceneEntityRef};
    use concerto_name::Name;
    use glam::{Quat, Vec3};

    use super::{NavEdge, NavGraph, NavPath, NavPointRecord};
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

    #[test]
    fn a_two_way_link_makes_one_edge_walkable_both_ways() {
        let [a, b] = [entity(1), entity(2)];
        let graph = NavGraph::build([
            NavPointRecord::new(a, Vec3::ZERO).with_link(link(b)),
            NavPointRecord::new(b, Vec3::X),
        ]);
        assert_eq!(
            graph.edges().collect::<Vec<_>>(),
            vec![NavEdge {
                from: a,
                to: b,
                one_way: false
            }]
        );
        assert_eq!(graph.path(a, b).unwrap().points, vec![a, b]);
        assert_eq!(graph.path(b, a).unwrap().points, vec![b, a]);
    }

    #[test]
    fn a_one_way_link_makes_one_directed_edge() {
        let [a, b] = [entity(1), entity(2)];
        let graph = NavGraph::build([
            NavPointRecord::new(a, Vec3::ZERO).with_link(one_way(b)),
            NavPointRecord::new(b, Vec3::X),
        ]);
        assert_eq!(
            graph.edges().collect::<Vec<_>>(),
            vec![NavEdge {
                from: a,
                to: b,
                one_way: true
            }]
        );
        assert_eq!(graph.path(a, b).unwrap().points, vec![a, b]);
        assert_eq!(graph.path(b, a), None);
    }

    #[test]
    fn unresolved_self_and_non_point_links_are_skipped() {
        let [a, b, stranger] = [entity(1), entity(2), entity(3)];
        let graph = NavGraph::build([
            NavPointRecord::new(a, Vec3::ZERO)
                .with_link(NavLink::default())
                .with_link(link(a))
                .with_link(link(stranger))
                .with_link(link(b)),
            NavPointRecord::new(b, Vec3::X),
        ]);
        assert_eq!(
            graph.edges().collect::<Vec<_>>(),
            vec![NavEdge {
                from: a,
                to: b,
                one_way: false
            }]
        );
        assert_eq!(graph.path(a, stranger), None);
    }

    #[test]
    fn a_duplicate_anchor_keeps_the_lower_entity() {
        let name = Name::new("nav_duplicate");
        let [low, high] = [entity(4), entity(9)];
        let graph = NavGraph::build([
            NavPointRecord::new(high, Vec3::X).with_anchor(name),
            NavPointRecord::new(low, Vec3::ZERO).with_anchor(name),
        ]);
        assert_eq!(graph.anchor(name), Some(low));
        assert_eq!(graph.point(low).unwrap().anchor, name);
        assert_eq!(graph.point(high).unwrap().anchor, Name::default());
        assert_eq!(
            graph.points().filter(|point| point.anchor == name).count(),
            1
        );
    }

    #[test]
    fn the_empty_anchor_names_no_point() {
        let graph = NavGraph::build([NavPointRecord::new(entity(1), Vec3::ZERO)]);
        assert_eq!(graph.anchor(Name::default()), None);
    }

    #[test]
    fn a_point_faces_the_yaw_of_its_rotation_applied_to_plus_z() {
        let a = entity(1);
        let rotation = Quat::from_rotation_y(0.7) * Quat::from_rotation_x(0.4);
        let graph = NavGraph::build([NavPointRecord::new(a, Vec3::ZERO).with_rotation(rotation)]);
        let facing = graph.point(a).unwrap().facing;
        assert!(facing.angle_between(Quat::from_rotation_y(0.7)) < 1e-4);
    }

    #[test]
    fn the_shorter_of_two_routes_is_chosen() {
        let [start, short, long, goal] = [entity(1), entity(2), entity(3), entity(4)];
        let graph = NavGraph::build([
            NavPointRecord::new(start, Vec3::ZERO)
                .with_link(link(short))
                .with_link(link(long)),
            NavPointRecord::new(short, Vec3::new(1.0, 0.0, 1.0)).with_link(link(goal)),
            NavPointRecord::new(long, Vec3::new(1.0, 0.0, -5.0)).with_link(link(goal)),
            NavPointRecord::new(goal, Vec3::new(2.0, 0.0, 0.0)),
        ]);
        let path = graph.path(start, goal).unwrap();
        assert_eq!(path.points, vec![start, short, goal]);
        assert_eq!(
            path.positions,
            vec![
                Vec3::ZERO,
                Vec3::new(1.0, 0.0, 1.0),
                Vec3::new(2.0, 0.0, 0.0)
            ]
        );
        assert!((path.length - 2.0 * 2f32.sqrt()).abs() < 1e-5);
    }

    #[test]
    fn a_route_whose_costs_round_is_still_found() {
        let [start, middle, goal] = [entity(1), entity(2), entity(3)];
        let graph = NavGraph::build([
            NavPointRecord::new(start, Vec3::ZERO).with_link(link(middle)),
            NavPointRecord::new(middle, Vec3::new(4.0, 0.0, -1.0)).with_link(link(goal)),
            NavPointRecord::new(goal, Vec3::new(4.0, 0.0, 4.0)),
        ]);
        assert_eq!(
            graph.path(start, goal).map(|path| path.points),
            Some(vec![start, middle, goal])
        );
    }

    #[test]
    fn edge_cost_counts_height() {
        let [a, b] = [entity(1), entity(2)];
        let graph = NavGraph::build([
            NavPointRecord::new(a, Vec3::ZERO).with_link(link(b)),
            NavPointRecord::new(b, Vec3::new(3.0, 4.0, 0.0)),
        ]);
        assert!((graph.path(a, b).unwrap().length - 5.0).abs() < 1e-5);
    }

    #[test]
    fn a_one_way_link_is_followed_only_forwards() {
        let [a, b, c] = [entity(1), entity(2), entity(3)];
        let graph = NavGraph::build([
            NavPointRecord::new(a, Vec3::ZERO)
                .with_link(one_way(c))
                .with_link(link(b)),
            NavPointRecord::new(b, Vec3::new(0.0, 0.0, 10.0)).with_link(link(c)),
            NavPointRecord::new(c, Vec3::X),
        ]);
        assert_eq!(graph.path(a, c).unwrap().points, vec![a, c]);
        assert_eq!(graph.path(c, a).unwrap().points, vec![c, b, a]);
    }

    #[test]
    fn there_is_no_path_without_a_route() {
        let [a, b] = [entity(1), entity(2)];
        let graph = NavGraph::build([
            NavPointRecord::new(a, Vec3::ZERO),
            NavPointRecord::new(b, Vec3::X),
        ]);
        assert_eq!(graph.path(a, b), None);
    }

    #[test]
    fn there_is_no_path_to_or_through_a_disabled_point() {
        let [a, b, c] = [entity(1), entity(2), entity(3)];
        let graph = NavGraph::build([
            NavPointRecord::new(a, Vec3::ZERO).with_link(link(b)),
            NavPointRecord::new(b, Vec3::X)
                .with_enabled(false)
                .with_link(link(c)),
            NavPointRecord::new(c, Vec3::X * 2.0),
        ]);
        assert_eq!(graph.path(a, b), None);
        assert_eq!(graph.path(a, c), None);
    }

    #[test]
    fn the_path_to_the_same_point_has_length_zero() {
        let a = entity(1);
        let graph = NavGraph::build([NavPointRecord::new(a, Vec3::ONE)]);
        assert_eq!(
            graph.path(a, a),
            Some(NavPath {
                points: vec![a],
                positions: vec![Vec3::ONE],
                length: 0.0
            })
        );
    }

    #[test]
    fn nearest_skips_disabled_points_and_ignores_height() {
        let [near_disabled, high, far] = [entity(1), entity(2), entity(3)];
        let graph = NavGraph::build([
            NavPointRecord::new(near_disabled, Vec3::ZERO).with_enabled(false),
            NavPointRecord::new(high, Vec3::new(1.0, 50.0, 0.0)),
            NavPointRecord::new(far, Vec3::new(3.0, 0.0, 0.0)),
        ]);
        assert_eq!(graph.nearest(Vec3::ZERO), Some(high));
    }

    #[test]
    fn path_from_starts_at_the_nearest_enabled_point_and_counts_the_leg_to_it() {
        let [disabled, a, b] = [entity(1), entity(2), entity(3)];
        let graph = NavGraph::build([
            NavPointRecord::new(disabled, Vec3::new(0.0, 0.0, 0.5)).with_enabled(false),
            NavPointRecord::new(a, Vec3::new(0.0, 0.0, 3.0)).with_link(link(b)),
            NavPointRecord::new(b, Vec3::new(4.0, 0.0, 3.0)),
        ]);
        let path = graph.path_from(Vec3::ZERO, b).unwrap();
        assert_eq!(path.points, vec![a, b]);
        assert_eq!(
            path.positions,
            vec![Vec3::new(0.0, 0.0, 3.0), Vec3::new(4.0, 0.0, 3.0)]
        );
        assert!((path.length - 7.0).abs() < 1e-5);
    }

    #[test]
    fn travel_time_is_length_over_speed() {
        let path = NavPath {
            points: Vec::new(),
            positions: Vec::new(),
            length: 6.0,
        };
        assert_eq!(path.travel_time(1.5), Some(Duration::from_secs(4)));
    }

    #[test]
    fn travel_time_needs_a_finite_positive_speed() {
        let path = NavPath {
            points: Vec::new(),
            positions: Vec::new(),
            length: 6.0,
        };
        for speed in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            assert_eq!(path.travel_time(speed), None, "speed {speed}");
        }
    }
}

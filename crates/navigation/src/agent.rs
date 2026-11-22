use concerto_ecs::{
    CommandQueue, Component, Entity, Or, Query, Ref, Res, With, Without, World,
    events::{Event, event_writer::EventWriter},
    query::change_detection::DetectChanges,
    reflect::{ReflectComponent, ReflectSceneComponent},
    system::input::SystemLocal,
    world::FromWorld,
};
use concerto_foundation::{
    assets::{AssetId, asset_server::AssetServer, asset_store::AssetStore},
    transform::GlobalTransform,
};
use concerto_name::Name;
use concerto_reflect::{Reflect, ReflectDefault};
use glam::{Quat, Vec2, Vec3};
use log::warn;

use crate::{
    asset::NavMesh,
    component::NavMeshComponent,
    graph::{NavGraph, NavGraphPoint, NavPath, horizontal_distance, yaw_facing},
    query::NavMeshQuery,
};

/// An entity that walks the nav graph; its steering is written to `NavSteering`.
#[derive(Component, Reflect, Clone, Debug, PartialEq)]
#[reflect(Component, Default, SceneComponent)]
pub struct NavAgent {
    pub speed: f32,
    pub arrival_radius: f32,
    pub slow_radius: f32,
}

impl NavAgent {
    /// An agent walking at 1.4 m/s that arrives within 0.25 m and slows inside 1.5 m.
    pub fn new() -> Self {
        Self {
            speed: 1.4,
            arrival_radius: 0.25,
            slow_radius: 1.5,
        }
    }

    /// Sets the speed used when a destination names none.
    pub fn with_speed(mut self, speed: f32) -> Self {
        self.speed = speed;
        self
    }

    /// Sets how close a waypoint must be to count as reached.
    pub fn with_arrival_radius(mut self, arrival_radius: f32) -> Self {
        self.arrival_radius = arrival_radius;
        self
    }

    /// Sets the distance from the last waypoint inside which the agent slows down.
    pub fn with_slow_radius(mut self, slow_radius: f32) -> Self {
        self.slow_radius = slow_radius;
        self
    }
}

impl Default for NavAgent {
    fn default() -> Self {
        Self::new()
    }
}

/// What an agent is walking to: a point's entity or its anchor.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum NavTarget {
    Point(Entity),
    Anchor(Name),
}

/// Where an agent is going; remove it to stop.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct NavDestination {
    pub target: NavTarget,
    pub speed: Option<f32>,
}

impl NavDestination {
    /// Walks to the point at `point`, at the agent's speed.
    pub fn to_point(point: Entity) -> Self {
        Self {
            target: NavTarget::Point(point),
            speed: None,
        }
    }

    /// Walks to the point named `anchor`, at the agent's speed.
    pub fn to_anchor(anchor: Name) -> Self {
        Self {
            target: NavTarget::Anchor(anchor),
            speed: None,
        }
    }

    /// Walks at `speed` instead of the agent's speed.
    pub fn with_speed(mut self, speed: f32) -> Self {
        self.speed = Some(speed);
        self
    }
}

/// The velocity and facing an agent's navigation wants; present once the agent first plans.
#[derive(Component, Reflect, Clone, Copy, Debug, Default, PartialEq)]
#[reflect(Component, Default)]
pub struct NavSteering {
    pub velocity: Vec3,
    pub facing: Quat,
}

/// The route an agent is following; read by gizmos.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct NavRoute {
    path: NavPath,
    next: usize,
    planned_on: Option<AssetId>,
}

impl NavRoute {
    /// The waypoints still ahead, the next one first.
    pub fn remaining(&self) -> &[Vec3] {
        &self.path.positions[self.next..]
    }

    /// The point the route ends at.
    pub fn destination(&self) -> Entity {
        self.path.destination
    }

    /// The navmesh asset the route was planned on, or `None` for the graph.
    pub fn planned_on(&self) -> Option<AssetId> {
        self.planned_on
    }
}

/// An agent reached the point it was sent to.
#[derive(Event, Clone, Copy, Debug, PartialEq)]
pub struct NavArrived {
    pub agent: Entity,
    pub point: Entity,
}

/// No route leads an agent to its target; its destination was removed.
#[derive(Event, Clone, Copy, Debug, PartialEq)]
pub struct NavUnreachable {
    pub agent: Entity,
    pub target: NavTarget,
}

/// Stops each agent on a route that lost its `NavDestination` or `NavAgent`, keeping its facing.
pub fn stop_nav_agents(
    mut agents: Query<
        (Entity, &mut NavSteering),
        (
            With<NavRoute>,
            Or<(Without<NavDestination>, Without<NavAgent>)>,
        ),
    >,
    mut cmd: CommandQueue,
) {
    for (agent, mut steering) in agents.iter_mut() {
        steering.velocity = Vec3::ZERO;
        cmd.remove::<NavRoute>(agent);
    }
}

/// Remembers that the choice between several [`NavMeshComponent`]s was reported.
pub struct MeshChoiceWarned {
    warned: bool,
}

impl FromWorld for MeshChoiceWarned {
    fn from_world(_: &World) -> Self {
        Self { warned: false }
    }
}

struct ActiveMesh<'a> {
    entity: Entity,
    id: AssetId,
    query: &'a NavMeshQuery,
}

fn loaded_query<'a>(
    navmeshes: &'a AssetStore<NavMesh>,
    component: &NavMeshComponent,
) -> Option<&'a NavMeshQuery> {
    let navmesh = navmeshes.get(&component.handle)?;
    if navmesh.data()?.polygons.is_empty() {
        return None;
    }
    navmesh.query()
}

fn mesh_path(query: &NavMeshQuery, from: Vec3, point: &NavGraphPoint) -> Option<NavPath> {
    if !point.enabled {
        return None;
    }
    let mut positions = query.path(from, point.position)?;
    if positions.is_empty() {
        positions.push(point.position);
    }
    let length = positions
        .iter()
        .scan(from, |previous, corner| {
            let leg = previous.distance(*corner);
            *previous = *corner;
            Some(leg)
        })
        .sum();
    Some(NavPath {
        destination: point.entity,
        positions,
        length,
    })
}

/// Plans a route for each agent without one, whose destination changed, whose route the changed graph may have broken, or whose route was planned on another navmesh than the one loaded now; while a `NavMeshComponent`'s mesh is still loading and none is loaded, an agent without a route, or whose destination changed, stops and waits, and every other route is kept as it is.
pub fn plan_nav_routes(
    mut agents: Query<
        (
            Entity,
            Ref<NavDestination>,
            &GlobalTransform,
            Option<&NavRoute>,
            Option<&mut NavSteering>,
        ),
        With<NavAgent>,
    >,
    components: Query<(Entity, &NavMeshComponent)>,
    navmeshes: Res<AssetStore<NavMesh>>,
    server: Res<AssetServer>,
    graph: Res<NavGraph>,
    mut warned: SystemLocal<MeshChoiceWarned>,
    mut unreachable: EventWriter<NavUnreachable>,
    mut cmd: CommandQueue,
) {
    let count = components.iter().count();
    if count > 1 && !warned.warned {
        warned.warned = true;
        warn!(
            "{count} entities hold a NavMeshComponent; agents route on the one with the lowest entity index whose navmesh is loaded"
        );
    }
    let mesh = components
        .iter()
        .filter_map(|(entity, component)| {
            Some(ActiveMesh {
                entity,
                id: component.handle.id(),
                query: loaded_query(&navmeshes, component)?,
            })
        })
        .min_by_key(|mesh| mesh.entity.index());
    let mesh_id = mesh.as_ref().map(|mesh| mesh.id);
    let mesh_loading = mesh.is_none()
        && components.iter().any(|(_, component)| {
            navmeshes.get(&component.handle).is_none()
                && server.is_loading_or_loaded(component.handle.id())
        });
    for (agent, destination, transform, route, steering) in agents.iter_mut() {
        match route {
            None if mesh_loading => continue,
            None => {}
            Some(_) if mesh_loading && destination.is_changed() => {
                cmd.remove::<NavRoute>(agent);
                if let Some(mut steering) = steering {
                    steering.velocity = Vec3::ZERO;
                }
                continue;
            }
            Some(_) if mesh_loading => continue,
            Some(route) => {
                let stale = route.planned_on != mesh_id || graph.is_changed();
                if !destination.is_changed() && !stale {
                    continue;
                }
            }
        }
        let point = match destination.target {
            NavTarget::Point(point) => Some(point),
            NavTarget::Anchor(anchor) => graph.anchor(anchor),
        };
        let path = match &mesh {
            Some(mesh) => point
                .and_then(|point| graph.point(point))
                .and_then(|point| mesh_path(mesh.query, transform.translation(), point)),
            None => point.and_then(|point| graph.path_from(transform.translation(), point)),
        };
        match path {
            Some(path) => {
                cmd.insert(
                    NavRoute {
                        path,
                        next: 0,
                        planned_on: mesh_id,
                    },
                    agent,
                );
                if steering.is_none() {
                    cmd.insert(
                        NavSteering {
                            velocity: Vec3::ZERO,
                            facing: yaw_facing(transform.rotation()),
                        },
                        agent,
                    );
                }
            }
            None => {
                cmd.remove::<NavDestination>(agent);
                cmd.remove::<NavRoute>(agent);
                if let Some(mut steering) = steering {
                    steering.velocity = Vec3::ZERO;
                }
                unreachable.write(NavUnreachable {
                    agent,
                    target: destination.target,
                });
            }
        }
    }
}

/// Moves each agent's `NavSteering` along its route, and stops it on the last point.
pub fn steer_nav_agents(
    mut agents: Query<(
        Entity,
        &NavAgent,
        &NavDestination,
        &GlobalTransform,
        &mut NavRoute,
        &mut NavSteering,
    )>,
    graph: Res<NavGraph>,
    mut arrived: EventWriter<NavArrived>,
    mut cmd: CommandQueue,
) {
    for (agent, settings, destination, transform, mut route, mut steering) in agents.iter_mut() {
        let position = transform.translation();
        let last = route.path.positions.len() - 1;
        while route.next < last && waypoint_reached(&route, position, settings.arrival_radius) {
            route.next += 1;
        }
        let target = route.path.positions[route.next];
        let distance = horizontal_distance(position, target);
        if route.next == last && distance <= settings.arrival_radius {
            let point = route.destination();
            steering.velocity = Vec3::ZERO;
            if let Some(point) = graph.point(point) {
                steering.facing = point.facing;
            }
            cmd.remove::<NavDestination>(agent);
            cmd.remove::<NavRoute>(agent);
            arrived.write(NavArrived { agent, point });
            continue;
        }
        if distance <= f32::EPSILON {
            steering.velocity = Vec3::ZERO;
            continue;
        }
        let mut speed = destination.speed.unwrap_or(settings.speed);
        if route.next == last && distance < settings.slow_radius {
            speed *= distance / settings.slow_radius;
        }
        let direction = Vec3::new(target.x - position.x, 0.0, target.z - position.z) / distance;
        steering.velocity = direction * speed;
        steering.facing = Quat::from_rotation_y(direction.x.atan2(direction.z));
    }
}

fn waypoint_reached(route: &NavRoute, position: Vec3, arrival_radius: f32) -> bool {
    let positions = &route.path.positions;
    let waypoint = positions[route.next];
    if horizontal_distance(position, waypoint) <= arrival_radius {
        return true;
    }
    if route.next == 0 {
        return false;
    }
    let from = positions[route.next - 1];
    let segment = Vec2::new(waypoint.x - from.x, waypoint.z - from.z);
    let along = Vec2::new(position.x - from.x, position.z - from.z);
    let length_squared = segment.length_squared();
    length_squared > f32::EPSILON && along.dot(segment) > length_squared
}

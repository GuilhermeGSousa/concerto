use std::collections::HashSet;

use concerto_app::{edit_time::AtEditTime, plugins::Plugin, schedule_groups::LateUpdate};
use concerto_color::Color;
use concerto_debug_gizmos::{DebugGizmos, DebugGizmosPlugin};
use concerto_ecs::{IntoSystemConfig, Query, Res, Resource};
use concerto_foundation::{assets::asset_store::AssetStore, transform::GlobalTransform};
use concerto_navigation::{
    NavGraph, NavGraphPoint, NavMesh, NavMeshComponent, NavMeshData, NavRoute, rebuild_nav_graph,
};
use concerto_render::plugin::RenderPlugin;
use glam::Vec3;

pub(crate) const ENABLED_COLOR: Color = Color::rgba(0.2, 0.9, 0.3, 1.0);
pub(crate) const DISABLED_COLOR: Color = Color::rgba(0.6, 0.6, 0.6, 1.0);
pub(crate) const LINK_COLOR: Color = Color::rgba(0.3, 0.7, 1.0, 1.0);
pub(crate) const ROUTE_COLOR: Color = Color::rgba(1.0, 0.4, 0.9, 1.0);
pub(crate) const MESH_COLOR: Color = Color::rgba(1.0, 0.75, 0.2, 1.0);
pub(crate) const MESH_LIFT: f32 = 0.02;
pub(crate) const POINT_RADIUS: f32 = 0.3;
pub(crate) const FACING_LENGTH: f32 = 0.6;
pub(crate) const ARROW_LENGTH: f32 = 0.3;
pub(crate) const ARROW_WIDTH: f32 = 0.15;

/// Which navigation layers are drawn: the nav graph, each agent's route, and each loaded navmesh.
#[derive(Resource, Clone, Debug, PartialEq)]
pub struct NavigationGizmos {
    pub graph: bool,
    pub routes: bool,
    pub mesh: bool,
}

impl Default for NavigationGizmos {
    fn default() -> Self {
        Self {
            graph: true,
            routes: true,
            mesh: true,
        }
    }
}

/// Draws the nav graph, agents' routes and loaded navmeshes; register it after `DefaultPlugins`.
pub struct NavigationGizmosPlugin;

impl Plugin for NavigationGizmosPlugin {
    fn build(&self, app: &mut concerto_app::App) {
        app.insert_resource(NavigationGizmos::default());
        if app.is_plugin_added::<RenderPlugin>() && !app.is_plugin_added::<DebugGizmosPlugin>() {
            app.register_plugin(DebugGizmosPlugin);
        }
        if app.is_plugin_added::<DebugGizmosPlugin>() {
            app.add_system(
                LateUpdate,
                draw_nav_graph.after(rebuild_nav_graph).at_edit_time(),
            );
            app.add_system(LateUpdate, draw_nav_routes.after(rebuild_nav_graph));
            app.add_system(LateUpdate, draw_nav_mesh.at_edit_time());
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum NavShape {
    Circle {
        center: Vec3,
        color: Color,
    },
    Line {
        start: Vec3,
        end: Vec3,
        color: Color,
    },
}

pub(crate) fn point_shapes(point: &NavGraphPoint) -> Vec<NavShape> {
    let color = if point.enabled {
        ENABLED_COLOR
    } else {
        DISABLED_COLOR
    };
    vec![
        NavShape::Circle {
            center: point.position,
            color,
        },
        NavShape::Line {
            start: point.position,
            end: point.position + point.facing * Vec3::Z * FACING_LENGTH,
            color,
        },
    ]
}

pub(crate) fn link_shapes(from: Vec3, to: Vec3, one_way: bool) -> Vec<NavShape> {
    let mut shapes = vec![NavShape::Line {
        start: from,
        end: to,
        color: LINK_COLOR,
    }];
    let Some(direction) = (to - from).try_normalize() else {
        return shapes;
    };
    if one_way {
        let side = direction.cross(Vec3::Y).try_normalize().unwrap_or(Vec3::X) * ARROW_WIDTH;
        let tip = to - direction * POINT_RADIUS;
        let back = tip - direction * ARROW_LENGTH;
        shapes.extend([back + side, back - side].map(|end| NavShape::Line {
            start: tip,
            end,
            color: LINK_COLOR,
        }));
    }
    shapes
}

pub(crate) fn graph_shapes(graph: &NavGraph) -> Vec<NavShape> {
    let points = graph.points().flat_map(point_shapes);
    let links =
        graph.edges().flat_map(
            |edge| match (graph.point(edge.from), graph.point(edge.to)) {
                (Some(from), Some(to)) => link_shapes(from.position, to.position, edge.one_way),
                _ => Vec::new(),
            },
        );
    points.chain(links).collect()
}

pub(crate) fn route_shapes(start: Vec3, remaining: &[Vec3]) -> Vec<NavShape> {
    let waypoints: Vec<Vec3> = std::iter::once(start)
        .chain(remaining.iter().copied())
        .collect();
    waypoints
        .windows(2)
        .map(|pair| NavShape::Line {
            start: pair[0],
            end: pair[1],
            color: ROUTE_COLOR,
        })
        .collect()
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct MeshEdge {
    low: u32,
    high: u32,
}

impl MeshEdge {
    fn new(a: u32, b: u32) -> Self {
        Self {
            low: a.min(b),
            high: a.max(b),
        }
    }
}

pub(crate) fn mesh_shapes(data: &NavMeshData) -> Vec<NavShape> {
    let mut drawn = HashSet::new();
    let mut shapes = Vec::new();
    for polygon in &data.polygons {
        let count = polygon.vertices.len();
        for (index, &a) in polygon.vertices.iter().enumerate() {
            let b = polygon.vertices[(index + 1) % count];
            if !drawn.insert(MeshEdge::new(a, b)) {
                continue;
            }
            let (Some(start), Some(end)) =
                (data.vertices.get(a as usize), data.vertices.get(b as usize))
            else {
                continue;
            };
            shapes.push(NavShape::Line {
                start: *start + Vec3::Y * MESH_LIFT,
                end: *end + Vec3::Y * MESH_LIFT,
                color: MESH_COLOR,
            });
        }
    }
    shapes
}

fn draw_shape(gizmos: &mut DebugGizmos, shape: NavShape) {
    match shape {
        NavShape::Circle { center, color } => gizmos.circle(center, Vec3::Y, POINT_RADIUS, color),
        NavShape::Line { start, end, color } => gizmos.line(start, end, color),
    }
}

/// Draws every nav point and link while `NavigationGizmos::graph` is on, in the editor too.
pub fn draw_nav_graph(graph: Res<NavGraph>, shown: Res<NavigationGizmos>, mut gizmos: DebugGizmos) {
    if !shown.graph {
        return;
    }
    for shape in graph_shapes(&graph) {
        draw_shape(&mut gizmos, shape);
    }
}

/// Draws each agent's remaining route while `NavigationGizmos::routes` is on.
pub fn draw_nav_routes(
    agents: Query<(&GlobalTransform, &NavRoute)>,
    shown: Res<NavigationGizmos>,
    mut gizmos: DebugGizmos,
) {
    if !shown.routes {
        return;
    }
    for (transform, route) in agents.iter() {
        for shape in route_shapes(transform.translation(), route.remaining()) {
            draw_shape(&mut gizmos, shape);
        }
    }
}

/// Draws the polygon edges of each loaded navmesh a `NavMeshComponent` holds while `NavigationGizmos::mesh` is on, in the editor too.
pub fn draw_nav_mesh(
    components: Query<&NavMeshComponent>,
    navmeshes: Res<AssetStore<NavMesh>>,
    shown: Res<NavigationGizmos>,
    mut gizmos: DebugGizmos,
) {
    if !shown.mesh {
        return;
    }
    let mut drawn = HashSet::new();
    for component in components.iter() {
        if !drawn.insert(component.handle.id()) {
            continue;
        }
        let Some(data) = navmeshes.get(&component.handle).and_then(NavMesh::data) else {
            continue;
        };
        for shape in mesh_shapes(data) {
            draw_shape(&mut gizmos, shape);
        }
    }
}

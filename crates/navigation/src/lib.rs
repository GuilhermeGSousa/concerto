//! Navigation for NPCs: authored points and the links between them, navmeshes baked from a scene's
//! static colliders, and agents steered along routes over either.

/// Agents, their destinations, and the systems that plan their routes and steer them.
pub mod agent;
/// The navmesh asset, baked from the static colliders of scenes.
pub mod asset;
mod bake;
/// The component that places a navmesh in a level, and its status.
pub mod component;
/// The graph of nav points and links, and the routes found on it.
pub mod graph;
/// Navmesh data, bake settings and bake errors.
pub mod navmesh;
/// The plugin that adds navigation to an app.
pub mod plugin;
/// Nav points and the links between them, as authored in a scene.
pub mod point;
mod query;
mod scene_colliders;

pub use agent::{
    MeshChoiceWarned, NavAgent, NavArrived, NavDestination, NavRoute, NavSteering, NavTarget,
    NavUnreachable, plan_nav_routes, steer_nav_agents, stop_nav_agents,
};
pub use asset::NavMesh;
pub use bake::bake;
pub use component::{NavMeshComponent, NavMeshStatus, update_nav_mesh_status};
pub use graph::{NavEdge, NavGraph, NavGraphPoint, NavPath, NavPointRecord};
pub use navmesh::{NavMeshBakeError, NavMeshData, NavMeshGeometry, NavMeshSettings, NavPolygon};
pub use plugin::{NavigationPlugin, rebuild_nav_graph};
pub use point::{NavLink, NavLinks, NavPoint};
pub use query::NavMeshQuery;

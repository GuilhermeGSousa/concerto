//! The debug drawing of navigation: the nav graph's points and links, each agent's route, and each loaded navmesh's polygons.

mod gizmos;

pub use gizmos::{
    NavMeshGizmo, NavigationGizmos, NavigationGizmosPlugin, draw_nav_graph, draw_nav_mesh,
    draw_nav_routes,
};

#[cfg(test)]
mod tests;

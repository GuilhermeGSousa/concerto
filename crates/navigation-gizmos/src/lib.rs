//! The debug drawing of navigation: the nav graph's points and links, and each agent's route.

mod gizmos;

pub use gizmos::{NavigationGizmos, NavigationGizmosPlugin, draw_nav_graph, draw_nav_routes};

#[cfg(test)]
mod tests;

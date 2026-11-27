//! Scenes: the [`Scene`](scene::Scene) asset, spawning it into a world and capturing it back, and
//! [`SceneLink`](link::SceneLink)s that place one scene inside another with changes to its nodes.

/// Reading entities back into a [`Scene`](scene::Scene).
pub mod capture;
/// [`SceneLink`](link::SceneLink), a placed scene, and the components and events of the nodes it
/// spawns.
pub mod link;
/// [`LinkPath`](link_path::LinkPath), the address of a node inside nested links.
pub mod link_path;
/// [`NodeId`](node_id::NodeId), a node's stable id within its scene.
pub mod node_id;
/// [`ScenePlugin`](plugin::ScenePlugin), which spawns scene links and the startup scene.
pub mod plugin;
/// The [`Scene`](scene::Scene) asset and its nodes.
pub mod scene;
/// Spawning a [`Scene`](scene::Scene)'s nodes as entities.
pub mod spawner;

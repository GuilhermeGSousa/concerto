use concerto_ecs::{CommandQueue, Entity};
use concerto_foundation::assets::{
    handle::AssetHandle, Asset, AssetId, CreatableAsset, LoadableAsset, PlaceableAsset,
};
use concerto_reflect::{Reflect, ReflectedValue};
use serde::{Deserialize, Serialize};

use crate::link_path::LinkPath;
use crate::node_id::NodeId;

/// One component as scene data: the registry key it was registered under plus
/// its reflected value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SerializedComponent {
    /// The component's type path, as its registration names it.
    pub type_name: String,
    /// The component's reflected value.
    pub value: ReflectedValue,
}

/// One node in a [`Scene`]: an id, a name, the indices of its children, and its components.
///
/// Every runtime concern (transform, mesh, material, camera, light, ...) is
/// carried as a [`SerializedComponent`]; the spawner applies them generically
/// through the component registry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SceneNode {
    /// Names this node within its scene, whatever its name or index.
    pub id: NodeId,
    /// The node's name, given to the spawned entity.
    pub name: String,
    /// Indices into `Scene::nodes`.
    pub children: Vec<usize>,
    /// The components the spawned entity gets.
    pub components: Vec<SerializedComponent>,
    /// The scene this node links to and the changes made to its nodes, when it is a link.
    pub link: Option<SceneLinkData>,
}

/// A link stored on a [`SceneNode`]: the linked scene and the changes made to its nodes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SceneLinkData {
    /// The id of the linked scene.
    pub scene: AssetId,
    /// The changes the link makes to the linked scene's nodes, in the order they apply.
    pub changes: Vec<LinkChange>,
}

/// One change a link makes to a node of its scene, addressed by the node's path of ids from the link.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum LinkChange {
    /// Adds a component to the node, or replaces the one of the same type.
    AddComponent {
        /// The path of the node.
        path: LinkPath,
        /// The component added.
        component: SerializedComponent,
    },
    /// Removes a component from the node.
    RemoveComponent {
        /// The path of the node.
        path: LinkPath,
        /// The type path of the component removed.
        type_name: String,
    },
    /// Leaves the node, and its descendants, out of the spawn.
    RemoveNode {
        /// The path of the node.
        path: LinkPath,
    },
}

impl LinkChange {
    /// The path of the linked node this change applies to.
    pub fn path(&self) -> &LinkPath {
        match self {
            LinkChange::AddComponent { path, .. }
            | LinkChange::RemoveComponent { path, .. }
            | LinkChange::RemoveNode { path } => path,
        }
    }
}

impl SceneNode {
    /// Stores `component` through reflection and appends it to this node.
    ///
    /// It is keyed by its full type path, the name its registration resolves.
    ///
    /// # Errors
    ///
    /// Fails when `component` cannot be converted to a reflected value.
    pub fn push_component<T: Reflect>(&mut self, component: &T) -> anyhow::Result<()> {
        self.components.push(SerializedComponent {
            type_name: std::any::type_name::<T>().to_owned(),
            value: concerto_reflect::to_value(component)?,
        });
        Ok(())
    }
}

/// A format-agnostic scene graph, serialized directly as its own asset.
///
/// `nodes[0]` is not special: roots are the nodes no other node lists as a child.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Scene {
    /// The scene's nodes; a node's children are indices into this list.
    pub nodes: Vec<SceneNode>,
    /// Every [`AssetId`] reachable from the nodes' components, so
    /// [`Asset::referenced_sub_assets`] need not decode them. Capture fills it
    /// from the handles it finds; importers record the ids as they emit.
    pub referenced_assets: Vec<AssetId>,
}

impl Asset for Scene {
    const FORMAT_VERSION: u32 = 2;

    fn name() -> &'static str {
        "Scene"
    }

    fn referenced_sub_assets(&self) -> Vec<AssetId> {
        let mut ids = self.referenced_assets.clone();
        for link in self.nodes.iter().filter_map(|node| node.link.as_ref()) {
            if !ids.contains(&link.scene) {
                ids.push(link.scene);
            }
        }
        ids
    }
}

impl CreatableAsset for Scene {
    fn new_asset() -> Self {
        Scene {
            nodes: vec![],
            referenced_assets: vec![],
        }
    }
}

impl LoadableAsset for Scene {}

impl PlaceableAsset for Scene {
    fn place(_asset: &Self, handle: AssetHandle<Self>, entity: Entity, cmd: &mut CommandQueue) {
        cmd.insert(crate::link::SceneLink::new(handle), entity);
    }
}

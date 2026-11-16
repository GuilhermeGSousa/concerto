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
    pub type_name: String,
    pub value: ReflectedValue,
}

/// One node in a [`Scene`]: an id, a name, the indices of its children within
/// `Scene::nodes`, and the list of components authored onto it. Every
/// runtime concern (transform, mesh, material, camera, light, ...) is carried
/// as a [`SerializedComponent`]; the spawner applies them generically through
/// the component registry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SceneNode {
    /// Names this node within its scene, whatever its name or index.
    pub id: NodeId,
    pub name: String,
    /// Indices into `Scene::nodes`.
    pub children: Vec<usize>,
    pub components: Vec<SerializedComponent>,
    /// The scene this node links to and the changes made to its nodes, when it is a link.
    pub link: Option<SceneLinkData>,
}

/// A link stored on a [`SceneNode`]: the linked scene and the changes made to its nodes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SceneLinkData {
    pub scene: AssetId,
    pub changes: Vec<LinkChange>,
}

/// One change a link makes to a node of its scene, addressed by the node's path of ids from the link.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum LinkChange {
    AddComponent {
        path: LinkPath,
        component: SerializedComponent,
    },
    RemoveComponent {
        path: LinkPath,
        type_name: String,
    },
    RemoveNode {
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
    /// Stores `component` through reflection and appends it to this node, keyed
    /// by its full type path, the name its registration resolves.
    pub fn push_component<T: Reflect>(&mut self, component: &T) -> anyhow::Result<()> {
        self.components.push(SerializedComponent {
            type_name: std::any::type_name::<T>().to_owned(),
            value: concerto_reflect::to_value(component)?,
        });
        Ok(())
    }
}

/// A format-agnostic scene graph, serialized directly (no separate DTO) and
/// serialized as its own asset. `nodes[0]` is not special — roots are simply the
/// nodes no other node lists as a child.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Scene {
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

    fn migrate(version: u32, payload: &[u8]) -> anyhow::Result<Self> {
        match version {
            1 => {
                let old: legacy::SceneV1 = bincode::deserialize(payload)?;
                Ok(old.into())
            }
            _ => anyhow::bail!(
                "Scene version {version} has no migration to version {}; re-import its source, or re-create it if it was authored in the editor",
                Self::FORMAT_VERSION
            ),
        }
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

mod legacy {
    use concerto_foundation::assets::AssetId;
    use serde::Deserialize;

    use super::{Scene, SceneNode, SerializedComponent};
    use crate::node_id::{derive_ids, NodeId};

    #[derive(Deserialize)]
    struct SceneNodeV1 {
        name: String,
        children: Vec<usize>,
        components: Vec<SerializedComponent>,
    }

    #[derive(Deserialize)]
    pub(super) struct SceneV1 {
        nodes: Vec<SceneNodeV1>,
        referenced_assets: Vec<AssetId>,
    }

    impl From<SceneV1> for Scene {
        fn from(old: SceneV1) -> Self {
            let mut nodes: Vec<SceneNode> = old
                .nodes
                .into_iter()
                .map(|node| SceneNode {
                    id: NodeId::default(),
                    name: node.name,
                    children: node.children,
                    components: node.components,
                    link: None,
                })
                .collect();
            derive_ids(&mut nodes);
            Self {
                nodes,
                referenced_assets: old.referenced_assets,
            }
        }
    }
}

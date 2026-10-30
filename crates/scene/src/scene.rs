use concerto_foundation::assets::{Asset, AssetId, CreatableAsset, LoadableAsset};
use concerto_reflect::{Reflect, ReflectedValue};
use serde::{Deserialize, Serialize};

/// One component as scene data: the registry key it was registered under plus
/// its reflected value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SerializedComponent {
    pub type_name: String,
    pub value: ReflectedValue,
}

/// One node in a [`Scene`]: a name, the indices of its children within
/// `Scene::nodes`, and the list of components authored onto it. Every
/// runtime concern (transform, mesh, material, camera, light, ...) is carried
/// as a [`SerializedComponent`]; the spawner applies them generically through
/// the component registry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SceneNode {
    pub name: String,
    /// Indices into `Scene::nodes`.
    pub children: Vec<usize>,
    pub components: Vec<SerializedComponent>,
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
    fn name() -> &'static str {
        "Scene"
    }

    fn referenced_sub_assets(&self) -> Vec<AssetId> {
        self.referenced_assets.clone()
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

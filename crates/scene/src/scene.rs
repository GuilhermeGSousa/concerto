use concerto_ecs::component::scene::SceneComponent;
use concerto_foundation::assets::{Asset, AssetId, LoadableAsset};
use serde::{Deserialize, Serialize};

/// One component's serialized payload: the registry key it was registered under
/// plus its serde-JSON encoding.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializedComponent {
    pub type_name: String,
    pub data: String,
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
    /// Serializes `component` and appends it to this node. Importers use this
    /// rather than building [`SerializedComponent`] by hand, so the registry
    /// key always comes from the component's [`SceneComponent`] registration
    /// name, and only components that can be applied during scene spawning can
    /// be authored through this method.
    pub fn push_component<T: Serialize + SceneComponent>(
        &mut self,
        component: &T,
    ) -> anyhow::Result<()> {
        self.components.push(SerializedComponent {
            type_name: T::name().to_string(),
            data: serde_json::to_string(component)?,
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
    /// Every [`AssetId`] reachable from the nodes' components. Component
    /// payloads are opaque strings, so [`Asset::referenced_sub_assets`]
    /// cannot introspect them — the importer records the ids here as it emits.
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

impl LoadableAsset for Scene {}

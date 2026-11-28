use std::collections::HashMap;

use anyhow::Context;
use concerto_foundation::{
    assets::{
        Asset, AssetId, CreatableAsset, LoadableAsset,
        derived::{DerivedData, DerivedInputs},
        handle::AssetHandle,
    },
    hash::Fnv1a,
};
use concerto_mesh::Mesh;
use concerto_reflect::{Reflect, ReflectDefault, ReflectedValue};
use concerto_scene::scene::Scene;
use serde::{Deserialize, Serialize};

use crate::{
    navmesh::{NavMeshData, NavMeshGeometry, NavMeshSettings},
    query::NavMeshQuery,
    scene_colliders::collider_meshes,
};

#[derive(Clone, Debug)]
struct LiveNavMesh {
    data: NavMeshData,
    query: NavMeshQuery,
}

impl LiveNavMesh {
    fn new(data: NavMeshData) -> Self {
        let query = NavMeshQuery::new(&data);
        Self { data, query }
    }
}

/// A navmesh baked from the static colliders of `scenes` for the agent `settings` describes.
#[derive(Reflect, Serialize, Deserialize, Clone, Debug, Default)]
#[reflect(Default)]
pub struct NavMesh {
    /// The scenes whose static colliders are baked.
    pub scenes: Vec<AssetHandle<Scene>>,
    /// The agent size and slope limits the mesh is baked for.
    pub settings: NavMeshSettings,
    #[reflect(skip)]
    #[serde(skip)]
    live: Option<LiveNavMesh>,
}

impl NavMesh {
    /// A navmesh of no scenes, with default settings.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds `scene` to the scenes the navmesh is baked from.
    pub fn with_scene(mut self, scene: AssetHandle<Scene>) -> Self {
        self.scenes.push(scene);
        self
    }

    /// Sets the agent and resolution the navmesh is baked for.
    pub fn with_settings(mut self, settings: NavMeshSettings) -> Self {
        self.settings = settings;
        self
    }

    /// A navmesh already baked into `data`.
    ///
    /// It is for [`AssetServer::add`](concerto_foundation::assets::asset_server::AssetServer::add),
    /// which derives nothing.
    pub fn baked(settings: NavMeshSettings, data: NavMeshData) -> Self {
        Self {
            scenes: Vec::new(),
            settings,
            live: Some(LiveNavMesh::new(data)),
        }
    }

    /// The baked polygons, once derived or built with [`NavMesh::baked`].
    pub fn data(&self) -> Option<&NavMeshData> {
        self.live.as_ref().map(|live| &live.data)
    }

    /// The path query over the baked polygons, once derived or built with [`NavMesh::baked`].
    pub fn query(&self) -> Option<&NavMeshQuery> {
        self.live.as_ref().map(|live| &live.query)
    }

    fn scene_ids(&self) -> anyhow::Result<Vec<AssetId>> {
        self.scenes
            .iter()
            .enumerate()
            .map(|(index, scene)| {
                let id = scene.id();
                anyhow::ensure!(
                    !id.is_nil(),
                    "scene {index} of the navmesh is not set; pick a scene for it or remove it"
                );
                Ok(id)
            })
            .collect()
    }

    fn collider_meshes(&self, inputs: &DerivedInputs) -> anyhow::Result<Vec<AssetId>> {
        let mut ids = Vec::new();
        for scene in self.scene_ids()? {
            for id in collider_meshes(&inputs.decode::<Scene>(scene)?) {
                if !ids.contains(&id) {
                    ids.push(id);
                }
            }
        }
        Ok(ids)
    }
}

fn input_bytes(inputs: &DerivedInputs, id: AssetId) -> anyhow::Result<&[u8]> {
    inputs
        .bytes(id)
        .with_context(|| format!("navmesh input {id:?} was not read"))
}

impl Asset for NavMesh {
    fn name() -> &'static str {
        "NavMesh"
    }
}

impl CreatableAsset for NavMesh {
    fn new_asset() -> Self {
        Self::new()
    }
}

impl LoadableAsset for NavMesh {}

impl DerivedData for NavMesh {
    const DERIVED_VERSION: u32 = 1;

    fn is_derived(&self) -> bool {
        !self.scenes.is_empty()
    }

    fn derived_inputs(&self) -> Vec<AssetId> {
        self.scenes
            .iter()
            .map(AssetHandle::id)
            .filter(|id| !id.is_nil())
            .collect()
    }

    fn further_inputs(&self, inputs: &DerivedInputs) -> anyhow::Result<Vec<AssetId>> {
        self.collider_meshes(inputs)
    }

    fn derived_key(&self, inputs: &DerivedInputs) -> anyhow::Result<u64> {
        let mut hash = Fnv1a::new();
        hash.value(
            &ReflectedValue::encode(&self.settings)
                .context("failed to encode the navmesh settings")?,
        );
        let scenes = self.scene_ids()?;
        let meshes = self.collider_meshes(inputs)?;
        hash.count(scenes.len());
        for id in scenes {
            hash.bytes(input_bytes(inputs, id)?);
        }
        hash.count(meshes.len());
        for id in meshes {
            hash.bytes(input_bytes(inputs, id)?);
        }
        Ok(hash.finish())
    }

    fn derive(
        &self,
        inputs: &DerivedInputs,
        progress: &mut dyn FnMut(usize, usize),
    ) -> anyhow::Result<Vec<u8>> {
        let scenes = self.scene_ids()?;
        let mut meshes = HashMap::new();
        for id in self.collider_meshes(inputs)? {
            meshes.insert(id, inputs.decode::<Mesh>(id)?);
        }
        let mut geometry = NavMeshGeometry::new();
        for (index, id) in scenes.iter().enumerate() {
            geometry.add_scene(&inputs.decode::<Scene>(*id)?, &meshes)?;
            progress(index + 1, scenes.len() + 1);
        }
        let data = geometry.bake(&self.settings)?;
        progress(scenes.len() + 1, scenes.len() + 1);
        bincode::serialize(&data).context("failed to encode a baked navmesh")
    }

    fn apply_derived(&mut self, blob: &[u8]) -> anyhow::Result<()> {
        let data: NavMeshData =
            bincode::deserialize(blob).context("failed to decode a baked navmesh")?;
        self.live = Some(LiveNavMesh::new(data));
        Ok(())
    }
}

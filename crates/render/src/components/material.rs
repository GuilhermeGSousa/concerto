use std::marker::PhantomData;

use concerto_core::assets::{
    asset_server::AssetServer, handle::AssetHandle, AssetId, LoadableAsset,
};
use concerto_ecs::component::scene::{SceneComponent, SceneSpawnContext};
use concerto_ecs::component::Component;
use concerto_ecs::Entity;
use serde::{Deserialize, Serialize};

use crate::{assets::material::StandardMaterial, Material};

/// Attach this component (alongside [`MeshComponent`]) to an entity to tell the engine
/// which material the mesh should be rendered with.
///
/// The type parameter `M` defaults to [`StandardMaterial`] so existing code that writes
/// `MaterialComponent { handle: … }` with a `StandardMaterial` handle continues to work
/// without any change.  Custom materials use `MaterialComponent::<MyMaterial> { handle: … }`.
#[derive(Component, Serialize, Deserialize)]
#[serde(bound = "")]
pub struct MaterialComponent<M: Material + Send + Sync + 'static = StandardMaterial> {
    pub handle: AssetHandle<M>,
}

impl<M: Material + LoadableAsset> SceneComponent for MaterialComponent<M> {
    fn apply(mut self, entity: Entity, ctx: &mut SceneSpawnContext<'_>) {
        if let Some(server) = ctx.get_resource::<AssetServer>() {
            self.handle = server.load(self.handle.id());
        }
        ctx.insert(self, entity);
    }
}

/// Render-world component placed on mesh entities to identify which material
/// asset they use for a specific material type `M`.
///
/// This component replaces the old `MaterialInstanceTag<M>` (which was a pure
/// phantom marker) by also carrying the material's [`AssetId`].  This means
/// [`super::mesh_component::RenderMeshInstance`] no longer needs to store the
/// material asset id — the two concerns (mesh geometry and material) are kept
/// in separate components.
///
/// The type parameter `M` ensures that `material_renderpass<M>` only picks up
/// entities belonging to pipeline `M`, so multiple `MaterialPlugin` instances
/// for different material types coexist without interfering with each other.
#[derive(Component)]
pub(crate) struct RenderMaterialComponent<M: Material + 'static> {
    pub(crate) material_asset_id: AssetId,
    _marker: PhantomData<fn() -> M>,
}

impl<M: Material + 'static> RenderMaterialComponent<M> {
    pub(crate) fn new(material_asset_id: AssetId) -> Self {
        Self {
            material_asset_id,
            _marker: PhantomData,
        }
    }
}

use std::marker::PhantomData;

use concerto_ecs::component::Component;
use concerto_ecs::reflect::{ReflectComponent, ReflectSceneComponent};
use concerto_foundation::assets::{handle::AssetHandle, AssetId};
use concerto_reflect::{Reflect, ReflectDefault};

use crate::{assets::material::StandardMaterial, Material};

/// Attach this component (alongside [`MeshComponent`]) to an entity to tell the engine
/// which material the mesh should be rendered with.
///
/// The type parameter `M` defaults to [`StandardMaterial`] so existing code that writes
/// `MaterialComponent { handle: … }` with a `StandardMaterial` handle continues to work
/// without any change.  Custom materials use `MaterialComponent::<MyMaterial> { handle: … }`.
#[derive(Component, Reflect)]
#[reflect(Component, Default, SceneComponent)]
#[require(crate::components::render_entity::SyncWithRenderWorld)]
pub struct MaterialComponent<M: Material + Send + Sync + 'static = StandardMaterial> {
    pub handle: AssetHandle<M>,
}

impl<M: Material + Send + Sync + 'static> Default for MaterialComponent<M> {
    fn default() -> Self {
        Self {
            handle: AssetHandle::default(),
        }
    }
}

/// On a render-world mesh entity: the asset id of its material of type `M`.
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

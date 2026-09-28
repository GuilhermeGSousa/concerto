use std::marker::PhantomData;

use concerto_ecs::component::scene::{SceneComponent, SceneSpawnContext};
use concerto_ecs::component::Component;
use concerto_ecs::Entity;
use concerto_foundation::assets::{
    asset_server::AssetServer, handle::AssetHandle, AssetId, LoadableAsset,
};
use serde::{Deserialize, Serialize};

use crate::{assets::material::StandardMaterial, Material};

/// Which material each of a mesh's primitive slots draws with.
#[derive(Serialize, Deserialize, Clone)]
#[serde(bound = "")]
pub enum SlotBinding<M: Material + Send + Sync + 'static = StandardMaterial> {
    /// One material for every slot, however many the mesh turns out to have.
    All(AssetHandle<M>),
    /// One entry per slot; `None` leaves the slot uncovered.
    PerSlot(Vec<Option<AssetHandle<M>>>),
}

impl<M: Material + Send + Sync + 'static> SlotBinding<M> {
    /// The material covering `index`, or `None` when this binding leaves it open.
    pub fn slot(&self, index: u32) -> Option<&AssetHandle<M>> {
        match self {
            SlotBinding::All(handle) => Some(handle),
            SlotBinding::PerSlot(slots) => slots.get(index as usize)?.as_ref(),
        }
    }
}

/// Which material each primitive of the entity's mesh draws with.
#[derive(Component, Serialize, Deserialize)]
#[serde(bound = "")]
pub struct MaterialComponent<M: Material + Send + Sync + 'static = StandardMaterial> {
    pub binding: SlotBinding<M>,
}

impl<M: Material + Send + Sync + 'static> MaterialComponent<M> {
    /// One material on every primitive — the common single-material case.
    pub fn all(handle: AssetHandle<M>) -> Self {
        Self {
            binding: SlotBinding::All(handle),
        }
    }

    /// One material per primitive slot, `None` leaving a slot uncovered.
    pub fn per_slot(slots: Vec<Option<AssetHandle<M>>>) -> Self {
        Self {
            binding: SlotBinding::PerSlot(slots),
        }
    }
}

impl<M: Material + LoadableAsset> SceneComponent for MaterialComponent<M> {
    fn apply(mut self, entity: Entity, ctx: &mut SceneSpawnContext<'_>) {
        if let Some(server) = ctx.get_resource::<AssetServer>() {
            match &mut self.binding {
                SlotBinding::All(handle) => *handle = server.load(handle.id()),
                SlotBinding::PerSlot(slots) => {
                    for handle in slots.iter_mut().flatten() {
                        *handle = server.load(handle.id());
                    }
                }
            }
        }
        ctx.insert(self, entity);
    }
}

/// Render-world component placed on mesh entities to identify which material
/// asset each primitive slot uses for a specific material type `M`.
///
/// The type parameter `M` ensures that `material_renderpass<M>` only picks up
/// entities belonging to pipeline `M`, so multiple `MaterialPlugin` instances
/// for different material types coexist without interfering with each other.
#[derive(Component)]
pub(crate) struct RenderMaterialComponent<M: Material + 'static> {
    pub(crate) slots: RenderSlots,
    _marker: PhantomData<fn() -> M>,
}

impl<M: Material + 'static> RenderMaterialComponent<M> {
    pub(crate) fn new(slots: RenderSlots) -> Self {
        Self {
            slots,
            _marker: PhantomData,
        }
    }
}

pub(crate) enum RenderSlots {
    All(AssetId),
    PerSlot(Vec<Option<AssetId>>),
}

impl RenderSlots {
    pub(crate) fn from_binding<M: Material + Send + Sync + 'static>(
        binding: &SlotBinding<M>,
    ) -> Self {
        match binding {
            SlotBinding::All(handle) => RenderSlots::All(handle.id()),
            SlotBinding::PerSlot(slots) => RenderSlots::PerSlot(
                slots
                    .iter()
                    .map(|slot| slot.as_ref().map(AssetHandle::id))
                    .collect(),
            ),
        }
    }

    pub(crate) fn matches<M: Material + Send + Sync + 'static>(
        &self,
        binding: &SlotBinding<M>,
    ) -> bool {
        match (self, binding) {
            (RenderSlots::All(id), SlotBinding::All(handle)) => *id == handle.id(),
            (RenderSlots::PerSlot(ids), SlotBinding::PerSlot(slots)) => {
                ids.len() == slots.len()
                    && ids
                        .iter()
                        .zip(slots)
                        .all(|(id, slot)| *id == slot.as_ref().map(AssetHandle::id))
            }
            _ => false,
        }
    }

    pub(crate) fn slot(&self, index: u32) -> Option<AssetId> {
        match self {
            RenderSlots::All(id) => Some(*id),
            RenderSlots::PerSlot(ids) => *ids.get(index as usize)?,
        }
    }
}

use std::marker::PhantomData;

use concerto_color::Color;
use concerto_ecs::resource::Resource;
use concerto_foundation::assets::{handle::AssetHandle, AssetId};

use crate::{assets::material::StandardMaterial, components::material::SlotBinding, Material};

/// The magenta stand-in drawn where no material covers a primitive slot.
pub fn fallback_material_asset() -> StandardMaterial {
    let mut material = StandardMaterial::new(None, None);
    material.set_base_color_factor(Color::rgba(1.0, 0.0, 1.0, 1.0));
    material.set_metallic_factor(0.0);
    material.set_roughness_factor(1.0);
    material
}

/// Main-world handle keeping the fallback material loaded.
#[derive(Resource)]
pub struct FallbackMaterial(pub AssetHandle<StandardMaterial>);

/// The material `M`'s pass draws uncovered slots with; `None` for every `M` but `StandardMaterial`.
pub struct RenderFallbackMaterial<M: 'static> {
    pub material: Option<AssetId>,
    _marker: PhantomData<fn() -> M>,
}

impl<M: 'static> RenderFallbackMaterial<M> {
    pub fn new(material: Option<AssetId>) -> Self {
        Self {
            material,
            _marker: PhantomData,
        }
    }
}

// Manual Resource impl — #[derive(Resource)] doesn't handle PhantomData<fn()>.
impl<M: 'static> Resource for RenderFallbackMaterial<M> {
    fn name() -> &'static str {
        std::any::type_name::<RenderFallbackMaterial<M>>()
    }
}

/// The primitive slots of one mesh instance that some material covered this frame.
#[derive(Default)]
pub struct SlotCoverage {
    all: bool,
    slots: Vec<bool>,
    overlap_reported: bool,
}

impl SlotCoverage {
    /// Covers every slot until the first `clear`, so a new instance skips the fallback until its materials are extracted.
    pub fn unresolved() -> Self {
        Self {
            all: true,
            ..Self::default()
        }
    }

    /// Uncovers every slot, keeping whether an overlap was already reported.
    pub fn clear(&mut self) {
        self.all = false;
        self.slots.fill(false);
    }

    /// Covers the slots `binding` covers; `true` the first time two bindings cover one slot.
    pub fn cover<M: Material + Send + Sync + 'static>(&mut self, binding: &SlotBinding<M>) -> bool {
        let overlap = match binding {
            SlotBinding::All(_) => {
                let overlap = self.all || self.slots.contains(&true);
                self.all = true;
                overlap
            }
            SlotBinding::PerSlot(slots) => {
                if self.slots.len() < slots.len() {
                    self.slots.resize(slots.len(), false);
                }
                let mut overlap = false;
                for (covered, slot) in self.slots.iter_mut().zip(slots) {
                    if slot.is_some() {
                        overlap |= self.all || *covered;
                        *covered = true;
                    }
                }
                overlap
            }
        };
        let first = overlap && !self.overlap_reported;
        self.overlap_reported |= overlap;
        first
    }

    /// Whether any material covered `slot` this frame.
    pub fn is_covered(&self, slot: u32) -> bool {
        self.all || self.slots.get(slot as usize).copied().unwrap_or(false)
    }
}

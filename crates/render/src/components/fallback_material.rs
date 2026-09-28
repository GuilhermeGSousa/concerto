use std::collections::HashSet;

use concerto_color::Color;
use concerto_ecs::{
    command::CommandQueue,
    component::Component,
    query::Query,
    resource::{Res, ResMut, Resource},
    Entity, With,
};
use concerto_foundation::assets::{handle::AssetHandle, AssetId};

use crate::assets::material::StandardMaterial;
use crate::components::{material::RenderMaterialComponent, mesh::RenderMeshInstance};

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

/// Render-world id of the fallback material, or `None` when the app renders
/// no `StandardMaterial` to draw it with.
#[derive(Resource)]
pub struct RenderFallbackMaterial(pub Option<AssetId>);

/// On a render-world primitive entity: it currently draws with the fallback.
#[derive(Component)]
pub struct UsesFallbackMaterial;

/// Primitive entities some `MaterialComponent<M>` claimed this frame.
#[derive(Resource, Default)]
pub struct ClaimedSlots {
    claimed: HashSet<Entity>,
    reported: HashSet<Entity>,
}

impl ClaimedSlots {
    /// Records a claim, returning `true` the first time another material type
    /// is found to have claimed the same entity in the same frame.
    pub fn claim(&mut self, entity: Entity) -> bool {
        !self.claimed.insert(entity) && self.reported.insert(entity)
    }

    /// Whether any material type claimed `entity` this frame.
    pub fn is_claimed(&self, entity: Entity) -> bool {
        self.claimed.contains(&entity)
    }
}

/// Gives every mesh instance no material claimed this frame the fallback.
pub(crate) fn insert_fallback_material(
    instances: Query<Entity, With<RenderMeshInstance>>,
    fallback_users: Query<&UsesFallbackMaterial>,
    fallback: Res<RenderFallbackMaterial>,
    mut claimed: ResMut<ClaimedSlots>,
    mut cmd: CommandQueue,
) {
    if let Some(fallback) = fallback.0 {
        for entity in instances.iter() {
            if claimed.is_claimed(entity) || fallback_users.get_entity(entity).is_some() {
                continue;
            }
            cmd.insert(
                (
                    RenderMaterialComponent::<StandardMaterial>::new(fallback),
                    UsesFallbackMaterial,
                ),
                entity,
            );
        }
    }

    claimed.claimed.clear();
}

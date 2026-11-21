use concerto_app::{
    App,
    plugins::Plugin,
    schedule_groups::{LateFixedUpdate, LateUpdate},
};
use concerto_ecs::IntoSystemConfig;
use concerto_foundation::{
    assets::handle::AssetHandle, transform::systems::propagate_global_transforms,
};
use concerto_mesh::Skeleton;
use concerto_physics::step_simulation;

use crate::{
    asset::Ragdoll,
    character::{
        CharacterRagdoll, create_character_ragdolls, drop_removed_ragdoll, drop_replaced_ragdoll,
        follow_animation,
    },
};

/// Registers the ragdoll asset and gives every `CharacterRagdoll` a physical body that follows its animation; register it after `PhysicsPlugin`.
pub struct RagdollPlugin;

impl Plugin for RagdollPlugin {
    fn build(&self, app: &mut App) {
        app.register_inspected_asset::<Ragdoll>();
        app.register_asset::<Skeleton>();
        app.register_type::<AssetHandle<Ragdoll>>();
        app.register_type::<CharacterRagdoll>();
        app.add_listener(drop_removed_ragdoll)
            .add_listener(drop_replaced_ragdoll)
            .add_system(
                LateUpdate,
                create_character_ragdolls.after(propagate_global_transforms),
            )
            .add_system(LateFixedUpdate, follow_animation.before(step_simulation));
    }
}

use concerto_animation::target::animate_targets;
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
        CharacterRagdoll, carry_capsules, create_character_ragdolls, drive_character_ragdolls,
        drop_removed_ragdoll, drop_replaced_ragdoll, record_ragdoll_poses, write_ragdoll_poses,
    },
};

/// Registers the ragdoll asset and gives every `CharacterRagdoll` a physical body that follows its animation or falls limp, drawn on its skeleton by weight; register it after `PhysicsPlugin`.
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
            .add_system(
                LateFixedUpdate,
                drive_character_ragdolls.before(step_simulation),
            )
            .add_system(
                LateFixedUpdate,
                carry_capsules
                    .after(drive_character_ragdolls)
                    .before(step_simulation),
            )
            .add_system(LateFixedUpdate, record_ragdoll_poses.after(step_simulation))
            .add_system(
                LateUpdate,
                write_ragdoll_poses
                    .after(animate_targets)
                    .before(propagate_global_transforms),
            );
    }
}

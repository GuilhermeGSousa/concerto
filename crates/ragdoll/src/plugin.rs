use concerto_animation::{
    drive::AnimationDriveSet, plugin::AnimationAppExt, target::animate_targets,
};
use concerto_app::{
    App,
    edit_time::AtPreviewTime,
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
        CharacterRagdoll, carry_capsules, create_character_ragdolls, drop_removed_ragdoll,
        drop_replaced_ragdoll, record_animated_poses, record_ragdoll_poses, write_ragdoll_poses,
    },
    drive::drive_from_graph,
    dying::{Settled, forward_death, settle_dead_bodies, wake_when_hit},
    messages::{
        self, BodyDead, BodyDown, Death, Hit, Recovered, Steadied, Toppled, apply_impulses,
        route_hits,
    },
    node::{
        BalanceNode, DyingNode, FallNode, FlinchNode, HoldNode, LimpNode, ShotNode, StaggerNode,
    },
    sensing::sense_bodies,
};

/// Registers the ragdoll asset and gives every `CharacterRagdoll` a physical body.
///
/// The body is driven and drawn on its skeleton as the physics attributes of
/// its animation graph say; with none it follows its animation. It registers the
/// [`HoldNode`], [`LimpNode`], [`FlinchNode`], [`DyingNode`], [`BalanceNode`],
/// [`StaggerNode`], [`ShotNode`] and [`FallNode`] graph nodes and the [`Hit`],
/// [`messages::Shot`], [`messages::Stagger`], [`Recovered`], [`Death`],
/// [`BodyDead`], [`Steadied`], [`Toppled`] and [`BodyDown`] messages, and sends a
/// character's hits, gunshots and death to its graph. Register the plugin
/// after `PhysicsPlugin` and `AnimationPlugin`.
///
/// Its systems also run in a preview `App`, where they do nothing until the
/// preview has a `PhysicsPipeline`.
pub struct RagdollPlugin;

impl Plugin for RagdollPlugin {
    fn build(&self, app: &mut App) {
        app.register_inspected_asset::<Ragdoll>();
        app.register_asset::<Skeleton>();
        app.register_type::<AssetHandle<Ragdoll>>();
        app.register_type::<CharacterRagdoll>();
        app.register_animation_node::<HoldNode>()
            .register_animation_node::<LimpNode>()
            .register_animation_node::<FlinchNode>()
            .register_animation_node::<DyingNode>()
            .register_animation_node::<BalanceNode>()
            .register_animation_node::<StaggerNode>()
            .register_animation_node::<ShotNode>()
            .register_animation_node::<FallNode>();
        app.register_animation_message::<Hit>()
            .register_animation_message::<messages::Shot>()
            .register_animation_message::<messages::Stagger>()
            .register_animation_message::<Recovered>()
            .register_animation_message::<Death>()
            .register_animation_message::<BodyDead>()
            .register_animation_message::<Steadied>()
            .register_animation_message::<Toppled>()
            .register_animation_message::<BodyDown>();
        app.add_listener(route_hits.at_preview_time()).add_system(
            LateFixedUpdate,
            apply_impulses
                .after(drive_from_graph)
                .before(step_simulation)
                .at_preview_time(),
        );
        app.add_system(
            LateFixedUpdate,
            sense_bodies.after(step_simulation).at_preview_time(),
        );
        app.register_type::<Settled>();
        app.add_listener(forward_death.at_preview_time())
            .add_listener(wake_when_hit.at_preview_time())
            .add_system(
                LateFixedUpdate,
                settle_dead_bodies.after(sense_bodies).at_preview_time(),
            );
        app.add_listener(drop_removed_ragdoll.at_preview_time())
            .add_listener(drop_replaced_ragdoll.at_preview_time())
            .add_system(
                LateUpdate,
                create_character_ragdolls
                    .after(propagate_global_transforms)
                    .at_preview_time(),
            )
            .add_system(
                LateUpdate,
                record_animated_poses
                    .after(propagate_global_transforms)
                    .after(create_character_ragdolls)
                    .at_preview_time(),
            )
            .add_system(
                LateFixedUpdate,
                drive_from_graph
                    .after(AnimationDriveSet)
                    .before(step_simulation)
                    .at_preview_time(),
            )
            .add_system(
                LateFixedUpdate,
                carry_capsules
                    .after(drive_from_graph)
                    .before(step_simulation)
                    .at_preview_time(),
            )
            .add_system(
                LateFixedUpdate,
                record_ragdoll_poses
                    .after(step_simulation)
                    .at_preview_time(),
            )
            .add_system(
                LateUpdate,
                write_ragdoll_poses
                    .after(animate_targets)
                    .before(propagate_global_transforms)
                    .at_preview_time(),
            );
    }
}

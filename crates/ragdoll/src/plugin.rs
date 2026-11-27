use concerto_animation::{
    drive::AnimationDriveSet, plugin::AnimationAppExt, target::animate_targets,
};
use concerto_app::{
    App,
    edit_time::AtPreviewTime,
    plugins::Plugin,
    schedule_groups::{FixedUpdate, LateFixedUpdate, LateUpdate},
};
use concerto_ecs::IntoSystemConfig;
use concerto_foundation::{
    assets::handle::AssetHandle, transform::systems::propagate_global_transforms,
};
use concerto_mesh::Skeleton;
use concerto_physics::step_simulation;

use crate::{
    asset::Ragdoll,
    balance::{Balance, balance_bodies},
    behaviour::BehaviourSet,
    behaviour::{PhysicalState, update_physical_states},
    character::{
        CharacterRagdoll, carry_capsules, create_character_ragdolls, drive_character_ragdolls,
        drop_removed_ragdoll, drop_replaced_ragdoll, record_animated_poses, record_ragdoll_poses,
        write_ragdoll_poses,
    },
    drive::drive_from_graph,
    dying::{
        Dying, Settled, die_when_told, drain_dying, forward_death, settle_dead_bodies,
        wake_when_hit,
    },
    fall::{Fall, FallReaction, fall_bodies, fall_when_falling},
    hit::{HitReaction, flinch_when_hit, update_flinches},
    messages::{BodyDead, Death, Hit, Recovered, apply_impulses, route_hits},
    node::{DyingNode, FlinchNode, HoldNode, LimpNode},
    sensing::sense_bodies,
    shot::{ShotReaction, end_shots, note_shot_balance_ended, react_when_shot, shot_bodies},
    stagger::{Stagger, end_staggers, note_balance_ended, stagger_bodies},
};

/// Registers the ragdoll asset and gives every `CharacterRagdoll` a physical body.
///
/// The body follows its animation, is powered towards it or falls limp, and
/// is drawn on its skeleton by weight; or, in the graph mode, does and is
/// drawn as its animation graph's physics attributes say. It registers the
/// [`HoldNode`], [`LimpNode`], [`FlinchNode`] and [`DyingNode`] graph nodes and
/// the [`Hit`], [`Recovered`], [`Death`] and [`BodyDead`] messages, and sends a
/// graph-mode character's hits and death to its graph. Register the plugin
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
            .register_animation_node::<DyingNode>();
        app.register_animation_message::<Hit>()
            .register_animation_message::<Recovered>()
            .register_animation_message::<Death>()
            .register_animation_message::<BodyDead>();
        app.add_listener(route_hits.at_preview_time()).add_system(
            LateFixedUpdate,
            apply_impulses
                .after(drive_from_graph)
                .before(step_simulation)
                .at_preview_time(),
        );
        app.register_type::<HitReaction>();
        app.add_listener(flinch_when_hit.at_preview_time())
            .add_system(
                LateFixedUpdate,
                update_flinches
                    .after(drive_character_ragdolls)
                    .before(step_simulation)
                    .at_preview_time(),
            );
        app.add_system(
            LateFixedUpdate,
            sense_bodies.after(step_simulation).at_preview_time(),
        );
        app.register_type::<PhysicalState>();
        app.add_system(
            LateFixedUpdate,
            update_physical_states.after(sense_bodies).at_preview_time(),
        );
        app.register_type::<Balance>();
        app.add_system(
            FixedUpdate,
            balance_bodies.in_set(BehaviourSet).at_preview_time(),
        );
        app.register_type::<Fall>();
        app.register_type::<FallReaction>();
        app.add_listener(fall_when_falling.at_preview_time())
            .add_system(
                FixedUpdate,
                fall_bodies.in_set(BehaviourSet).at_preview_time(),
            );
        app.register_type::<Dying>();
        app.register_type::<Settled>();
        app.add_listener(die_when_told.at_preview_time())
            .add_listener(forward_death.at_preview_time())
            .add_listener(wake_when_hit.at_preview_time())
            .add_system(
                FixedUpdate,
                drain_dying.in_set(BehaviourSet).at_preview_time(),
            )
            .add_system(
                LateFixedUpdate,
                settle_dead_bodies
                    .after(update_physical_states)
                    .at_preview_time(),
            );
        app.register_type::<ShotReaction>();
        app.add_listener(react_when_shot.at_preview_time())
            .add_listener(note_shot_balance_ended.at_preview_time())
            .add_system(
                FixedUpdate,
                end_shots.in_set(BehaviourSet).at_preview_time(),
            )
            .add_system(
                FixedUpdate,
                shot_bodies
                    .in_set(BehaviourSet)
                    .before(balance_bodies)
                    .at_preview_time(),
            );
        app.register_type::<Stagger>();
        app.add_listener(note_balance_ended.at_preview_time())
            .add_system(
                FixedUpdate,
                end_staggers.in_set(BehaviourSet).at_preview_time(),
            )
            .add_system(
                FixedUpdate,
                stagger_bodies
                    .in_set(BehaviourSet)
                    .before(balance_bodies)
                    .at_preview_time(),
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
                drive_character_ragdolls
                    .before(step_simulation)
                    .at_preview_time(),
            )
            .add_system(
                LateFixedUpdate,
                drive_from_graph
                    .after(AnimationDriveSet)
                    .after(drive_character_ragdolls)
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

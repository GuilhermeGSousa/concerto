use std::sync::Arc;

use concerto_ecs::Entity;
use concerto_physics::ragdoll::{BodyPose, RagdollId};
use glam::Vec3;

use crate::{
    asset::{Ragdoll, StrengthProfile},
    character::RagdollBody,
    resolve::ResolvedRagdoll,
    sensing::BodySense,
    shot::Wounds,
};

/// A character's body as the nodes of its animation graph see it: a snapshot handed to its `AnimationPlayer` as an extra each physics step and before each frame's update.
#[derive(Clone)]
pub struct BodyView {
    entity: Entity,
    ragdoll_id: RagdollId,
    resolved: Arc<ResolvedRagdoll>,
    profiles: Vec<StrengthProfile>,
    pose: Vec<BodyPose>,
    animated: Vec<BodyPose>,
    velocities: Vec<Vec3>,
    sense: Option<BodySense>,
    wounds: Wounds,
    part_of_bone: Vec<Option<usize>>,
}

impl BodyView {
    pub(crate) fn of(
        entity: Entity,
        ragdoll_id: RagdollId,
        body: &RagdollBody,
        ragdoll: Option<&Ragdoll>,
        sense: Option<&BodySense>,
        wounds: Option<&Wounds>,
    ) -> Self {
        let resolved = body.shared_resolved();
        let mut part_of_bone = vec![None; body.skeleton_bone_count()];
        for (part, bone) in resolved.bones().iter().enumerate() {
            if let Some(slot) = part_of_bone.get_mut(*bone) {
                *slot = Some(part);
            }
        }
        Self {
            entity,
            ragdoll_id,
            resolved,
            profiles: ragdoll.map_or_else(Vec::new, |ragdoll| ragdoll.profiles.clone()),
            pose: body.simulated_pose().to_vec(),
            animated: body.animated_pose().to_vec(),
            velocities: body.velocities().to_vec(),
            sense: sense.cloned(),
            wounds: wounds.cloned().unwrap_or_default(),
            part_of_bone,
        }
    }

    /// The character.
    pub fn entity(&self) -> Entity {
        self.entity
    }

    /// The character's ragdoll in the physics world.
    pub fn ragdoll_id(&self) -> RagdollId {
        self.ragdoll_id
    }

    /// The ragdoll asset fitted to the character's skeleton.
    pub fn resolved(&self) -> &ResolvedRagdoll {
        &self.resolved
    }

    /// The strength profiles of the ragdoll asset.
    pub fn profiles(&self) -> &[StrengthProfile] {
        &self.profiles
    }

    /// The profile called `name`.
    pub fn profile(&self, name: &str) -> Option<&StrengthProfile> {
        self.profiles.iter().find(|profile| &*profile.name == name)
    }

    /// The world pose of every part after the last physics step, in part order.
    pub fn pose(&self) -> &[BodyPose] {
        &self.pose
    }

    /// The world pose the animation asked of every part on the last frame, in part order.
    pub fn animated(&self) -> &[BodyPose] {
        &self.animated
    }

    /// The linear velocity of `part` after the last physics step, in m/s.
    pub fn velocity(&self, part: usize) -> Vec3 {
        self.velocities.get(part).copied().unwrap_or(Vec3::ZERO)
    }

    /// What the body knows about itself, when the character keeps a `BodySense`.
    pub fn sense(&self) -> Option<&BodySense> {
        self.sense.as_ref()
    }

    /// The share of its strength each part keeps.
    pub fn wounds(&self) -> &Wounds {
        &self.wounds
    }

    /// The part on the skeleton bone at index `bone`, if it has one.
    pub fn part_of_bone(&self, bone: usize) -> Option<usize> {
        self.part_of_bone.get(bone).copied().flatten()
    }

    /// The skeleton bone index of each of `parts`, skipping any the ragdoll does not have.
    pub fn bones_of(&self, parts: impl IntoIterator<Item = usize>) -> impl Iterator<Item = usize> {
        parts
            .into_iter()
            .filter_map(|part| self.resolved.bones().get(part).copied())
    }
}

use std::sync::Arc;

use concerto_ecs::{Entity, Query, system::input::SystemInputData};
use concerto_physics::ragdoll::{BodyPose, RagdollId};
use glam::Quat;

use crate::{
    asset::StrengthProfile, character::RagdollBody, resolve::ResolvedRagdoll, sensing::BodySense,
    shot::Wounds,
};

/// What a node reads of a character's body: its ragdoll id and body, and its sense and wounds when it has them.
pub type BodyInput = Query<
    'static,
    'static,
    (
        &'static RagdollId,
        &'static RagdollBody,
        Option<&'static BodySense>,
        Option<&'static Wounds>,
    ),
>;

/// A character's body as the nodes of its animation graph see it: borrowed from the [`BodyInput`] query they name.
#[derive(Clone, Copy)]
pub struct BodyView<'a> {
    entity: Entity,
    ragdoll_id: RagdollId,
    body: &'a RagdollBody,
    sense: Option<&'a BodySense>,
    wounds: &'a Wounds,
}

impl<'a> BodyView<'a> {
    /// The body of `entity`, or none when it lacks a `RagdollId` or a `RagdollBody`.
    pub fn read(
        entity: Entity,
        input: &'a SystemInputData<'_, '_, BodyInput>,
    ) -> Option<BodyView<'a>> {
        let (id, body, sense, wounds) = input.get_entity(entity)?;
        Some(Self::new(entity, *id, body, sense, wounds))
    }

    pub(crate) fn new(
        entity: Entity,
        ragdoll_id: RagdollId,
        body: &'a RagdollBody,
        sense: Option<&'a BodySense>,
        wounds: Option<&'a Wounds>,
    ) -> Self {
        Self {
            entity,
            ragdoll_id,
            body,
            sense,
            wounds: wounds.unwrap_or(Wounds::none()),
        }
    }

    /// The character.
    pub fn entity(self) -> Entity {
        self.entity
    }

    /// The character's ragdoll in the physics world.
    pub fn ragdoll_id(self) -> RagdollId {
        self.ragdoll_id
    }

    /// The ragdoll asset fitted to the character's skeleton.
    pub fn resolved(self) -> &'a ResolvedRagdoll {
        self.body.resolved()
    }

    /// The strength profiles of the ragdoll asset the body was built from.
    pub fn profiles(self) -> &'a [StrengthProfile] {
        self.body.profiles()
    }

    /// The profile called `name`.
    pub fn profile(self, name: &str) -> Option<&'a StrengthProfile> {
        self.profiles()
            .iter()
            .find(|profile| &*profile.name == name)
    }

    /// The world pose of every part after the last physics step, in part order.
    pub fn pose(self) -> &'a [BodyPose] {
        self.body.simulated_pose()
    }

    /// The world pose the animation asked of every part on the last frame, in part order.
    pub fn animated(self) -> &'a [BodyPose] {
        self.body.animated_pose()
    }

    /// The rotation `part` has relative to its parent part in the animated pose, the target a motor holding it to the animation aims at; identity for the root part.
    pub fn animated_target(self, part: usize) -> Quat {
        let animated = self.body.animated_pose();
        let parent = self
            .resolved()
            .settings()
            .parts
            .get(part)
            .and_then(|settings| settings.parent);
        match (
            parent.and_then(|parent| animated.get(parent)),
            animated.get(part),
        ) {
            (Some(parent), Some(child)) => (parent.rotation.inverse() * child.rotation).normalize(),
            _ => Quat::IDENTITY,
        }
    }

    pub(crate) fn shared_resolved(self) -> &'a Arc<ResolvedRagdoll> {
        self.body.shared_resolved()
    }

    /// What the body knows about itself, when the character keeps a `BodySense`.
    pub fn sense(self) -> Option<&'a BodySense> {
        self.sense
    }

    /// The share of its strength each part keeps.
    pub fn wounds(self) -> &'a Wounds {
        self.wounds
    }
}

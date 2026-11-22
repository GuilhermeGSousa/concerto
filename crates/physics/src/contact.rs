use concerto_ecs::component::{Component, ComponentLifecycleCallback, ComponentLifecycleContext};
use concerto_ecs::signal::{EntitySignal, Signal};
use concerto_ecs::world::RestrictedWorld;
use concerto_ecs::{CommandQueue, Entity, Query, ResMut};
use glam::Vec3;

use crate::body::BodyId;
use crate::physics_state::PhysicsState;

/// One solid body touching the body of the entity that holds the `Contacts` listing it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Contact {
    /// The entity of the other body, when it has one.
    pub other: Option<Entity>,
    /// World-space point of the contact.
    pub point: Vec3,
    /// World-space normal, pointing from the other body towards this one.
    pub normal: Vec3,
    /// The normal impulse the contact applied over the last fixed step it was simulated in, in N·s.
    pub impulse: f32,
}

/// Makes the entity's body report its contacts with solid bodies, and lists the ones touching it after each fixed step.
#[derive(Clone, Debug, Default)]
pub struct Contacts {
    contacts: Vec<Contact>,
}

impl Contacts {
    /// The contacts of the last fixed step, one for each body touching this one.
    pub fn iter(&self) -> impl Iterator<Item = &Contact> {
        self.contacts.iter()
    }

    pub fn is_empty(&self) -> bool {
        self.contacts.is_empty()
    }

    /// The contact with `other`, while it touches this body.
    pub fn with(&self, other: Entity) -> Option<&Contact> {
        self.contacts
            .iter()
            .find(|contact| contact.other == Some(other))
    }
}

impl Component for Contacts {
    fn on_add() -> Option<ComponentLifecycleCallback> {
        Some(start_reporting)
    }

    fn on_remove() -> Option<ComponentLifecycleCallback> {
        Some(stop_reporting)
    }
}

fn start_reporting(world: RestrictedWorld, context: ComponentLifecycleContext) {
    set_reporting(world, context, true);
}

fn stop_reporting(world: RestrictedWorld, context: ComponentLifecycleContext) {
    set_reporting(world, context, false);
}

fn set_reporting(mut world: RestrictedWorld, context: ComponentLifecycleContext, enabled: bool) {
    let Some(&body) = world.get_component_for_entity::<BodyId>(context.entity) else {
        return;
    };
    if let Some(state) = world.get_resource_mut::<PhysicsState>() {
        state.set_contact_reporting(body, enabled);
    }
}

/// A solid body began touching the body of the entity this signal is aimed at.
pub struct ContactStart {
    pub other: Option<Entity>,
}

impl Signal for ContactStart {}
impl EntitySignal for ContactStart {}

/// A solid body stopped touching the body of the entity this signal is aimed at, or went away while touching it.
pub struct ContactEnd {
    pub other: Option<Entity>,
}

impl Signal for ContactEnd {}
impl EntitySignal for ContactEnd {}

pub(crate) struct ContactChange {
    pub(crate) entity: Entity,
    pub(crate) other: Option<Entity>,
    pub(crate) started: bool,
}

/// Rewrites every `Contacts` from the last fixed step and triggers `ContactStart` and `ContactEnd` in the order they happened.
pub fn report_contacts(
    mut reporting: Query<(&BodyId, &mut Contacts)>,
    mut state: ResMut<PhysicsState>,
    mut cmd: CommandQueue,
) {
    for change in state.take_contact_changes() {
        let mut entity = cmd.entity(change.entity);
        if change.started {
            entity.trigger(ContactStart {
                other: change.other,
            });
        } else {
            entity.trigger(ContactEnd {
                other: change.other,
            });
        }
    }
    for (body, mut contacts) in reporting.iter_mut() {
        let touching = state.contacts_of(*body);
        if contacts.contacts != touching {
            contacts.contacts = touching;
        }
    }
}

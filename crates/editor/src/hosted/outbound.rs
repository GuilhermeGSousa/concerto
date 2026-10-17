use std::collections::VecDeque;

use concerto_ecs::{
    Entity, Event, Resource, World,
    signal::{On, Signal},
};

use super::HostedApp;

/// An event of a hosted `App`, re-raised in the editor's world.
pub struct Hosted<E> {
    pub document: Entity,
    pub event: E,
}

impl<E: Event + Clone + Send + Sync + 'static> Signal for Hosted<E> {}

/// The event types that leave every hosted `App`, in the order they are forwarded.
#[derive(Resource, Default)]
pub struct OutboundEvents {
    forwarders: Vec<fn(&mut World, Entity)>,
    forwarding: bool,
    waiting: VecDeque<Entity>,
}

impl OutboundEvents {
    pub fn register<E: Event + Clone + Send + Sync + 'static>(&mut self) {
        let forwarder: fn(&mut World, Entity) = forward_one::<E>;
        if !self.forwarders.contains(&forwarder) {
            self.forwarders.push(forwarder);
        }
    }
}

fn forward_one<E: Event + Clone + Send + Sync + 'static>(world: &mut World, document: Entity) {
    let Some(hosted) = world.get_component_for_entity_mut::<HostedApp>(document) else {
        return;
    };
    for event in hosted.read_outbound::<E>() {
        world.trigger(Hosted { document, event });
    }
}

/// Re-raises everything the `App` on `document` has written since the last call.
pub fn forward(world: &mut World, document: Entity) {
    let Some(outbound) = world.get_resource_mut::<OutboundEvents>() else {
        return;
    };
    if outbound.forwarding {
        if outbound.waiting.back() != Some(&document) {
            outbound.waiting.push_back(document);
        }
        return;
    }
    outbound.forwarding = true;
    let mut next = Some(document);
    while let Some(document) = next {
        let forwarders = world
            .get_resource::<OutboundEvents>()
            .map(|outbound| outbound.forwarders.clone())
            .unwrap_or_default();
        for forwarder in forwarders {
            forwarder(world, document);
        }
        next = world
            .get_resource_mut::<OutboundEvents>()
            .and_then(|outbound| outbound.waiting.pop_front());
    }
    if let Some(outbound) = world.get_resource_mut::<OutboundEvents>() {
        outbound.forwarding = false;
    }
}

pub(crate) struct Forward(pub Entity);
impl Signal for Forward {}

pub(crate) fn forward_on_request(on: On<Forward>, world: &mut World) {
    forward(world, on.signal().0);
}

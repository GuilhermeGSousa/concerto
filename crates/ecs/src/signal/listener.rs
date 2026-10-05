use std::{any::TypeId, marker::PhantomData};

use crate::{
    Component, Entity, IntoSystem, System,
    command::Command,
    signal::{On, Signal},
    utilities::TypeIdMap,
};

pub struct Listener<S: Signal> {
    system: Box<dyn ListenerSystem<S>>,
    marker: PhantomData<S>,
}

impl<S: Signal> Component for Listener<S> {
    fn on_add() -> Option<crate::component::ComponentLifecycleCallback> {
        Some(|mut world, context| {
            world.queue_command(RegisterListenerCommand::<S>::new(context.entity));
        })
    }

    fn on_remove() -> Option<crate::component::ComponentLifecycleCallback> {
        Some(|mut world, context| {
            world.queue_command(UnregisterListenerCommand::<S>::new(context.entity));
        })
    }
}

pub struct RegisterListenerCommand<S: Signal>(Entity, PhantomData<fn() -> S>);

impl<S: Signal> RegisterListenerCommand<S> {
    pub fn new(entity: Entity) -> Self {
        Self(entity, PhantomData)
    }
}

impl<S: Signal> Command for RegisterListenerCommand<S> {
    fn execute(self, world: &mut crate::World) {
        // Earlier lifecycle commands may already have removed this listener.
        if world
            .get_component_for_entity::<Listener<S>>(self.0)
            .is_some()
        {
            world.listeners_mut().register_listener::<S>(self.0);
        }
    }
}

pub struct UnregisterListenerCommand<S: Signal>(Entity, PhantomData<fn() -> S>);

impl<S: Signal> UnregisterListenerCommand<S> {
    pub fn new(entity: Entity) -> Self {
        Self(entity, PhantomData)
    }
}

impl<S: Signal> Command for UnregisterListenerCommand<S> {
    fn execute(self, world: &mut crate::World) {
        world.listeners_mut().unregister_listener::<S>(self.0);
    }
}

pub trait ListenerSystem<S: Signal>: System<In = On<'static, S>> + 'static {}

impl<S: Signal, T: System<In = On<'static, S>>> ListenerSystem<S> for T {}

pub trait IntoListener<S: Signal, M> {
    fn into_listener(self) -> Listener<S>;
}

impl<S: Signal, T, M> IntoListener<S, M> for T
where
    T: IntoListenerSystem<S, M>,
{
    fn into_listener(self) -> Listener<S> {
        Listener {
            system: Box::new(self.into_listener_system()),
            marker: PhantomData,
        }
    }
}

pub trait IntoListenerSystem<S: Signal, Marker> {
    type ListenerSystem: ListenerSystem<S>;

    fn into_listener_system(self) -> Self::ListenerSystem;
}

impl<S, Marker, T> IntoListenerSystem<S, Marker> for T
where
    S: Signal,
    T: IntoSystem<On<'static, S>, Marker>,
{
    type ListenerSystem = T::System;

    fn into_listener_system(self) -> Self::ListenerSystem {
        self.into_system()
    }
}

#[derive(Default)]
pub(crate) struct Listeners {
    entities: TypeIdMap<Vec<Entity>>,
}

impl Listeners {
    pub(crate) fn register_listener<S: Signal>(&mut self, entity: Entity) {
        let entities = self.entities.entry(TypeId::of::<S>()).or_default();
        // Replacement fires on_add again; keep the original registration order.
        if !entities.contains(&entity) {
            entities.push(entity);
        }
    }

    pub(crate) fn unregister_listener<S: Signal>(&mut self, entity: Entity) {
        let id = TypeId::of::<S>();
        if let Some(entities) = self.entities.get_mut(&id) {
            entities.retain(|registered| *registered != entity);
            if entities.is_empty() {
                self.entities.remove(&id);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::World;

    struct First;
    impl Signal for First {}

    struct Second;
    impl Signal for Second {}

    fn first(_: On<First>) {}

    fn registered<S: Signal>(world: &World) -> &[Entity] {
        world
            .listeners()
            .entities
            .get(&TypeId::of::<S>())
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    #[test]
    fn add_listener_registers_its_component_hooks() {
        let mut world = World::new();
        world.add_listener(first);
        let entity = registered::<First>(&world)[0];

        world.despawn(entity);

        assert!(world.listeners().entities.is_empty());
    }

    #[test]
    fn replacement_keeps_one_registration_in_order() {
        let mut world = World::default();
        world.register_component::<Listener<First>>();
        let first_entity = world.spawn(first.into_listener());
        let second_entity = world.spawn(first.into_listener());

        world.insert(first.into_listener(), first_entity);

        assert_eq!(registered::<First>(&world), &[first_entity, second_entity]);
        world.remove_component::<Listener<First>>(first_entity);
        assert_eq!(registered::<First>(&world), &[second_entity]);
        world.insert(first.into_listener(), first_entity);
        assert_eq!(registered::<First>(&world), &[second_entity, first_entity]);
    }

    #[test]
    fn removal_is_scoped_to_the_signal_type() {
        let mut world = World::new();
        world.register_component::<Listener<First>>();
        world.register_component::<Listener<Second>>();
        let entity = world.spawn((first.into_listener(), (|_: On<Second>| {}).into_listener()));

        world.remove_component::<Listener<First>>(entity);

        assert!(registered::<First>(&world).is_empty());
        assert_eq!(registered::<Second>(&world), &[entity]);
        world.despawn(entity);
        assert!(world.listeners().entities.is_empty());
    }

    #[test]
    fn stale_commands_do_not_register_removed_or_recycled_entities() {
        let mut world = World::new();
        world.register_component::<Listener<First>>();
        let old = world.spawn(first.into_listener());
        world.despawn(old);
        let new = world.spawn(first.into_listener());

        RegisterListenerCommand::<First>::new(old).execute(&mut world);
        UnregisterListenerCommand::<First>::new(old).execute(&mut world);
        assert_eq!(registered::<First>(&world), &[new]);

        world.remove_component::<Listener<First>>(new);
        RegisterListenerCommand::<First>::new(new).execute(&mut world);
        assert!(world.listeners().entities.is_empty());
    }
}

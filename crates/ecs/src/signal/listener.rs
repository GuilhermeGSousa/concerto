use std::{any::TypeId, collections::HashMap, marker::PhantomData};

use crate::{
    Component, Entity, IntoSystem, System,
    command::Command,
    signal::{On, Signal},
    system::{access::SystemAccess, meta::SystemMetadata},
    utilities::TypeIdMap,
    world::UnsafeWorldCell,
};

/// Marks an entity that `World::add_listener` spawned to hold a listener and nothing else.
#[derive(Component)]
pub struct GlobalListener;

pub struct Listener<S: Signal> {
    // Taken while running so the system can safely access or reshape the world.
    state: Option<ListenerState<S>>,
    marker: PhantomData<S>,
}

struct ListenerState<S: Signal> {
    system: Box<dyn ListenerSystem<S>>,
    initialized: bool,
}

impl<S: Signal> Listener<S> {
    pub(crate) fn run(entity: Entity, signal: &mut S, world: &mut crate::World) {
        let Some(listener) = world.get_component_for_entity_mut::<Self>(entity) else {
            return;
        };
        let state = listener
            .state
            .take()
            .expect("Cannot recursively run an active listener");

        let mut guard = RunningListener {
            world,
            entity,
            state: Some(state),
        };
        let state = guard.state.as_mut().unwrap();
        if !state.initialized {
            state.system.initialize(guard.world);
            state.initialized = true;
        }
        state
            .system
            .run_and_apply(On { signal, entity }, guard.world);
    }
}

impl<S: Signal> Listener<S> {
    pub(crate) fn initialize(&mut self, world: &mut crate::World) {
        if let Some(state) = self.state.as_mut()
            && !state.initialized
        {
            state.system.initialize(world);
            state.initialized = true;
        }
    }

    pub(crate) fn access(&self) -> SystemAccess {
        let mut meta = SystemMetadata::default();
        let mut access = SystemAccess::default();
        if let Some(state) = &self.state {
            state.system.fill_access(&mut meta, &mut access);
        }
        access
    }

    pub(crate) fn system_name(&self) -> &'static str {
        self.state
            .as_ref()
            .map_or("a listener", |state| state.system.name())
    }

    /// Runs the listener held by `holder` without applying its commands; returns whether it ran.
    ///
    /// # Safety
    /// The caller must hold the world exclusively, and the listener must have been initialized.
    pub(crate) unsafe fn run_in_operation(
        holder: Entity,
        signal: &mut S,
        cell: UnsafeWorldCell,
    ) -> bool {
        // SAFETY: the caller holds the world exclusively.
        let Some(listener) =
            unsafe { cell.world_mut() }.get_component_for_entity_mut::<Self>(holder)
        else {
            return false;
        };
        let state = listener
            .state
            .take()
            .expect("Cannot recursively run an active listener");
        let mut guard = OperationListener {
            cell,
            holder,
            state: Some(state),
        };
        let state = guard.state.as_mut().unwrap();
        // SAFETY: the caller holds the world exclusively, and a lifecycle listener was refused
        // at registration if it declared access to the whole world.
        unsafe {
            state.system.run_unsafe(
                On {
                    signal,
                    entity: holder,
                },
                cell,
            )
        };
        true
    }
}

/// Puts a listener's state back after it ran inside a world operation, even if it panicked.
struct OperationListener<'w, S: Signal> {
    cell: UnsafeWorldCell<'w>,
    holder: Entity,
    state: Option<ListenerState<S>>,
}

impl<S: Signal> Drop for OperationListener<'_, S> {
    fn drop(&mut self) {
        // SAFETY: the operation that ran the listener still holds the world exclusively.
        if let Some(listener) = unsafe { self.cell.world_mut() }
            .get_component_for_entity_mut::<Listener<S>>(self.holder)
            && listener.state.is_none()
        {
            listener.state = self.state.take();
        }
    }
}

/// Applies the commands a listener queued while it ran inside a world operation.
pub(crate) struct ApplyListenerCommand<S: Signal>(Entity, PhantomData<fn() -> S>);

impl<S: Signal> ApplyListenerCommand<S> {
    pub(crate) fn new(holder: Entity) -> Self {
        Self(holder, PhantomData)
    }
}

impl<S: Signal> Command for ApplyListenerCommand<S> {
    fn execute(self, world: &mut crate::World) {
        let Some(listener) = world.get_component_for_entity_mut::<Listener<S>>(self.0) else {
            return;
        };
        let Some(state) = listener.state.take() else {
            return;
        };
        let mut guard = RunningListener {
            world,
            entity: self.0,
            state: Some(state),
        };
        guard.state.as_mut().unwrap().system.apply(guard.world);
    }
}

/// Restores system state even if initialization, execution, or command application panics.
struct RunningListener<'w, S: Signal> {
    world: &'w mut crate::World,
    entity: Entity,
    state: Option<ListenerState<S>>,
}

impl<S: Signal> Drop for RunningListener<'_, S> {
    fn drop(&mut self) {
        if let Some(listener) = self
            .world
            .get_component_for_entity_mut::<Listener<S>>(self.entity)
            && listener.state.is_none()
        {
            listener.state = self.state.take();
        }
    }
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

#[doc(hidden)]
pub struct IsListenerSystem;

impl<S: Signal, T, M> IntoListener<S, (IsListenerSystem, M)> for T
where
    T: IntoListenerSystem<S, M>,
{
    fn into_listener(self) -> Listener<S> {
        Listener {
            state: Some(ListenerState {
                system: Box::new(self.into_listener_system()),
                initialized: false,
            }),
            marker: PhantomData,
        }
    }
}

#[doc(hidden)]
pub struct IsListener;

/// A listener already built, such as one returned by a function whose closure captured data.
impl<S: Signal> IntoListener<S, IsListener> for Listener<S> {
    fn into_listener(self) -> Listener<S> {
        self
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
struct ListenerOrder {
    slots: Vec<Option<Entity>>,
    index: HashMap<Entity, usize>,
}

impl ListenerOrder {
    fn insert(&mut self, entity: Entity) {
        if !self.index.contains_key(&entity) {
            self.index.insert(entity, self.slots.len());
            self.slots.push(Some(entity));
        }
    }

    fn remove(&mut self, entity: Entity) {
        let Some(slot) = self.index.remove(&entity) else {
            return;
        };
        self.slots[slot] = None;
        if self.slots.len() > 2 * self.index.len() {
            self.slots.retain(Option::is_some);
            for (slot, entity) in self.slots.iter().enumerate() {
                if let Some(entity) = entity {
                    self.index.insert(*entity, slot);
                }
            }
        }
    }

    fn is_empty(&self) -> bool {
        self.index.is_empty()
    }
}

#[derive(Default)]
pub(crate) struct Listeners {
    entities: TypeIdMap<ListenerOrder>,
}

impl Listeners {
    pub(crate) fn get<S: Signal>(&self) -> Vec<Entity> {
        self.entities
            .get(&TypeId::of::<S>())
            .map(|order| order.slots.iter().flatten().copied().collect())
            .unwrap_or_default()
    }

    pub(crate) fn register_listener<S: Signal>(&mut self, entity: Entity) {
        self.entities
            .entry(TypeId::of::<S>())
            .or_default()
            .insert(entity);
    }

    pub(crate) fn unregister_listener<S: Signal>(&mut self, entity: Entity) {
        let id = TypeId::of::<S>();
        if let Some(entities) = self.entities.get_mut(&id) {
            entities.remove(entity);
            if entities.is_empty() {
                self.entities.remove(&id);
            }
        }
    }
}

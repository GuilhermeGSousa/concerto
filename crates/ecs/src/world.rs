use anymap3::AnyMap;
use log::warn;
use std::collections::hash_map::Entry::{Occupied, Vacant};
use std::{any::TypeId, cell::UnsafeCell, collections::HashMap, marker::PhantomData, ptr};

use crate::command::{Command, CommandQueue, CommandQueueState};
use crate::component::bundle::{ComponentBundle, IntoBundle, MergeRow, PushRow, ReplaceRow};
use crate::component::registry::{ComponentRegistry, TypeInfo};
use crate::component::scene::{SceneComponent, SceneSpawnContext};
use crate::entity::EntityWorldMut;
use crate::entity::entity_store::EntityStore;
use crate::query::QueryData;
use crate::query::filter::QueryFilter;
use crate::query::state::QueryState;
use crate::resource::ResourceStorage;
use crate::signal::Signal;
use crate::signal::listener::{IntoListener, Listener, Listeners};
use crate::system::schedule::{CompiledSchedules, ScheduleLabel};
use crate::table::MutableCellAccessor;
use crate::{
    archetype::Archetype,
    common::generate_type_id,
    component::{Component, ComponentId, ComponentLifecycleCallbacks, ComponentLifecycleContext},
    entity::{Entity, EntityLocation, EntityStructuralVersion, EntityType},
    resource::Resource,
    system::input::SystemInput,
    table::TableRowIndex,
    utilities::TypeIdMap,
};
use crate::{component::Tick, system::meta::SystemMetadata};

/// The central container of the ECS.
///
/// A `World` stores all entities together with their components and all
/// globally-shared resources. Systems receive a reference to the world (or
/// typed wrappers around it) and use it to read and write game state.
///
/// # Example
/// ```
/// use concerto_ecs::{World, Component};
///
/// #[derive(Component)]
/// struct Health(f32);
///
/// let mut world = World::new();
/// let entity = world.spawn(Health(100.0));
/// ```
#[derive(Default)]
pub struct World {
    archetypes: Vec<Archetype>,
    resources: AnyMap,
    component_registry: ComponentRegistry,
    entity_store: EntityStore,
    archetype_index: HashMap<EntityType, usize>,
    component_lifetimes: TypeIdMap<ComponentLifecycleCallbacks>,
    listeners: Listeners,
    current_tick: u32,
    command_queue_start: usize,
    command_queue: CommandQueueState,
}

unsafe impl Send for World {}
unsafe impl Sync for World {}

impl World {
    /// Creates a new, empty `World` with no entities or resources.
    pub fn new() -> World {
        Self::default()
    }

    /// Spawns a new entity with the given component bundle and returns its [`Entity`] handle.
    pub fn spawn<T: IntoBundle>(&mut self, bundle: T) -> Entity {
        let entity = self.entity_store.alloc();
        self.spawn_allocated_internal(entity, bundle.into_bundle());
        self.flush_commands();
        entity
    }

    /// Spawns onto an entity handle reserved earlier, typically by a command.
    pub(crate) fn spawn_allocated<T: ComponentBundle>(&mut self, entity: Entity, bundle: T) {
        self.spawn_allocated_internal(entity, bundle);
        self.flush_commands();
    }

    fn spawn_allocated_internal<T: ComponentBundle>(&mut self, entity: Entity, bundle: T) {
        let type_ids = T::get_component_ids();
        let entity_type = generate_type_id(&type_ids);

        let archetype_id = self.archetypes().len().into();
        let archetype_index = self
            .archetype_index
            .entry(entity_type.clone())
            .or_insert_with(|| {
                let archetype = Archetype::new(T::generate_empty_table(), type_ids, archetype_id);
                self.archetypes.push(archetype);
                self.archetypes.len() - 1
            });

        let archetype: &mut Archetype = &mut self.archetypes[*archetype_index];

        let row = TableRowIndex::new(archetype.len());
        archetype.add_entity(entity);
        bundle.write_into(&mut PushRow(archetype), self.current_tick);

        let new_location = EntityLocation {
            archetype_index: *archetype_index as u32,
            row,
        };

        self.entity_store.set_location(entity, new_location);

        let cell = self.as_unsafe_world_cell_mut();
        cell.trigger_on_add(entity, &T::get_component_ids());
    }

    /// Removes an entity, its components, and its descendants from the world.
    pub fn despawn(&mut self, entity: Entity) {
        self.despawn_internal(entity);
        self.flush_commands();
    }

    fn despawn_internal(&mut self, entity: Entity) {
        match self.entity_store.find_location(entity) {
            Some(location) => {
                {
                    let component_ids = self.archetypes[location.archetype_index as usize]
                        .component_ids()
                        .to_vec();
                    let cell = self.as_unsafe_world_cell_mut();
                    cell.trigger_on_despawn(entity, &component_ids);
                }

                let archetype = &mut self.archetypes[location.archetype_index as usize];

                if let Some(swapped_entity) = archetype.entities().last()
                    && *swapped_entity != entity
                {
                    self.entity_store.set_location(*swapped_entity, location);
                }

                archetype.drop_row(location.row);
                self.entity_store.free(entity);
            }
            None => warn!("Attempted to despawn invalid entity {:?}", entity),
        }
    }

    pub fn archetypes(&self) -> &[Archetype] {
        &self.archetypes
    }

    pub fn get_archetypes_mut(&mut self) -> &mut Vec<Archetype> {
        &mut self.archetypes
    }

    pub fn query<T: QueryData, F: QueryFilter>(&mut self) -> QueryState<T, F> {
        QueryState::<T, F>::new(self)
    }

    /// Adds components to an existing entity, migrating it to the appropriate archetype.
    ///
    /// If the entity already has a component of type `T`, the existing value is replaced.
    pub fn insert<T: IntoBundle>(&mut self, bundle: T, entity: Entity) {
        self.insert_internal(bundle.into_bundle(), entity);
        self.flush_commands();
    }

    /// Version of a live entity's component set, or `None` if `entity` is not live.
    pub fn structural_version(&self, entity: Entity) -> Option<EntityStructuralVersion> {
        self.entity_store.structural_version(entity)
    }

    pub fn entity_is_valid(&self, entity: Entity) -> bool {
        self.entity_store.find_location(entity).is_some()
    }

    /// Removes a component of type `T` from an entity, migrating it to the appropriate archetype.
    ///
    /// If the entity does not have a component of type `T`, a warning is logged and the call
    /// is a no-op.
    pub fn remove_component<T: Component>(&mut self, entity: Entity) {
        self.remove_component_internal::<T>(entity);
        self.flush_commands();
    }

    fn insert_internal<T: ComponentBundle>(&mut self, components: T, entity: Entity) {
        let Some(location) = self.entity_store.find_location(entity) else {
            panic!("Entity should exist in the world");
        };

        let source_index = location.archetype_index as usize;
        let inserted_ids = T::get_component_ids();

        let replaced_ids: Vec<ComponentId> = inserted_ids
            .iter()
            .copied()
            .filter(|id| self.archetypes[source_index].component_ids().contains(id))
            .collect();
        self.as_unsafe_world_cell_mut()
            .trigger_on_replace(entity, &replaced_ids);

        let mut component_ids = self.archetypes[source_index].component_ids().to_vec();
        for id in &inserted_ids {
            if !component_ids.contains(id) {
                component_ids.push(*id);
            }
        }

        let entity_type = generate_type_id(&component_ids);

        let destination_index = match self.archetype_index.entry(entity_type) {
            Occupied(occupied_entry) => *occupied_entry.get(),
            Vacant(vacant_entry) => {
                let new_index = self.archetypes.len();
                vacant_entry.insert(new_index);

                let mut table = self.archetypes[source_index].clone_empty_table();
                table.merge(T::generate_empty_table());
                self.archetypes.push(Archetype::new(
                    table,
                    component_ids,
                    self.archetypes().len().into(),
                ));

                new_index
            }
        };

        if destination_index == source_index {
            let archetype = &mut self.archetypes[source_index];
            components.write_into(&mut ReplaceRow(archetype, location.row), self.current_tick);
        } else {
            if let Some(swapped_entity) = self.archetypes[source_index].entities().last()
                && *swapped_entity != entity
            {
                self.entity_store.set_location(*swapped_entity, location);
            }

            let [source, destination] = self
                .archetypes
                .get_disjoint_mut([source_index, destination_index])
                .expect("source and destination archetypes are distinct");

            source.move_row_to(location.row, destination);
            let new_row = TableRowIndex::new(destination.len() - 1);
            components.write_into(&mut MergeRow(destination, new_row), self.current_tick);
            let new_location = EntityLocation {
                archetype_index: destination_index as u32,
                row: new_row,
            };
            self.entity_store.set_location(entity, new_location);
        }

        self.as_unsafe_world_cell_mut()
            .trigger_on_add(entity, &inserted_ids);
    }

    fn remove_component_internal<T: Component>(&mut self, entity: Entity) {
        let Some(location) = self.entity_store.find_location(entity) else {
            panic!("Entity should exist in the world");
        };

        let removed_id = TypeId::of::<T>();
        let source_index = location.archetype_index as usize;
        let mut component_ids = self.archetypes[source_index].component_ids().to_vec();
        let Some(removed_index) = component_ids.iter().position(|id| *id == removed_id) else {
            warn!("Entity does not have the component being removed.");
            return;
        };

        self.as_unsafe_world_cell_mut()
            .trigger_on_remove_component(entity, &removed_id);

        component_ids.swap_remove(removed_index);

        let entity_type = generate_type_id(&component_ids);

        let destination_index = match self.archetype_index.entry(entity_type) {
            Occupied(occupied_entry) => *occupied_entry.get(),
            Vacant(vacant_entry) => {
                let new_index = self.archetypes.len();
                vacant_entry.insert(new_index);

                let mut table = self.archetypes[source_index].clone_empty_table();
                table.remove_column(removed_id);
                self.archetypes.push(Archetype::new(
                    table,
                    component_ids,
                    self.archetypes().len().into(),
                ));

                new_index
            }
        };

        // The source archetype's last row is about to be swapped into the hole this migration
        // leaves behind, so that entity's recorded location must follow it. This must stay
        // below the missing-component early return — a no-op removal must not touch other
        // entities' locations.
        if let Some(swapped_entity) = self.archetypes[source_index].entities().last()
            && *swapped_entity != entity
        {
            self.entity_store.set_location(*swapped_entity, location);
        }

        let [source, destination] = self
            .archetypes
            .get_disjoint_mut([source_index, destination_index])
            .expect("source and destination archetypes are distinct");

        // `destination` has no column for `T`, so the move drops the removed component.
        source.move_row_to(location.row, destination);

        let new_location = EntityLocation {
            archetype_index: destination_index as u32,
            row: TableRowIndex::new(destination.len() - 1),
        };
        self.entity_store.set_location(entity, new_location);
    }

    pub(crate) fn entity_store(&self) -> &EntityStore {
        &self.entity_store
    }

    /// Returns a shared reference to the component of type `T` on `entity`, or `None` if absent.
    pub fn get_component_for_entity<T: Component>(&self, entity: Entity) -> Option<&T> {
        self.entity_store
            .find_location(entity)
            .and_then(|location| self.get_component_for_entity_location(location))
    }

    /// Returns an exclusive reference to the component of type `T` on `entity`, or `None`.
    ///
    /// Marks the component as changed for this tick so change-detection filters fire correctly.
    pub fn get_component_for_entity_mut<T: Component>(&mut self, entity: Entity) -> Option<&mut T> {
        let current_tick = self.current_tick;
        self.entity_store
            .find_location(entity)
            .and_then(|location| {
                self.get_component_for_entity_location_mut(location)
                    .map(|accessor| {
                        accessor.changed_tick.set(current_tick);
                        accessor.data
                    })
            })
    }

    pub(crate) fn get_component_for_entity_location<T: Component>(
        &self,
        entity_location: EntityLocation,
    ) -> Option<&T> {
        self.archetypes
            .get(entity_location.archetype_index as usize)
            .and_then(|archetype| unsafe { archetype.get_component_unsafe(entity_location.row) })
    }

    pub(crate) fn get_component_for_entity_location_mut<T: Component>(
        &mut self,
        entity_location: EntityLocation,
    ) -> Option<MutableCellAccessor<'_, T>> {
        self.archetypes
            .get_mut(entity_location.archetype_index as usize)
            .and_then(|archetype| unsafe {
                archetype.get_component_unsafe_mut(entity_location.row)
            })
    }

    /// Inserts a resource into the world.  If one of the same type already exists it is replaced.
    pub fn insert_resource<T: Resource>(&mut self, resource: T) {
        self.resources
            .insert(ResourceStorage::new(resource, self.current_tick));
    }

    // Inserts a resource into the world with its default value.
    pub fn init_resource<T: Resource + Default>(&mut self) {
        self.resources
            .insert(ResourceStorage::new(T::default(), self.current_tick));
    }

    /// Removes and returns the resource of type `T`, or `None` if it was not present.
    pub fn remove_resource<T: Resource + 'static>(&mut self) -> Option<T> {
        self.resources
            .remove::<ResourceStorage<T>>()
            .map(|resource_storage| resource_storage.data)
    }

    /// Returns a shared reference to the resource of type `T`, or `None` if not present.
    pub fn get_resource<T: Resource + 'static>(&self) -> Option<&T> {
        self.resources
            .get::<ResourceStorage<T>>()
            .map(|resource_storage| &resource_storage.data)
    }

    /// Returns an exclusive reference to the resource of type `T`, or `None` if not present.
    pub fn get_resource_mut<T: Resource + 'static>(&mut self) -> Option<&mut T> {
        self.resources
            .get_mut::<ResourceStorage<T>>()
            .map(|resource_storage| &mut resource_storage.data)
    }

    /// The tick of the last write to `T`, or `None` if it is not present.
    pub fn resource_changed_tick<T: Resource + 'static>(&self) -> Option<Tick> {
        self.get_resource_storage::<T>()
            .map(|storage| storage.changed_tick)
    }

    pub(crate) fn get_resource_storage<T: Resource + 'static>(
        &self,
    ) -> Option<&ResourceStorage<T>> {
        self.resources.get()
    }

    pub(crate) fn get_resource_storage_mut<T: Resource + 'static>(
        &mut self,
    ) -> Option<&mut ResourceStorage<T>> {
        self.resources.get_mut()
    }

    pub fn as_unsafe_world_cell_mut(&mut self) -> UnsafeWorldCell<'_> {
        UnsafeWorldCell::new_mutable(self)
    }

    pub fn as_unsafe_world_cell(&self) -> UnsafeWorldCell<'_> {
        UnsafeWorldCell::new_ref(self)
    }

    /// Advances the world's internal tick counter.
    ///
    /// Call this once per frame (after all systems have run) so that change-detection
    /// filters (`Added`, `Changed`) produce fresh results on the next frame.
    pub fn tick(&mut self) {
        self.current_tick += 1;
    }

    pub fn current_tick(&self) -> Tick {
        Tick::new(self.current_tick)
    }

    pub fn was_component_added(&self, entity: Entity, component_id: ComponentId) -> bool {
        if let Some(location) = self.entity_store.find_location(entity) {
            self.archetypes[location.archetype_index as usize].was_entity_added(
                component_id,
                location.row,
                self.current_tick,
            )
        } else {
            false
        }
    }

    /// Whether an existing component was added, replaced, or mutably accessed
    /// between `since` and the current tick, inclusive. Returns false if absent.
    ///
    /// Unlike `was_component_changed`, this also observes changes from earlier
    /// frames. Pass the tick of the last read. The inclusive boundary catches
    /// writes later in that same tick; it can conservatively report a change
    /// already seen by the reader. Keep observations less than a full u32 tick
    /// cycle apart. This query does not mark the component changed.
    pub fn has_component_changed_since(
        &self,
        entity: Entity,
        component_id: ComponentId,
        since: Tick,
    ) -> bool {
        self.entity_store
            .find_location(entity)
            .is_some_and(|location| {
                self.archetypes[location.archetype_index as usize].has_component_changed_since(
                    component_id,
                    location.row,
                    since,
                    self.current_tick(),
                )
            })
    }

    pub fn was_component_changed(&self, entity: Entity, component_id: ComponentId) -> bool {
        if let Some(location) = self.entity_store.find_location(entity) {
            self.archetypes[location.archetype_index as usize].was_entity_changed(
                component_id,
                location.row,
                self.current_tick,
            )
        } else {
            false
        }
    }

    pub(crate) fn register_component_lifetimes<T: Component>(&mut self) {
        self.component_lifetimes
            .entry(ComponentId::of::<T>())
            .or_insert(ComponentLifecycleCallbacks::from_component::<T>());
    }

    /// Borrows one entity for hierarchy edits.
    pub fn entity_mut(&mut self, entity: Entity) -> EntityWorldMut<'_> {
        EntityWorldMut::new(self, entity)
    }

    pub fn register_component<T: Component>(&mut self) {
        self.component_registry.register_component::<T>();
        self.register_component_lifetimes::<T>();
    }

    pub fn register_component_type<T: SceneComponent>(&mut self) {
        self.component_registry.register_scene_component::<T>();
    }

    /// Every component `entity` carries, including ones that are not scene
    /// components. Empty for a stale entity.
    pub fn component_ids(&self, entity: Entity) -> &[ComponentId] {
        self.entity_store
            .find_location(entity)
            .map(|location| self.archetypes[location.archetype_index as usize].component_ids())
            .unwrap_or(&[])
    }

    /// The read side of a registered scene component, or `None` if `id` is not one.
    pub fn type_info(&self, id: ComponentId) -> Option<&TypeInfo> {
        self.component_registry.type_info(&id)
    }

    /// The registered scene components `entity` currently carries.
    ///
    /// Empty for a stale entity handle. Components registered only through
    /// [`register_component`](Self::register_component) — engine plumbing such
    /// as render-world mirrors — are deliberately not listed: this reports what
    /// a scene can describe, which is what a tool wants to show.
    pub fn component_types(&self, entity: Entity) -> impl Iterator<Item = &TypeInfo> {
        self.component_ids(entity)
            .iter()
            .filter_map(|component_id| self.component_registry.type_info(component_id))
    }

    /// Deserializes `json` into the component registered under `type_name` and
    /// applies it to `entity`. Returns `false` (having logged why) when the name
    /// is unregistered or the payload does not parse — a scene may carry
    /// components this application does not know about.
    pub fn apply_scene_component(
        &mut self,
        type_name: &str,
        json: &str,
        entity: Entity,
        node_entities: &[Entity],
    ) -> bool {
        let applied = self.apply_scene_component_internal(type_name, json, entity, node_entities);
        self.flush_commands();
        applied
    }

    fn apply_scene_component_internal(
        &mut self,
        type_name: &str,
        json: &str,
        entity: Entity,
        node_entities: &[Entity],
    ) -> bool {
        let Some(apply) = self.component_registry.get_scene_component(type_name) else {
            warn!(
                "Skipping component '{type_name}': no type registered under that name \
                 (register it with App::register_component)"
            );
            return false;
        };

        let mut ctx = SceneSpawnContext::new(self, node_entities);
        match apply(json, entity, &mut ctx) {
            Ok(()) => true,
            Err(err) => {
                warn!("Failed to deserialize component '{type_name}' from `{json}`: {err}");
                false
            }
        }
    }

    pub(crate) fn make_command_queue<'world>(&'world mut self) -> CommandQueue<'world, 'world> {
        CommandQueue::for_callbacks(&mut self.command_queue, &self.entity_store)
    }

    /// Appends `commands` to the pending queue and runs everything in it.
    pub(crate) fn apply_commands(&mut self, commands: &mut CommandQueueState) {
        self.command_queue.append(commands);
        self.flush_commands();
    }

    fn command_queue_is_empty(&self) -> bool {
        self.command_queue_start >= self.command_queue.len()
    }

    fn flush_commands(&mut self) {
        if self.command_queue_is_empty() {
            return;
        }

        struct FlushGuard<'w> {
            world: &'w mut World,
            start: usize,
        }

        impl Drop for FlushGuard<'_> {
            fn drop(&mut self) {
                self.world.command_queue_start = self.start;
                self.world.command_queue.truncate(self.start);
            }
        }

        let start = self.command_queue_start;
        let end = self.command_queue.len();
        let guard = FlushGuard { world: self, start };
        guard.world.command_queue_start = end;

        for cursor in start..end {
            let Some((command, consume)) = guard.world.command_queue.take_command(cursor) else {
                continue;
            };

            // SAFETY: `take_command` hands out a live command once, paired with
            // the function that was written alongside it.
            unsafe { consume(command, Some(guard.world)) };

            guard.world.flush_commands();
        }
    }

    pub fn run_schedule(&mut self, label: impl ScheduleLabel) {
        let label = label.intern();

        let Some(schedules) = self.get_resource_mut::<CompiledSchedules>() else {
            return;
        };
        let Some(mut schedule) = schedules.remove(label) else {
            return;
        };

        schedule.run(self);

        if let Some(schedules) = self.get_resource_mut::<CompiledSchedules>() {
            schedules.insert(label, schedule);
        }
    }

    pub fn add_listener<T: Signal, M>(&mut self, system: impl IntoListener<T, M>) {
        self.register_component_lifetimes::<Listener<T>>();
        self.spawn(system.into_listener());
    }

    /// Runs matching listeners in registration order, applying each system's commands.
    ///
    /// Listeners added during dispatch start receiving signals on the next trigger.
    /// Listeners removed before their turn are skipped. System state is initialized
    /// on first use and retained between triggers.
    ///
    /// # Panics
    /// Panics if a nested trigger tries to run a listener that is already running.
    pub fn trigger<T: Signal>(&mut self, mut signal: T) {
        let entities = self.listeners().get::<T>().to_vec();
        for entity in entities {
            Listener::<T>::run(entity, &mut signal, self);
        }
    }

    pub fn trigger_default<T: Signal + Default>(&mut self) {
        self.trigger(T::default());
    }

    pub(crate) fn listeners(&self) -> &Listeners {
        &self.listeners
    }

    pub(crate) fn listeners_mut(&mut self) -> &mut Listeners {
        &mut self.listeners
    }
}

#[derive(Copy, Clone)]
/// A raw, lifetime-scoped pointer to a [`World`].
///
/// `UnsafeWorldCell` is used internally so that multiple system parameters
/// (queries, resources, event readers/writers) can all borrow from the same
/// world simultaneously at runtime.  Using it incorrectly can cause undefined
/// behaviour; prefer the safe wrappers [`Query`](crate::query::Query),
/// [`Res`](crate::resource::Res) and [`ResMut`](crate::resource::ResMut) in
/// normal system code.
pub struct UnsafeWorldCell<'w> {
    ptr: *mut World,
    is_mutable: bool,
    _marker: PhantomData<(&'w World, &'w UnsafeCell<World>)>,
}

unsafe impl Send for UnsafeWorldCell<'_> {}
unsafe impl Sync for UnsafeWorldCell<'_> {}

impl<'w> From<&'w mut World> for UnsafeWorldCell<'w> {
    fn from(value: &'w mut World) -> Self {
        value.as_unsafe_world_cell_mut()
    }
}

impl<'w> From<&'w World> for UnsafeWorldCell<'w> {
    fn from(value: &'w World) -> Self {
        value.as_unsafe_world_cell()
    }
}

impl SystemInput for &World {
    type State = ();
    type Data<'world, 'state> = &'world World;

    fn init_state(_world: &mut World) -> Self::State {}

    fn get_data<'world, 'state>(
        _state: &'state mut Self::State,
        world: UnsafeWorldCell<'world>,
    ) -> Self::Data<'world, 'state> {
        world.world()
    }

    fn fill_access(_meta: &mut SystemMetadata, access: &mut crate::system::access::SystemAccess) {
        access.read_world();
    }
}

impl SystemInput for &mut World {
    type State = ();
    type Data<'world, 'state> = &'world mut World;

    fn init_state(_world: &mut World) -> Self::State {}

    fn get_data<'world, 'state>(
        _state: &'state mut Self::State,
        world: UnsafeWorldCell<'world>,
    ) -> Self::Data<'world, 'state> {
        world.world_mut()
    }

    fn fill_access(_meta: &mut SystemMetadata, access: &mut crate::system::access::SystemAccess) {
        access.write_world();
    }
}

impl<'w> UnsafeWorldCell<'w> {
    fn assert_mutable(&self) {
        debug_assert!(self.is_mutable, "UnsafeWorldCell is not mutable");
    }

    pub(crate) fn new_mutable(world: &'w mut World) -> Self {
        Self {
            ptr: ptr::from_mut(world),
            is_mutable: true,
            _marker: PhantomData,
        }
    }

    pub(crate) fn new_ref(world: &'w World) -> Self {
        Self {
            ptr: ptr::from_ref(world).cast_mut(),
            is_mutable: false,
            _marker: PhantomData,
        }
    }

    pub fn world(&self) -> &'w World {
        unsafe { &*self.ptr }
    }

    pub fn world_mut(&self) -> &'w mut World {
        self.assert_mutable();
        unsafe { &mut *self.ptr }
    }

    pub(crate) fn into_restricted(self) -> RestrictedWorld<'w> {
        RestrictedWorld { world_cell: self }
    }

    pub(crate) fn trigger_on_add(&self, entity: Entity, ids: &[ComponentId]) {
        for id in ids {
            self.trigger_on_add_component(entity, id);
        }
    }

    pub(crate) fn trigger_on_add_component(&self, entity: Entity, id: &ComponentId) {
        let world = self.world();
        if let Some(lifetimes) = world.component_lifetimes.get(id)
            && let Some(add) = lifetimes.on_add
        {
            add(self.into_restricted(), ComponentLifecycleContext { entity });
        }
    }

    pub(crate) fn trigger_on_replace(&self, entity: Entity, ids: &[ComponentId]) {
        for id in ids {
            self.trigger_on_replace_component(entity, id);
        }
    }

    pub(crate) fn trigger_on_replace_component(&self, entity: Entity, id: &ComponentId) {
        let world = self.world();

        if let Some(lifetimes) = world.component_lifetimes.get(id)
            && let Some(replace) = lifetimes.on_replace
        {
            replace(self.into_restricted(), ComponentLifecycleContext { entity });
        }
    }

    pub(crate) fn trigger_on_remove_component(&self, entity: Entity, id: &ComponentId) {
        let world = self.world();

        if let Some(lifetimes) = world.component_lifetimes.get(id)
            && let Some(remove) = lifetimes.on_remove
        {
            remove(self.into_restricted(), ComponentLifecycleContext { entity });
        }
    }

    /// Despawning removes every component, so each one's `on_remove` runs before the
    /// `on_despawn` reserved for teardown that only makes sense for a dying entity.
    pub(crate) fn trigger_on_despawn(&self, entity: Entity, ids: &[ComponentId]) {
        for id in ids {
            self.trigger_on_remove_component(entity, id);
            self.trigger_on_despawn_component(entity, id);
        }
    }

    pub(crate) fn trigger_on_despawn_component(&self, entity: Entity, id: &ComponentId) {
        let world = self.world();

        if let Some(lifetimes) = world.component_lifetimes.get(id)
            && let Some(despawn) = lifetimes.on_despawn
        {
            despawn(self.into_restricted(), ComponentLifecycleContext { entity });
        }
    }

    pub(crate) fn archetypes(&self) -> &[Archetype] {
        self.world().archetypes()
    }
}

/// A scoped, restricted view of a [`World`] that is safe to pass into component lifecycle
/// callbacks.
///
/// `RestrictedWorld` is provided to component lifecycle callbacks. It
/// permits immediate changes to existing data, but all structural changes go
/// through commands applied after the enclosing world operation completes.
pub struct RestrictedWorld<'w> {
    world_cell: UnsafeWorldCell<'w>,
}

impl<'w> RestrictedWorld<'w> {
    fn commands(&mut self) -> CommandQueue<'_, '_> {
        self.world_cell.world_mut().make_command_queue()
    }

    /// Reserves an entity and queues its spawn after the enclosing operation.
    pub fn spawn<T: IntoBundle<Bundle: 'static>>(&mut self, components: T) -> Entity {
        self.commands().spawn(components).entity()
    }

    /// Queues despawning after the enclosing operation; the entity stays readable meanwhile.
    pub fn despawn(&mut self, entity: Entity) {
        self.commands().despawn(entity);
    }

    /// Queues an arbitrary command after the enclosing operation, for structural work
    /// whose conditions must be re-checked at flush time.
    pub(crate) fn queue_command<C: Command + 'static>(&mut self, command: C) {
        self.commands().push(command);
    }

    /// Queues insertion after the enclosing operation.
    pub fn insert<T: Component>(&mut self, component: T, entity: Entity) {
        self.commands().insert(component, entity);
    }

    /// Queues removal after the enclosing operation.
    pub fn remove_component<T: Component>(&mut self, entity: Entity) {
        self.commands().remove::<T>(entity);
    }

    pub fn get_component_for_entity<T: Component>(&self, entity: Entity) -> Option<&T> {
        self.world_cell.world().get_component_for_entity(entity)
    }

    pub fn get_component_for_entity_mut<T: Component>(&mut self, entity: Entity) -> Option<&mut T> {
        self.world_cell
            .world_mut()
            .get_component_for_entity_mut(entity)
    }

    pub fn get_resource<T: Resource>(&self) -> Option<&T> {
        self.world_cell.world().get_resource()
    }

    pub fn get_resource_mut<T: Resource>(&mut self) -> Option<&mut T> {
        self.world_cell.world_mut().get_resource_mut::<T>()
    }
}

pub trait FromWorld {
    fn from_world(world: &World) -> Self;
}

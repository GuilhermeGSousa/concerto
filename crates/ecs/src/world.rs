use anymap3::AnyMap;
use log::warn;
use std::collections::hash_map::Entry::{Occupied, Vacant};
use std::sync::atomic::{AtomicU32, Ordering};
use std::{any::TypeId, cell::UnsafeCell, collections::HashMap, marker::PhantomData, ptr};

use crate::command::{Command, CommandQueue, CommandQueueState};
use crate::component::bundle::{ComponentBundle, IntoBundle, MergeRow, PushRow, ReplaceRow};
use crate::component::scene::SceneEntityRef;
use crate::entity::EntityWorldMut;
use crate::entity::entity_store::EntityStore;
use crate::query::QueryData;
use crate::query::filter::QueryFilter;
use crate::query::state::QueryState;
use crate::reflect::{AppTypeRegistry, ReflectSceneComponent, SceneComponentType};
use crate::resource::ResourceStorage;
use crate::signal::listener::{GlobalListener, IntoListener, Listener, Listeners};
use crate::signal::{EntitySignal, Signal};
use crate::system::access::SystemAccess;
use crate::system::schedule::{CompiledSchedules, ScheduleLabel};
use crate::table::{CellAccessor, MutableCellAccessor};
use crate::world_id::WorldId;
use crate::{
    archetype::Archetype,
    common::generate_type_id,
    component::{Component, ComponentId, ComponentLifecycleCallbacks, ComponentLifecycleContext},
    entity::{Entity, EntityLocation, EntityStructuralVersion, EntityType},
    resource::Resource,
    system::input::{SystemInput, ValidationError},
    table::TableRowIndex,
    utilities::TypeIdMap,
};
use crate::{
    component::{CHECK_TICK_THRESHOLD, SystemTicks, Tick},
    system::meta::SystemMetadata,
};
use concerto_reflect::ReflectedValue;
use concerto_reflect::{GetTypeRegistration, TypeRegistration};

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
pub struct World {
    id: WorldId,
    archetypes: Vec<Archetype>,
    resources: AnyMap,
    resource_tick_checks: TypeIdMap<ResourceTickCheck>,
    entity_store: EntityStore,
    archetype_index: HashMap<EntityType, usize>,
    component_lifetimes: TypeIdMap<ComponentLifecycleCallbacks>,
    listeners: Listeners,
    change_tick: AtomicU32,
    last_change_tick: Tick,
    last_check_tick: Tick,
    command_queue_start: usize,
    command_queue: CommandQueueState,
}

type ResourceTickCheck = fn(&mut AnyMap, Tick);

impl Default for World {
    fn default() -> Self {
        Self {
            id: WorldId::default(),
            archetypes: Vec::new(),
            resources: AnyMap::new(),
            resource_tick_checks: TypeIdMap::default(),
            entity_store: EntityStore::default(),
            archetype_index: HashMap::new(),
            component_lifetimes: TypeIdMap::default(),
            listeners: Listeners::default(),
            change_tick: AtomicU32::new(1),
            last_change_tick: Tick::new(0),
            last_check_tick: Tick::new(0),
            command_queue_start: 0,
            command_queue: CommandQueueState::default(),
        }
    }
}

unsafe impl Send for World {}
unsafe impl Sync for World {}

impl World {
    /// Creates a new, empty `World` with no entities or resources.
    pub fn new() -> World {
        let mut world = Self::default();
        world.type_registry_mut();
        world
    }

    /// This world's identity, which it keeps when moved.
    pub fn id(&self) -> WorldId {
        self.id
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

        let change_tick = *self.change_tick();
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
        bundle.write_into(&mut PushRow(archetype), change_tick);

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
            let change_tick = *self.change_tick();
            let archetype = &mut self.archetypes[source_index];
            components.write_into(&mut ReplaceRow(archetype, location.row), change_tick);
        } else {
            let change_tick = *self.change_tick();
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
            components.write_into(&mut MergeRow(destination, new_row), change_tick);
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
    /// Marks the component as changed with the counter's current value so change-detection filters fire correctly.
    pub fn get_component_for_entity_mut<T: Component>(&mut self, entity: Entity) -> Option<&mut T> {
        let change_tick = self.change_tick();
        self.entity_store
            .find_location(entity)
            .and_then(|location| {
                self.get_component_for_entity_location_mut(location)
                    .map(|accessor| {
                        *accessor.changed_tick = change_tick;
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

    pub(crate) fn get_component_for_entity_location_ticked<T: Component>(
        &self,
        entity_location: EntityLocation,
    ) -> Option<CellAccessor<'_, T>> {
        self.archetypes
            .get(entity_location.archetype_index as usize)
            .and_then(|archetype| unsafe {
                archetype.get_component_unsafe_ticked(entity_location.row)
            })
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
        self.resource_tick_checks
            .insert(TypeId::of::<T>(), ResourceStorage::<T>::check_change_ticks);
        self.resources
            .insert(ResourceStorage::new(resource, *self.change_tick()));
    }

    // Inserts a resource into the world with its default value.
    pub fn init_resource<T: Resource + Default>(&mut self) {
        self.insert_resource(T::default());
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

    /// Returns an exclusive reference to the resource of type `T` and marks it changed, or `None` if not present.
    pub fn get_resource_mut<T: Resource + 'static>(&mut self) -> Option<&mut T> {
        let change_tick = self.change_tick();
        self.resources
            .get_mut::<ResourceStorage<T>>()
            .map(|resource_storage| {
                resource_storage.changed_tick = change_tick;
                &mut resource_storage.data
            })
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

    /// Marks the frame boundary for change detection read outside any system, and clamps old ticks when due.
    pub fn tick(&mut self) {
        self.last_change_tick = self.increment_change_tick();
        self.check_change_ticks();
    }

    /// The counter's current value: the tick the next system run will take.
    pub fn change_tick(&self) -> Tick {
        Tick::new(self.change_tick.load(Ordering::Relaxed))
    }

    /// Advances the counter by one and returns its previous value.
    pub fn increment_change_tick(&self) -> Tick {
        Tick::new(self.change_tick.fetch_add(1, Ordering::Relaxed))
    }

    /// The tick of the last [`tick`](Self::tick), from which readers outside any system look.
    pub fn last_change_tick(&self) -> Tick {
        self.last_change_tick
    }

    /// The ticks a reader outside any system uses: since the last [`tick`](Self::tick), until now.
    pub fn out_of_system_ticks(&self) -> SystemTicks {
        SystemTicks {
            last_run: self.last_change_tick,
            this_run: self.change_tick(),
        }
    }

    #[cfg(test)]
    pub(crate) fn set_change_tick(&mut self, tick: Tick) {
        *self.change_tick.get_mut() = *tick;
    }

    fn check_change_ticks(&mut self) {
        let change_tick = self.change_tick();
        if *change_tick.relative_to(self.last_check_tick) < CHECK_TICK_THRESHOLD {
            return;
        }

        for archetype in &mut self.archetypes {
            archetype.check_change_ticks(change_tick);
        }
        for check in self.resource_tick_checks.values() {
            check(&mut self.resources, change_tick);
        }
        if let Some(schedules) = self.get_resource_storage_mut::<CompiledSchedules>() {
            schedules.data.check_change_ticks(change_tick);
        }
        for check in self.listeners.tick_checks() {
            check(self, change_tick);
        }
        self.last_change_tick.check_tick(change_tick);
        self.last_check_tick = change_tick;
    }

    pub fn was_component_added(&self, entity: Entity, component_id: ComponentId) -> bool {
        self.was_component_added_in(entity, component_id, self.out_of_system_ticks())
    }

    pub(crate) fn was_component_added_in(
        &self,
        entity: Entity,
        component_id: ComponentId,
        ticks: SystemTicks,
    ) -> bool {
        self.entity_store
            .find_location(entity)
            .is_some_and(|location| {
                self.archetypes[location.archetype_index as usize].was_entity_added(
                    component_id,
                    location.row,
                    ticks,
                )
            })
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
                    self.change_tick(),
                )
            })
    }

    pub fn was_component_changed(&self, entity: Entity, component_id: ComponentId) -> bool {
        self.was_component_changed_in(entity, component_id, self.out_of_system_ticks())
    }

    pub(crate) fn was_component_changed_in(
        &self,
        entity: Entity,
        component_id: ComponentId,
        ticks: SystemTicks,
    ) -> bool {
        self.entity_store
            .find_location(entity)
            .is_some_and(|location| {
                self.archetypes[location.archetype_index as usize].was_entity_changed(
                    component_id,
                    location.row,
                    ticks,
                )
            })
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
        self.register_component_lifetimes::<T>();
    }

    /// Registers `T` in this world's [`AppTypeRegistry`] with the type data its derive declared.
    pub fn register_type<T: GetTypeRegistration>(&mut self) {
        self.type_registry_mut().register::<T>();
    }

    fn type_registry_mut(&mut self) -> &mut AppTypeRegistry {
        if self.get_resource::<AppTypeRegistry>().is_none() {
            let mut registry = AppTypeRegistry::default();
            registry.register::<SceneEntityRef>();
            self.insert_resource(registry);
        }
        self.get_resource_mut::<AppTypeRegistry>().unwrap()
    }

    /// Every component `entity` carries, including ones that are not scene
    /// components. Empty for a stale entity.
    pub fn component_ids(&self, entity: Entity) -> &[ComponentId] {
        self.entity_store
            .find_location(entity)
            .map(|location| self.archetypes[location.archetype_index as usize].component_ids())
            .unwrap_or(&[])
    }

    /// The registration of the type `id`, or `None` if it was never registered.
    pub fn registration(&self, id: ComponentId) -> Option<&TypeRegistration> {
        self.get_resource::<AppTypeRegistry>()?.get(id)
    }

    /// The registered scene components `entity` currently carries.
    ///
    /// Empty for a stale entity handle. Components registered only through
    /// [`register_component`](Self::register_component) — engine plumbing such
    /// as render-world mirrors — are deliberately not listed: this reports what
    /// a scene can describe, which is what a tool wants to show.
    pub fn component_types(&self, entity: Entity) -> impl Iterator<Item = SceneComponentType<'_>> {
        let registry = self.get_resource::<AppTypeRegistry>();
        self.component_ids(entity)
            .iter()
            .filter_map(move |component_id| {
                let registration = registry?.get(*component_id)?;
                let scene = *registration.data::<ReflectSceneComponent>()?;
                Some(SceneComponentType::new(registration, scene))
            })
    }

    /// Spawns the component registered under `type_name` from `value` onto `entity`,
    /// resolving node references through `node_entities`. Returns `false`, having
    /// logged why, when no scene component is registered under that name; parts of
    /// `value` that do not fit are skipped with a warning.
    pub fn apply_scene_component(
        &mut self,
        type_name: &str,
        value: &ReflectedValue,
        entity: Entity,
        node_entities: &[Entity],
    ) -> bool {
        let spawn = self
            .get_resource::<AppTypeRegistry>()
            .and_then(|registry| registry.get_by_name(type_name))
            .and_then(|registration| registration.data::<ReflectSceneComponent>())
            .map(|scene| scene.spawn);
        let Some(spawn) = spawn else {
            warn!(
                "Skipping component '{type_name}': no scene component registered under that name \
                 (derive Reflect with #[reflect(SceneComponent)] and register it with App::register_type)"
            );
            return false;
        };
        for warning in spawn(self, entity, value, node_entities) {
            warn!("Component '{type_name}': {warning}");
        }
        self.flush_commands();
        true
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
        let mut listener = system.into_listener();
        T::listener_added(self, &mut listener);
        self.register_component_lifetimes::<Listener<T>>();
        self.spawn((listener, GlobalListener));
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

    /// Runs `target`'s own listener for this signal, if it has one.
    pub fn trigger_on<T: EntitySignal>(&mut self, target: Entity, mut signal: T) {
        Listener::<T>::run(target, &mut signal, self);
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

    unsafe fn get_data<'world, 'state>(
        _state: &'state mut Self::State,
        world: UnsafeWorldCell<'world>,
        _ticks: SystemTicks,
    ) -> Result<Self::Data<'world, 'state>, ValidationError> {
        // SAFETY: this input declares read access to the whole world.
        Ok(unsafe { world.world() })
    }

    fn fill_access(_meta: &mut SystemMetadata, access: &mut SystemAccess) {
        access.read_world();
    }
}

impl SystemInput for &mut World {
    type State = ();
    type Data<'world, 'state> = &'world mut World;

    fn init_state(_world: &mut World) -> Self::State {}

    unsafe fn get_data<'world, 'state>(
        _state: &'state mut Self::State,
        world: UnsafeWorldCell<'world>,
        _ticks: SystemTicks,
    ) -> Result<Self::Data<'world, 'state>, ValidationError> {
        // SAFETY: this input declares write access to the whole world.
        Ok(unsafe { world.world_mut() })
    }

    fn fill_access(_meta: &mut SystemMetadata, access: &mut SystemAccess) {
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

    /// Borrows the whole world for reading.
    ///
    /// # Safety
    /// The caller may read only what its declared access covers, and nothing may write
    /// that data while the borrow is alive.
    pub unsafe fn world(&self) -> &'w World {
        // SAFETY: the pointer came from a live world borrowed for `'w`; the caller upholds
        // the access rules.
        unsafe { &*self.ptr }
    }

    /// Borrows the whole world for writing.
    ///
    /// # Safety
    /// The caller must have exclusive access to everything it touches, and no other
    /// reference into the world may be used while the borrow is alive.
    pub unsafe fn world_mut(&self) -> &'w mut World {
        self.assert_mutable();
        // SAFETY: the pointer came from a live, mutably borrowed world; the caller upholds
        // exclusivity.
        unsafe { &mut *self.ptr }
    }

    /// Advances the world's counter by one and returns its previous value.
    pub fn increment_change_tick(&self) -> Tick {
        // SAFETY: only the atomic counter is borrowed, which any holder of the cell may advance.
        let counter = unsafe { &(*self.ptr).change_tick };
        Tick::new(counter.fetch_add(1, Ordering::Relaxed))
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
        // SAFETY: lifecycle hooks run inside an exclusive world operation.
        let world = unsafe { self.world() };
        let Some(lifetimes) = world.component_lifetimes.get(id) else {
            return;
        };
        let (hook, signal) = (lifetimes.on_add, lifetimes.signal_add);
        if let Some(hook) = hook {
            hook(self.into_restricted(), ComponentLifecycleContext { entity });
        }
        // SAFETY: the operation that runs this hook holds the world exclusively.
        unsafe { signal(*self, entity) };
    }

    pub(crate) fn trigger_on_replace(&self, entity: Entity, ids: &[ComponentId]) {
        for id in ids {
            self.trigger_on_replace_component(entity, id);
        }
    }

    pub(crate) fn trigger_on_replace_component(&self, entity: Entity, id: &ComponentId) {
        // SAFETY: lifecycle hooks run inside an exclusive world operation.
        let world = unsafe { self.world() };
        let Some(lifetimes) = world.component_lifetimes.get(id) else {
            return;
        };
        let (hook, signal) = (lifetimes.on_replace, lifetimes.signal_replace);
        if let Some(hook) = hook {
            hook(self.into_restricted(), ComponentLifecycleContext { entity });
        }
        // SAFETY: the operation that runs this hook holds the world exclusively.
        unsafe { signal(*self, entity) };
    }

    pub(crate) fn trigger_on_remove_component(&self, entity: Entity, id: &ComponentId) {
        // SAFETY: lifecycle hooks run inside an exclusive world operation.
        let world = unsafe { self.world() };
        let Some(lifetimes) = world.component_lifetimes.get(id) else {
            return;
        };
        let (hook, signal) = (lifetimes.on_remove, lifetimes.signal_remove);
        if let Some(hook) = hook {
            hook(self.into_restricted(), ComponentLifecycleContext { entity });
        }
        // SAFETY: the operation that runs this hook holds the world exclusively.
        unsafe { signal(*self, entity) };
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
        // SAFETY: lifecycle hooks run inside an exclusive world operation.
        let world = unsafe { self.world() };
        let Some(lifetimes) = world.component_lifetimes.get(id) else {
            return;
        };
        let (hook, signal) = (lifetimes.on_despawn, lifetimes.signal_despawn);
        if let Some(hook) = hook {
            hook(self.into_restricted(), ComponentLifecycleContext { entity });
        }
        // SAFETY: the operation that runs this hook holds the world exclusively.
        unsafe { signal(*self, entity) };
    }

    pub(crate) fn archetypes(&self) -> &'w [Archetype] {
        // SAFETY: archetypes change only under exclusive access, so no running system can
        // alter them while this is read.
        unsafe { self.world() }.archetypes()
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
        // SAFETY: a `RestrictedWorld` lives only inside an exclusive world operation.
        unsafe { self.world_cell.world_mut() }.make_command_queue()
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
        // SAFETY: a `RestrictedWorld` lives only inside an exclusive world operation.
        unsafe { self.world_cell.world() }.get_component_for_entity(entity)
    }

    pub fn get_component_for_entity_mut<T: Component>(&mut self, entity: Entity) -> Option<&mut T> {
        // SAFETY: a `RestrictedWorld` lives only inside an exclusive world operation.
        unsafe { self.world_cell.world_mut() }.get_component_for_entity_mut(entity)
    }

    pub fn get_resource<T: Resource>(&self) -> Option<&T> {
        // SAFETY: a `RestrictedWorld` lives only inside an exclusive world operation.
        unsafe { self.world_cell.world() }.get_resource()
    }

    pub fn get_resource_mut<T: Resource>(&mut self) -> Option<&mut T> {
        // SAFETY: a `RestrictedWorld` lives only inside an exclusive world operation.
        unsafe { self.world_cell.world_mut() }.get_resource_mut::<T>()
    }
}

pub trait FromWorld {
    fn from_world(world: &World) -> Self;
}

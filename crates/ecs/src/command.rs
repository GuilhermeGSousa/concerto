use std::{marker::PhantomData, mem::MaybeUninit, ptr::NonNull};

use crate::{
    component::{Component, bundle::ComponentBundle},
    entity::{
        Entity,
        entity_store::EntityStore,
        hierarchy::{ChildOf, DespawnChildren},
    },
    resource::Resource,
    system::{input::SystemInput, meta::SystemMetadata},
    world::World,
};

pub struct EntityCommandQueue<'a> {
    entity: Entity,
    command_queue: CommandQueue<'a, 'a>,
}

impl<'a> EntityCommandQueue<'a> {
    pub fn entity(&self) -> Entity {
        self.entity
    }

    pub fn add_child<T: ComponentBundle + 'static>(self, components: T) -> Self {
        self.add_child_with(components, |_| {})
    }

    pub fn add_child_with<T: ComponentBundle + 'static>(
        mut self,
        components: T,
        f: impl Fn(EntityCommandQueue),
    ) -> Self {
        let child_ctx = self.command_queue.spawn(components);
        let child_entity = child_ctx.entity();
        f(child_ctx);
        self.command_queue.add_child(self.entity, child_entity);
        self
    }

    /// Spawns a child of this entity and returns the child's command queue.
    pub fn spawn_child_queue<T: ComponentBundle + 'static>(
        &mut self,
        components: T,
    ) -> EntityCommandQueue<'_> {
        let child = self.command_queue.spawn(components).entity();
        self.command_queue.add_child(self.entity, child);
        self.command_queue.entity(child)
    }

    pub fn despawn_children(mut self) -> Self {
        self.command_queue.push(DespawnChildren {
            parent: self.entity,
        });
        self
    }

    pub fn insert<T: ComponentBundle + 'static>(&mut self, component: T) {
        self.command_queue.insert(component, self.entity);
    }

    pub fn despawn(mut self) {
        self.command_queue.despawn(self.entity());
    }
}

pub struct CommandQueue<'world, 'state> {
    queue_state: &'state mut CommandQueueState,
    entities: &'world mut EntityStore,
}

impl<'w, 's> CommandQueue<'w, 's> {
    pub(crate) fn new(state: &'s mut CommandQueueState, entities: &'w mut EntityStore) -> Self {
        Self {
            queue_state: state,
            entities,
        }
    }

    pub(crate) fn for_callbacks(
        state: &'s mut CommandQueueState,
        entities: &'w mut EntityStore,
    ) -> Self {
        Self {
            queue_state: state,
            entities,
        }
    }

    pub fn spawn<T: ComponentBundle + 'static>(&mut self, components: T) -> EntityCommandQueue<'_> {
        let spawned_entity = self.entities.alloc();
        self.queue_state
            .push(SpawnCommand::new(components, spawned_entity));

        EntityCommandQueue {
            entity: spawned_entity,
            command_queue: CommandQueue {
                queue_state: &mut *self.queue_state,
                entities: &mut *self.entities,
            },
        }
    }

    /// Scopes further commands to an existing entity.
    pub fn entity(&mut self, entity: Entity) -> EntityCommandQueue<'_> {
        EntityCommandQueue {
            entity,
            command_queue: CommandQueue {
                queue_state: &mut *self.queue_state,
                entities: &mut *self.entities,
            },
        }
    }

    pub fn despawn(&mut self, entity: Entity) {
        self.queue_state.push(DespawnCommand::new(entity));
    }

    pub fn insert<T: ComponentBundle + 'static>(&mut self, component: T, entity: Entity) {
        self.queue_state.push(InsertCommand { component, entity });
    }

    pub fn remove<T: Component>(&mut self, entity: Entity) {
        self.queue_state.push(RemoveCommand::<T> {
            entity,
            marker: PhantomData,
        });
    }

    pub fn add_child(&mut self, parent: Entity, child: Entity) {
        self.insert(ChildOf::new(parent), child);
    }

    /// Queues a command that is not part of this queue's typed API.
    pub(crate) fn push<C: Command + 'static>(&mut self, command: C) {
        self.queue_state.push(command);
    }

    pub fn insert_resource<T: Resource>(&mut self, resource: T) {
        self.queue_state.push(InsertResource::<T>::new(resource));
    }

    pub fn insert_from_json(
        &mut self,
        component_name: String,
        component_data: String,
        entity: Entity,
    ) {
        self.queue_state.push(InsertErasedCommand::new(
            component_name,
            component_data,
            entity,
        ));
    }

    /// Queues a scene component for deserialization and application.
    /// `node_entities` lets the component resolve `SceneEntityRef`s to the
    /// other entities spawned for the same scene.
    pub fn apply_scene_component(
        &mut self,
        type_name: String,
        data: String,
        entity: Entity,
        node_entities: std::sync::Arc<[Entity]>,
    ) {
        self.queue_state.push(ApplySceneComponentCommand {
            type_name,
            data,
            entity,
            node_entities,
        });
    }
}

/// Runs a command previously written into the queue's buffer, or drops it in
/// place when `world` is `None`.
pub(crate) type ConsumeCommand = unsafe fn(NonNull<MaybeUninit<u8>>, Option<&mut World>);

/// # Safety
/// `ptr` must address a `C` written by [`CommandQueueState::add_command`] that
/// has not been consumed yet.
unsafe fn consume_command<C: Command>(ptr: NonNull<MaybeUninit<u8>>, world: Option<&mut World>) {
    // The command is moved out before `world` is touched: executing it can push
    // onto the same buffer and reallocate it, leaving `ptr` dangling.
    let command = unsafe { ptr.as_ptr().cast::<C>().read_unaligned() };

    if let Some(world) = world {
        command.execute(world);
    }
}

struct CommandEntry {
    offset: usize,
    consume: Option<ConsumeCommand>,
}

/// Pending commands, packed into one buffer rather than boxed individually.
///
/// An entry's index stays valid as the buffer grows, so a command can queue
/// more commands while it runs.
pub struct CommandQueueState {
    bytes: Vec<MaybeUninit<u8>>,
    entries: Vec<CommandEntry>,
}

impl CommandQueueState {
    pub fn new() -> Self {
        CommandQueueState {
            bytes: Vec::new(),
            entries: Vec::new(),
        }
    }

    pub fn push<C: Command + 'static>(&mut self, command: C) {
        let offset = self.bytes.len();
        self.bytes.reserve(size_of::<C>());

        // SAFETY: `reserve` guarantees room for a `C` at `offset`, and
        // `MaybeUninit<u8>` needs no initialization to be valid.
        unsafe {
            self.bytes
                .as_mut_ptr()
                .add(offset)
                .cast::<C>()
                .write_unaligned(command);
            self.bytes.set_len(offset + size_of::<C>());
        }

        self.entries.push(CommandEntry {
            offset,
            consume: Some(consume_command::<C>),
        });
    }

    /// Runs the queued commands, and everything their callbacks queue, on `world`.
    pub fn execute_commands(&mut self, world: &mut World) {
        world.apply_commands(self);
    }

    /// Hands out the command at `index`, or `None` if it was already taken.
    ///
    /// The caller must pass the pointer to the returned function exactly once.
    pub(crate) fn take_command(
        &mut self,
        index: usize,
    ) -> Option<(NonNull<MaybeUninit<u8>>, ConsumeCommand)> {
        let entry = self.entries.get_mut(index)?;
        let offset = entry.offset;
        let consume = entry.consume.take()?;

        // SAFETY: `offset` is in bounds of a buffer that has been allocated.
        let ptr = unsafe { NonNull::new_unchecked(self.bytes.as_mut_ptr().add(offset)) };

        Some((ptr, consume))
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub(crate) fn append(&mut self, other: &mut Self) {
        let base = self.bytes.len();
        self.bytes.append(&mut other.bytes);
        self.entries
            .extend(other.entries.drain(..).map(|entry| CommandEntry {
                offset: base + entry.offset,
                consume: entry.consume,
            }));
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Drops the commands from `new_len` on that have not run yet.
    pub fn truncate(&mut self, new_len: usize) {
        if new_len >= self.entries.len() {
            return;
        }

        let byte_len = self.entries[new_len].offset;

        for entry in self.entries.drain(new_len..) {
            let Some(consume) = entry.consume else {
                continue;
            };

            // SAFETY: the entry is live, so its command is still in the buffer,
            // and taking it here means it cannot be consumed again.
            unsafe {
                let ptr = NonNull::new_unchecked(self.bytes.as_mut_ptr().add(entry.offset));
                consume(ptr, None);
            }
        }

        // SAFETY: shrinking a buffer of `MaybeUninit<u8>` to a length it held.
        unsafe { self.bytes.set_len(byte_len) };
    }
}

impl Drop for CommandQueueState {
    fn drop(&mut self) {
        self.truncate(0);
    }
}

impl Default for CommandQueueState {
    fn default() -> Self {
        Self::new()
    }
}

impl SystemInput for CommandQueue<'_, '_> {
    type State = CommandQueueState;
    type Data<'world, 'state> = CommandQueue<'world, 'state>;

    fn init_state(_world: &mut World) -> Self::State {
        CommandQueueState::new()
    }

    fn get_data<'world, 'state>(
        state: &'state mut Self::State,
        world: crate::world::UnsafeWorldCell<'world>,
    ) -> Self::Data<'world, 'state> {
        CommandQueue::new(state, world.world_mut().entity_store_mut())
    }

    fn apply(state: &mut Self::State, world: &mut World) {
        if !state.is_empty() {
            state.execute_commands(world);
        }
    }

    fn fill_access(_meta: &mut SystemMetadata, access: &mut crate::system::access::SystemAccess) {
        access.set_needs_apply();
    }
}

pub trait Command: Send + Sync {
    fn execute(self, world: &mut World);
}

pub(crate) struct SpawnCommand<T: ComponentBundle> {
    components: T,
    entity: Entity,
}

impl<T: ComponentBundle> SpawnCommand<T> {
    pub fn new(components: T, entity: Entity) -> Self {
        SpawnCommand { components, entity }
    }
}

impl<T: ComponentBundle> Command for SpawnCommand<T> {
    fn execute(self, world: &mut World) {
        world.spawn_allocated(self.entity, self.components);
    }
}

pub(crate) struct DespawnCommand {
    entity: Entity,
}

impl DespawnCommand {
    pub fn new(entity: Entity) -> Self {
        DespawnCommand { entity }
    }
}

impl Command for DespawnCommand {
    fn execute(self, world: &mut World) {
        world.despawn(self.entity);
    }
}

pub(crate) struct InsertCommand<T: ComponentBundle> {
    component: T,
    entity: Entity,
}

impl<T: ComponentBundle> Command for InsertCommand<T> {
    fn execute(self, world: &mut World) {
        if !world.entity_is_valid(self.entity) {
            return;
        }
        world.insert(self.component, self.entity);
    }
}

pub(crate) struct InsertErasedCommand {
    entity: Entity,
    component_name: String,
    component_data: String,
}

impl InsertErasedCommand {
    pub fn new(component_name: String, component_data: String, entity: Entity) -> Self {
        Self {
            entity,
            component_name,
            component_data,
        }
    }
}

impl Command for InsertErasedCommand {
    fn execute(self, world: &mut World) {
        world.apply_scene_component(&self.component_name, &self.component_data, self.entity, &[]);
    }
}

pub(crate) struct ApplySceneComponentCommand {
    type_name: String,
    data: String,
    entity: Entity,
    node_entities: std::sync::Arc<[Entity]>,
}

impl Command for ApplySceneComponentCommand {
    fn execute(self, world: &mut World) {
        world.apply_scene_component(
            &self.type_name,
            &self.data,
            self.entity,
            &self.node_entities,
        );
    }
}

pub(crate) struct RemoveCommand<T: Component> {
    entity: Entity,
    marker: PhantomData<fn(T)>,
}

impl<T: Component> Command for RemoveCommand<T> {
    fn execute(self, world: &mut World) {
        if !world.entity_is_valid(self.entity) {
            return;
        }

        world.remove_component::<T>(self.entity);
    }
}

pub(crate) struct InsertResource<T: Resource> {
    resource: T,
}

impl<T: Resource> InsertResource<T> {
    fn new(resource: T) -> Self {
        Self { resource }
    }
}

impl<T: Resource> Command for InsertResource<T> {
    fn execute(self, world: &mut World) {
        world.insert_resource(self.resource);
    }
}

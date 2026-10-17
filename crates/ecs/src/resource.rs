use std::{
    any::TypeId,
    ops::{Deref, DerefMut},
};

pub use concerto_ecs_macros::Resource;

use crate::{
    World,
    component::Tick,
    query::change_detection::DetectChanges,
    system::{input::SystemInput, meta::SystemMetadata},
    world::UnsafeWorldCell,
};

/// A unique identifier for a resource type, based on its Rust [`TypeId`].
pub type ResourceId = TypeId;

/// Marker trait for globally-shared data stored in a [`World`](crate::world::World).
///
/// Derive with `#[derive(Resource)]`.  Each type may have at most one instance per world.
///
/// Access resources in systems using [`Res<T>`] (read-only) or [`ResMut<T>`] (mutable).
///
/// # Example
/// ```
/// use concerto_ecs::resource::Resource;
///
/// #[derive(Resource)]
/// struct Score(u32);
/// ```
pub trait Resource: Send + Sync + 'static {
    /// Returns the human-readable name of this resource (usually the type name).
    fn name() -> &'static str;
}

#[allow(dead_code)]
pub(crate) struct ResourceStorage<T: Resource> {
    pub(crate) data: T,
    pub(crate) added_tick: Tick,
    pub(crate) changed_tick: Tick,
}

impl<T: Resource> ResourceStorage<T> {
    pub(crate) fn new(resource: T, current_tick: u32) -> Self {
        Self {
            data: resource,
            added_tick: Tick::new(current_tick),
            changed_tick: Tick::new(current_tick),
        }
    }
}

/// Read-only system parameter that borrows a [`Resource`] from the world.
///
/// Derefs to `&T`.  Implements [`DetectChanges`](crate::query::change_detection::DetectChanges)
/// so systems can react when the resource value changes.
///
/// # Panics
/// Panics at system execution time if the resource has not been inserted into the world.
pub struct Res<'world, T: Resource> {
    pub value: &'world T,
    changed_tick: &'world Tick,
    current_tick: Tick,
}

impl<'world, T: Resource> Res<'world, T> {
    pub fn new(world: UnsafeWorldCell<'world>) -> Self {
        let world = world.world();
        let res_storage = world
            .get_resource_storage::<T>()
            .unwrap_or_else(|| panic!("Could not find resource {}", T::name()));
        Self {
            value: &res_storage.data,
            changed_tick: &res_storage.changed_tick,
            current_tick: world.current_tick(),
        }
    }
}

impl<T: Resource> Res<'_, T> {
    pub fn changed_tick(&self) -> Tick {
        *self.changed_tick
    }
}

impl<'a, T> SystemInput for Res<'a, T>
where
    T: Resource,
{
    type State = ();
    type Data<'world, 'state> = Res<'world, T>;

    fn init_state(_world: &mut World) -> Self::State {}

    fn get_data<'world, 'state>(
        _state: &'state mut Self::State,
        world: crate::world::UnsafeWorldCell<'world>,
    ) -> Self::Data<'world, 'state> {
        Res::new(world)
    }

    fn fill_access(_meta: &mut SystemMetadata, access: &mut crate::system::access::SystemAccess) {
        access.read_resource::<T>();
    }
}

impl<T> crate::system::input::ReadOnlySystemInput for Res<'_, T> where T: Resource {}

impl<'a, T> SystemInput for Option<Res<'a, T>>
where
    T: Resource,
{
    type State = ();
    type Data<'world, 'state> = Option<Res<'world, T>>;

    fn init_state(_world: &mut World) -> Self::State {}

    fn get_data<'world, 'state>(
        _state: &'state mut Self::State,
        world: crate::world::UnsafeWorldCell<'world>,
    ) -> Self::Data<'world, 'state> {
        world
            .world()
            .get_resource_storage::<T>()
            .is_some()
            .then(|| Res::new(world))
    }

    fn fill_access(_meta: &mut SystemMetadata, access: &mut crate::system::access::SystemAccess) {
        access.read_resource::<T>();
    }
}

impl<T> crate::system::input::ReadOnlySystemInput for Option<Res<'_, T>> where T: Resource {}

impl<T> Deref for Res<'_, T>
where
    T: Resource,
{
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.value
    }
}

impl<T> DetectChanges for Res<'_, T>
where
    T: Resource,
{
    fn has_changed(&self) -> bool {
        *self.changed_tick == self.current_tick
    }
}

/// Mutable system parameter that borrows a [`Resource`] from the world.
///
/// Derefs to `&mut T` and automatically marks the resource as changed when
/// [`DerefMut`] is called, so change-detection works correctly.
///
/// # Panics
/// Panics at system execution time if the resource has not been inserted into the world.
pub struct ResMut<'world, T: Resource> {
    pub value: &'world mut T,
    changed_tick: &'world mut Tick,
    current_tick: Tick,
    has_changed: bool,
}

impl<'world, T: Resource> ResMut<'world, T> {
    pub fn new(world: UnsafeWorldCell<'world>) -> Self {
        let world = world.world_mut();
        let current_tick = world.current_tick();
        let res_storage = world.get_resource_storage_mut::<T>().unwrap();

        Self {
            value: &mut res_storage.data,
            changed_tick: &mut res_storage.changed_tick,
            current_tick,
            has_changed: false,
        }
    }

    pub fn mark_changed(&mut self) {
        self.deref_mut();
    }

    pub fn changed_tick(&self) -> Tick {
        *self.changed_tick
    }
}

impl<T> SystemInput for ResMut<'_, T>
where
    T: Resource,
{
    type State = ();
    type Data<'world, 'state> = ResMut<'world, T>;

    fn init_state(_world: &mut World) -> Self::State {}

    fn get_data<'world, 'state>(
        _state: &'state mut Self::State,
        world: crate::world::UnsafeWorldCell<'world>,
    ) -> Self::Data<'world, 'state> {
        ResMut::new(world)
    }

    fn fill_access(_meta: &mut SystemMetadata, access: &mut crate::system::access::SystemAccess) {
        access.write_resource::<T>();
    }
}

impl<T> Deref for ResMut<'_, T>
where
    T: Resource,
{
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.value
    }
}

impl<T> DerefMut for ResMut<'_, T>
where
    T: Resource,
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        if !self.has_changed {
            self.has_changed = true;
            *self.changed_tick = self.current_tick;
        }
        self.value
    }
}

impl<T> DetectChanges for ResMut<'_, T>
where
    T: Resource,
{
    fn has_changed(&self) -> bool {
        *self.changed_tick == self.current_tick
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        IntoSystem,
        system::{System, access::SystemAccess, meta::SystemMetadata},
    };

    #[derive(Resource, Default)]
    struct Counter(usize);

    #[derive(Resource, Default)]
    struct Seen(Option<usize>);

    fn observe(counter: Option<Res<Counter>>, mut seen: ResMut<Seen>) {
        seen.0 = counter.map(|counter| counter.0);
    }

    fn access_of<M>(system: impl IntoSystem<(), M>) -> SystemAccess {
        let system = system.into_system();
        let mut access = SystemAccess::default();
        system.fill_access(&mut SystemMetadata::default(), &mut access);
        access
    }

    fn optional_reader(_: Option<Res<Counter>>) {}

    fn plain_reader(_: Res<Counter>) {}

    fn writer(_: ResMut<Counter>) {}

    #[test]
    fn an_optional_resource_is_none_while_absent_and_some_once_inserted() {
        let mut world = World::new();
        world.insert_resource(Seen::default());
        let mut system = observe.into_system();
        system.initialize(&mut world);

        system.run_and_apply((), &mut world);
        assert_eq!(world.get_resource::<Seen>().unwrap().0, None);

        world.insert_resource(Counter(7));
        system.run_and_apply((), &mut world);
        assert_eq!(world.get_resource::<Seen>().unwrap().0, Some(7));

        world.remove_resource::<Counter>();
        system.run_and_apply((), &mut world);
        assert_eq!(world.get_resource::<Seen>().unwrap().0, None);
    }

    #[test]
    fn an_optional_resource_declares_the_access_of_a_plain_one() {
        let optional = access_of(optional_reader);
        let plain = access_of(plain_reader);
        let write = access_of(writer);
        let reader_pair = SystemAccess::are_disjoint(&optional, &plain);
        assert_eq!(reader_pair, SystemAccess::are_disjoint(&plain, &plain));
        assert!(!SystemAccess::are_disjoint(&optional, &write));
        assert!(!SystemAccess::are_disjoint(&write, &optional));
    }
}

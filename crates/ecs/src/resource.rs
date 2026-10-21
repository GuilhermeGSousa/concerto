use std::{
    any::TypeId,
    ops::{Deref, DerefMut},
};

use anymap3::AnyMap;

pub use concerto_ecs_macros::Resource;

use crate::{
    World,
    component::{SystemTicks, Tick},
    query::change_detection::DetectChanges,
    system::{
        input::{SystemInput, ValidationError},
        meta::SystemMetadata,
    },
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

    pub(crate) fn check_change_ticks(resources: &mut AnyMap, change_tick: Tick) {
        if let Some(storage) = resources.get_mut::<Self>() {
            storage.added_tick.check_tick(change_tick);
            storage.changed_tick.check_tick(change_tick);
        }
    }
}

/// Read-only system parameter that borrows a [`Resource`] from the world.
///
/// Derefs to `&T`.  Implements [`DetectChanges`](crate::query::change_detection::DetectChanges)
/// so systems can react when the resource value changes.
///
/// A system taking it is skipped with a warning while the resource is missing; take
/// `Option<Res<T>>` for a resource that may be absent.
pub struct Res<'world, T: Resource> {
    pub value: &'world T,
    added_tick: &'world Tick,
    changed_tick: &'world Tick,
    ticks: SystemTicks,
}

impl<'world, T: Resource> Res<'world, T> {
    /// Borrows `T` from `world` for a run with `ticks`, or reports it missing.
    ///
    /// # Safety
    /// The caller must hold read access to `T`.
    pub unsafe fn new(
        world: UnsafeWorldCell<'world>,
        ticks: SystemTicks,
    ) -> Result<Self, ValidationError> {
        // SAFETY: only `T`'s storage is read, which the caller covers.
        let world = unsafe { world.world() };
        let res_storage = world
            .get_resource_storage::<T>()
            .ok_or_else(|| missing::<T>())?;
        Ok(Self {
            value: &res_storage.data,
            added_tick: &res_storage.added_tick,
            changed_tick: &res_storage.changed_tick,
            ticks,
        })
    }
}

fn missing<T: Resource>() -> ValidationError {
    ValidationError::missing::<T>("the resource does not exist")
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

    unsafe fn get_data<'world, 'state>(
        _state: &'state mut Self::State,
        world: crate::world::UnsafeWorldCell<'world>,
        ticks: SystemTicks,
    ) -> Result<Self::Data<'world, 'state>, ValidationError> {
        // SAFETY: `fill_access` declares read access to `T`.
        unsafe { Res::new(world, ticks) }
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

    unsafe fn get_data<'world, 'state>(
        _state: &'state mut Self::State,
        world: crate::world::UnsafeWorldCell<'world>,
        ticks: SystemTicks,
    ) -> Result<Self::Data<'world, 'state>, ValidationError> {
        // SAFETY: `fill_access` declares read access to `T`.
        Ok(unsafe { Res::new(world, ticks) }.ok())
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
    fn is_changed(&self) -> bool {
        self.ticks.is_changed(*self.changed_tick)
    }

    fn is_added(&self) -> bool {
        self.ticks.is_changed(*self.added_tick)
    }
}

/// Mutable system parameter that borrows a [`Resource`] from the world.
///
/// Derefs to `&mut T` and automatically marks the resource as changed when
/// [`DerefMut`] is called, so change-detection works correctly.
///
/// A system taking it is skipped with a warning while the resource is missing.
pub struct ResMut<'world, T: Resource> {
    pub value: &'world mut T,
    added_tick: &'world Tick,
    changed_tick: &'world mut Tick,
    ticks: SystemTicks,
    written: bool,
}

impl<'world, T: Resource> ResMut<'world, T> {
    /// Borrows `T` mutably from `world` for a run with `ticks`, or reports it missing.
    ///
    /// # Safety
    /// The caller must hold write access to `T`.
    pub unsafe fn new(
        world: UnsafeWorldCell<'world>,
        ticks: SystemTicks,
    ) -> Result<Self, ValidationError> {
        // SAFETY: only `T`'s storage is written, which the caller covers.
        let world = unsafe { world.world_mut() };
        let res_storage = world
            .get_resource_storage_mut::<T>()
            .ok_or_else(|| missing::<T>())?;

        Ok(Self {
            value: &mut res_storage.data,
            added_tick: &res_storage.added_tick,
            changed_tick: &mut res_storage.changed_tick,
            ticks,
            written: false,
        })
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

    unsafe fn get_data<'world, 'state>(
        _state: &'state mut Self::State,
        world: crate::world::UnsafeWorldCell<'world>,
        ticks: SystemTicks,
    ) -> Result<Self::Data<'world, 'state>, ValidationError> {
        // SAFETY: `fill_access` declares write access to `T`.
        unsafe { ResMut::new(world, ticks) }
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
        if !self.written {
            self.written = true;
            *self.changed_tick = self.ticks.this_run;
        }
        self.value
    }
}

impl<T> ResMut<'_, T>
where
    T: Resource + PartialEq,
{
    /// Writes `value` and stamps the change only if it differs; returns whether it wrote.
    pub fn set_if_neq(&mut self, value: T) -> bool {
        if *self.value == value {
            return false;
        }
        *self.deref_mut() = value;
        true
    }
}

impl<T> DetectChanges for ResMut<'_, T>
where
    T: Resource,
{
    fn is_changed(&self) -> bool {
        self.ticks.is_changed(*self.changed_tick)
    }

    fn is_added(&self) -> bool {
        self.ticks.is_changed(*self.added_tick)
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

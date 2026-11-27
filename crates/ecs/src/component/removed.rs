use std::marker::PhantomData;

use crate::{
    World,
    component::{Component, SystemTicks},
    entity::Entity,
    events::{
        Event, event_channel::EventChannel, event_cursor::EventCursor, event_writer::EventWriter,
    },
    resource::Res,
    signal::{On, lifecycle::Remove},
    system::{
        access::SystemAccess,
        input::{ReadOnlySystemInput, SystemInput, SystemLocal, ValidationError},
        meta::SystemMetadata,
    },
    world::UnsafeWorldCell,
};

/// One removal of `T` from `entity`, explicit or by despawn.
#[derive(Event)]
pub struct ComponentRemoved<T: Component> {
    /// The entity that lost the component.
    pub entity: Entity,
    marker: PhantomData<fn() -> T>,
}

type ChannelParam<T> = Res<'static, EventChannel<ComponentRemoved<T>>>;
type CursorParam<T> = SystemLocal<'static, EventCursor<ComponentRemoved<T>>>;

/// Reads the entities that lost `T`, each once per system.
///
/// Removals are recorded only after `App::track_removals::<T>()` or
/// [`World::track_removals::<T>()`](World::track_removals) was called.
pub struct RemovedComponents<'w, 's, T: Component> {
    channel: Res<'w, EventChannel<ComponentRemoved<T>>>,
    cursor: SystemLocal<'s, EventCursor<ComponentRemoved<T>>>,
}

impl<T: Component> RemovedComponents<'_, '_, T> {
    /// Every entity that lost `T` since this system last read, in removal order.
    pub fn read(&mut self) -> impl Iterator<Item = Entity> + '_ {
        self.channel
            .read_from(&mut self.cursor)
            .map(|removed| removed.entity)
    }
}

impl<T: Component> SystemInput for RemovedComponents<'_, '_, T> {
    type State = <CursorParam<T> as SystemInput>::State;
    type Data<'world, 'state> = RemovedComponents<'world, 'state, T>;

    fn init_state(world: &mut World) -> Self::State {
        <CursorParam<T>>::init_state(world)
    }

    unsafe fn get_data<'world, 'state>(
        state: &'state mut Self::State,
        world: UnsafeWorldCell<'world>,
        ticks: SystemTicks,
    ) -> Result<Self::Data<'world, 'state>, ValidationError> {
        Ok(RemovedComponents {
            // SAFETY: this input declares exactly the access of its two parts.
            channel: unsafe { <ChannelParam<T>>::get_data(&mut (), world, ticks) }?,
            // SAFETY: this input declares exactly the access of its two parts.
            cursor: unsafe { <CursorParam<T>>::get_data(state, world, ticks) }?,
        })
    }

    fn fill_access(meta: &mut SystemMetadata, access: &mut SystemAccess) {
        <ChannelParam<T>>::fill_access(meta, access);
        <CursorParam<T>>::fill_access(meta, access);
    }
}

impl<T: Component> ReadOnlySystemInput for RemovedComponents<'_, '_, T> {}

impl World {
    /// Records every removal of `T` in this world, for systems taking [`RemovedComponents<T>`].
    ///
    /// A second call does nothing.
    pub fn track_removals<T: Component>(&mut self) {
        if self
            .get_resource::<EventChannel<ComponentRemoved<T>>>()
            .is_some()
        {
            return;
        }
        self.insert_resource(EventChannel::<ComponentRemoved<T>>::new());
        self.add_listener(record_removal::<T>);
    }
}

/// Writes a [`ComponentRemoved<T>`] for the entity that lost `T`.
pub fn record_removal<T: Component>(
    on: On<Remove<T>>,
    mut removed: EventWriter<ComponentRemoved<T>>,
) {
    removed.write(ComponentRemoved {
        entity: on.signal().entity,
        marker: PhantomData,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        IntoSystem,
        resource::{ResMut, Resource},
        system::BoxedSystem,
    };

    #[derive(Component)]
    struct Health;

    #[derive(Resource, Default)]
    struct Seen(Vec<Entity>);

    fn collect(mut removed: RemovedComponents<Health>, mut seen: ResMut<Seen>) {
        seen.0.extend(removed.read());
    }

    fn read(world: &mut World, system: &mut BoxedSystem) -> Vec<Entity> {
        system.run_and_apply((), world);
        std::mem::take(&mut world.get_resource_mut::<Seen>().unwrap().0)
    }

    #[test]
    fn a_removal_or_a_despawn_is_read_once() {
        let mut world = World::new();
        world.track_removals::<Health>();
        world.insert_resource(Seen::default());
        let mut system = collect.into_boxed_system();
        system.initialize(&mut world);
        let removed = world.spawn(Health);
        let despawned = world.spawn(Health);

        world.remove_component::<Health>(removed);
        world.despawn(despawned);

        assert_eq!(read(&mut world, &mut system), vec![removed, despawned]);
        assert!(read(&mut world, &mut system).is_empty());
    }
}

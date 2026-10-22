use super::{ReadOnlySystemInput, SystemInput, ValidationError};
use crate::{
    component::{Component, ComponentId, SystemTicks},
    entity::{Entity, EntityStructuralVersion},
    system::{access::SystemAccess, meta::SystemMetadata},
    world::{UnsafeWorldCell, World},
};

/// Read-only, runtime-selected access to components and entity metadata.
pub struct ComponentMetadata<'w> {
    world: UnsafeWorldCell<'w>,
    ticks: SystemTicks,
}

impl<'w> ComponentMetadata<'w> {
    pub fn new(world: &'w World) -> Self {
        Self {
            world: world.as_unsafe_world_cell(),
            ticks: world.out_of_system_ticks(),
        }
    }

    fn read(&self) -> &'w World {
        // SAFETY: built from a `&World`, or by `get_data` for a system that declared read
        // access to every component; entity and type metadata change only under exclusive
        // access.
        unsafe { self.world.world() }
    }

    /// See [`World::structural_version`].
    pub fn structural_version(&self, entity: Entity) -> Option<EntityStructuralVersion> {
        self.read().structural_version(entity)
    }

    pub fn entity_is_valid(&self, entity: Entity) -> bool {
        self.read().entity_is_valid(entity)
    }

    pub fn component_ids(&self, entity: Entity) -> &'w [ComponentId] {
        self.read().component_ids(entity)
    }

    pub fn registration(&self, id: ComponentId) -> Option<&'w concerto_reflect::TypeRegistration> {
        self.read().registration(id)
    }

    pub fn get<T: Component>(&self, entity: Entity) -> Option<&'w T> {
        self.read().get_component_for_entity::<T>(entity)
    }

    /// Whether `entity`'s component `id` was added or written since the reading system last ran, or, from [`new`](Self::new), since the last [`World::tick`].
    pub fn is_component_changed(&self, entity: Entity, id: ComponentId) -> bool {
        self.read().was_component_changed_in(entity, id, self.ticks)
    }
}

impl SystemInput for ComponentMetadata<'_> {
    type State = ();
    type Data<'world, 'state> = ComponentMetadata<'world>;
    fn init_state(_: &mut World) {}
    unsafe fn get_data<'world, 'state>(
        _: &'state mut (),
        world: UnsafeWorldCell<'world>,
        ticks: SystemTicks,
    ) -> Result<Self::Data<'world, 'state>, ValidationError> {
        Ok(ComponentMetadata { world, ticks })
    }
    fn fill_access(_: &mut SystemMetadata, access: &mut SystemAccess) {
        access.read_all_components();
    }
}
impl ReadOnlySystemInput for ComponentMetadata<'_> {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Component, IntoSystem, ResMut, Resource, System};

    #[derive(Component)]
    struct Value(u32);
    #[derive(Resource)]
    struct Unrelated;

    #[test]
    fn component_reader_tracks_values_membership_and_stale_entities() {
        let mut world = World::default();
        let entity = world.spawn(Value(1));
        let mut read = (move |components: ComponentMetadata| {
            assert!(components.entity_is_valid(entity));
            assert!(
                components
                    .component_ids(entity)
                    .contains(&ComponentId::of::<Value>())
            );
            assert_eq!(components.get::<Value>(entity).unwrap().0, 2);
            assert!(components.is_component_changed(entity, ComponentId::of::<Value>()));
        })
        .into_system();
        read.initialize(&mut world);
        world.tick();
        world
            .get_component_for_entity_mut::<Value>(entity)
            .unwrap()
            .0 = 2;
        world.tick();
        read.run_and_apply((), &mut world);
        world.tick();
        let value = ComponentId::of::<Value>();
        assert!(!ComponentMetadata::new(&world).is_component_changed(entity, value));
        world
            .get_component_for_entity_mut::<Value>(entity)
            .unwrap()
            .0 = 3;
        assert!(ComponentMetadata::new(&world).is_component_changed(entity, value));
        world.despawn(entity);
        let components = ComponentMetadata::new(&world);
        assert!(!components.entity_is_valid(entity));
        assert!(components.component_ids(entity).is_empty());
        assert!(components.get::<Value>(entity).is_none());
    }

    #[derive(Resource, Default)]
    struct Seen(Vec<bool>);

    #[test]
    fn component_reader_sees_changes_since_its_system_last_ran() {
        let mut world = World::default();
        world.insert_resource(Seen::default());
        let entity = world.spawn(Value(1));
        let mut read = (move |components: ComponentMetadata, mut seen: ResMut<Seen>| {
            seen.0
                .push(components.is_component_changed(entity, ComponentId::of::<Value>()));
        })
        .into_system();
        read.initialize(&mut world);
        read.run_and_apply((), &mut world);
        read.run_and_apply((), &mut world);
        world
            .get_component_for_entity_mut::<Value>(entity)
            .unwrap()
            .0 = 2;
        read.run_and_apply((), &mut world);
        read.run_and_apply((), &mut world);
        assert_eq!(
            world.get_resource::<Seen>().unwrap().0,
            [true, false, true, false]
        );
    }

    #[test]
    fn component_reader_access_survives_combine_and_conflicts_symmetrically() {
        let mut reader = SystemAccess::default();
        ComponentMetadata::fill_access(&mut SystemMetadata::default(), &mut reader);
        let mut combined = SystemAccess::default();
        combined.combine(reader);
        let mut resource_write = SystemAccess::default();
        resource_write.write_resource::<Unrelated>();
        let mut component_write = SystemAccess::default();
        component_write.write_component::<Value>();
        let mut component_read = SystemAccess::default();
        component_read.read_component::<Value>();
        let mut exclusive = SystemAccess::default();
        exclusive.write_world();
        for (other, disjoint) in [
            (&resource_write, true),
            (&component_read, true),
            (&component_write, false),
            (&exclusive, false),
        ] {
            assert_eq!(SystemAccess::are_disjoint(&combined, other), disjoint);
            assert_eq!(SystemAccess::are_disjoint(other, &combined), disjoint);
        }
        assert!(!combined.is_exclusive());
    }
}

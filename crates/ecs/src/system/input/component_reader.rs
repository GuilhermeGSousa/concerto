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
    /// Creates a reader over `world` for use outside systems; change checks look back to the last
    /// [`World::tick`].
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

    /// Returns whether `entity` is alive.
    pub fn entity_is_valid(&self, entity: Entity) -> bool {
        self.read().entity_is_valid(entity)
    }

    /// Returns the ids of the components `entity` has, or an empty slice when it is not alive.
    pub fn component_ids(&self, entity: Entity) -> &'w [ComponentId] {
        self.read().component_ids(entity)
    }

    /// Returns the registration of the component type `id`, or `None` when it was never registered.
    pub fn registration(
        &self,
        id: ComponentId,
    ) -> Option<std::sync::Arc<concerto_reflect::TypeRegistration>> {
        self.read().registration(id)
    }

    /// Returns the component `T` on `entity`, or `None` when it has none.
    pub fn get<T: Component>(&self, entity: Entity) -> Option<&'w T> {
        self.read().get_component_for_entity::<T>(entity)
    }

    /// Whether `entity`'s component `id` was added or written since the reading system last ran.
    ///
    /// A reader built with [`new`](Self::new) looks back to the last [`World::tick`] instead.
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
        assert!(SystemAccess::are_disjoint(&combined, &resource_write));
        assert!(SystemAccess::are_disjoint(&resource_write, &combined));
        assert!(!SystemAccess::are_disjoint(&combined, &component_write));
        assert!(!SystemAccess::are_disjoint(&component_write, &combined));
        assert!(!combined.is_exclusive());
    }
}

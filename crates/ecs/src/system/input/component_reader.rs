use super::{ReadOnlySystemInput, SystemInput};
use crate::{
    component::{Component, ComponentId, Tick, registry::TypeInfo},
    entity::{Entity, EntityStructuralVersion},
    system::{access::SystemAccess, meta::SystemMetadata},
    world::{UnsafeWorldCell, World},
};

/// Read-only access to runtime-selected components and entity metadata.
/// Resources and mutation are deliberately unavailable. Prefer typed queries
/// when the component types are known statically.
pub struct ComponentMetadata<'w> {
    world: UnsafeWorldCell<'w>,
}

impl<'w> ComponentMetadata<'w> {
    pub fn new(world: &'w World) -> Self {
        Self {
            world: world.as_unsafe_world_cell(),
        }
    }

    /// See [`World::entity_structure_version`].
    pub fn entity_structure_version(&self, entity: Entity) -> Option<EntityStructuralVersion> {
        self.world.world().structural_version(entity)
    }

    pub fn entity_is_valid(&self, entity: Entity) -> bool {
        self.world.world().entity_is_valid(entity)
    }

    pub fn component_ids(&self, entity: Entity) -> &'w [ComponentId] {
        self.world.world().component_ids(entity)
    }

    pub fn type_info(&self, id: ComponentId) -> Option<&'w TypeInfo> {
        self.world.world().type_info(id)
    }

    pub fn get<T: Component>(&self, entity: Entity) -> Option<&'w T> {
        self.world.world().get_component_for_entity::<T>(entity)
    }

    pub fn current_tick(&self) -> Tick {
        self.world.world().current_tick()
    }

    pub fn has_component_changed_since(
        &self,
        entity: Entity,
        id: ComponentId,
        since: Tick,
    ) -> bool {
        self.world
            .world()
            .has_component_changed_since(entity, id, since)
    }
}

impl SystemInput for ComponentMetadata<'_> {
    type State = ();
    type Data<'world, 'state> = ComponentMetadata<'world>;
    fn init_state(_: &mut World) {}
    fn get_data<'world, 'state>(
        _: &'state mut (),
        world: UnsafeWorldCell<'world>,
    ) -> Self::Data<'world, 'state> {
        ComponentMetadata { world }
    }
    fn fill_access(_: &mut SystemMetadata, access: &mut SystemAccess) {
        access.read_all_components();
    }
}
impl ReadOnlySystemInput for ComponentMetadata<'_> {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Component, IntoSystem, Resource, System};

    #[derive(Component)]
    struct Value(u32);
    #[derive(Resource)]
    struct Unrelated;

    #[test]
    fn component_reader_tracks_values_membership_and_stale_entities() {
        let mut world = World::default();
        let entity = world.spawn(Value(1));
        let since = world.current_tick();
        let mut read = (move |components: ComponentMetadata| {
            assert!(components.entity_is_valid(entity));
            assert!(
                components
                    .component_ids(entity)
                    .contains(&ComponentId::of::<Value>())
            );
            assert_eq!(components.get::<Value>(entity).unwrap().0, 2);
            assert!(components.has_component_changed_since(
                entity,
                ComponentId::of::<Value>(),
                since
            ));
        })
        .into_system();
        read.initialize(&mut world);
        world.tick();
        world
            .get_component_for_entity_mut::<Value>(entity)
            .unwrap()
            .0 = 2;
        world.tick();
        read.run_and_apply(&mut world);
        world.despawn(entity);
        let components = ComponentMetadata::new(&world);
        assert!(!components.entity_is_valid(entity));
        assert!(components.component_ids(entity).is_empty());
        assert!(components.get::<Value>(entity).is_none());
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

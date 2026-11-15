use std::collections::{HashMap, HashSet};

use concerto_app::extractor::extract as extract_main_world;
use concerto_ecs::{
    component::Component, entity::EntityHash, reflect::ReflectSceneComponent, Entity, With,
    Without, World,
};
use concerto_reflect::Reflect;
use derive_more::Deref;

/// Marks a main-world entity as needing a mirror entity in the render world.
#[doc(hidden)]
#[derive(Component, Reflect, Default)]
#[reflect(SceneComponent)]
pub struct SyncWithRenderWorld;

/// On a main-world entity: the id of its mirror entity in the render world.
#[derive(Component, Deref)]
pub struct RenderEntity(Entity);

impl RenderEntity {
    pub fn new(entity: Entity) -> Self {
        Self(entity)
    }
}

/// On a render-world entity: the id of the main-world entity it mirrors.
#[derive(Component, Deref, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MainEntity(Entity);

impl MainEntity {
    pub fn new(entity: Entity) -> Self {
        Self(entity)
    }
}

/// A hash map keyed by [`MainEntity`], hashing each key as one `u64`.
pub type MainEntityHashMap<V> = HashMap<MainEntity, V, EntityHash>;

/// A hash set of [`MainEntity`], hashing each one as one `u64`.
pub type MainEntityHashSet = HashSet<MainEntity, EntityHash>;

pub(crate) fn extract(main: &mut World, render: &mut World) {
    sync_render_entities(main, render);

    extract_main_world(main, render);
}

fn sync_render_entities(main: &mut World, render: &mut World) {
    spawn_new_render_entities(main, render);
    despawn_stale_render_entities(main, render);
}

fn spawn_new_render_entities(main: &mut World, render: &mut World) {
    for main_entity in main
        .query::<Entity, (With<SyncWithRenderWorld>, Without<RenderEntity>)>()
        .iter(main)
        .collect::<Vec<_>>()
    {
        let render_entity = render.spawn(MainEntity::new(main_entity));
        main.insert(RenderEntity::new(render_entity), main_entity);
    }
}

fn despawn_stale_render_entities(main: &mut World, render: &mut World) {
    let stale: Vec<Entity> = render
        .query::<(Entity, &MainEntity), ()>()
        .iter(render)
        .filter_map(|(render_entity, main_entity)| {
            (!main.entity_is_valid(**main_entity)).then_some(render_entity)
        })
        .collect();

    for render_entity in stale {
        render.despawn(render_entity);
    }
}

#[cfg(test)]
mod tests {
    use concerto_ecs::World;

    use super::*;

    fn linked(main: &World, entity: Entity) -> Entity {
        **main
            .get_component_for_entity::<RenderEntity>(entity)
            .expect("the main entity is linked to a render entity")
    }

    #[test]
    fn a_synced_entity_keeps_one_mirror_for_as_long_as_it_lives() {
        let mut main = World::new();
        let mut render = World::new();
        let tagged = main.spawn(SyncWithRenderWorld);

        sync_render_entities(&mut main, &mut render);
        let mirror = linked(&main, tagged);
        assert_eq!(
            render
                .get_component_for_entity::<MainEntity>(mirror)
                .map(|back| **back),
            Some(tagged)
        );

        let gone = main.spawn(SyncWithRenderWorld);
        let stale = render.spawn(MainEntity::new(gone));
        main.despawn(gone);
        despawn_stale_render_entities(&mut main, &mut render);
        assert!(
            render
                .get_component_for_entity::<MainEntity>(stale)
                .is_none(),
            "a despawned entity's mirror goes with it"
        );
        assert!(render
            .get_component_for_entity::<MainEntity>(mirror)
            .is_some());
    }
}

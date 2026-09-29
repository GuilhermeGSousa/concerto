use concerto_app::extractor::extract as extract_main_world;
use concerto_ecs::{
    component::scene::{SceneComponent, SceneSpawnContext},
    component::Component,
    Entity, With, Without, World,
};
use concerto_mesh::mesh::MeshComponent;
use derive_more::Deref;
use serde::{Deserialize, Serialize};

use crate::components::{camera::Camera, light::Light};

/// Marks a main-world entity as needing a mirror entity in the render world.
#[derive(Component, Serialize, Deserialize)]
pub struct SyncWithRenderWorld;

impl SceneComponent for SyncWithRenderWorld {
    fn apply(self, entity: Entity, ctx: &mut SceneSpawnContext<'_>) {
        ctx.insert(self, entity);
    }
}

/// On a main-world entity: the id of its mirror entity in the render world.
#[derive(Component, Deref)]
pub struct RenderEntity(Entity);

impl RenderEntity {
    pub fn new(entity: Entity) -> Self {
        Self(entity)
    }
}

/// On a render-world entity: the id of the main-world entity it mirrors.
#[derive(Component, Deref)]
pub struct MainEntity(Entity);

impl MainEntity {
    pub fn new(entity: Entity) -> Self {
        Self(entity)
    }
}

pub(crate) fn extract(main: &mut World, render: &mut World) {
    sync_render_entities(main, render);

    extract_main_world(main, render);
}

fn sync_render_entities(main: &mut World, render: &mut World) {
    spawn_new_render_entities(main, render);
    despawn_stale_render_entities(main, render);
}

fn entities_needing_render_mirror(main: &mut World) -> Vec<Entity> {
    let mut entities = main
        .query::<Entity, (With<SyncWithRenderWorld>, Without<RenderEntity>)>()
        .iter(main)
        .collect::<Vec<_>>();
    entities.extend(
        main.query::<Entity, (With<MeshComponent>, Without<RenderEntity>)>()
            .iter(main),
    );
    entities.extend(
        main.query::<Entity, (With<Light>, Without<RenderEntity>)>()
            .iter(main),
    );
    entities.extend(
        main.query::<Entity, (With<Camera>, Without<RenderEntity>)>()
            .iter(main),
    );
    let mut seen = std::collections::HashSet::new();
    entities.retain(|entity| seen.insert(*entity));
    entities
}

fn spawn_new_render_entities(main: &mut World, render: &mut World) {
    for main_entity in entities_needing_render_mirror(main) {
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

    #[test]
    fn links_newly_tagged_main_entities_to_a_new_render_entity() {
        let mut main = World::new();
        let mut render = World::new();

        let main_entity = main.spawn(SyncWithRenderWorld);

        sync_render_entities(&mut main, &mut render);

        let render_entity = **main
            .get_component_for_entity::<RenderEntity>(main_entity)
            .expect("main entity should have been linked to a render entity");

        let linked_main_entity = **render
            .get_component_for_entity::<MainEntity>(render_entity)
            .expect("render entity should carry a back-reference to the main entity");

        assert_eq!(linked_main_entity, main_entity);
    }

    #[test]
    fn does_not_relink_an_already_linked_entity() {
        let mut main = World::new();
        let mut render = World::new();

        let main_entity = main.spawn(SyncWithRenderWorld);
        sync_render_entities(&mut main, &mut render);
        let first_render_entity = **main
            .get_component_for_entity::<RenderEntity>(main_entity)
            .unwrap();

        // A second sync pass with nothing changed must not spawn another mirror.
        sync_render_entities(&mut main, &mut render);
        let second_render_entity = **main
            .get_component_for_entity::<RenderEntity>(main_entity)
            .unwrap();

        assert_eq!(first_render_entity, second_render_entity);
    }

    #[test]
    fn despawns_the_render_entity_when_the_main_entity_is_despawned() {
        let mut main = World::new();
        let mut render = World::new();

        let main_entity = main.spawn(SyncWithRenderWorld);
        // Built by hand rather than via `sync_render_entities`, so `main_entity`
        // never gets a `RenderEntity` component — despawning it below must not
        // touch `RenderEntity::on_remove` (see the TODO on that impl: it isn't
        // safe to fire in a two-world test since it doesn't know which world
        // its target id belongs to).
        let render_entity = render.spawn(MainEntity::new(main_entity));

        main.despawn(main_entity);
        despawn_stale_render_entities(&mut main, &mut render);

        assert!(
            render
                .get_component_for_entity::<MainEntity>(render_entity)
                .is_none(),
            "render mirror should have been despawned once its main entity was gone"
        );
    }

    #[test]
    fn keeps_the_render_entity_when_only_the_sync_marker_is_removed() {
        let mut main = World::new();
        let mut render = World::new();

        let main_entity = main.spawn(SyncWithRenderWorld);
        sync_render_entities(&mut main, &mut render);
        let render_entity = **main
            .get_component_for_entity::<RenderEntity>(main_entity)
            .unwrap();

        // Staleness is judged by whether the main entity is still alive, not
        // by the marker — an entity that opts out of syncing without being
        // despawned keeps its (now stale-data) mirror rather than losing it.
        main.remove_component::<SyncWithRenderWorld>(main_entity);
        sync_render_entities(&mut main, &mut render);

        assert!(
            render
                .get_component_for_entity::<MainEntity>(render_entity)
                .is_some(),
            "render mirror should not be despawned while the main entity is still alive"
        );
    }
}

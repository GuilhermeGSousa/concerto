//! Covers the two shapes SceneComponent must support: a type that inserts
//! itself, and a type that expands into other components (including onto a
//! different entity) without ever inserting one of itself.
use concerto_ecs::component::scene::{SceneComponent, SceneEntityRef, SceneSpawnContext};
use concerto_ecs::{Component, Entity, World};
use serde::{Deserialize, Serialize};

#[derive(Component, Serialize, Deserialize, PartialEq, Debug)]
struct PlainMarker {
    value: u32,
}

impl SceneComponent for PlainMarker {
    fn apply(self, entity: Entity, ctx: &mut SceneSpawnContext<'_>) {
        ctx.insert(self, entity);
    }
}

#[derive(Component, Serialize, Deserialize)]
struct ExpandingAuthoringData {
    target: SceneEntityRef,
}

impl SceneComponent for ExpandingAuthoringData {
    fn apply(self, _entity: Entity, ctx: &mut SceneSpawnContext<'_>) {
        // Deliberately inserts onto a *different* entity and never inserts
        // one of itself.
        if let Some(other) = ctx.entity_for(self.target) {
            ctx.insert(PlainMarker { value: 99 }, other);
        }
    }
}

#[test]
fn apply_can_insert_self() {
    let mut world = World::default();
    let entity = world.spawn(());
    let nodes = [entity];

    {
        let mut ctx = SceneSpawnContext::new(&mut world, &nodes);
        PlainMarker { value: 7 }.apply(entity, &mut ctx);
    }

    assert_eq!(
        world.get_component_for_entity::<PlainMarker>(entity),
        Some(&PlainMarker { value: 7 }),
        "a SceneComponent that inserts itself must land on the entity"
    );
}

#[test]
fn apply_can_expand_onto_another_entity_without_inserting_itself() {
    let mut world = World::default();
    let owner = world.spawn(());
    let other = world.spawn(());
    let nodes = [owner, other];

    {
        let mut ctx = SceneSpawnContext::new(&mut world, &nodes);
        ExpandingAuthoringData {
            target: SceneEntityRef::Index(1),
        }
        .apply(owner, &mut ctx);
    }

    assert_eq!(
        world.get_component_for_entity::<PlainMarker>(other),
        Some(&PlainMarker { value: 99 }),
        "expansion must be able to write to an entity other than its own"
    );
    assert!(
        world
            .get_component_for_entity::<ExpandingAuthoringData>(owner)
            .is_none(),
        "authoring data that never inserts itself must leave nothing on its own entity"
    );
}

#[test]
fn entity_for_returns_none_for_an_out_of_range_ref() {
    let mut world = World::default();
    let entity = world.spawn(());
    let nodes = [entity];

    let ctx = SceneSpawnContext::new(&mut world, &nodes);
    assert!(
        ctx.entity_for(SceneEntityRef::Index(5)).is_none(),
        "a malformed scene must yield None, never a panic"
    );
}

#[test]
fn scene_entity_refs_serialize_only_before_resolution() {
    let serialized = serde_json::to_string(&SceneEntityRef::Index(3)).unwrap();
    assert_eq!(serialized, "3");
    assert_eq!(
        serde_json::from_str::<SceneEntityRef>(&serialized).unwrap(),
        SceneEntityRef::Index(3)
    );

    let mut world = World::default();
    let entity = world.spawn(());
    assert!(serde_json::to_string(&SceneEntityRef::Entity(entity)).is_err());
}

#[test]
fn name_is_the_full_type_path_and_distinguishes_generics() {
    #[derive(Component, Serialize, Deserialize)]
    struct Wrapper<T: Send + Sync + 'static> {
        value: u32,
        _marker: std::marker::PhantomData<T>,
    }

    assert!(
        PlainMarker::name().ends_with("PlainMarker"),
        "name must be the full path ending in the type identifier, got: {}",
        PlainMarker::name()
    );
    assert_ne!(
        Wrapper::<u8>::name(),
        Wrapper::<u16>::name(),
        "generic instantiations must not collide on one registry key"
    );
}

#[test]
fn resolved_reference_serializes_as_its_index_inside_a_scope() {
    let mut world = World::default();
    let first = world.spawn(());
    let second = world.spawn(());
    let stranger = world.spawn(());
    let indices = std::collections::HashMap::from([(first, 0), (second, 1)]);

    let inside = concerto_ecs::component::scene::with_entity_indices(&indices, || {
        (
            serde_json::to_string(&SceneEntityRef::Entity(second)).ok(),
            serde_json::to_string(&SceneEntityRef::Entity(stranger)).ok(),
        )
    });

    assert_eq!(inside.0.as_deref(), Some("1"));
    assert_eq!(
        inside.1, None,
        "an entity outside the captured tree must fail rather than serialize"
    );
    assert!(
        serde_json::to_string(&SceneEntityRef::Entity(second)).is_err(),
        "the map must not outlive its scope"
    );
}

#[test]
fn nested_scopes_restore_the_outer_map() {
    let mut world = World::default();
    let entity = world.spawn(());
    let outer = std::collections::HashMap::from([(entity, 4)]);
    let inner = std::collections::HashMap::new();

    let after_inner = concerto_ecs::component::scene::with_entity_indices(&outer, || {
        concerto_ecs::component::scene::with_entity_indices(&inner, || ());
        serde_json::to_string(&SceneEntityRef::Entity(entity)).ok()
    });

    assert_eq!(after_inner.as_deref(), Some("4"));
}

use super::*;
use concerto_ecs::{
    IntoSystem, System, World,
    component::scene::{SceneComponent, SceneSpawnContext},
    entity::hierarchy::{ChildOf, Children},
};
use glam::Vec3;
use serde::{Deserialize, Serialize};

pub(super) fn update(world: &mut World) {
    for mut system in [
        collect_inspector_data.into_system(),
        sync_inspected_components.into_system(),
        build_property_widgets.into_system(),
    ] {
        system.initialize(world);
        system.run_and_apply(world);
    }
    world.tick();
}

#[test]
fn inspector_presentation_systems_do_not_request_exclusive_access() {
    for system in [
        collect_inspector_data.into_system(),
        sync_inspected_components.into_system(),
        build_property_widgets.into_system(),
    ] {
        let mut meta = concerto_ecs::system::meta::SystemMetadata::default();
        let mut access = concerto_ecs::system::access::SystemAccess::default();
        system.fill_access(&mut meta, &mut access);
        assert!(!access.is_exclusive());
    }
}

#[test]
fn metadata_tracks_selection_and_despawn_without_spurious_changes() {
    let (mut world, target, _) = world();
    let mut collect = collect_inspector_data.into_system();
    collect.initialize(&mut world);
    collect.run_and_apply(&mut world);
    let tick = world.resource_changed_tick::<InspectorData>();
    assert_eq!(
        world.get_resource::<InspectorData>().unwrap().entity,
        Some(target)
    );
    world.tick();
    collect.run_and_apply(&mut world);
    assert_eq!(world.resource_changed_tick::<InspectorData>(), tick);
    world.despawn(target);
    collect.run_and_apply(&mut world);
    assert!(
        world
            .get_resource::<InspectorData>()
            .unwrap()
            .entity
            .is_none()
    );
}

#[test]
fn deferred_row_creation_makes_new_bodies_visible() {
    let (mut world, _, _) = world();
    update(&mut world);
    let row = rows(&mut world)[0].0;
    let body = world
        .get_component_for_entity::<concerto_ecs::entity::hierarchy::ChildOf>(row)
        .unwrap()
        .parent();
    assert!(
        world
            .get_component_for_entity::<UINode>(body)
            .unwrap()
            .visible
    );
}

#[test]
fn alternating_selections_keep_showing_their_properties() {
    let (mut world, a, _) = world();
    let b = world.spawn(Transform::IDENTITY);
    for target in [a, b, a, b, a, b] {
        world
            .get_resource_mut::<Selection>()
            .unwrap()
            .select_entity(target);
        update(&mut world);
        world.tick();
        let rows = rows(&mut world);
        assert!(
            !rows.is_empty(),
            "selecting {target:?} built no property rows"
        );
        let body = world
            .get_component_for_entity::<concerto_ecs::entity::hierarchy::ChildOf>(rows[0].0)
            .unwrap()
            .parent();
        assert!(
            world
                .get_component_for_entity::<UINode>(body)
                .unwrap()
                .visible,
            "the card body for {target:?} is hidden, so its properties cannot be edited"
        );
    }
}

fn world() -> (World, Entity, Entity) {
    let mut world = World::default();
    world.register_component::<ChildOf>();
    world.register_component::<Children>();
    let entity = world.spawn(Transform::IDENTITY);
    let mut registry = InspectorRegistry::default();
    registry.register_component::<Transform>();
    world.insert_resource(registry);
    world.insert_resource(UITheme::default());
    let mut selection = Selection::default();
    selection.select_entity(entity);
    world.insert_resource(selection);
    world.insert_resource(InspectorData::default());
    let stack = world.spawn((UINode::default(), ComponentStack::default()));
    (world, entity, stack)
}

fn cards(world: &mut World) -> Vec<(Entity, InspectedComponent)> {
    let mut query = world.query::<(Entity, &InspectedComponent), ()>();
    query
        .iter(world)
        .map(|(entity, card)| (entity, *card))
        .collect()
}

fn rows(world: &mut World) -> Vec<(Entity, PropertyRow)> {
    let mut query = world.query::<(Entity, &PropertyRow), ()>();
    query
        .iter(world)
        .map(|(entity, row)| (entity, row.clone()))
        .collect()
}

#[derive(Component, Serialize, Deserialize)]
struct Tag;
impl SceneComponent for Tag {
    fn apply(self, entity: Entity, ctx: &mut SceneSpawnContext<'_>) {
        ctx.insert(self, entity);
    }
}
#[derive(Component)]
struct Plumbing;

#[test]
fn lists_editable_and_scene_components_as_queryable_cards_and_hides_plumbing() {
    let (mut world, entity, stack) = world();
    world.register_component_type::<Tag>();
    world.insert((Tag, Plumbing), entity);
    update(&mut world);
    let children: Vec<_> = world
        .get_component_for_entity::<Children>(stack)
        .unwrap()
        .iter()
        .copied()
        .collect();
    let names: Vec<_> = children
        .iter()
        .filter_map(|&entity| {
            world
                .get_component_for_entity::<InspectedComponent>(entity)
                .map(|card| card.name)
        })
        .collect();
    assert_eq!(names, ["Tag", "Transform"]);
    assert_eq!(cards(&mut world).len(), 2);
    assert_eq!(rows(&mut world).len(), 3);
    assert!(
        rows(&mut world)
            .iter()
            .all(|(_, row)| row.component == TypeId::of::<Transform>())
    );
}

#[test]
fn structural_changes_rebuild_the_entire_stack_once() {
    let (mut world, entity, stack) = world();
    update(&mut world);
    let old_card = cards(&mut world)[0].0;
    let old_rows = rows(&mut world);
    world.register_component_type::<Tag>();
    world.insert(Tag, entity);
    update(&mut world);
    assert!(!world.entity_is_valid(old_card));
    assert!(old_rows.iter().all(|(row, _)| !world.entity_is_valid(*row)));
    let children = world.get_component_for_entity::<Children>(stack).unwrap();
    let names: Vec<_> = children
        .iter()
        .filter_map(|&child| {
            world
                .get_component_for_entity::<InspectedComponent>(child)
                .map(|card| card.name)
        })
        .collect();
    assert_eq!(names, ["Tag", "Transform"]);
    let before = cards(&mut world);
    update(&mut world);
    assert!(before.iter().all(|(card, _)| world.entity_is_valid(*card)));
    world.remove_component::<Tag>(entity);
    update(&mut world);
    assert!(before.iter().all(|(card, _)| !world.entity_is_valid(*card)));
    assert_eq!(cards(&mut world).len(), 1);

    let old = cards(&mut world)[0].0;
    world.insert(Plumbing, entity);
    world.remove_component::<Plumbing>(entity);
    update(&mut world);
    assert!(!world.entity_is_valid(old));
    assert_eq!(cards(&mut world).len(), 1);
}

struct WholeTransform;
impl PropertyEditor<Transform> for WholeTransform {
    type Snapshot = Vec3;
    type Edit = Vec3;
    fn snapshot(&self, value: &Transform) -> Vec3 {
        value.translation
    }
    fn build(&self, cmd: &mut CommandQueue, row: Entity, _: &Vec3, theme: &UITheme) {
        let child = cmd
            .spawn((UINode::default(), text(theme, "Whole transform")))
            .entity();
        cmd.add_child(row, child);
    }
    fn apply(&self, value: &mut Transform, edit: &Vec3) -> Result<(), EditError> {
        if !edit.is_finite() {
            return Err(EditError::Rejected);
        }
        value.translation = *edit;
        Ok(())
    }
}

fn registry_mut(world: &mut World) -> ResMut<'_, InspectorRegistry> {
    ResMut::new(world.as_unsafe_world_cell_mut())
}

#[test]
fn value_changes_rebuild_rows_and_registering_an_adapter_rebuilds_the_stack() {
    let (mut world, entity, _) = world();
    update(&mut world);
    let original_card = cards(&mut world)[0].0;
    let original_rows = rows(&mut world);
    world
        .get_component_for_entity_mut::<Transform>(entity)
        .unwrap()
        .translation
        .x = 8.0;
    update(&mut world);
    let translation = rows(&mut world)
        .iter()
        .find(|(_, row)| row.path.name() == "translation")
        .unwrap()
        .0;
    assert!(
        original_rows
            .iter()
            .all(|(entity, _)| !world.entity_is_valid(*entity))
    );
    assert!(world.entity_is_valid(original_card));
    assert!(matches!(
        world
            .get_component_for_entity::<PropertyRowValue>(translation)
            .unwrap()
            .snapshot::<Vec3, numeric::NumericFields>(),
        Some(numeric::NumericSnapshot::Vec3([8.0, 0.0, 0.0]))
    ));
    registry_mut(&mut world).register_property_editor::<Vec3, _>(numeric::NumericFields);
    update(&mut world);
    assert!(!world.entity_is_valid(original_card));
    assert!(
        original_rows
            .iter()
            .all(|(entity, _)| !world.entity_is_valid(*entity))
    );
    let original_rows = rows(&mut world);
    let original_card = cards(&mut world)[0].0;
    registry_mut(&mut world).register_property_editor::<Transform, _>(WholeTransform);
    update(&mut world);
    assert!(!world.entity_is_valid(original_card));
    assert!(
        original_rows
            .iter()
            .all(|(entity, _)| !world.entity_is_valid(*entity))
    );
    let root_rows = rows(&mut world);
    assert_eq!(root_rows.len(), 1);
    assert_eq!(root_rows[0].1.path, PropertyPath::default());
    let mut texts = world.query::<&UIText, ()>();
    assert!(
        texts
            .iter(&mut world)
            .any(|text| text.text == "Whole transform")
    );
}

#[test]
fn switching_selection_despawns_old_widgets_but_queued_commits_keep_their_target() {
    let (mut world, a, _) = world();
    update(&mut world);
    let original_cards = cards(&mut world);
    let original_rows = rows(&mut world);
    let row = &original_rows
        .iter()
        .find(|(_, row)| row.path.name() == "translation")
        .unwrap()
        .1;
    let commit = PropertyCommit::new::<Vec3, numeric::NumericFields>(
        row,
        numeric::NumericEdit {
            slot: 0,
            number: 5.0,
        },
    )
    .unwrap();
    let b = world.spawn(Transform::IDENTITY);
    world
        .get_resource_mut::<Selection>()
        .unwrap()
        .select_entity(b);
    update(&mut world);
    assert!(
        original_cards
            .iter()
            .all(|(entity, _)| !world.entity_is_valid(*entity))
    );
    assert!(
        original_rows
            .iter()
            .all(|(entity, _)| !world.entity_is_valid(*entity))
    );
    apply_property_commit(&mut world, commit).unwrap();
    assert_eq!(
        world
            .get_component_for_entity::<Transform>(a)
            .unwrap()
            .translation
            .x,
        5.0
    );
    assert_eq!(
        world
            .get_component_for_entity::<Transform>(b)
            .unwrap()
            .translation
            .x,
        0.0
    );
    assert!(rows(&mut world).iter().all(|(_, row)| row.entity == b));
    world.get_resource_mut::<Selection>().unwrap().clear();
    update(&mut world);
    assert!(cards(&mut world).is_empty());
    assert!(rows(&mut world).is_empty());
    let mut inputs = world.query::<
        &concerto_ui::text_input::UITextInput,
        concerto_ecs::query::filter::Without<add_component::AddComponentSearch>,
    >();
    assert_eq!(inputs.iter(&mut world).count(), 0);
}

#[test]
fn unsupported_values_build_an_explicit_read_only_row() {
    #[derive(Component, concerto_editable::Editable)]
    struct Unsupported {
        title: String,
    }
    let (mut world, _, _) = world();
    world
        .get_resource_mut::<InspectorRegistry>()
        .unwrap()
        .register_component::<Unsupported>();
    let entity = world.spawn(Unsupported {
        title: "Example".into(),
    });
    world
        .get_resource_mut::<Selection>()
        .unwrap()
        .select_entity(entity);
    update(&mut world);
    let all_rows = rows(&mut world);
    assert_eq!(all_rows.len(), 1);
    assert!(!all_rows[0].1.has_editor());
    let mut texts = world.query::<&UIText, ()>();
    assert!(
        texts
            .iter(&mut world)
            .any(|text| text.text == "Unsupported type")
    );
}

#[test]
fn labels_capitalise_the_field_name() {
    assert_eq!(
        label_for(&PropertyPath::new(["translation"])),
        "Translation"
    );
    assert_eq!(label_for(&PropertyPath::new(["inner", "weight"])), "Weight");
}

#[test]
fn changed_property_layout_recreates_rows_in_visitor_order() {
    use concerto_editable::{Editable, PropertyVisitor, PropertyVisitorMut};
    #[derive(Component)]
    struct Dynamic {
        reverse: bool,
        show_b: bool,
        a: f32,
        b: f32,
    }
    impl Editable for Dynamic {
        fn visit(&self, visitor: &mut dyn PropertyVisitor) {
            if self.reverse && self.show_b {
                visitor.field("b", &self.b);
            }
            visitor.field("a", &self.a);
            if !self.reverse && self.show_b {
                visitor.field("b", &self.b);
            }
        }
        fn visit_mut(&mut self, visitor: &mut dyn PropertyVisitorMut) {
            visitor.field("a", &mut self.a);
            if self.show_b {
                visitor.field("b", &mut self.b);
            }
        }
    }
    let (mut world, _, _) = world();
    world
        .get_resource_mut::<InspectorRegistry>()
        .unwrap()
        .register_component::<Dynamic>();
    let target = world.spawn(Dynamic {
        reverse: false,
        show_b: true,
        a: 1.0,
        b: 2.0,
    });
    world
        .get_resource_mut::<Selection>()
        .unwrap()
        .select_entity(target);
    update(&mut world);
    let original = rows(&mut world);
    let a = original
        .iter()
        .find(|(_, row)| row.path.name() == "a")
        .unwrap()
        .0;
    let b = original
        .iter()
        .find(|(_, row)| row.path.name() == "b")
        .unwrap()
        .0;
    let body = world
        .get_component_for_entity::<concerto_ecs::entity::hierarchy::ChildOf>(a)
        .unwrap()
        .parent();
    assert_eq!(
        world
            .get_component_for_entity::<Children>(body)
            .unwrap()
            .iter()
            .copied()
            .collect::<Vec<_>>(),
        [a, b]
    );
    world
        .get_component_for_entity_mut::<Dynamic>(target)
        .unwrap()
        .reverse = true;
    update(&mut world);
    assert!(!world.entity_is_valid(a) && !world.entity_is_valid(b));
    let reordered: Vec<_> = world
        .get_component_for_entity::<Children>(body)
        .unwrap()
        .iter()
        .copied()
        .collect();
    let names: Vec<_> = reordered
        .iter()
        .map(|&row| {
            world
                .get_component_for_entity::<PropertyRow>(row)
                .unwrap()
                .path
                .name()
        })
        .collect();
    assert_eq!(names, ["b", "a"]);
    world
        .get_component_for_entity_mut::<Dynamic>(target)
        .unwrap()
        .show_b = false;
    update(&mut world);
    assert!(reordered.iter().all(|&row| !world.entity_is_valid(row)));
    let remaining = rows(&mut world);
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].1.path.name(), "a");
    assert_eq!(
        world
            .get_component_for_entity::<Children>(body)
            .unwrap()
            .iter()
            .copied()
            .collect::<Vec<_>>(),
        [remaining[0].0]
    );
}

#[test]
fn snapshots_are_released_when_their_card_is_despawned() {
    use std::sync::Arc;
    struct Tracked(Arc<()>);
    impl PropertyEditor<Transform> for Tracked {
        type Snapshot = Arc<()>;
        type Edit = ();
        fn snapshot(&self, _: &Transform) -> Arc<()> {
            self.0.clone()
        }
        fn build(&self, _: &mut CommandQueue, _: Entity, _: &Arc<()>, _: &UITheme) {}
        fn apply(&self, _: &mut Transform, _: &()) -> Result<(), EditError> {
            Ok(())
        }
    }
    let (mut world, target, _) = world();
    let owner = Arc::new(());
    world
        .get_resource_mut::<InspectorRegistry>()
        .unwrap()
        .register_property_editor::<Transform, _>(Tracked(owner.clone()));
    update(&mut world);
    assert_eq!(
        Arc::strong_count(&owner),
        3,
        "test, adapter, and one row-owned snapshot"
    );
    world.remove_component::<Transform>(target);
    update(&mut world);
    assert!(cards(&mut world).is_empty());
    assert!(rows(&mut world).is_empty());
    assert_eq!(
        Arc::strong_count(&owner),
        2,
        "no resource retained the removed row's snapshot"
    );
}

#[test]
fn snapshots_follow_change_ticks_including_late_writes_and_skipped_runs() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    struct Counting(Arc<AtomicUsize>);
    impl PropertyEditor<Transform> for Counting {
        type Snapshot = Vec3;
        type Edit = ();
        fn snapshot(&self, value: &Transform) -> Vec3 {
            self.0.fetch_add(1, Ordering::Relaxed);
            value.translation
        }
        fn build(&self, _: &mut CommandQueue, _: Entity, _: &Vec3, _: &UITheme) {}
        fn apply(&self, _: &mut Transform, _: &()) -> Result<(), EditError> {
            Ok(())
        }
    }
    let (mut world, target, _) = world();
    let count = Arc::new(AtomicUsize::new(0));
    registry_mut(&mut world).register_property_editor::<Transform, _>(Counting(count.clone()));
    update(&mut world);
    update(&mut world);
    let settled = count.load(Ordering::Relaxed);
    let row = rows(&mut world)[0].0;
    let row_tick = world.current_tick();
    for _ in 0..3 {
        update(&mut world);
    }
    assert_eq!(count.load(Ordering::Relaxed), settled);
    assert!(!world.has_component_changed_since(row, TypeId::of::<PropertyRowValue>(), row_tick));

    let mut sync = sync_inspected_components.into_system();
    sync.initialize(&mut world);
    world
        .get_component_for_entity_mut::<Transform>(target)
        .unwrap()
        .translation
        .x = 1.0;
    sync.run_and_apply(&mut world);
    world
        .get_component_for_entity_mut::<Transform>(target)
        .unwrap()
        .translation
        .x = 2.0;
    world.tick();
    update(&mut world);
    assert!(!world.entity_is_valid(row));
    let row = rows(&mut world)[0].0;
    assert_eq!(
        world
            .get_component_for_entity::<PropertyRowValue>(row)
            .unwrap()
            .snapshot::<Transform, Counting>(),
        Some(&Vec3::new(2.0, 0.0, 0.0))
    );

    world
        .get_component_for_entity_mut::<Transform>(target)
        .unwrap()
        .translation
        .x = 3.0;
    for _ in 0..3 {
        world.tick();
    }
    let before = count.load(Ordering::Relaxed);
    update(&mut world);
    assert_eq!(count.load(Ordering::Relaxed), before + 1);
    assert!(!world.entity_is_valid(row));
    let row = rows(&mut world)[0].0;
    assert_eq!(
        world
            .get_component_for_entity::<PropertyRowValue>(row)
            .unwrap()
            .snapshot::<Transform, Counting>(),
        Some(&Vec3::new(3.0, 0.0, 0.0))
    );

    update(&mut world);
    let before = count.load(Ordering::Relaxed);
    world.spawn(Transform::IDENTITY);
    update(&mut world);
    assert_eq!(count.load(Ordering::Relaxed), before);
}

#[test]
fn panel_recreation_and_target_despawn_reconcile_without_selection_changes() {
    let (mut world, target, stack) = world();
    update(&mut world);
    let old_card = cards(&mut world)[0].0;
    world.despawn(stack);
    update(&mut world);
    assert!(!world.entity_is_valid(old_card));
    let stack = world.spawn((UINode::default(), ComponentStack::default()));
    update(&mut world);
    assert_eq!(cards(&mut world).len(), 1);
    assert_eq!(rows(&mut world).len(), 3);
    assert_eq!(
        world
            .get_component_for_entity::<ComponentStack>(stack)
            .unwrap()
            .target,
        Some(target)
    );
    world.despawn(target);
    update(&mut world);
    assert!(cards(&mut world).is_empty());
    assert!(rows(&mut world).is_empty());
    assert_eq!(
        world
            .get_component_for_entity::<ComponentStack>(stack)
            .unwrap()
            .target,
        None
    );
    let since = world.current_tick();
    update(&mut world);
    assert!(!world.has_component_changed_since(stack, TypeId::of::<ComponentStack>(), since));
}

#[test]
fn inspector_access_only_blocks_component_and_declared_resource_writes() {
    use concerto_ecs::system::{access::SystemAccess, meta::SystemMetadata};
    let system = sync_inspected_components.into_system();
    let mut access = SystemAccess::default();
    system.fill_access(&mut SystemMetadata::default(), &mut access);
    let mut unrelated_resource = SystemAccess::default();
    unrelated_resource.write_resource::<PropertyCommits>();
    assert!(SystemAccess::are_disjoint(&access, &unrelated_resource));
    assert!(SystemAccess::are_disjoint(&unrelated_resource, &access));
    let mut registry = SystemAccess::default();
    registry.write_resource::<InspectorRegistry>();
    assert!(!SystemAccess::are_disjoint(&access, &registry));
    let mut component = SystemAccess::default();
    component.write_component::<Transform>();
    assert!(!SystemAccess::are_disjoint(&access, &component));
    assert!(!SystemAccess::are_disjoint(&component, &access));
}

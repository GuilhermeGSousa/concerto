use super::*;
use ecs::{
    component::scene::{SceneComponent, SceneSpawnContext},
    entity::hierarchy::{ChildOf, Children},
    IntoSystem, System, World,
};
use glam::Vec3;
use serde::{Deserialize, Serialize};

pub(super) fn update(world: &mut World) {
    for mut system in [
        collect_inspector_data.into_system(),
        sync_inspected_components.into_system(),
        order_inspector_children.into_system(),
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
        order_inspector_children.into_system(),
        build_property_widgets.into_system(),
    ] {
        let mut meta = ecs::system::meta::SystemMetadata::default();
        let mut access = ecs::system::access::SystemAccess::default();
        system.fill_access(&mut meta, &mut access);
        assert!(!access.is_exclusive());
    }
}

#[test]
fn metadata_tracks_live_names_children_and_removal_without_changing_selection() {
    let (mut world, target, _) = world();
    let mut collect = collect_inspector_data.into_system();
    collect.initialize(&mut world);
    collect.run_and_apply(&mut world);
    world.tick();
    collect.run_and_apply(&mut world);
    let mut unchanged = (|data: Res<InspectorData>| {
        use ecs::query::change_detection::DetectChanges;
        assert!(!data.has_changed());
    })
    .into_system();
    unchanged.initialize(&mut world);
    unchanged.run_and_apply(&mut world);
    world.insert(Name::new("Renamed"), target);
    let child = world.spawn(());
    world.entity_mut(target).add_child(child);
    collect.run_and_apply(&mut world);
    let data = world.get_resource::<InspectorData>().unwrap();
    assert!(data.heading.starts_with("Renamed"));
    assert!(data.heading.ends_with("1 children"));
    world.despawn(target);
    collect.run_and_apply(&mut world);
    let data = world.get_resource::<InspectorData>().unwrap();
    assert!(data.entity.is_none());
    assert_eq!(data.heading, "Selection is no longer in the world.");
}

#[test]
fn deferred_reconciliation_makes_new_bodies_visible_and_consumes_order_requests() {
    let (mut world, _, _) = world();
    update(&mut world);
    let row = rows(&mut world)[0].0;
    let body = world
        .get_component_for_entity::<ecs::entity::hierarchy::ChildOf>(row)
        .unwrap()
        .parent();
    assert!(
        world
            .get_component_for_entity::<UINode>(body)
            .unwrap()
            .visible
    );
    assert_eq!(
        world
            .query::<&sync::PendingChildOrder, ()>()
            .iter(&mut world)
            .count(),
        0
    );
}

/// Switching selection despawns a card and builds the next one in the same
/// pass, so the new card's entities come from recycled indexes. A recycled
/// index that still resolves to its previous owner's row made the card's
/// not-yet-spawned body read a live `UINode`, look already visible, and keep
/// the hidden node it was spawned with — properties gone, uneditable.
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
            .get_component_for_entity::<ecs::entity::hierarchy::ChildOf>(rows[0].0)
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
    let stack = world.spawn((UINode::default(), ComponentStack));
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
        .map(|&entity| {
            world
                .get_component_for_entity::<InspectedComponent>(entity)
                .unwrap()
                .name
        })
        .collect();
    assert_eq!(names, ["Tag", "Transform"]);
    assert_eq!(cards(&mut world).len(), 2);
    assert_eq!(rows(&mut world).len(), 3);
    assert!(rows(&mut world)
        .iter()
        .all(|(_, row)| row.component == TypeId::of::<Transform>()));
}

#[test]
fn adding_and_removing_a_component_keeps_other_cards_rows_and_edit_buffers() {
    use ui::text_input::UITextInput;
    let (mut world, entity, stack) = world();
    update(&mut world);
    let original_card = cards(&mut world)[0].0;
    let original_rows: Vec<_> = rows(&mut world).iter().map(|(entity, _)| *entity).collect();
    let mut inputs = world.query::<Entity, ecs::query::filter::With<UITextInput>>();
    let field = inputs.iter(&mut world).next().unwrap();
    world
        .get_component_for_entity_mut::<UITextInput>(field)
        .unwrap()
        .value = "unfinished edit".into();
    world.register_component_type::<Tag>();
    world.insert(Tag, entity);
    update(&mut world);
    let children: Vec<_> = world
        .get_component_for_entity::<Children>(stack)
        .unwrap()
        .iter()
        .copied()
        .collect();
    assert_eq!(
        children[1], original_card,
        "new Tag card is inserted before existing Transform"
    );
    assert!(original_rows.iter().all(|&row| world.entity_is_valid(row)));
    assert_eq!(
        world
            .get_component_for_entity::<UITextInput>(field)
            .unwrap()
            .value,
        "unfinished edit"
    );
    let tag_card = cards(&mut world)
        .iter()
        .find(|(_, card)| card.type_id == TypeId::of::<Tag>())
        .unwrap()
        .0;
    world.remove_component::<Tag>(entity);
    update(&mut world);
    assert!(!world.entity_is_valid(tag_card));
    assert_eq!(cards(&mut world)[0].0, original_card);
    assert!(original_rows.iter().all(|&row| world.entity_is_valid(row)));
    assert_eq!(
        world
            .get_component_for_entity::<UITextInput>(field)
            .unwrap()
            .value,
        "unfinished edit"
    );
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

/// Mutating through `ResMut` marks the registry changed, which is how an app's
/// registration reaches the inspector.
fn registry_mut(world: &mut World) -> ResMut<'_, InspectorRegistry> {
    ResMut::new(world.as_unsafe_world_cell_mut())
}

#[test]
fn values_refresh_in_place_and_registering_an_adapter_rebuilds_the_cards_rows() {
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
    let translation = original_rows
        .iter()
        .find(|(_, row)| row.path.name() == "translation")
        .unwrap()
        .0;
    assert!(original_rows
        .iter()
        .all(|(entity, _)| world.entity_is_valid(*entity)));
    assert!(matches!(
        world
            .get_component_for_entity::<PropertyRowValue>(translation)
            .unwrap()
            .snapshot::<Vec3, numeric::NumericFields>(),
        Some(numeric::NumericSnapshot::Vec3([8.0, 0.0, 0.0]))
    ));
    registry_mut(&mut world).register_property_editor::<Vec3, _>(numeric::NumericFields);
    update(&mut world);
    // Rows carry the registry's change tick, so any registration rebuilds them
    // all; the card itself is keyed by component and survives.
    assert!(world.entity_is_valid(original_card));
    assert!(original_rows
        .iter()
        .all(|(entity, _)| !world.entity_is_valid(*entity)));
    let original_rows = rows(&mut world);
    registry_mut(&mut world).register_property_editor::<Transform, _>(WholeTransform);
    update(&mut world);
    assert_eq!(cards(&mut world)[0].0, original_card);
    assert!(original_rows
        .iter()
        .all(|(entity, _)| !world.entity_is_valid(*entity)));
    let root_rows = rows(&mut world);
    assert_eq!(root_rows.len(), 1);
    assert_eq!(root_rows[0].1.path, PropertyPath::default());
    let mut texts = world.query::<&TextComponent, ()>();
    assert!(texts
        .iter(&mut world)
        .any(|text| text.text == "Whole transform"));
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
    assert!(original_cards
        .iter()
        .all(|(entity, _)| !world.entity_is_valid(*entity)));
    assert!(original_rows
        .iter()
        .all(|(entity, _)| !world.entity_is_valid(*entity)));
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
    let mut inputs = world.query::<&ui::text_input::UITextInput, ()>();
    assert_eq!(inputs.iter(&mut world).count(), 0);
}

#[test]
fn unsupported_values_build_an_explicit_read_only_row() {
    #[derive(Component, editable::Editable)]
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
    let mut texts = world.query::<&TextComponent, ()>();
    assert!(texts
        .iter(&mut world)
        .any(|text| text.text == "Unsupported type"));
}

#[test]
fn panel_count_comes_from_card_entities() {
    let (mut world, entity, _) = world();
    let label = world.spawn((Label::Count, TextComponent::default()));
    update(&mut world);
    let mut refresh = refresh_inspector.into_system();
    refresh.initialize(&mut world);
    refresh.run_and_apply(&mut world);
    assert_eq!(
        world
            .get_component_for_entity::<TextComponent>(label)
            .unwrap()
            .text,
        "COMPONENTS  1"
    );
    world.remove_component::<Transform>(entity);
    update(&mut world);
    refresh.run_and_apply(&mut world);
    assert_eq!(
        world
            .get_component_for_entity::<TextComponent>(label)
            .unwrap()
            .text,
        ""
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
fn row_reordering_preserves_widgets_and_removed_rows_are_despawned() {
    use editable::{Editable, PropertyVisitor, PropertyVisitorMut};
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
        .get_component_for_entity::<ecs::entity::hierarchy::ChildOf>(a)
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
    assert_eq!(
        world
            .get_component_for_entity::<Children>(body)
            .unwrap()
            .iter()
            .copied()
            .collect::<Vec<_>>(),
        [b, a]
    );
    assert!(world.entity_is_valid(a) && world.entity_is_valid(b));
    world
        .get_component_for_entity_mut::<Dynamic>(target)
        .unwrap()
        .show_b = false;
    update(&mut world);
    assert!(world.entity_is_valid(a));
    assert!(!world.entity_is_valid(b));
    assert_eq!(
        world
            .get_component_for_entity::<Children>(body)
            .unwrap()
            .iter()
            .copied()
            .collect::<Vec<_>>(),
        [a]
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

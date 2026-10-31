use super::*;
use crate::{
    hosted::{HostedApp, testing::SceneFixture},
    tools::selection::Select,
};
use concerto_ecs::{
    IntoSystem, System, World,
    command::EntityCommandQueue,
    component::bundle::IntoBundle,
    entity::hierarchy::{ChildOf, Children},
    query::filter::With,
    reflect::{AppTypeRegistry, ReflectComponent, ReflectSceneComponent},
    system::BoxedSystem,
};
use concerto_foundation::transform::Transform;
use concerto_reflect::{GetTypeRegistration, PathSegment, ReflectDefault};
use concerto_ui::{node::UIBox, text::UIText};
use glam::{Vec2, Vec3};

struct Inspecting {
    f: SceneFixture,
    document: Entity,
    systems: [BoxedSystem; 3],
}

impl Inspecting {
    fn scene(&mut self) -> &mut World {
        self.f.scene_world_mut(self.document)
    }
    fn spawn_in_scene(&mut self, bundle: impl IntoBundle) -> Entity {
        let root = self.f.scene_root(self.document);
        let entity = self.scene().spawn(bundle);
        self.scene().entity_mut(root).add_child(entity);
        entity
    }
    fn register<T: GetTypeRegistration>(&mut self) {
        self.scene().register_type::<T>();
    }
    fn select(&mut self, entity: Option<Entity>) {
        self.scene().trigger(Select(entity));
    }
    fn tick(&mut self) {
        self.f.world.tick();
        self.scene().tick();
    }
    fn update(&mut self) {
        for system in &mut self.systems {
            system.run_and_apply((), &mut self.f.world);
        }
        self.tick();
    }
    fn data(&self) -> &InspectorData {
        self.f.world.get_resource::<InspectorData>().unwrap()
    }
    fn get<T: Component>(&self, entity: Entity) -> Option<&T> {
        self.f
            .scene_world(self.document)
            .get_component_for_entity::<T>(entity)
    }
    fn close(&mut self) {
        self.f.close(self.document);
        self.f.frame();
        self.f.frame();
    }
}

fn inspecting() -> (Inspecting, Entity, Entity) {
    let mut f = SceneFixture::new();
    f.world.register_component::<ChildOf>();
    f.world.register_component::<Children>();
    f.world.insert_resource(InspectorRegistry::default());
    f.world.insert_resource(UITheme::default());
    let document = f.open();
    let mut systems = [
        collect_inspector_data.into_boxed_system(),
        sync_inspected_components.into_boxed_system(),
        build_property_widgets.into_boxed_system(),
    ];
    for system in &mut systems {
        system.initialize(&mut f.world);
    }
    let mut w = Inspecting {
        f,
        document,
        systems,
    };
    let entity = w.spawn_in_scene(Transform::IDENTITY);
    w.select(Some(entity));
    let stack =
        w.f.world
            .spawn((UINode::default(), ComponentStack::default()));
    (w, entity, stack)
}

#[test]
fn inspector_presentation_systems_do_not_request_exclusive_access() {
    for system in [
        collect_inspector_data.into_boxed_system(),
        sync_inspected_components.into_boxed_system(),
        build_property_widgets.into_boxed_system(),
    ] {
        let mut meta = concerto_ecs::system::meta::SystemMetadata::default();
        let mut access = concerto_ecs::system::access::SystemAccess::default();
        system.fill_access(&mut meta, &mut access);
        assert!(!access.is_exclusive());
    }
}

#[test]
fn metadata_tracks_selection_and_despawn_without_spurious_changes() {
    let (mut w, target, _) = inspecting();
    let mut collect = collect_inspector_data.into_system();
    collect.initialize(&mut w.f.world);
    collect.run_and_apply((), &mut w.f.world);
    let tick = w.f.world.resource_changed_tick::<InspectorData>();
    assert_eq!(w.data().scene_entity, Some(target));
    w.tick();
    collect.run_and_apply((), &mut w.f.world);
    assert_eq!(w.f.world.resource_changed_tick::<InspectorData>(), tick);
    w.scene().despawn(target);
    collect.run_and_apply((), &mut w.f.world);
    assert!(w.data().scene_entity.is_none());
}

#[test]
fn deferred_row_creation_makes_new_bodies_visible() {
    let (mut w, _, _) = inspecting();
    w.update();
    let row = rows(&mut w.f.world)[0].0;
    let body =
        w.f.world
            .get_component_for_entity::<ChildOf>(row)
            .unwrap()
            .parent();
    assert!(
        w.f.world
            .get_component_for_entity::<UINode>(body)
            .unwrap()
            .visible
    );
}

#[test]
fn alternating_selections_keep_showing_their_properties() {
    let (mut w, a, _) = inspecting();
    let b = w.spawn_in_scene(Transform::IDENTITY);
    for target in [a, b, a, b, a, b] {
        w.select(Some(target));
        w.update();
        w.tick();
        let rows = rows(&mut w.f.world);
        assert!(
            !rows.is_empty(),
            "selecting {target:?} built no property rows"
        );
        let body =
            w.f.world
                .get_component_for_entity::<ChildOf>(rows[0].0)
                .unwrap()
                .parent();
        assert!(
            w.f.world
                .get_component_for_entity::<UINode>(body)
                .unwrap()
                .visible,
            "the card body for {target:?} is hidden, so its properties cannot be edited"
        );
    }
}

fn layout(height: f32) -> UILayout {
    let rect = UIBox {
        min: Vec2::ZERO,
        size: Vec2::new(200.0, height),
    };
    UILayout {
        rect,
        content_rect: rect,
        clip_rect: rect,
        paint_order: 0,
    }
}

#[test]
fn a_scrolled_inspector_keeps_its_offset_across_runs() {
    let (mut w, _, stack) = inspecting();
    let other = w.spawn_in_scene(Transform::IDENTITY);
    w.f.world.insert(layout(1000.0), stack);
    let view = w.f.world.spawn((
        DetailsView,
        UIScrollArea {
            content: Some(stack),
            ..Default::default()
        },
        layout(100.0),
    ));
    let mut collect = collect_inspector_data.into_system();
    let mut scroll = sync_inspector_scroll.into_system();
    collect.initialize(&mut w.f.world);
    scroll.initialize(&mut w.f.world);
    let mut frame = |w: &mut Inspecting| {
        collect.run_and_apply((), &mut w.f.world);
        scroll.run_and_apply((), &mut w.f.world);
        w.tick();
    };
    let offset = |w: &Inspecting| {
        w.f.world
            .get_component_for_entity::<UIScrollArea>(view)
            .unwrap()
            .offset
    };
    let scroll_and_hold = |w: &mut Inspecting, frame: &mut dyn FnMut(&mut Inspecting)| {
        w.f.world
            .get_component_for_entity_mut::<UIScrollArea>(view)
            .unwrap()
            .offset = 300.0;
        frame(w);
        frame(w);
        assert_eq!(offset(w), 300.0);
    };
    frame(&mut w);
    scroll_and_hold(&mut w, &mut frame);

    w.select(Some(other));
    frame(&mut w);
    assert_eq!(offset(&w), 0.0);
    scroll_and_hold(&mut w, &mut frame);
}

#[derive(Resource, Default)]
struct ScrollChanges(Vec<usize>);

fn count_scroll_changes(
    areas: Query<Entity, concerto_ecs::Changed<UIScrollArea>>,
    mut changes: ResMut<ScrollChanges>,
) {
    changes.0.push(areas.iter().count());
}

#[test]
fn a_settled_inspector_leaves_its_scroll_area_unchanged() {
    let (mut w, _, stack) = inspecting();
    w.f.world.insert(layout(1000.0), stack);
    w.f.world.spawn((
        DetailsView,
        UIScrollArea {
            content: Some(stack),
            offset: 300.0,
            ..Default::default()
        },
        layout(100.0),
    ));
    w.f.world.insert_resource(ScrollChanges::default());
    let mut collect = collect_inspector_data.into_system();
    let mut scroll = sync_inspector_scroll.into_system();
    let mut watch = count_scroll_changes.into_system();
    collect.initialize(&mut w.f.world);
    scroll.initialize(&mut w.f.world);
    watch.initialize(&mut w.f.world);
    for _ in 0..4 {
        collect.run_and_apply((), &mut w.f.world);
        scroll.run_and_apply((), &mut w.f.world);
        watch.run_and_apply((), &mut w.f.world);
        w.tick();
    }

    let changes = &w.f.world.get_resource::<ScrollChanges>().unwrap().0;
    assert_eq!(changes[2..], [0, 0]);
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

#[derive(Component, concerto_reflect::Reflect, Default)]
#[reflect(SceneComponent)]
struct Tag;
#[derive(Component)]
struct Plumbing;

fn stacked_names(world: &World, stack: Entity) -> Vec<&'static str> {
    world
        .get_component_for_entity::<Children>(stack)
        .unwrap()
        .iter()
        .filter_map(|&child| {
            world
                .get_component_for_entity::<InspectedComponent>(child)
                .map(|card| card.name)
        })
        .collect()
}

#[test]
fn lists_reflected_and_scene_components_as_queryable_cards_and_hides_plumbing() {
    let (mut w, entity, stack) = inspecting();
    w.register::<Tag>();
    w.scene().insert((Tag, Plumbing), entity);
    w.update();
    assert_eq!(stacked_names(&w.f.world, stack), ["Tag", "Transform"]);
    assert_eq!(cards(&mut w.f.world).len(), 2);
    assert_eq!(rows(&mut w.f.world).len(), 3);
    assert!(
        rows(&mut w.f.world)
            .iter()
            .all(|(_, row)| row.component == TypeId::of::<Transform>())
    );
}

#[test]
fn structural_changes_rebuild_the_entire_stack_once() {
    let (mut w, entity, stack) = inspecting();
    w.update();
    let old_card = cards(&mut w.f.world)[0].0;
    let old_rows = rows(&mut w.f.world);
    w.register::<Tag>();
    w.scene().insert(Tag, entity);
    w.update();
    assert!(!w.f.world.entity_is_valid(old_card));
    assert!(
        old_rows
            .iter()
            .all(|(row, _)| !w.f.world.entity_is_valid(*row))
    );
    assert_eq!(stacked_names(&w.f.world, stack), ["Tag", "Transform"]);
    let before = cards(&mut w.f.world);
    w.update();
    assert!(
        before
            .iter()
            .all(|(card, _)| w.f.world.entity_is_valid(*card))
    );
    w.scene().remove_component::<Tag>(entity);
    w.update();
    assert!(
        before
            .iter()
            .all(|(card, _)| !w.f.world.entity_is_valid(*card))
    );
    assert_eq!(cards(&mut w.f.world).len(), 1);

    let old = cards(&mut w.f.world)[0].0;
    w.scene().insert(Plumbing, entity);
    w.scene().remove_component::<Plumbing>(entity);
    w.update();
    assert!(!w.f.world.entity_is_valid(old));
    assert_eq!(cards(&mut w.f.world).len(), 1);
}

#[derive(Default)]
struct WholeTransform;
impl PropertyEditor<Transform> for WholeTransform {
    type Snapshot = Vec3;
    type Edit = Vec3;
    fn snapshot(&self, value: &Transform) -> Vec3 {
        value.translation
    }
    fn build(&self, row: &mut EntityCommandQueue, _: &Vec3, theme: &UITheme) {
        row.add_child(theme.label("Whole transform"));
    }
    fn apply(&self, value: &mut Transform, edit: &Vec3, _: &EditContext) -> Result<(), EditError> {
        if !edit.is_finite() {
            return Err(EditError::Rejected);
        }
        value.translation = *edit;
        Ok(())
    }
}

fn registry_mut(world: &mut World) -> ResMut<'_, InspectorRegistry> {
    let ticks = world.out_of_system_ticks();
    // SAFETY: the exclusive borrow of the world covers any access.
    unsafe { ResMut::new(world.as_unsafe_world_cell_mut(), ticks) }.unwrap()
}

#[test]
fn value_changes_keep_numeric_rows_and_registering_an_adapter_rebuilds_the_stack() {
    let (mut w, entity, _) = inspecting();
    w.update();
    let original_card = cards(&mut w.f.world)[0].0;
    let original_rows = rows(&mut w.f.world);
    let mut slots = w.f.world.query::<(Entity, &numeric::NumericSlot), ()>();
    let fields: Vec<Entity> = slots
        .iter(&mut w.f.world)
        .map(|(entity, _)| entity)
        .collect();
    assert_eq!(fields.len(), 9);
    w.scene()
        .get_component_for_entity_mut::<Transform>(entity)
        .unwrap()
        .translation
        .x = 8.0;
    w.update();
    w.update();
    let translation = rows(&mut w.f.world)
        .iter()
        .find(|(_, row)| row.path.name() == "translation")
        .unwrap()
        .0;
    assert!(
        original_rows
            .iter()
            .all(|(entity, _)| w.f.world.entity_is_valid(*entity)),
        "a row whose editor follows its snapshot must survive a value change"
    );
    assert!(
        fields.iter().all(|field| w.f.world.entity_is_valid(*field)),
        "a field that may hold focus must not be respawned under the pointer"
    );
    assert!(w.f.world.entity_is_valid(original_card));
    assert!(matches!(
        w.f.world
            .get_component_for_entity::<PropertyRowValue>(translation)
            .unwrap()
            .snapshot::<Vec3, numeric::NumericFields>(),
        Some(numeric::NumericSnapshot::Vec3([8.0, 0.0, 0.0]))
    ));
    registry_mut(&mut w.f.world).register_property_editor::<Vec3, numeric::NumericFields>();
    w.update();
    assert!(!w.f.world.entity_is_valid(original_card));
    assert!(
        original_rows
            .iter()
            .all(|(entity, _)| !w.f.world.entity_is_valid(*entity))
    );
    let original_rows = rows(&mut w.f.world);
    let original_card = cards(&mut w.f.world)[0].0;
    registry_mut(&mut w.f.world).register_property_editor::<Transform, WholeTransform>();
    w.update();
    assert!(!w.f.world.entity_is_valid(original_card));
    assert!(
        original_rows
            .iter()
            .all(|(entity, _)| !w.f.world.entity_is_valid(*entity))
    );
    let root_rows = rows(&mut w.f.world);
    assert_eq!(root_rows.len(), 1);
    assert_eq!(root_rows[0].1.path, PropertyPath::default());
    let mut texts = w.f.world.query::<&UIText, ()>();
    assert!(
        texts
            .iter(&mut w.f.world)
            .any(|text| text.text == "Whole transform")
    );
}

#[test]
fn switching_selection_despawns_old_widgets_but_queued_commits_keep_their_target() {
    let (mut w, a, _) = inspecting();
    w.update();
    let original_cards = cards(&mut w.f.world);
    let original_rows = rows(&mut w.f.world);
    let row = &original_rows
        .iter()
        .find(|(_, row)| row.path.name() == "translation")
        .unwrap()
        .1;
    let commit = PropertyCommit::new::<Vec3, numeric::NumericFields>(
        row,
        numeric::NumericEdit::Slot {
            slot: 0,
            number: 5.0,
            space: None,
        },
    )
    .unwrap();
    let b = w.spawn_in_scene(Transform::IDENTITY);
    w.select(Some(b));
    w.update();
    assert!(
        original_cards
            .iter()
            .all(|(entity, _)| !w.f.world.entity_is_valid(*entity))
    );
    assert!(
        original_rows
            .iter()
            .all(|(entity, _)| !w.f.world.entity_is_valid(*entity))
    );
    w.f.world
        .get_resource_mut::<PropertyCommits>()
        .unwrap()
        .0
        .push(commit);
    run(&mut w.f.world, send_property_commits);
    assert_eq!(w.get::<Transform>(a).unwrap().translation.x, 5.0);
    assert_eq!(w.get::<Transform>(b).unwrap().translation.x, 0.0);
    assert!(w.f.unsaved(w.document));
    assert!(rows(&mut w.f.world).iter().all(|(_, row)| row.entity == b));
    w.select(None);
    w.update();
    assert!(cards(&mut w.f.world).is_empty());
    assert!(rows(&mut w.f.world).is_empty());
    let mut inputs = w.f.world.query::<&concerto_ui::text_input::UITextInput, (
        concerto_ecs::query::filter::Without<add_component::AddComponentSearch>,
        concerto_ecs::query::filter::Without<crate::hierarchy::Filter>,
    )>();
    assert_eq!(inputs.iter(&mut w.f.world).count(), 0);
}

#[test]
fn unsupported_values_build_an_explicit_read_only_row() {
    #[derive(Component, concerto_reflect::Reflect, Default)]
    #[reflect(Component, Default)]
    struct Unsupported {
        title: String,
    }
    let (mut w, _, _) = inspecting();
    w.register::<Unsupported>();
    let entity = w.spawn_in_scene(Unsupported {
        title: "Example".into(),
    });
    w.select(Some(entity));
    w.update();
    let all_rows = rows(&mut w.f.world);
    assert_eq!(all_rows.len(), 1);
    assert!(!all_rows[0].1.has_editor());
    let mut texts = w.f.world.query::<&UIText, ()>();
    assert!(
        texts
            .iter(&mut w.f.world)
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
fn labels_turn_snake_case_into_words() {
    assert_eq!(label_for(&PropertyPath::new(["light_type"])), "Light type");
    assert_eq!(
        label_for(&PropertyPath::new(["shadowmaps_enabled"])),
        "Shadowmaps enabled"
    );
    assert_eq!(
        label_for(&PropertyPath::new(["light_type", "cone_angle"])),
        "Cone angle"
    );
}

#[test]
fn labels_drop_stray_underscores() {
    assert_eq!(label_for(&PropertyPath::new(["_marker"])), "Marker");
    assert_eq!(label_for(&PropertyPath::new(["value_"])), "Value");
    assert_eq!(label_for(&PropertyPath::new(["near__far"])), "Near far");
}

#[test]
fn a_tuple_index_takes_the_label_of_the_nearest_named_segment() {
    assert_eq!(label_for(&PropertyPath::new(["ambient", "0"])), "Ambient");
    assert_eq!(label_for(&PropertyPath::new(["pair", "0", "1"])), "Pair");
    assert_eq!(
        label_for(&PropertyPath::new(["fog", "0", "density"])),
        "Density"
    );
}

#[test]
fn a_list_element_takes_the_label_of_its_index() {
    let samples = PathSegment::Field("samples");
    let element = PathSegment::Index(2);
    assert_eq!(label_for(&PropertyPath::new([samples, element])), "2");
    assert_eq!(
        label_for(&PropertyPath::new([
            samples,
            element,
            PathSegment::Field("0")
        ])),
        "2"
    );
    assert_eq!(
        label_for(&PropertyPath::new([
            samples,
            element,
            PathSegment::Field("max_weight")
        ])),
        "Max weight"
    );
}

#[test]
fn changed_property_layout_recreates_rows_in_visitor_order() {
    use concerto_reflect::{
        Fields, FromType, PropertyVisitor, PropertyVisitorMut, Reflect, ReflectMut, ReflectRef,
        TypeRegistration,
    };
    #[derive(Component, Default)]
    struct Dynamic {
        reverse: bool,
        show_b: bool,
        a: f32,
        b: f32,
    }
    impl Fields for Dynamic {
        fn field_len(&self) -> usize {
            2
        }
        fn name_at(&self, index: usize) -> Option<&'static str> {
            ["a", "b"].get(index).copied()
        }
        fn field_at(&self, index: usize) -> Option<&dyn Reflect> {
            match index {
                0 => Some(&self.a),
                1 => Some(&self.b),
                _ => None,
            }
        }
        fn field_at_mut(&mut self, index: usize) -> Option<&mut dyn Reflect> {
            match index {
                0 => Some(&mut self.a),
                1 => Some(&mut self.b),
                _ => None,
            }
        }
    }
    impl Reflect for Dynamic {
        fn reflect_ref(&self) -> ReflectRef<'_> {
            ReflectRef::Struct(self)
        }
        fn reflect_mut(&mut self) -> ReflectMut<'_> {
            ReflectMut::Struct(self)
        }
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
    impl GetTypeRegistration for Dynamic {
        fn get_type_registration() -> TypeRegistration {
            let mut registration = TypeRegistration::of::<Self>();
            registration.insert(<ReflectComponent as FromType<Self>>::from_type());
            registration.insert(<ReflectDefault as FromType<Self>>::from_type());
            registration
        }
    }
    let (mut w, _, _) = inspecting();
    w.register::<Dynamic>();
    let target = w.spawn_in_scene(Dynamic {
        reverse: false,
        show_b: true,
        a: 1.0,
        b: 2.0,
    });
    w.select(Some(target));
    w.update();
    let original = rows(&mut w.f.world);
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
    let body =
        w.f.world
            .get_component_for_entity::<ChildOf>(a)
            .unwrap()
            .parent();
    let body_rows = |world: &World| -> Vec<Entity> {
        world
            .get_component_for_entity::<Children>(body)
            .unwrap()
            .iter()
            .copied()
            .collect()
    };
    assert_eq!(body_rows(&w.f.world), [a, b]);
    w.scene()
        .get_component_for_entity_mut::<Dynamic>(target)
        .unwrap()
        .reverse = true;
    w.update();
    assert!(!w.f.world.entity_is_valid(a) && !w.f.world.entity_is_valid(b));
    let reordered = body_rows(&w.f.world);
    let names: Vec<_> = reordered
        .iter()
        .map(|&row| {
            w.f.world
                .get_component_for_entity::<PropertyRow>(row)
                .unwrap()
                .path
                .name()
        })
        .collect();
    assert_eq!(names, ["b", "a"]);
    w.scene()
        .get_component_for_entity_mut::<Dynamic>(target)
        .unwrap()
        .show_b = false;
    w.update();
    assert!(reordered.iter().all(|&row| !w.f.world.entity_is_valid(row)));
    let remaining = rows(&mut w.f.world);
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].1.path.name(), "a");
    assert_eq!(body_rows(&w.f.world), [remaining[0].0]);
}

#[test]
fn snapshots_are_released_when_their_card_is_despawned() {
    use std::sync::{Arc, LazyLock};
    static OWNER: LazyLock<Arc<()>> = LazyLock::new(|| Arc::new(()));
    #[derive(Default)]
    struct Tracked;
    impl PropertyEditor<Transform> for Tracked {
        type Snapshot = Arc<()>;
        type Edit = ();
        fn snapshot(&self, _: &Transform) -> Arc<()> {
            OWNER.clone()
        }
        fn build(&self, _: &mut EntityCommandQueue, _: &Arc<()>, _: &UITheme) {}
        fn apply(&self, _: &mut Transform, _: &(), _: &EditContext) -> Result<(), EditError> {
            Ok(())
        }
    }
    let (mut w, target, _) = inspecting();
    w.f.world
        .get_resource_mut::<InspectorRegistry>()
        .unwrap()
        .register_property_editor::<Transform, Tracked>();
    w.update();
    assert_eq!(
        Arc::strong_count(&OWNER),
        2,
        "the static and one row-owned snapshot"
    );
    w.scene().remove_component::<Transform>(target);
    w.update();
    assert!(cards(&mut w.f.world).is_empty());
    assert!(rows(&mut w.f.world).is_empty());
    assert_eq!(
        Arc::strong_count(&OWNER),
        1,
        "no resource retained the removed row's snapshot"
    );
}

#[test]
fn snapshots_follow_change_ticks_including_late_writes_and_skipped_runs() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static COUNT: AtomicUsize = AtomicUsize::new(0);
    #[derive(Default)]
    struct Counting;
    impl PropertyEditor<Transform> for Counting {
        type Snapshot = Vec3;
        type Edit = ();
        fn snapshot(&self, value: &Transform) -> Vec3 {
            COUNT.fetch_add(1, Ordering::Relaxed);
            value.translation
        }
        fn build(&self, _: &mut EntityCommandQueue, _: &Vec3, _: &UITheme) {}
        fn apply(&self, _: &mut Transform, _: &(), _: &EditContext) -> Result<(), EditError> {
            Ok(())
        }
    }
    let (mut w, target, _) = inspecting();
    let count = &COUNT;
    registry_mut(&mut w.f.world).register_property_editor::<Transform, Counting>();
    w.update();
    w.update();
    let settled = count.load(Ordering::Relaxed);
    let row = rows(&mut w.f.world)[0].0;
    let row_tick = w.f.world.change_tick();
    for _ in 0..3 {
        w.update();
    }
    assert_eq!(count.load(Ordering::Relaxed), settled);
    assert!(!w.f.world.has_component_changed_since(
        row,
        TypeId::of::<PropertyRowValue>(),
        row_tick
    ));

    let mut sync = sync_inspected_components.into_system();
    sync.initialize(&mut w.f.world);
    let set_x = |w: &mut Inspecting, x: f32| {
        w.scene()
            .get_component_for_entity_mut::<Transform>(target)
            .unwrap()
            .translation
            .x = x;
    };
    set_x(&mut w, 1.0);
    sync.run_and_apply((), &mut w.f.world);
    set_x(&mut w, 2.0);
    w.tick();
    w.update();
    assert!(!w.f.world.entity_is_valid(row));
    let row = rows(&mut w.f.world)[0].0;
    assert_eq!(
        w.f.world
            .get_component_for_entity::<PropertyRowValue>(row)
            .unwrap()
            .snapshot::<Transform, Counting>(),
        Some(&Vec3::new(2.0, 0.0, 0.0))
    );

    set_x(&mut w, 3.0);
    for _ in 0..3 {
        w.tick();
    }
    let before = count.load(Ordering::Relaxed);
    w.update();
    assert_eq!(count.load(Ordering::Relaxed), before + 1);
    assert!(!w.f.world.entity_is_valid(row));
    let row = rows(&mut w.f.world)[0].0;
    assert_eq!(
        w.f.world
            .get_component_for_entity::<PropertyRowValue>(row)
            .unwrap()
            .snapshot::<Transform, Counting>(),
        Some(&Vec3::new(3.0, 0.0, 0.0))
    );

    w.update();
    let before = count.load(Ordering::Relaxed);
    w.spawn_in_scene(Transform::IDENTITY);
    w.update();
    assert_eq!(count.load(Ordering::Relaxed), before);
}

#[test]
fn a_component_is_read_again_only_when_the_scenes_own_clock_says_it_changed() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static COUNT: AtomicUsize = AtomicUsize::new(0);
    #[derive(Default)]
    struct Counting;
    impl PropertyEditor<Transform> for Counting {
        type Snapshot = Vec3;
        type Edit = ();
        fn snapshot(&self, value: &Transform) -> Vec3 {
            COUNT.fetch_add(1, Ordering::Relaxed);
            value.translation
        }
        fn build(&self, _: &mut EntityCommandQueue, _: &Vec3, _: &UITheme) {}
        fn apply(&self, _: &mut Transform, _: &(), _: &EditContext) -> Result<(), EditError> {
            Ok(())
        }
    }
    let (mut w, _, _) = inspecting();
    let count = &COUNT;
    registry_mut(&mut w.f.world).register_property_editor::<Transform, Counting>();
    w.update();
    w.update();
    let settled = count.load(Ordering::Relaxed);

    for _ in 0..40 {
        w.f.world.tick();
    }
    w.update();
    w.update();

    assert_eq!(
        count.load(Ordering::Relaxed),
        settled,
        "the editor's clock running ahead of the scene's is not a change in the scene"
    );
}

#[test]
fn a_card_built_for_a_component_unchanged_since_the_last_run_reads_it_in_full() {
    let (mut w, _, _) = inspecting();
    let other = w.spawn_in_scene(Transform::IDENTITY);
    w.update();
    w.update();
    w.update();

    w.select(Some(other));
    w.update();

    let shown: Vec<Entity> = rows(&mut w.f.world)
        .into_iter()
        .map(|(_, row)| row.entity)
        .collect();
    assert_eq!(shown, [other; 3]);
}

#[test]
fn panel_recreation_and_target_despawn_reconcile_without_selection_changes() {
    let (mut w, target, stack) = inspecting();
    w.update();
    let old_card = cards(&mut w.f.world)[0].0;
    w.f.world.despawn(stack);
    w.update();
    assert!(!w.f.world.entity_is_valid(old_card));
    let stack =
        w.f.world
            .spawn((UINode::default(), ComponentStack::default()));
    w.update();
    assert_eq!(cards(&mut w.f.world).len(), 1);
    assert_eq!(rows(&mut w.f.world).len(), 3);
    let shown = |w: &Inspecting| {
        w.f.world
            .get_component_for_entity::<ComponentStack>(stack)
            .unwrap()
            .scene_target
    };
    assert_eq!(shown(&w), Some(target));
    w.scene().despawn(target);
    w.update();
    assert!(cards(&mut w.f.world).is_empty());
    assert!(rows(&mut w.f.world).is_empty());
    assert_eq!(shown(&w), None);
    let since = w.f.world.change_tick();
    w.update();
    assert!(
        !w.f.world
            .has_component_changed_since(stack, TypeId::of::<ComponentStack>(), since)
    );
}

#[test]
fn inspector_access_blocks_writes_to_the_hosted_app_and_the_widget_registry_only() {
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
    let mut hosted = SystemAccess::default();
    hosted.write_component::<HostedApp>();
    assert!(!SystemAccess::are_disjoint(&access, &hosted));
    assert!(!SystemAccess::are_disjoint(&hosted, &access));
    let mut table = SystemAccess::default();
    table.write_resource::<AppTypeRegistry>();
    assert!(
        SystemAccess::are_disjoint(&access, &table),
        "the table it reads is the scene's, not one in the editor's world"
    );
}

#[derive(Component, concerto_reflect::Reflect, Default)]
#[reflect(Component, Default)]
struct Gain {
    amount: f32,
}

fn structural() -> (Inspecting, Entity, Entity) {
    use concerto_ecs::events::event_channel::EventChannel;
    let (mut w, entity, stack) = inspecting();
    w.register::<Gain>();
    w.f.world
        .insert_resource(EventChannel::<concerto_ui::text_input::UITextInputSubmitted>::default());
    w.update();
    (w, entity, stack)
}

fn run<M>(world: &mut World, system: impl IntoSystem<(), M>) {
    let mut system = system.into_system();
    system.initialize(world);
    system.run_and_apply((), world);
}

fn click(world: &mut World, entity: Entity) {
    world.trigger_on(
        entity,
        concerto_ui::interaction::UIClick {
            position: glam::Vec2::ZERO,
            button: concerto_window::input::MouseButton::Left,
        },
    );
}

fn only<T: Component>(world: &mut World) -> Entity {
    let mut query = world.query::<(Entity, &T), ()>();
    let found: Vec<Entity> = query.iter(world).map(|(entity, _)| entity).collect();
    assert_eq!(found.len(), 1, "expected exactly one match");
    found[0]
}

fn is_open(world: &World, panel: Entity) -> bool {
    world
        .get_component_for_entity::<concerto_ui::anchor::UIAnchoredPanel>(panel)
        .unwrap()
        .open
}

fn assert_overlay_fill(world: &World, menu: Entity) {
    assert_eq!(
        world
            .get_component_for_entity::<concerto_ui::material::UIMaterial>(menu)
            .unwrap()
            .color,
        concerto_ui::theme::UITheme::default()
            .surface_overlay
            .to_linear()
    );
}

fn open_add_menu(world: &mut World) -> Entity {
    let menu = only::<add_component::AddComponentMenu>(world);
    world
        .get_component_for_entity_mut::<concerto_ui::anchor::UIAnchoredPanel>(menu)
        .unwrap()
        .open = true;
    run(world, populate_add_component_menu);
    assert_overlay_fill(world, menu);
    menu
}

fn card_names(world: &mut World) -> Vec<&'static str> {
    let mut names: Vec<_> = cards(world).iter().map(|(_, card)| card.name).collect();
    names.sort();
    names
}

fn queued(world: &World) -> Vec<ComponentEdit> {
    world.get_resource::<ComponentEdits>().unwrap().0.clone()
}

#[test]
fn clicking_a_menu_entry_adds_the_component_and_closes_the_menu() {
    let (mut w, entity, _) = structural();
    let menu = open_add_menu(&mut w.f.world);
    let entry = only::<add_component::AddComponentEntry>(&mut w.f.world);

    click(&mut w.f.world, entry);
    assert_eq!(
        queued(&w.f.world),
        [ComponentEdit::Add {
            entity,
            component: TypeId::of::<Gain>(),
        }]
    );
    assert!(!is_open(&w.f.world, menu));

    run(&mut w.f.world, send_component_edits);
    assert!(queued(&w.f.world).is_empty());
    assert!(w.get::<Gain>(entity).is_some());
    assert!(w.f.unsaved(w.document));
    w.update();
    assert_eq!(card_names(&mut w.f.world), ["Gain", "Transform"]);
}

#[test]
fn submitting_the_search_adds_the_highlighted_entry() {
    use concerto_ecs::events::event_channel::EventChannel;
    use concerto_ui::text_input::UITextInputSubmitted;
    let (mut w, entity, _) = structural();
    let menu = open_add_menu(&mut w.f.world);
    let search = only::<add_component::AddComponentSearch>(&mut w.f.world);
    let stray = w.f.world.spawn(());
    for field in [stray, search] {
        w.f.world
            .get_resource_mut::<EventChannel<UITextInputSubmitted>>()
            .unwrap()
            .push_event(UITextInputSubmitted {
                entity: field,
                value: String::new(),
            });
    }

    run(&mut w.f.world, add_component::add_highlighted_component);
    assert_eq!(
        queued(&w.f.world),
        [ComponentEdit::Add {
            entity,
            component: TypeId::of::<Gain>(),
        }]
    );
    assert!(!is_open(&w.f.world, menu));
}

#[test]
fn a_cards_menu_removes_its_component() {
    let (mut w, entity, _) = structural();
    w.scene().insert(Gain::default(), entity);
    w.update();
    let menu = only::<component_menu::ComponentMenu>(&mut w.f.world);
    let buttons: Vec<Entity> = {
        let mut query =
            w.f.world
                .query::<(Entity, &component_menu::ComponentMenuButton), ()>();
        query
            .iter(&mut w.f.world)
            .filter(|(_, button)| button.component == TypeId::of::<Gain>())
            .map(|(entity, _)| entity)
            .collect()
    };
    assert_eq!(buttons.len(), 1);
    let button = buttons[0];

    click(&mut w.f.world, button);
    assert!(is_open(&w.f.world, menu));
    click(&mut w.f.world, button);
    assert!(!is_open(&w.f.world, menu));
    click(&mut w.f.world, button);
    let panel =
        w.f.world
            .get_component_for_entity::<concerto_ui::anchor::UIAnchoredPanel>(menu)
            .unwrap();
    assert!(panel.open);
    assert_eq!(panel.owner, Some(button));
    assert_overlay_fill(&w.f.world, menu);

    let remove = only::<component_menu::RemoveComponentItem>(&mut w.f.world);
    click(&mut w.f.world, remove);
    assert_eq!(
        queued(&w.f.world),
        [ComponentEdit::Remove {
            entity,
            component: TypeId::of::<Gain>(),
        }]
    );
    assert!(!is_open(&w.f.world, menu));

    run(&mut w.f.world, send_component_edits);
    assert!(w.get::<Gain>(entity).is_none());
    w.update();
    assert_eq!(card_names(&mut w.f.world), ["Transform"]);
    assert_eq!(
        w.f.world
            .query::<&component_menu::ComponentMenu, ()>()
            .iter(&mut w.f.world)
            .count(),
        1
    );
}

#[test]
fn only_registered_components_offer_a_menu() {
    let (mut w, entity, _) = structural();
    w.register::<Tag>();
    w.scene().insert(Tag, entity);
    w.update();
    assert_eq!(card_names(&mut w.f.world), ["Tag", "Transform"]);
    let mut query =
        w.f.world
            .query::<&component_menu::ComponentMenuButton, ()>();
    let components: Vec<TypeId> = query.iter(&mut w.f.world).map(|b| b.component).collect();
    assert_eq!(components, [TypeId::of::<Transform>()]);
}

#[test]
fn the_scene_root_wrapper_shows_no_cards() {
    let (mut w, _, _) = inspecting();
    let root = w.f.scene_root(w.document);
    w.select(Some(root));
    w.update();
    assert!(w.data().scene_entity.is_none());
    assert_eq!(w.data().closable_scene_root, Some(root));
    assert!(cards(&mut w.f.world).is_empty());
}

#[test]
fn readonly_components_are_edited_and_removed_but_never_added() {
    #[derive(Component, concerto_reflect::Reflect)]
    #[reflect(Component)]
    struct Lens {
        focal: f32,
    }
    let lens = TypeId::of::<Lens>();
    let (mut w, entity, _) = structural();
    w.register::<Lens>();
    w.update();

    open_add_menu(&mut w.f.world);
    let entry = only::<add_component::AddComponentEntry>(&mut w.f.world);
    click(&mut w.f.world, entry);
    assert_eq!(
        queued(&w.f.world),
        [ComponentEdit::Add {
            entity,
            component: TypeId::of::<Gain>(),
        }]
    );
    w.f.world.get_resource_mut::<ComponentEdits>().unwrap().0 = vec![ComponentEdit::Add {
        entity,
        component: lens,
    }];
    run(&mut w.f.world, send_component_edits);
    assert!(w.get::<Lens>(entity).is_none());

    w.scene().insert(Lens { focal: 35.0 }, entity);
    w.update();
    assert_eq!(card_names(&mut w.f.world), ["Lens", "Transform"]);
    assert!(
        rows(&mut w.f.world)
            .iter()
            .any(|(_, row)| row.component == lens)
    );
    let mut buttons =
        w.f.world
            .query::<&component_menu::ComponentMenuButton, ()>();
    assert!(buttons.iter(&mut w.f.world).any(|b| b.component == lens));
    w.f.world
        .get_resource_mut::<ComponentEdits>()
        .unwrap()
        .0
        .push(ComponentEdit::Remove {
            entity,
            component: lens,
        });
    run(&mut w.f.world, send_component_edits);
    assert!(w.get::<Lens>(entity).is_none());
}

#[test]
fn closing_the_tab_empties_the_cards_and_closes_the_menus() {
    let (mut w, entity, stack) = structural();
    w.scene().insert(Gain::default(), entity);
    w.update();
    let add_menu = open_add_menu(&mut w.f.world);
    let button = {
        let mut query =
            w.f.world
                .query::<(Entity, &component_menu::ComponentMenuButton), ()>();
        query.iter(&mut w.f.world).next().unwrap().0
    };
    click(&mut w.f.world, button);
    let component_menu = only::<component_menu::ComponentMenu>(&mut w.f.world);
    assert!(!cards(&mut w.f.world).is_empty());
    assert!(is_open(&w.f.world, component_menu));

    w.close();

    assert!(cards(&mut w.f.world).is_empty());
    assert!(rows(&mut w.f.world).is_empty());
    assert_eq!(
        w.f.world
            .get_component_for_entity::<ComponentStack>(stack)
            .unwrap()
            .scene_target,
        None
    );
    assert!(!is_open(&w.f.world, add_menu));
    assert!(!is_open(&w.f.world, component_menu));
    assert_eq!(
        w.f.world
            .query::<&add_component::AddComponentEntry, ()>()
            .iter(&mut w.f.world)
            .count(),
        0
    );
    let remove = only::<component_menu::RemoveComponentItem>(&mut w.f.world);
    click(&mut w.f.world, remove);
    assert!(queued(&w.f.world).is_empty());
}

#[test]
fn edits_still_queued_when_the_edit_target_changes_are_never_sent() {
    let (mut w, entity, _) = structural();
    w.update();
    let row = rows(&mut w.f.world)
        .into_iter()
        .find(|(_, row)| row.path.name() == "translation")
        .unwrap()
        .1;
    w.f.world
        .get_resource_mut::<PropertyCommits>()
        .unwrap()
        .push::<Vec3, numeric::NumericFields>(
            &row,
            numeric::NumericEdit::Slot {
                slot: 0,
                number: 5.0,
                space: None,
            },
        )
        .unwrap();
    w.f.world
        .get_resource_mut::<ComponentEdits>()
        .unwrap()
        .0
        .push(ComponentEdit::Add {
            entity,
            component: TypeId::of::<Gain>(),
        });

    w.f.activate(None);
    w.f.frame();
    w.f.activate(Some(w.document));
    w.f.frame();

    assert!(queued(&w.f.world).is_empty());
    assert!(
        w.f.world
            .get_resource::<PropertyCommits>()
            .unwrap()
            .0
            .is_empty()
    );
    run(&mut w.f.world, send_property_commits);
    run(&mut w.f.world, send_component_edits);
    assert!(!w.f.unsaved(w.document));
    assert!(w.get::<Gain>(entity).is_none());
}

#[test]
fn a_commit_still_queued_when_a_replacement_is_ready_refuses_it_and_is_sent() {
    let (mut w, entity, _) = inspecting();
    w.update();
    let row = rows(&mut w.f.world)
        .into_iter()
        .find(|(_, row)| row.path.name() == "translation")
        .unwrap()
        .1;
    let before = w.f.scene_world(w.document).id();
    w.f.request_other_scene(w.document);
    w.f.frames_until_building(w.document);
    w.f.frame();
    w.f.wait_until_the_build_is_ready(w.document);
    w.f.world
        .get_resource_mut::<PropertyCommits>()
        .unwrap()
        .push::<Vec3, numeric::NumericFields>(
            &row,
            numeric::NumericEdit::Slot {
                slot: 0,
                number: 5.0,
                space: None,
            },
        )
        .unwrap();

    w.f.frame();

    assert_eq!(w.f.scene_world(w.document).id(), before);
    assert!(w.f.status(w.document).contains("unsaved changes"));
    run(&mut w.f.world, send_property_commits);
    assert_eq!(w.get::<Transform>(entity).unwrap().translation.x, 5.0);
    assert!(w.f.unsaved(w.document));
}

mod name_field {
    use super::*;
    use crate::tools::entity_ops::EntityEdit;
    use concerto_ecs::{component::name::Name, events::event_channel::EventChannel, signal::On};
    use concerto_ui::{
        focus::{FocusedWidget, UIFocusGained, UIFocusLost},
        text_input::{UITextInput, UITextInputCancelled, UITextInputSubmitted},
    };

    #[derive(Resource, Default)]
    struct Renames(u32);

    struct Fixture {
        w: Inspecting,
        field: Entity,
        root: Entity,
        first: Entity,
        second: Entity,
    }

    fn fixture() -> Fixture {
        let (mut w, _, _) = inspecting();
        let world = &mut w.f.world;
        world.insert_resource(FocusedWidget::default());
        world.insert_resource(FocusNameField::default());
        world.insert_resource(EventChannel::<UITextInputSubmitted>::default());
        world.insert_resource(EventChannel::<UITextInputCancelled>::default());
        world.insert_resource(EventChannel::<UIFocusLost>::default());
        world.insert_resource(EventChannel::<UIFocusGained>::default());
        let field = world.spawn((
            UINode::default().with_visible(false),
            UITextInput::new("Name"),
            NameField::default(),
        ));
        w.scene().insert_resource(Renames::default());
        w.scene()
            .add_listener(|_: On<EntityEdit>, mut renames: ResMut<Renames>| renames.0 += 1);
        let root = w.f.scene_root(w.document);
        let first = w.f.scene_entity(w.document, "first");
        let second = w.f.scene_entity(w.document, "second");
        let mut fixture = Fixture {
            w,
            field,
            root,
            first,
            second,
        };
        fixture.select(first);
        fixture
    }

    impl Fixture {
        fn world(&mut self) -> &mut World {
            &mut self.w.f.world
        }
        fn step(&mut self) {
            for mut system in [
                collect_inspector_data.into_boxed_system(),
                update_name_field.into_boxed_system(),
            ] {
                system.initialize(self.world());
                system.run_and_apply((), self.world());
            }
            self.world()
                .get_resource_mut::<EventChannel<UITextInputSubmitted>>()
                .unwrap()
                .update();
            self.world()
                .get_resource_mut::<EventChannel<UITextInputCancelled>>()
                .unwrap()
                .update();
            self.world()
                .get_resource_mut::<EventChannel<UIFocusLost>>()
                .unwrap()
                .update();
            self.world()
                .get_resource_mut::<EventChannel<UIFocusGained>>()
                .unwrap()
                .update();
        }
        fn select(&mut self, entity: Entity) {
            self.w.select(Some(entity));
            self.step();
        }
        fn focus(&mut self) {
            let field = self.field;
            **self.world().get_resource_mut::<FocusedWidget>().unwrap() = Some(field);
        }
        fn blur(&mut self) {
            **self.world().get_resource_mut::<FocusedWidget>().unwrap() = None;
            let field = self.field;
            self.world()
                .get_resource_mut::<EventChannel<UIFocusLost>>()
                .unwrap()
                .push_event(UIFocusLost(field));
        }
        fn type_text(&mut self, text: &str) {
            self.focus();
            let field = self.field;
            self.world()
                .get_component_for_entity_mut::<UITextInput>(field)
                .unwrap()
                .value = text.into();
        }
        fn submit(&mut self, text: &str) {
            let field = self.field;
            self.world()
                .get_resource_mut::<EventChannel<UITextInputSubmitted>>()
                .unwrap()
                .push_event(UITextInputSubmitted {
                    entity: field,
                    value: text.into(),
                });
        }
        fn text(&self) -> &str {
            &self
                .w
                .f
                .world
                .get_component_for_entity::<UITextInput>(self.field)
                .unwrap()
                .value
        }
        fn visible(&self) -> bool {
            self.w
                .f
                .world
                .get_component_for_entity::<UINode>(self.field)
                .unwrap()
                .visible
        }
        fn name(&self, entity: Entity) -> &str {
            self.w.get::<Name>(entity).unwrap().as_str()
        }
        fn renames(&self) -> u32 {
            self.w
                .f
                .scene_world(self.w.document)
                .get_resource::<Renames>()
                .unwrap()
                .0
        }
        fn focused(&self) -> Option<Entity> {
            **self.w.f.world.get_resource::<FocusedWidget>().unwrap()
        }
    }

    #[test]
    fn the_field_shows_the_selected_name_and_hides_for_the_root() {
        let mut f = fixture();
        assert_eq!(f.text(), "first");
        assert!(f.visible());
        f.select(f.second);
        assert_eq!(f.text(), "second");
        f.select(f.root);
        assert!(!f.visible());
        assert_eq!(f.text(), "");
    }

    #[test]
    fn submitting_a_changed_name_renames_and_an_unchanged_one_does_nothing() {
        let mut f = fixture();
        f.type_text("first");
        f.submit("first");
        f.step();
        assert_eq!(f.renames(), 0);

        f.type_text("lamp");
        f.submit("lamp");
        f.step();
        assert_eq!(f.name(f.first), "lamp");
        assert_eq!(f.renames(), 1);
        assert!(f.w.f.unsaved(f.w.document));
    }

    #[test]
    fn losing_focus_to_another_selection_renames_the_entity_being_edited() {
        let mut f = fixture();
        f.type_text("lamp");
        f.blur();
        f.select(f.second);
        assert_eq!(f.name(f.first), "lamp");
        assert_eq!(f.name(f.second), "second");
        f.step();
        assert_eq!(f.text(), "second");
    }

    #[test]
    fn escape_restores_the_name_and_releases_focus() {
        let mut f = fixture();
        f.type_text("lamp");
        let field = f.field;
        f.world()
            .get_resource_mut::<EventChannel<UITextInputCancelled>>()
            .unwrap()
            .push_event(UITextInputCancelled { entity: field });
        f.step();
        assert_eq!(f.text(), "first");
        assert_eq!(f.name(f.first), "first");
        assert!(f.focused().is_none());
        assert_eq!(f.renames(), 0);
    }

    #[test]
    fn a_focus_request_focuses_the_visible_field() {
        let mut f = fixture();
        f.world().get_resource_mut::<FocusNameField>().unwrap().0 = true;
        f.step();
        assert_eq!(f.focused(), Some(f.field));

        **f.world().get_resource_mut::<FocusedWidget>().unwrap() = None;
        f.select(f.root);
        f.world().get_resource_mut::<FocusNameField>().unwrap().0 = true;
        f.step();
        assert!(f.focused().is_none());
    }

    #[test]
    fn the_field_is_hidden_and_forgets_its_entity_when_the_tab_closes() {
        let mut f = fixture();
        f.type_text("lamp");
        assert!(f.visible());

        f.w.close();
        f.blur();
        f.step();

        assert!(!f.visible());
        assert_eq!(f.text(), "");
    }

    #[test]
    fn a_name_being_typed_when_the_scene_is_replaced_renames_nothing_in_the_new_one() {
        let mut f = fixture();
        f.type_text("lamp");

        f.w.f.request_other_scene(f.w.document);
        f.w.f.frames_until_idle();
        f.blur();
        f.step();

        assert_eq!(f.w.f.scene_names(f.w.document), ["third"]);
        assert!(!f.w.f.unsaved(f.w.document));
    }
}

mod bool_field {
    use super::*;
    use concerto_ui::checkbox::UICheckbox;

    #[derive(Component, concerto_reflect::Reflect)]
    #[reflect(Component)]
    struct Switch {
        on: bool,
    }

    fn fixture() -> (Inspecting, Entity) {
        let (mut w, entity, _) = inspecting();
        w.register::<Switch>();
        w.scene().insert(Switch { on: false }, entity);
        w.update();
        w.update();
        (w, entity)
    }

    fn checked(world: &World, checkbox: Entity) -> bool {
        world
            .get_component_for_entity::<UICheckbox>(checkbox)
            .unwrap()
            .checked
    }

    #[test]
    fn a_bool_property_is_shown_as_a_checkbox_holding_its_value() {
        let (mut w, _) = fixture();
        let checkbox = only::<UICheckbox>(&mut w.f.world);
        assert!(!checked(&w.f.world, checkbox));
        let mut texts = w.f.world.query::<&UIText, ()>();
        assert!(
            texts
                .iter(&mut w.f.world)
                .all(|text| text.text != "Unsupported type")
        );
    }

    #[test]
    fn toggling_the_checkbox_commits_the_new_value() {
        let (mut w, entity) = fixture();
        let checkbox = only::<UICheckbox>(&mut w.f.world);

        click(&mut w.f.world, checkbox);
        run(&mut w.f.world, send_property_commits);

        assert!(w.get::<Switch>(entity).unwrap().on);
    }

    #[test]
    fn a_value_changed_elsewhere_reaches_the_same_checkbox() {
        let (mut w, entity) = fixture();
        let checkbox = only::<UICheckbox>(&mut w.f.world);

        w.scene()
            .get_component_for_entity_mut::<Switch>(entity)
            .unwrap()
            .on = true;
        w.update();
        w.update();
        run(&mut w.f.world, boolean::refresh_bool_fields);

        assert_eq!(only::<UICheckbox>(&mut w.f.world), checkbox);
        assert!(checked(&w.f.world, checkbox));
    }
}

mod variant_rows {
    use super::*;
    use concerto_render::components::{
        CameraEnvironment, Light, camera_environment::Fog, light::LightType,
    };
    use concerto_ui::{anchor::UIAnchoredPanel, checkbox::UICheckbox};

    fn fixture(component: impl Component) -> (Inspecting, Entity) {
        let (mut w, entity, _) = inspecting();
        w.register::<Light>();
        w.register::<CameraEnvironment>();
        w.scene().insert(component, entity);
        w.update();
        w.update();
        (w, entity)
    }

    pub(super) fn row(world: &mut World, path: &[impl Copy + Into<PathSegment>]) -> Option<Entity> {
        let path = PropertyPath::new(path.iter().copied());
        rows(world)
            .into_iter()
            .find(|(_, row)| row.path == path)
            .map(|(entity, _)| entity)
    }

    pub(super) fn children(world: &World, entity: Entity) -> Vec<Entity> {
        world
            .get_component_for_entity::<Children>(entity)
            .map(|children| children.iter().copied().collect())
            .unwrap_or_default()
    }

    pub(super) fn widget(world: &mut World, path: &[&'static str]) -> Entity {
        let row = row(world, path).expect("the row exists");
        children(world, row)[1]
    }

    pub(super) fn text(world: &World, entity: Entity) -> String {
        world
            .get_component_for_entity::<UIText>(entity)
            .unwrap()
            .text
            .clone()
    }

    fn button_text(world: &mut World) -> String {
        let button = widget(world, &["light_type"]);
        text(world, children(world, button)[0])
    }

    fn menu(world: &mut World, button: Entity) -> Option<Entity> {
        let mut query = world.query::<(Entity, &UIAnchoredPanel), ()>();
        query
            .iter(world)
            .find(|(_, panel)| panel.open && panel.owner == Some(button))
            .map(|(entity, _)| entity)
    }

    fn items(world: &mut World, button: Entity) -> Vec<(Entity, String)> {
        let menu = menu(world, button).expect("the menu is open");
        children(world, menu)
            .into_iter()
            .map(|item| (item, text(world, children(world, item)[0])))
            .collect()
    }

    fn pick(w: &mut Inspecting, name: &str) {
        let button = widget(&mut w.f.world, &["light_type"]);
        click(&mut w.f.world, button);
        let (item, _) = items(&mut w.f.world, button)
            .into_iter()
            .find(|(_, text)| text == name)
            .expect("the menu lists the variant");
        click(&mut w.f.world, item);
        assert!(menu(&mut w.f.world, button).is_none());
    }

    pub(super) fn queued(world: &World) -> usize {
        world.get_resource::<PropertyCommits>().unwrap().0.len()
    }

    fn checked(world: &mut World, path: &[&'static str]) -> bool {
        let checkbox = widget(world, path);
        world
            .get_component_for_entity::<UICheckbox>(checkbox)
            .unwrap()
            .checked
    }

    fn fog(w: &Inspecting, entity: Entity) -> Option<Fog> {
        w.get::<CameraEnvironment>(entity).unwrap().fog
    }

    #[test]
    fn the_variant_menu_lists_the_variants_and_the_button_names_the_active_one() {
        let (mut w, _) = fixture(Light::point_light());
        assert_eq!(button_text(&mut w.f.world), "Point");
        let button = widget(&mut w.f.world, &["light_type"]);

        click(&mut w.f.world, button);

        let open = menu(&mut w.f.world, button).expect("the menu is open");
        assert_overlay_fill(&w.f.world, open);
        let names: Vec<_> = items(&mut w.f.world, button)
            .into_iter()
            .map(|(_, name)| name)
            .collect();
        assert_eq!(names, ["Point", "Spot", "Directional"]);
    }

    #[test]
    fn the_variant_buttons_caret_is_a_node_so_it_is_laid_out_and_drawn() {
        let (mut w, _) = fixture(Light::point_light());
        let button = widget(&mut w.f.world, &["light_type"]);
        let caret = children(&w.f.world, button)[1];

        assert_eq!(
            text(&w.f.world, caret),
            crate::fonts::glyph::CARET_DOWN.to_string()
        );
        assert!(
            w.f.world
                .get_component_for_entity::<UINode>(caret)
                .is_some(),
            "a text without a node never reaches layout or the draw lists"
        );
    }

    #[test]
    fn picking_spot_switches_the_light_and_adds_its_cone_angle_row() {
        let (mut w, entity) = fixture(Light::point_light());

        pick(&mut w, "Spot");
        run(&mut w.f.world, send_property_commits);

        assert!(matches!(
            w.get::<Light>(entity).unwrap().light_type,
            LightType::Spot { cone_angle } if cone_angle == 0.0
        ));
        assert!(w.f.unsaved(w.document));
        w.update();
        w.update();
        assert!(row(&mut w.f.world, &["light_type", "cone_angle"]).is_some());
        assert_eq!(button_text(&mut w.f.world), "Spot");
    }

    #[test]
    fn a_switch_that_keeps_the_rows_keeps_the_button_and_renames_it() {
        let (mut w, entity) = fixture(Light::point_light());
        let button = widget(&mut w.f.world, &["light_type"]);

        pick(&mut w, "Directional");
        run(&mut w.f.world, send_property_commits);
        w.update();
        w.update();
        run(&mut w.f.world, variants::refresh_variant_labels);

        assert!(matches!(
            w.get::<Light>(entity).unwrap().light_type,
            LightType::Directional
        ));
        assert_eq!(widget(&mut w.f.world, &["light_type"]), button);
        assert_eq!(button_text(&mut w.f.world), "Directional");
    }

    #[test]
    fn closing_the_tab_closes_the_variant_menu() {
        let (mut w, _) = fixture(Light::point_light());
        let button = widget(&mut w.f.world, &["light_type"]);
        click(&mut w.f.world, button);
        let menu = menu(&mut w.f.world, button).unwrap();

        w.close();

        assert!(!is_open(&w.f.world, menu));
    }

    #[test]
    fn picking_the_active_variant_queues_nothing_and_leaves_the_scene_saved() {
        let (mut w, entity) = fixture(Light::spot_light(0.5));

        pick(&mut w, "Spot");

        assert_eq!(queued(&w.f.world), 0);
        run(&mut w.f.world, send_property_commits);
        assert!(matches!(
            w.get::<Light>(entity).unwrap().light_type,
            LightType::Spot { cone_angle } if cone_angle == 0.5
        ));
        assert!(!w.f.unsaved(w.document));
    }

    #[test]
    fn toggling_fog_on_adds_a_default_fog_and_its_rows_and_off_removes_them() {
        let (mut w, entity) = fixture(CameraEnvironment::default());
        assert!(!checked(&mut w.f.world, &["fog"]));
        assert!(row(&mut w.f.world, &["fog", "0", "density"]).is_none());

        let checkbox = widget(&mut w.f.world, &["fog"]);
        click(&mut w.f.world, checkbox);
        run(&mut w.f.world, send_property_commits);
        assert_eq!(fog(&w, entity), Some(Fog::default()));
        assert!(w.f.unsaved(w.document));
        w.update();
        w.update();
        assert!(checked(&mut w.f.world, &["fog"]));
        assert!(row(&mut w.f.world, &["fog", "0", "density"]).is_some());

        let checkbox = widget(&mut w.f.world, &["fog"]);
        click(&mut w.f.world, checkbox);
        run(&mut w.f.world, send_property_commits);
        assert_eq!(fog(&w, entity), None);
        w.update();
        w.update();
        assert!(!checked(&mut w.f.world, &["fog"]));
        assert!(row(&mut w.f.world, &["fog", "0", "density"]).is_none());
    }

    #[test]
    fn a_refused_toggle_commit_leaves_the_checkbox_showing_the_value() {
        let (mut w, entity) = fixture(CameraEnvironment::default());
        let checkbox = widget(&mut w.f.world, &["fog"]);
        w.scene()
            .remove_resource::<concerto_foundation::assets::registry::AssetRegistry>();

        click(&mut w.f.world, checkbox);
        assert!(checked(&mut w.f.world, &["fog"]));
        run(&mut w.f.world, send_property_commits);
        w.update();
        w.update();
        run(&mut w.f.world, variants::refresh_option_fields);

        assert_eq!(fog(&w, entity), None);
        assert_eq!(widget(&mut w.f.world, &["fog"]), checkbox);
        assert!(!checked(&mut w.f.world, &["fog"]));
    }
}

mod list_rows {
    use super::color_rows::editor_of;
    use super::variant_rows::{children, row, text};
    use super::*;
    use crate::tools::edits::PropertyCommitted;
    use concerto_ecs::events::event_channel::EventChannel;
    use concerto_foundation::assets::{asset_server::AssetServer, registry::AssetRegistry};
    use list::{ListEdit, ListEditor};
    use numeric::NumericFields;

    const VALUES: PathSegment = PathSegment::Field("values");

    #[derive(Component, concerto_reflect::Reflect, Default)]
    #[reflect(Component, Default)]
    struct Samples {
        values: Vec<f32>,
    }

    #[derive(concerto_reflect::Reflect, Default, Clone, Debug, PartialEq)]
    struct Stop {
        offset: f32,
        on: bool,
    }

    #[derive(Component, concerto_reflect::Reflect, Default)]
    #[reflect(Component, Default)]
    struct Stops {
        entries: Vec<Stop>,
    }

    #[derive(Component, concerto_reflect::Reflect, Default)]
    #[reflect(Component, Default)]
    struct Titles {
        names: Vec<String>,
    }

    #[derive(Component, concerto_reflect::Reflect, Default)]
    #[reflect(Component, Default)]
    struct Grid {
        cells: Vec<Vec<f32>>,
    }

    #[derive(Component, concerto_reflect::Reflect, Default)]
    #[reflect(Component, Default)]
    struct Curve {
        points: Option<Vec<f32>>,
    }

    fn inspected<C: Component + GetTypeRegistration>(w: &mut Inspecting, component: C) -> Entity {
        w.register::<C>();
        let entity = w.spawn_in_scene(component);
        w.select(Some(entity));
        w.update();
        w.update();
        entity
    }

    fn fixture<C: Component + GetTypeRegistration>(component: C) -> (Inspecting, Entity) {
        let (mut w, _, _) = inspecting();
        let entity = inspected(&mut w, component);
        (w, entity)
    }

    fn body(world: &mut World) -> Vec<Entity> {
        let first = rows(world)[0].0;
        let body = world
            .get_component_for_entity::<ChildOf>(first)
            .unwrap()
            .parent();
        children(world, body)
    }

    fn labels(world: &mut World) -> Vec<String> {
        body(world)
            .into_iter()
            .map(|row| text(world, children(world, row)[0]))
            .collect()
    }

    fn last_child(world: &mut World, path: &[impl Copy + Into<PathSegment>]) -> Entity {
        let row = row(world, path).expect("the row exists");
        *children(world, row).last().expect("the row has widgets")
    }

    fn press(w: &mut Inspecting, button: Entity) {
        click(&mut w.f.world, button);
        run(&mut w.f.world, send_property_commits);
        w.update();
        w.update();
    }

    fn committed(w: &Inspecting) -> usize {
        w.f.scene_world(w.document)
            .get_resource::<EventChannel<PropertyCommitted>>()
            .unwrap()
            .len()
    }

    fn values(w: &Inspecting, entity: Entity) -> Vec<f32> {
        w.get::<Samples>(entity).unwrap().values.clone()
    }

    #[test]
    fn a_list_shows_its_length_and_a_row_per_element_labelled_by_index() {
        let (mut w, _) = fixture(Samples {
            values: vec![1.0, 2.0, 3.0],
        });
        let world = &mut w.f.world;
        assert_eq!(labels(world), ["Values", "0", "1", "2"]);
        assert_eq!(
            editor_of(world, &["values"]),
            Some(TypeId::of::<ListEditor>())
        );
        let header = row(world, &["values"]).unwrap();
        assert_eq!(text(world, children(world, header)[1]), "Length 3");
        assert_eq!(
            editor_of(world, &[VALUES, PathSegment::Index(2)]),
            Some(TypeId::of::<NumericFields>())
        );
    }

    #[test]
    fn an_empty_list_shows_only_its_header() {
        let (mut w, _) = fixture(Samples::default());
        let world = &mut w.f.world;
        assert_eq!(labels(world), ["Values"]);
        let header = row(world, &["values"]).unwrap();
        assert_eq!(text(world, children(world, header)[1]), "Length 0");
    }

    #[test]
    fn add_appends_a_default_and_rebuilds_and_remove_takes_that_element_out() {
        let (mut w, entity) = fixture(Samples {
            values: vec![1.0, 2.0, 3.0],
        });

        let add = last_child(&mut w.f.world, &["values"]);
        press(&mut w, add);
        assert_eq!(values(&w, entity), [1.0, 2.0, 3.0, 0.0]);
        assert_eq!(committed(&w), 1);
        assert!(w.f.unsaved(w.document));
        assert_eq!(labels(&mut w.f.world), ["Values", "0", "1", "2", "3"]);

        let remove = last_child(&mut w.f.world, &[VALUES, PathSegment::Index(1)]);
        press(&mut w, remove);
        assert_eq!(values(&w, entity), [1.0, 3.0, 0.0]);
        assert_eq!(committed(&w), 2);
        assert_eq!(labels(&mut w.f.world), ["Values", "0", "1", "2"]);
    }

    #[test]
    fn a_right_click_on_a_list_button_changes_nothing() {
        let (mut w, entity) = fixture(Samples { values: vec![1.0] });
        let add = last_child(&mut w.f.world, &["values"]);
        w.f.world.trigger_on(
            add,
            concerto_ui::interaction::UIClick {
                position: Vec2::ZERO,
                button: concerto_window::input::MouseButton::Right,
            },
        );
        assert!(
            w.f.world
                .get_resource::<PropertyCommits>()
                .unwrap()
                .0
                .is_empty()
        );
        assert_eq!(values(&w, entity), [1.0]);
    }

    #[test]
    fn a_remove_past_the_end_is_refused_and_keeps_the_list() {
        let mut list = vec![1.0_f32];
        let context = EditContext {
            assets: &AssetServer::new(),
            registry: &AssetRegistry::default(),
        };
        assert_eq!(
            ListEditor.apply(&mut list, &ListEdit::Remove(1), &context),
            Err(EditError::NotFound)
        );
        assert_eq!(list, [1.0]);
    }

    #[test]
    fn a_list_offers_no_value_edit() {
        assert!(matches!(ListEditor.edit_to(&vec![1.0_f32]), Ok(None)));
    }

    #[test]
    fn a_list_of_structs_shows_a_heading_with_a_remove_button_and_field_rows() {
        let first = Stop {
            offset: 1.0,
            on: true,
        };
        let second = Stop {
            offset: 2.0,
            on: false,
        };
        let (mut w, entity) = fixture(Stops {
            entries: vec![first.clone(), second.clone()],
        });
        const ENTRIES: PathSegment = PathSegment::Field("entries");
        const ON: PathSegment = PathSegment::Field("on");
        assert_eq!(
            labels(&mut w.f.world),
            ["Entries", "0", "Offset", "On", "1", "Offset", "On"]
        );

        let world = &mut w.f.world;
        assert_eq!(editor_of(world, &[ENTRIES, PathSegment::Index(1)]), None);
        assert_eq!(
            editor_of(world, &[ENTRIES, PathSegment::Index(1), ON]),
            Some(TypeId::of::<boolean::BoolCheckbox>())
        );
        let mut texts = world.query::<&UIText, ()>();
        assert!(
            texts
                .iter(world)
                .all(|text| text.text != "Unsupported type")
        );

        let remove = last_child(world, &[ENTRIES, PathSegment::Index(0)]);
        press(&mut w, remove);
        assert_eq!(w.get::<Stops>(entity).unwrap().entries, [second]);
        assert_eq!(labels(&mut w.f.world), ["Entries", "0", "Offset", "On"]);
    }

    #[test]
    fn an_element_with_no_editor_and_no_fields_is_unsupported_and_can_be_removed() {
        let (mut w, entity) = fixture(Titles {
            names: vec!["first".into(), "second".into()],
        });
        const NAMES: PathSegment = PathSegment::Field("names");
        let world = &mut w.f.world;
        assert_eq!(labels(world), ["Names", "0", "1"]);
        let first = row(world, &[NAMES, PathSegment::Index(0)]).unwrap();
        assert_eq!(text(world, children(world, first)[1]), "Unsupported type");

        let remove = last_child(world, &[NAMES, PathSegment::Index(0)]);
        press(&mut w, remove);
        assert_eq!(w.get::<Titles>(entity).unwrap().names, ["second"]);
    }

    #[test]
    fn a_list_in_a_list_edits_each_level() {
        let (mut w, entity) = fixture(Grid {
            cells: vec![vec![1.0, 2.0], vec![]],
        });
        const CELLS: PathSegment = PathSegment::Field("cells");
        let inner = |index| [CELLS, PathSegment::Index(index)];
        let cells = |w: &Inspecting| w.get::<Grid>(entity).unwrap().cells.clone();
        assert_eq!(labels(&mut w.f.world), ["Cells", "0", "0", "1", "1"]);

        let remove = last_child(
            &mut w.f.world,
            &[CELLS, PathSegment::Index(0), PathSegment::Index(0)],
        );
        press(&mut w, remove);
        assert_eq!(cells(&w), [vec![2.0], vec![]]);

        let header = row(&mut w.f.world, &inner(1)).unwrap();
        let children = children(&w.f.world, header);
        press(&mut w, children[children.len() - 2]);
        assert_eq!(cells(&w), [vec![2.0], vec![0.0]]);

        let remove = last_child(&mut w.f.world, &inner(0));
        press(&mut w, remove);
        assert_eq!(cells(&w), [vec![0.0]]);
    }

    #[test]
    fn a_list_in_an_option_is_added_to_and_removed_from() {
        let (mut w, entity) = fixture(Curve {
            points: Some(vec![1.0]),
        });
        const POINTS: PathSegment = PathSegment::Field("points");
        const SOME: PathSegment = PathSegment::Field("0");
        let points = |w: &Inspecting| w.get::<Curve>(entity).unwrap().points.clone();
        assert_eq!(labels(&mut w.f.world), ["Points", "Points", "0"]);

        let add = last_child(&mut w.f.world, &[POINTS, SOME]);
        press(&mut w, add);
        assert_eq!(points(&w), Some(vec![1.0, 0.0]));

        let remove = last_child(&mut w.f.world, &[POINTS, SOME, PathSegment::Index(0)]);
        press(&mut w, remove);
        assert_eq!(points(&w), Some(vec![0.0]));
    }

    #[test]
    fn a_list_the_game_rewrites_keeps_rows_for_new_values_and_rebuilds_for_a_new_length() {
        let (mut w, entity) = fixture(Samples {
            values: vec![1.0, 2.0],
        });
        let element = [VALUES, PathSegment::Index(1)];
        let before = row(&mut w.f.world, &element).unwrap();
        let shown = |w: &Inspecting, row| {
            w.f.world
                .get_component_for_entity::<PropertyRowValue>(row)
                .unwrap()
                .clone()
        };
        let old = shown(&w, before);

        w.scene()
            .get_component_for_entity_mut::<Samples>(entity)
            .unwrap()
            .values[1] = 5.0;
        w.update();
        w.update();
        assert_eq!(row(&mut w.f.world, &element), Some(before));
        assert!(shown(&w, before) != old);

        w.scene()
            .get_component_for_entity_mut::<Samples>(entity)
            .unwrap()
            .values
            .push(9.0);
        w.update();
        w.update();
        assert_ne!(row(&mut w.f.world, &element), Some(before));
        assert_eq!(labels(&mut w.f.world), ["Values", "0", "1", "2"]);
    }

    #[derive(Default)]
    struct WholeList;

    impl PropertyEditor<Vec<f32>> for WholeList {
        type Snapshot = ();
        type Edit = ();
        fn snapshot(&self, _: &Vec<f32>) {}
        fn build(&self, _: &mut EntityCommandQueue, _: &(), _: &UITheme) {}
        fn apply(&self, _: &mut Vec<f32>, _: &(), _: &EditContext) -> Result<(), EditError> {
            Ok(())
        }
    }

    #[test]
    fn an_editor_registered_for_the_exact_list_type_wins_and_hides_the_elements() {
        let (mut w, _, _) = inspecting();
        registry_mut(&mut w.f.world).register_property_editor::<Vec<f32>, WholeList>();
        inspected(
            &mut w,
            Samples {
                values: vec![1.0, 2.0],
            },
        );
        let world = &mut w.f.world;
        assert_eq!(labels(world), ["Values"]);
        assert_eq!(
            editor_of(world, &["values"]),
            Some(TypeId::of::<WholeList>())
        );
    }
}

mod color_rows {
    use super::variant_rows::{children, queued, row, text, widget};
    use super::*;
    use concerto_color::{Color, LinearRgba};
    use concerto_ecs::events::event_channel::EventChannel;
    use concerto_render::components::{CameraEnvironment, Light, camera_environment::Fog};
    use concerto_ui::{
        checkbox::UICheckbox,
        focus::UIFocusLost,
        material::UIMaterial,
        node::FlexWrap,
        text_input::{UITextInput, UITextInputSubmitted},
    };

    #[derive(Debug, PartialEq)]
    struct ColorRow {
        row: Entity,
        swatch: Entity,
        letters: Vec<Entity>,
        fields: Vec<Entity>,
    }

    fn fixture(component: impl Component) -> (Inspecting, Entity) {
        let (mut w, entity, _) = inspecting();
        w.register::<Light>();
        w.register::<CameraEnvironment>();
        w.f.world
            .insert_resource(EventChannel::<UITextInputSubmitted>::default());
        w.f.world
            .insert_resource(EventChannel::<UIFocusLost>::default());
        w.scene().insert(component, entity);
        refreshed(&mut w);
        (w, entity)
    }

    fn light(color: Color) -> Light {
        Light {
            color,
            ..Light::point_light()
        }
    }

    fn refreshed(w: &mut Inspecting) {
        w.update();
        w.update();
        run(&mut w.f.world, numeric::refresh_numeric_fields);
        run(&mut w.f.world, numeric::refresh_numeric_labels);
        run(&mut w.f.world, color::refresh_color_swatches);
    }

    fn collected(w: &mut Inspecting) {
        for _ in 0..2 {
            for system in &mut w.systems[0..2] {
                system.run_and_apply((), &mut w.f.world);
            }
            w.tick();
        }
    }

    #[test]
    fn a_color_row_has_its_wrapping_node_before_its_widgets_are_built() {
        let (mut w, entity, _) = inspecting();
        w.register::<Light>();
        w.scene().insert(Light::point_light(), entity);
        collected(&mut w);
        let theme = UITheme::default();
        let row = row(&mut w.f.world, &["color"]).expect("the row exists");

        assert_eq!(
            node(&w.f.world, row),
            &standard_row_node(&theme).with_flex_wrap(FlexWrap::Wrap)
        );
    }

    fn color_row(world: &mut World, path: &[&'static str]) -> ColorRow {
        let row = row(world, path).expect("the row exists");
        let mut swatches = world.query::<(Entity, &color::ColorSwatch), ()>();
        let swatches: Vec<Entity> = swatches
            .iter(world)
            .filter(|(_, swatch)| swatch.row == row)
            .map(|(entity, _)| entity)
            .collect();
        assert_eq!(swatches.len(), 1, "a color row has one swatch");
        let mut labels = world.query::<(Entity, &numeric::NumericSlotLabel), ()>();
        let mut letters: Vec<(usize, Entity)> = labels
            .iter(world)
            .filter(|(_, label)| label.row == row)
            .map(|(entity, label)| (label.slot, entity))
            .collect();
        letters.sort_by_key(|(slot, _)| *slot);
        let mut slots = world.query::<(Entity, &numeric::NumericSlot), ()>();
        let mut fields: Vec<(usize, Entity)> = slots
            .iter(world)
            .filter(|(_, slot)| slot.row == row)
            .map(|(entity, slot)| (slot.slot, entity))
            .collect();
        fields.sort_by_key(|(slot, _)| *slot);
        ColorRow {
            row,
            swatch: swatches[0],
            letters: letters.into_iter().map(|(_, entity)| entity).collect(),
            fields: fields.into_iter().map(|(_, entity)| entity).collect(),
        }
    }

    fn parent(world: &World, entity: Entity) -> Entity {
        world
            .get_component_for_entity::<ChildOf>(entity)
            .unwrap()
            .parent()
    }

    fn node(world: &World, entity: Entity) -> &UINode {
        world.get_component_for_entity::<UINode>(entity).unwrap()
    }

    fn letters(world: &World, row: &ColorRow) -> Vec<String> {
        row.letters
            .iter()
            .map(|letter| text(world, *letter))
            .collect()
    }

    fn values(world: &World, row: &ColorRow) -> Vec<String> {
        row.fields
            .iter()
            .map(|field| {
                world
                    .get_component_for_entity::<UITextInput>(*field)
                    .unwrap()
                    .value
                    .clone()
            })
            .collect()
    }

    fn swatch(world: &World, row: &ColorRow) -> LinearRgba {
        world
            .get_component_for_entity::<UIMaterial>(row.swatch)
            .unwrap()
            .color
    }

    fn submit(w: &mut Inspecting, path: &[&'static str], slot: usize, text: &str) {
        let field = color_row(&mut w.f.world, path).fields[slot];
        let world = &mut w.f.world;
        world
            .get_component_for_entity_mut::<UITextInput>(field)
            .unwrap()
            .value = text.into();
        world
            .get_resource_mut::<EventChannel<UITextInputSubmitted>>()
            .unwrap()
            .push_event(UITextInputSubmitted {
                entity: field,
                value: text.into(),
            });
        run(world, numeric::commit_numeric_fields);
    }

    pub(super) fn editor_of(
        world: &mut World,
        path: &[impl Copy + Into<PathSegment>],
    ) -> Option<TypeId> {
        let path = PropertyPath::new(path.iter().copied());
        rows(world)
            .into_iter()
            .find(|(_, row)| row.path == path)
            .and_then(|(_, row)| row.editor_type)
    }

    fn light_color(w: &Inspecting, entity: Entity) -> Color {
        w.get::<Light>(entity).unwrap().color
    }

    fn set_light_color(w: &mut Inspecting, entity: Entity, color: Color) {
        w.scene()
            .get_component_for_entity_mut::<Light>(entity)
            .unwrap()
            .color = color;
    }

    #[test]
    fn a_color_property_is_a_swatch_and_four_labelled_fields() {
        let (mut w, _) = fixture(Light::point_light());
        let world = &mut w.f.world;

        let color = color_row(world, &["color"]);

        assert_eq!(
            editor_of(world, &["color"]),
            Some(TypeId::of::<numeric::NumericFields>())
        );
        assert!(
            world
                .get_component_for_entity::<color::ColorSwatch>(color.swatch)
                .is_some()
        );
        assert_eq!(color.fields.len(), 4);
        assert!(color.fields.iter().all(|field| {
            world
                .get_component_for_entity::<numeric::NumericSlot>(*field)
                .is_some()
        }));
        assert!(color.letters.iter().all(|letter| {
            world
                .get_component_for_entity::<numeric::NumericSlotLabel>(*letter)
                .is_some()
        }));
        assert_eq!(letters(world, &color), ["R", "G", "B", "A"]);
        assert_eq!(values(world, &color), ["1.000"; 4]);
        let mut texts = world.query::<&UIText, ()>();
        assert!(
            texts
                .iter(world)
                .all(|text| text.text != "Unsupported type")
        );
    }

    #[test]
    fn the_swatch_shares_the_label_line_and_the_fields_take_a_line_below() {
        let (mut w, _) = fixture(Light::point_light());
        let spacing = UITheme::default().spacing_xs;
        let world = &mut w.f.world;
        let color = color_row(world, &["color"]);

        let parts = children(world, color.row);
        assert_eq!(parts.len(), 3, "label, swatch, then the line of fields");
        assert_eq!(text(world, parts[0]), "Color");
        assert_eq!(parts[1], color.swatch);
        let line = parts[2];
        assert_eq!(node(world, color.row).flex_wrap, FlexWrap::Wrap);
        assert_eq!(node(world, line).width, UIValue::Percent(100.0));
        assert_eq!(color.fields.len(), 4);
        for (letter, field) in color.letters.iter().zip(&color.fields) {
            let stack = parent(world, *field);
            assert_eq!(parent(world, *letter), stack);
            assert_eq!(parent(world, stack), line);
            assert_eq!(node(world, stack).gap.x, spacing);
            for shrinking in [stack, *field] {
                assert_eq!(node(world, shrinking).min_width, UIValue::Px(0.0));
                assert!(node(world, shrinking).flex_shrink > 0.0);
            }
        }
        let translation = row(world, &["translation"]).expect("the transform row exists");
        assert_eq!(node(world, translation).flex_wrap, FlexWrap::NoWrap);
    }

    #[test]
    fn an_srgb_color_is_edited_in_srgb_and_marks_the_scene_unsaved() {
        let (mut w, entity) = fixture(light(Color::srgba(0.5, 0.25, 0.75, 1.0)));

        submit(&mut w, &["color"], 0, "0.8");
        run(&mut w.f.world, send_property_commits);

        assert_eq!(light_color(&w, entity), Color::srgba(0.8, 0.25, 0.75, 1.0));
        assert!(w.f.unsaved(w.document));
    }

    #[test]
    fn an_hsl_color_shows_hue_in_degrees_and_wraps_it() {
        let (mut w, entity) = fixture(light(Color::hsl(0.25, 0.5, 0.5, 1.0)));
        let color = color_row(&mut w.f.world, &["color"]);
        assert_eq!(letters(&w.f.world, &color), ["H", "S", "L", "A"]);
        assert_eq!(values(&w.f.world, &color)[0], "90.000");

        submit(&mut w, &["color"], 0, "450");
        run(&mut w.f.world, send_property_commits);

        let Color::Hsla(hsl) = light_color(&w, entity) else {
            panic!("the color stays in HSL");
        };
        assert_eq!(hsl.h, 0.25);
    }

    #[test]
    fn out_of_range_text_queues_nothing_and_the_field_shows_the_value_again() {
        let (mut w, entity) = fixture(light(Color::srgba(0.5, 0.25, 0.75, 1.0)));

        submit(&mut w, &["color"], 0, "1.5");

        assert_eq!(queued(&w.f.world), 0);
        let color = color_row(&mut w.f.world, &["color"]);
        assert_eq!(values(&w.f.world, &color)[0], "0.500");
        run(&mut w.f.world, send_property_commits);
        assert_eq!(light_color(&w, entity), Color::srgba(0.5, 0.25, 0.75, 1.0));
        assert!(!w.f.unsaved(w.document));
    }

    #[test]
    fn an_edit_queued_before_the_space_changed_is_refused() {
        let (mut w, entity) = fixture(light(Color::srgba(0.5, 0.5, 0.5, 1.0)));
        let before = color_row(&mut w.f.world, &["color"]);

        submit(&mut w, &["color"], 0, "0.8");
        assert_eq!(queued(&w.f.world), 1);
        let hsl = Color::hsl(0.25, 0.5, 0.5, 1.0);
        set_light_color(&mut w, entity, hsl);
        run(&mut w.f.world, send_property_commits);

        assert_eq!(light_color(&w, entity), hsl);
        assert!(!w.f.unsaved(w.document));
        refreshed(&mut w);
        let after = color_row(&mut w.f.world, &["color"]);
        assert_eq!(after, before);
        assert_eq!(letters(&w.f.world, &after), ["H", "S", "L", "A"]);
    }

    #[test]
    fn a_color_changed_elsewhere_reaches_the_same_fields_and_swatch() {
        let (mut w, entity) = fixture(Light::point_light());
        let before = color_row(&mut w.f.world, &["color"]);

        set_light_color(&mut w, entity, Color::rgba(0.2, 0.4, 0.6, 1.0));
        refreshed(&mut w);

        let after = color_row(&mut w.f.world, &["color"]);
        assert_eq!(after, before);
        assert_eq!(
            values(&w.f.world, &after),
            ["0.200", "0.400", "0.600", "1.000"]
        );
        assert_eq!(
            swatch(&w.f.world, &after),
            LinearRgba::new(0.2, 0.4, 0.6, 1.0)
        );

        set_light_color(&mut w, entity, Color::hsl(0.25, 0.5, 0.5, 1.0));
        refreshed(&mut w);

        let relabelled = color_row(&mut w.f.world, &["color"]);
        assert_eq!(relabelled, before);
        assert_eq!(letters(&w.f.world, &relabelled), ["H", "S", "L", "A"]);
    }

    #[test]
    fn a_transparent_color_has_an_opaque_swatch() {
        let (mut w, _) = fixture(light(Color::rgba(1.0, 0.0, 0.0, 0.25)));
        let color = color_row(&mut w.f.world, &["color"]);

        assert_eq!(
            swatch(&w.f.world, &color),
            LinearRgba::new(1.0, 0.0, 0.0, 1.0)
        );
        assert_eq!(values(&w.f.world, &color)[3], "0.250");
    }

    #[test]
    fn ambient_is_a_color_row() {
        let (mut w, entity) = fixture(CameraEnvironment::default());
        assert_eq!(
            only::<UICheckbox>(&mut w.f.world),
            widget(&mut w.f.world, &["fog"])
        );
        assert!(row(&mut w.f.world, &["ambient", "0"]).is_none());

        assert_eq!(
            editor_of(&mut w.f.world, &["ambient"]),
            Some(TypeId::of::<numeric::NumericFields>())
        );
        let color = color_row(&mut w.f.world, &["ambient"]);
        let label = children(&w.f.world, color.row)[0];
        assert_eq!(text(&w.f.world, label), "Ambient");
        assert_eq!(letters(&w.f.world, &color), ["R", "G", "B", "A"]);
        assert_eq!(
            values(&w.f.world, &color),
            ["0.030", "0.030", "0.030", "1.000"]
        );

        submit(&mut w, &["ambient"], 0, "0.1");
        run(&mut w.f.world, send_property_commits);

        assert_eq!(
            w.get::<CameraEnvironment>(entity).unwrap().ambient,
            Color::rgba(0.1, 0.03, 0.03, 1.0)
        );
    }

    #[test]
    fn fog_color_is_edited_under_fog() {
        let (mut w, entity) = fixture(CameraEnvironment {
            fog: Some(Fog {
                color: Color::srgba(0.5, 0.5, 0.5, 1.0),
                ..Default::default()
            }),
            ..Default::default()
        });

        submit(&mut w, &["fog", "0", "color"], 2, "0.25");
        run(&mut w.f.world, send_property_commits);

        assert_eq!(
            w.get::<CameraEnvironment>(entity)
                .unwrap()
                .fog
                .unwrap()
                .color,
            Color::srgba(0.5, 0.5, 0.25, 1.0)
        );
    }
}

#[test]
fn a_non_color_numeric_row_keeps_the_standard_node() {
    let (mut w, _, _) = inspecting();
    w.update();
    w.update();
    let theme = UITheme::default();
    let (translation, _) = rows(&mut w.f.world)
        .into_iter()
        .find(|(_, row)| row.path.name() == "translation")
        .unwrap();

    assert_eq!(
        w.f.world
            .get_component_for_entity::<UINode>(translation)
            .unwrap(),
        &standard_row_node(&theme)
    );
}

#[test]
fn a_row_is_spawned_with_the_node_its_editor_chooses() {
    #[derive(Default)]
    struct Spaced;
    impl PropertyEditor<Transform> for Spaced {
        type Snapshot = Vec3;
        type Edit = ();
        fn snapshot(&self, value: &Transform) -> Vec3 {
            value.translation
        }
        fn row_node(&self, _: &Vec3, theme: &UITheme) -> UINode {
            standard_row_node(theme).with_gap(Vec2::new(31.0, 0.0))
        }
        fn build(&self, _: &mut EntityCommandQueue, _: &Vec3, _: &UITheme) {}
        fn apply(&self, _: &mut Transform, _: &(), _: &EditContext) -> Result<(), EditError> {
            Ok(())
        }
    }
    let (mut w, _, _) = inspecting();
    registry_mut(&mut w.f.world).register_property_editor::<Transform, Spaced>();
    w.update();
    w.update();
    let theme = UITheme::default();
    let row = rows(&mut w.f.world)[0].0;

    assert_eq!(
        w.f.world.get_component_for_entity::<UINode>(row).unwrap(),
        &standard_row_node(&theme).with_gap(Vec2::new(31.0, 0.0))
    );
}

#[test]
fn a_row_whose_editor_does_not_follow_its_snapshot_is_respawned_with_the_new_node() {
    #[derive(Default)]
    struct Growing;
    impl PropertyEditor<Transform> for Growing {
        type Snapshot = Vec3;
        type Edit = ();
        fn snapshot(&self, value: &Transform) -> Vec3 {
            value.translation
        }
        fn follows_snapshot(&self) -> bool {
            false
        }
        fn row_node(&self, snapshot: &Vec3, theme: &UITheme) -> UINode {
            standard_row_node(theme).with_gap(Vec2::new(snapshot.x, 0.0))
        }
        fn build(&self, _: &mut EntityCommandQueue, _: &Vec3, _: &UITheme) {}
        fn apply(&self, _: &mut Transform, _: &(), _: &EditContext) -> Result<(), EditError> {
            Ok(())
        }
    }
    let (mut w, entity, _) = inspecting();
    registry_mut(&mut w.f.world).register_property_editor::<Transform, Growing>();
    w.update();
    w.update();
    let theme = UITheme::default();
    let before = rows(&mut w.f.world)[0].0;
    assert_eq!(
        w.f.world
            .get_component_for_entity::<UINode>(before)
            .unwrap(),
        &standard_row_node(&theme).with_gap(Vec2::new(0.0, 0.0))
    );

    w.scene()
        .get_component_for_entity_mut::<Transform>(entity)
        .unwrap()
        .translation
        .x = 12.0;
    w.update();
    w.update();
    let after = rows(&mut w.f.world)[0].0;

    assert!(!w.f.world.entity_is_valid(before));
    assert_eq!(
        w.f.world.get_component_for_entity::<UINode>(after).unwrap(),
        &standard_row_node(&theme).with_gap(Vec2::new(12.0, 0.0))
    );
}

#[test]
fn the_component_stack_leaves_room_for_the_scrollbar() {
    fn build(mut cmd: CommandQueue, theme: Res<UITheme>) {
        let parent = cmd.spawn(UINode::default()).entity();
        spawn_panel(&mut cmd, parent, &theme);
    }
    let mut world = World::new();
    world.register_component::<ChildOf>();
    world.register_component::<Children>();
    world.insert_resource(UITheme::default());
    let mut panel = build.into_system();
    panel.initialize(&mut world);
    panel.run_and_apply((), &mut world);

    let padding = world
        .query::<&UINode, With<ComponentStack>>()
        .iter(&mut world)
        .next()
        .expect("the panel spawns a component stack")
        .padding;
    assert!(
        padding.right >= SCROLLBAR_WIDTH,
        "the scrollbar is drawn over the last {SCROLLBAR_WIDTH} px of the stack, which keeps only {} px clear",
        padding.right
    );
    let name_margin = world
        .query::<&UINode, With<NameField>>()
        .iter(&mut world)
        .next()
        .expect("the panel spawns a name field")
        .margin;
    assert_eq!(
        name_margin.right, padding.right,
        "the name field ends where the component cards end"
    );
}

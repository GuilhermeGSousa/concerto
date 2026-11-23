use super::*;
use crate::{hosted::testing::SceneFixture, tools::selection::Select};
use concerto_color::Color;
use concerto_ecs::{
    IntoSystem, System, World,
    component::bundle::IntoBundle,
    entity::hierarchy::{ChildOf, Children},
    events::event_channel::EventChannel,
    reflect::ReflectComponent,
    system::BoxedSystem,
};
use concerto_foundation::transform::Transform;
use concerto_reflect::{GetTypeRegistration, ReflectDefault};
use concerto_ui::{
    focus::{FocusedWidget, UIFocusLost},
    node::UIBox,
    text_input::{UITextInput, UITextInputCancelled, UITextInputSubmitted},
};
use glam::Vec2;

struct Inspecting {
    f: SceneFixture,
    document: Entity,
    systems: [BoxedSystem; 4],
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
        self.scene()
            .trigger(Select::Replace(entity.into_iter().collect()));
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
    fn get<T: Component>(&self, entity: Entity) -> Option<&T> {
        self.f
            .scene_world(self.document)
            .get_component_for_entity::<T>(entity)
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
        sync::mark_mixed_rows.into_boxed_system(),
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

fn run<M>(world: &mut World, system: impl IntoSystem<(), M>) {
    let mut system = system.into_system();
    system.initialize(world);
    system.run_and_apply((), world);
}

fn row(world: &mut World, path: &[&'static str]) -> Entity {
    let path = PropertyPath::new(path.iter().copied());
    let mut rows = world.query::<(Entity, &PropertyRow), ()>();
    rows.iter(world)
        .find(|(_, row)| row.address.path == path)
        .map(|(entity, _)| entity)
        .expect("the row exists")
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

#[derive(Component, concerto_reflect::Reflect)]
#[reflect(Component)]
struct Spawner {
    title: String,
    label: concerto_name::Name,
    lives: u32,
    anchor: Vec2,
    tint: Color,
}

fn spawner() -> Spawner {
    Spawner {
        title: "Gate".into(),
        label: concerto_name::Name::new("idle"),
        lives: 3,
        anchor: Vec2::new(1.0, 2.0),
        tint: Color::srgba(0.5, 0.25, 0.75, 1.0),
    }
}

fn slot(world: &mut World, path: &[&'static str], index: usize) -> Entity {
    let row = row(world, path);
    let mut slots = world.query::<(Entity, &numeric::NumericSlot), ()>();
    slots
        .iter(world)
        .find(|(_, slot)| slot.row == row && slot.slot == index)
        .map(|(entity, _)| entity)
        .expect("the row has the slot")
}

fn widget(world: &mut World, path: &[&'static str]) -> Entity {
    let row = row(world, path);
    world
        .get_component_for_entity::<Children>(row)
        .unwrap()
        .iter()
        .copied()
        .nth(1)
        .expect("the row has a widget after its label")
}

fn type_text(world: &mut World, field: Entity, text: &str) {
    world
        .get_component_for_entity_mut::<UITextInput>(field)
        .unwrap()
        .value = text.into();
    **world.get_resource_mut::<FocusedWidget>().unwrap() = Some(field);
}

fn submit(world: &mut World, field: Entity, text: &str) {
    type_text(world, field, text);
    world
        .get_resource_mut::<EventChannel<UITextInputSubmitted>>()
        .unwrap()
        .push_event(UITextInputSubmitted {
            entity: field,
            value: text.into(),
        });
}

#[test]
fn typed_values_are_committed_to_the_entity_their_rows_were_built_for() {
    let (mut w, _, _) = inspecting();
    w.register::<Spawner>();
    let world = &mut w.f.world;
    world.insert_resource(FocusedWidget::default());
    world.insert_resource(EventChannel::<UITextInputSubmitted>::default());
    world.insert_resource(EventChannel::<UITextInputCancelled>::default());
    world.insert_resource(EventChannel::<UIFocusLost>::default());
    let edited = w.spawn_in_scene(spawner());
    let other = w.spawn_in_scene(spawner());
    w.select(Some(edited));
    w.update();
    w.update();
    let world = &mut w.f.world;
    run(world, text_field::refresh_text_fields);
    run(world, name_text::refresh_name_fields);
    run(world, numeric::refresh_numeric_fields);

    let title = widget(world, &["title"]);
    type_text(world, title, "unfinished");
    world
        .get_resource_mut::<EventChannel<UITextInputCancelled>>()
        .unwrap()
        .push_event(UITextInputCancelled { entity: title });
    run(world, text_field::cancel_text_fields);
    assert_eq!(
        world
            .get_component_for_entity::<UITextInput>(title)
            .unwrap()
            .value,
        "Gate"
    );
    assert_eq!(**world.get_resource::<FocusedWidget>().unwrap(), None);
    assert!(
        world
            .get_resource::<PropertyCommits>()
            .unwrap()
            .0
            .is_empty()
    );

    submit(world, title, "Portcullis");
    let label = widget(world, &["label"]);
    submit(world, label, "walk");
    let lives = slot(world, &["lives"], 0);
    submit(world, lives, "7");
    let x = slot(world, &["anchor"], 0);
    submit(world, x, "7");
    let y = slot(world, &["anchor"], 1);
    submit(world, y, "-4.5");
    let red = slot(world, &["tint"], 0);
    submit(world, red, "0.8");
    run(world, text_field::commit_text_fields);
    run(world, name_text::commit_name_fields);
    run(world, numeric::commit_numeric_fields);
    w.select(Some(other));
    w.update();
    run(&mut w.f.world, send_property_commits);

    let edited = w.get::<Spawner>(edited).unwrap();
    assert_eq!(edited.title, "Portcullis");
    assert_eq!(edited.label, concerto_name::Name::new("walk"));
    assert_eq!(edited.lives, 7);
    assert_eq!(edited.anchor, Vec2::new(7.0, -4.5));
    assert_eq!(edited.tint, Color::srgba(0.8, 0.25, 0.75, 1.0));
    let other = w.get::<Spawner>(other).unwrap();
    assert_eq!(other.title, "Gate");
    assert_eq!(other.anchor, Vec2::new(1.0, 2.0));
    assert!(w.f.unsaved(w.document));

    w.register::<Track>();
    let lit = w.spawn_in_scene((
        Transform::from_translation(glam::Vec3::new(1.0, 0.0, 0.0)),
        spawner(),
        Track { steps: vec![1.0] },
    ));
    let plain = w.spawn_in_scene((Transform::IDENTITY, Track::default()));
    w.scene().trigger(Select::Replace(vec![plain, lit]));
    w.update();
    w.update();
    let world = &mut w.f.world;
    let mut cards = world.query::<&InspectedComponent, ()>();
    let names: Vec<&str> = cards.iter(world).map(|card| card.name).collect();
    assert!(names.contains(&"Transform"), "{names:?}");
    assert!(!names.contains(&"Spawner"), "{names:?}");
    let mut buttons = world.query::<(Entity, &component_menu::ComponentMenuButton), ()>();
    let buttons: Vec<Entity> = buttons.iter(world).map(|(button, _)| button).collect();
    assert!(!buttons.is_empty());
    for button in buttons {
        assert!(
            world
                .get_component_for_entity::<UIDisabled>(button)
                .is_some()
        );
    }
    let mut menus = world.query::<
        &concerto_ui::anchor::UIAnchoredPanel,
        concerto_ecs::query::filter::With<add_component::AddComponentMenu>,
    >();
    let add = menus.iter(world).next().unwrap().owner.unwrap();
    assert!(world.get_component_for_entity::<UIDisabled>(add).is_some());
    let label = |world: &mut World, path: &[&'static str]| {
        let row = row(world, path);
        let cell = world
            .get_component_for_entity::<Children>(row)
            .unwrap()
            .iter()
            .copied()
            .next()
            .unwrap();
        world
            .get_component_for_entity::<concerto_ui::text::UIText>(cell)
            .unwrap()
            .clone()
    };
    let translation = label(world, &["translation"]);
    assert!(translation.text.ends_with(" —"), "{}", translation.text);
    assert!(!label(world, &["scale"]).text.ends_with(" —"));
    let translation_row = row(world, &["translation"]);
    run(world, numeric::refresh_numeric_fields);
    let x = slot(world, &["translation"], 0);
    submit(world, x, "5");
    run(world, numeric::commit_numeric_fields);
    run(world, send_property_commits);
    for entity in [lit, plain] {
        assert_eq!(
            w.get::<Transform>(entity).unwrap().translation,
            glam::Vec3::new(5.0, 0.0, 0.0)
        );
    }
    w.update();
    w.update();
    let world = &mut w.f.world;
    assert_eq!(row(world, &["translation"]), translation_row);
    assert!(!label(world, &["translation"]).text.ends_with(" —"));

    let steps = row(world, &["steps"]);
    let push = |world: &mut World| {
        let row = world
            .get_component_for_entity::<PropertyRow>(steps)
            .unwrap()
            .clone();
        let commit = PropertyCommit::erased::<ListEditor>(&row, ListEdit::Push).unwrap();
        world
            .get_resource_mut::<PropertyCommits>()
            .unwrap()
            .0
            .push(commit);
        run(world, send_property_commits);
    };
    push(world);
    assert_eq!(w.get::<Track>(lit).unwrap().steps.len(), 2);
    assert_eq!(w.get::<Track>(plain).unwrap().steps.len(), 0);
    w.scene()
        .get_component_for_entity_mut::<Track>(plain)
        .unwrap()
        .steps = vec![0.0, 0.0];
    push(&mut w.f.world);
    assert_eq!(w.get::<Track>(lit).unwrap().steps.len(), 3);
    assert_eq!(w.get::<Track>(plain).unwrap().steps.len(), 3);
}

#[derive(Component, concerto_reflect::Reflect, Default)]
#[reflect(Component, Default)]
struct Track {
    steps: Vec<f32>,
}

#[derive(Component, concerto_reflect::Reflect, Default)]
#[reflect(Component, Default)]
struct Gain {
    amount: f32,
}

#[test]
fn edits_still_queued_when_the_edit_target_changes_are_never_sent() {
    let (mut w, entity, _) = inspecting();
    w.register::<Gain>();
    w.update();
    let translation = row(&mut w.f.world, &["translation"]);
    let row =
        w.f.world
            .get_component_for_entity::<PropertyRow>(translation)
            .unwrap()
            .clone();
    w.f.world
        .get_resource_mut::<PropertyCommits>()
        .unwrap()
        .push::<glam::Vec3, numeric::NumericFields>(
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

    assert!(
        w.f.world
            .get_resource::<ComponentEdits>()
            .unwrap()
            .0
            .is_empty()
    );
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

mod name_field {
    use super::*;
    use concerto_ecs::component::name::Name;
    use concerto_ui::focus::UIFocusGained;

    struct Fixture {
        w: Inspecting,
        field: Entity,
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
        let first = w.f.scene_entity(w.document, "first");
        let second = w.f.scene_entity(w.document, "second");
        let mut fixture = Fixture {
            w,
            field,
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
        fn blur(&mut self) {
            **self.world().get_resource_mut::<FocusedWidget>().unwrap() = None;
            let field = self.field;
            self.world()
                .get_resource_mut::<EventChannel<UIFocusLost>>()
                .unwrap()
                .push_event(UIFocusLost(field));
        }
        fn name(&self, entity: Entity) -> &str {
            self.w.get::<Name>(entity).unwrap().as_str()
        }
    }

    #[test]
    fn submitting_the_field_or_leaving_it_for_another_selection_renames_the_entity_being_edited() {
        let mut f = fixture();
        let field = f.field;

        submit(f.world(), field, "lamp");
        f.step();
        assert_eq!(f.name(f.first), "lamp");
        assert!(f.w.f.unsaved(f.w.document));

        type_text(f.world(), field, "torch");
        f.blur();
        f.select(f.second);
        assert_eq!(f.name(f.first), "torch");
        assert_eq!(f.name(f.second), "second");

        let both = vec![f.first, f.second];
        f.w.scene().trigger(Select::Replace(both));
        f.step();
        assert_eq!(
            f.world()
                .get_component_for_entity::<UITextInput>(field)
                .unwrap()
                .value,
            "2 entities"
        );
        assert!(
            f.world()
                .get_component_for_entity::<UIDisabled>(field)
                .is_some()
        );
    }
}

#[test]
fn a_color_swatch_opens_a_picker_that_edits_the_scene_and_escape_restores_the_color() {
    use concerto_ui::{
        anchor::{UIAnchoredPanel, UIPanelEscaped},
        color_picker::{UIColorChanged, UIColorPicker, setup_color_pickers_system},
        interaction::{UIClick, UIPointerDown},
    };
    use concerto_window::input::MouseButton;

    fn click(world: &mut World, swatch: Entity) {
        world.trigger_on(
            swatch,
            UIClick {
                position: Vec2::ZERO,
                button: MouseButton::Left,
            },
        );
    }
    fn open(world: &mut World) -> bool {
        let mut panels = world.query::<(&UIAnchoredPanel, &color::ColorPickerPopup), ()>();
        let open: Vec<bool> = panels.iter(world).map(|(panel, _)| panel.open).collect();
        assert_eq!(open.len(), 1, "the color rows share one popup");
        open[0]
    }
    fn picker(world: &mut World) -> Entity {
        let mut pickers = world.query::<(Entity, &UIColorPicker), ()>();
        let found: Vec<Entity> = pickers.iter(world).map(|(entity, _)| entity).collect();
        assert_eq!(found.len(), 1, "the popup holds one picker");
        found[0]
    }

    let original = Color::hsl(200.0, 0.5, 0.5, 1.0);
    let (mut w, _, _) = inspecting();
    w.register::<Spawner>();
    let edited = w.spawn_in_scene(Spawner {
        tint: original,
        ..spawner()
    });
    w.select(Some(edited));
    w.update();
    w.update();
    let world = &mut w.f.world;
    let tint = row(world, &["tint"]);
    let mut swatches = world.query::<(Entity, &color::ColorSwatch), ()>();
    let swatch = swatches
        .iter(world)
        .find(|(_, swatch)| swatch.row == tint)
        .map(|(entity, _)| entity)
        .expect("the color row has a swatch");

    click(world, swatch);
    let mut setup = setup_color_pickers_system();
    setup.initialize(world);
    setup.run_and_apply((), world);
    assert!(open(world));
    let shown = picker(world);
    assert_eq!(
        world
            .get_component_for_entity::<UIColorPicker>(shown)
            .unwrap()
            .color(),
        original
    );

    let square = world
        .get_component_for_entity::<Children>(shown)
        .unwrap()
        .iter()
        .copied()
        .next()
        .expect("the picker has its square");
    world.insert(layout(150.0), square);
    world.trigger_on(
        square,
        UIPointerDown {
            position: Vec2::new(0.0, 75.0),
            button: MouseButton::Left,
        },
    );
    run(world, send_property_commits);
    let Color::Hsla(grey) = w.get::<Spawner>(edited).unwrap().tint else {
        panic!("the color stays HSL");
    };
    assert_eq!(grey.h, 200.0);
    assert_eq!(grey.s, 0.0);
    assert!(w.f.unsaved(w.document));

    let picked = Color::hsl(90.0, 0.25, 0.5, 0.5);
    let world = &mut w.f.world;
    world.trigger_on(shown, UIColorChanged { color: picked });
    run(world, send_property_commits);
    assert_eq!(w.get::<Spawner>(edited).unwrap().tint, picked);

    let world = &mut w.f.world;
    let mut popups = world.query::<(Entity, &color::ColorPickerPopup), ()>();
    let popup = popups.iter(world).map(|(entity, _)| entity).next().unwrap();
    world.trigger_on(popup, UIPanelEscaped);
    run(world, send_property_commits);
    assert_eq!(w.get::<Spawner>(edited).unwrap().tint, original);

    let world = &mut w.f.world;
    click(world, swatch);
    assert!(!open(world));
    click(world, swatch);
    assert!(open(world));

    let other = w.spawn_in_scene(spawner());
    w.select(Some(other));
    w.update();
    w.update();
    run(&mut w.f.world, color::close_color_picker);
    assert!(!open(&mut w.f.world));
    assert_eq!(w.get::<Spawner>(other).unwrap().tint, spawner().tint);
}

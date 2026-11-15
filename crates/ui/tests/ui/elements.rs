use concerto_color::Color;
use concerto_ecs::{
    Component, Entity, ResMut, Resource, World, component::bundle::IntoBundle, signal::On,
};
use concerto_ui::{
    anchor::{UIAnchoredPanel, toggle_owned_panels},
    checkbox::{UICheckbox, UICheckboxChanged},
    elements::prelude::*,
    interaction::{UIClick, UIDrag, UIInteractionStyle, UIPointerDown, UIPointerEnter},
    material::UIMaterial,
    node::{UIBox, UILayout, UINode},
    scroll::UISplitAxis,
    slider::{UISlider, UISliderChanged},
    splitter::UISplitterDragged,
    text_input::UITextInput,
    theme::{ButtonVariant, UITheme},
    transform::UIValue,
};
use concerto_window::input::MouseButton;
use glam::Vec2;

struct Spawned {
    world: World,
    entity: Entity,
}

impl Spawned {
    fn get<T: Component>(&self) -> &T {
        self.world
            .get_component_for_entity::<T>(self.entity)
            .unwrap_or_else(|| panic!("missing {}", std::any::type_name::<T>()))
    }
}

fn spawn(bundle: impl IntoBundle) -> Spawned {
    let mut world = World::default();
    let entity = world.spawn(bundle);
    Spawned { world, entity }
}

fn theme() -> UITheme {
    UITheme::nocturne()
}

fn colors(style: &UIInteractionStyle) -> [Color; 4] {
    [style.normal, style.hovered, style.pressed, style.disabled]
}

#[test]
fn modifiers_compose_in_any_order_and_explicit_settings_win() {
    let t = theme();
    let a = spawn(t.button("x").pressed(t.error).ghost().selected(true));
    let b = spawn(t.button("x").selected(true).ghost().pressed(t.error));
    let mut expected = t.interaction(ButtonVariant::Ghost, true);
    expected.pressed = t.error;
    assert_eq!(colors(a.get::<UIInteractionStyle>()), colors(&expected));
    assert_eq!(colors(b.get::<UIInteractionStyle>()), colors(&expected));
    assert_eq!(a.get::<UIMaterial>().color, t.selection().to_linear());

    let field = spawn(t.text_field("Search…").value("héllo"));
    let input = field.get::<UITextInput>();
    assert_eq!(input.value, "héllo");
    assert_eq!(
        input.cursor,
        "héllo".len(),
        "a value moves the cursor to its end"
    );
    assert_eq!(input.placeholder, "Search…");
}

#[derive(Resource, Default)]
struct Heard(Vec<String>);

fn listening_world() -> World {
    let mut world = World::default();
    world.insert_resource(Heard::default());
    world
}

fn heard(world: &World) -> &[String] {
    &world.get_resource::<Heard>().unwrap().0
}

fn click(world: &mut World, entity: Entity, button: MouseButton) {
    world.trigger_on(
        entity,
        UIClick {
            position: Vec2::ZERO,
            button,
        },
    );
}

#[test]
fn an_element_runs_its_listeners_on_itself_and_keeps_its_modifiers() {
    let t = theme();
    let mut world = listening_world();
    let button = world.spawn(
        t.button("Save")
            .on_click(|_: On<UIClick>, mut heard: ResMut<Heard>| heard.0.push("save".into()))
            .ghost()
            .width(UIValue::Px(90.0))
            .small(),
    );
    let listener = |on: On<UIClick>, mut heard: ResMut<Heard>| {
        heard.0.push(format!("{:?}", on.entity()));
    };
    let first = world.spawn(t.chip("mesh").on_click(listener));
    let second = world.spawn(t.chip("scene").on_click(listener));
    let row = world.spawn(
        t.pressable()
            .on_click(|_: On<UIClick>, mut heard: ResMut<Heard>| heard.0.push("click".into()))
            .on(|_: On<UIPointerEnter>, mut heard: ResMut<Heard>| heard.0.push("enter".into())),
    );

    click(&mut world, button, MouseButton::Left);
    click(&mut world, second, MouseButton::Left);
    click(&mut world, first, MouseButton::Left);
    world.trigger_on(
        row,
        UIPointerEnter {
            position: Vec2::ZERO,
        },
    );
    click(&mut world, row, MouseButton::Left);

    assert_eq!(
        heard(&world),
        [
            "save".to_owned(),
            format!("{second:?}"),
            format!("{first:?}"),
            "enter".to_owned(),
            "click".to_owned()
        ]
    );
    let node = world.get_component_for_entity::<UINode>(button).unwrap();
    assert_eq!(node.width, UIValue::Px(90.0));
}

#[test]
fn a_checkbox_toggles_on_left_click_and_reports_the_change() {
    let t = theme();
    let mut world = listening_world();
    let checkbox = world.spawn(t.checkbox("Shadows", false).on_change(
        |on: On<UICheckboxChanged>, mut heard: ResMut<Heard>| {
            heard.0.push(on.signal().checked.to_string());
        },
    ));
    let checked = |world: &World| {
        world
            .get_component_for_entity::<UICheckbox>(checkbox)
            .unwrap()
            .checked
    };

    click(&mut world, checkbox, MouseButton::Right);
    assert!(!checked(&world), "only the left button toggles");

    click(&mut world, checkbox, MouseButton::Left);
    assert!(checked(&world));
    click(&mut world, checkbox, MouseButton::Left);
    assert!(!checked(&world));

    assert_eq!(heard(&world), ["true", "false"]);
}

#[test]
fn a_slider_follows_the_pointer_and_reports_each_change() {
    let t = theme();
    let mut world = listening_world();
    let slider = world.spawn(t.slider(0.0, 0.0, 10.0).on_change(
        |on: On<UISliderChanged>, mut heard: ResMut<Heard>| {
            heard.0.push(on.signal().value.to_string());
        },
    ));
    let rect = UIBox {
        min: Vec2::new(100.0, 0.0),
        size: Vec2::new(200.0, 20.0),
    };
    world.insert(
        UILayout {
            rect,
            content_rect: rect,
            clip_rect: rect,
            paint_order: 0,
        },
        slider,
    );
    let value = |world: &World| {
        world
            .get_component_for_entity::<UISlider>(slider)
            .unwrap()
            .value
    };

    world.trigger_on(
        slider,
        UIPointerDown {
            position: Vec2::new(150.0, 10.0),
            button: MouseButton::Left,
        },
    );
    assert_eq!(value(&world), 2.5);

    let drag = |x: f32, button| UIDrag {
        position: Vec2::new(x, 10.0),
        delta: Vec2::ZERO,
        button,
    };
    world.trigger_on(slider, drag(200.0, MouseButton::Left));
    assert_eq!(value(&world), 5.0);
    world.trigger_on(slider, drag(200.0, MouseButton::Left));
    world.trigger_on(slider, drag(250.0, MouseButton::Right));
    assert_eq!(value(&world), 5.0, "only the left button drags");
    world.trigger_on(slider, drag(900.0, MouseButton::Left));
    assert_eq!(
        value(&world),
        10.0,
        "the value stops at the end of the track"
    );

    assert_eq!(heard(&world), ["2.5", "5", "10"]);
}

#[test]
fn a_splitter_drag_reports_the_movement_along_its_axis_only() {
    let t = theme();
    let mut world = listening_world();
    let report = |on: On<UISplitterDragged>, mut heard: ResMut<Heard>| {
        heard.0.push(on.signal().delta.to_string());
    };
    let columns = world.spawn(t.splitter(UISplitAxis::Horizontal).on_drag(report));
    let rows = world.spawn(t.splitter(UISplitAxis::Vertical).on_drag(report));
    let drag = |delta: Vec2, button| UIDrag {
        position: Vec2::ZERO,
        delta,
        button,
    };

    world.trigger_on(columns, drag(Vec2::new(7.0, 3.0), MouseButton::Left));
    world.trigger_on(rows, drag(Vec2::new(7.0, -3.0), MouseButton::Left));
    world.trigger_on(columns, drag(Vec2::new(0.0, 9.0), MouseButton::Left));
    world.trigger_on(columns, drag(Vec2::new(5.0, 0.0), MouseButton::Right));

    assert_eq!(
        heard(&world),
        ["7", "-3"],
        "movement across the axis and other buttons send nothing"
    );
}

#[test]
fn a_dropdown_opens_and_closes_when_its_trigger_is_clicked() {
    let t = theme();
    let mut world = World::default();
    let trigger = world.spawn(t.button("Menu").on_click(toggle_owned_panels));
    let bystander = world.spawn(t.button("Other").on_click(toggle_owned_panels));
    let dropdown = world.spawn(t.dropdown(trigger));
    let open = |world: &World, panel| {
        world
            .get_component_for_entity::<UIAnchoredPanel>(panel)
            .unwrap()
            .open
    };

    click(&mut world, bystander, MouseButton::Left);
    assert!(!open(&world, dropdown), "only the owner toggles its panel");

    click(&mut world, trigger, MouseButton::Left);
    assert!(open(&world, dropdown));

    click(&mut world, trigger, MouseButton::Left);
    assert!(!open(&world, dropdown));
}

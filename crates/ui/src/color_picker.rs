use concerto_color::{Color, LinearRgba, Srgba};
use concerto_ecs::{
    IntoSystem,
    command::CommandQueue,
    component::Component,
    entity::Entity,
    events::event_reader::EventReader,
    query::{Query, filter::Added},
    resource::{Res, ResMut},
    signal::{
        EntitySignal, On, Signal,
        listener::{IntoListener, Listener},
    },
    system::BoxedSystem,
};
use concerto_window::input::MouseButton;
use glam::Vec2;

use crate::{
    anchor::UIPanelEscaped,
    elements::prelude::*,
    focus::{FocusedWidget, UIFocusLost},
    interaction::{Interactable, UIDrag, UIPointerDown},
    material::{UIGradient, UIMaterial},
    node::{AlignItems, FlexDirection, Position, UIInset, UILayout, UINode},
    text_input::{UITextInput, UITextInputCancelled, UITextInputSubmitted},
    theme::UITheme,
    transform::UIValue,
};

const SQUARE_WIDTH: f32 = 200.0;
const SQUARE_HEIGHT: f32 = 150.0;
const STRIP_HEIGHT: f32 = 12.0;
const MARKER: f32 = 10.0;
const THUMB_WIDTH: f32 = 4.0;
const THUMB_OVERHANG: f32 = 2.0;
const CHECKER_CELL: f32 = 4.0;
const HEX_WIDTH: f32 = 96.0;
const INTENSITY_WIDTH: f32 = 64.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Space {
    Linear,
    Srgb,
    Hsl,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Shown {
    hsv: [f32; 3],
    alpha: f32,
    intensity: f32,
}

impl Shown {
    fn srgb(self) -> Srgba {
        let [r, g, b] = hsv_to_srgb(self.hsv);
        Srgba::new(r, g, b, self.alpha)
    }

    fn with_srgb(self, srgb: Srgba) -> Self {
        let mut hsv = srgb_to_hsv([srgb.r, srgb.g, srgb.b]);
        if hsv[1] == 0.0 {
            hsv[0] = self.hsv[0];
        }
        Self {
            hsv,
            alpha: srgb.a,
            ..self
        }
    }
}

/// A color picker: a saturation/value square, a hue strip, an alpha strip, a hex field and, for linear colors, an intensity field.
#[derive(Component)]
pub struct UIColorPicker {
    shown: Shown,
    opened: Shown,
    space: Space,
    original: Color,
}

impl UIColorPicker {
    /// A picker showing `color`, which is also the color Escape restores.
    pub fn new(color: Color) -> Self {
        let (shown, space) = shown_of(color);
        Self {
            shown,
            opened: shown,
            space,
            original: color,
        }
    }

    /// The color the picker shows, in the space it was given.
    pub fn color(&self) -> Color {
        if self.shown == self.opened {
            return self.original;
        }
        color_of(self.shown, self.space)
    }
}

/// Sent to a [`UIColorPicker`] whenever the user changes its color, in the space the picker was given.
pub struct UIColorChanged {
    pub color: Color,
}

impl Signal for UIColorChanged {}
impl EntitySignal for UIColorChanged {}

/// A listener for the picker's popup that restores the color the picker opened with when Escape closes the popup.
pub fn restore_on_escape(picker: Entity) -> Listener<UIPanelEscaped> {
    (move |_on: On<UIPanelEscaped>, pickers: Query<&mut UIColorPicker>, mut cmd: CommandQueue| {
        let Some(mut state) = pickers.get_entity(picker) else {
            return;
        };
        let original = state.original;
        if state.color() == original {
            return;
        }
        state.shown = state.opened;
        cmd.entity(picker)
            .trigger(UIColorChanged { color: original });
    })
    .into_listener()
}

fn srgb_to_hsv([r, g, b]: [f32; 3]) -> [f32; 3] {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;
    let sector = if delta == 0.0 {
        0.0
    } else if max == r {
        ((g - b) / delta).rem_euclid(6.0)
    } else if max == g {
        (b - r) / delta + 2.0
    } else {
        (r - g) / delta + 4.0
    };
    let saturation = if max == 0.0 { 0.0 } else { delta / max };
    [(sector * 60.0).rem_euclid(360.0), saturation, max]
}

fn hsv_to_srgb([hue, saturation, value]: [f32; 3]) -> [f32; 3] {
    let channel = |n: f32| {
        let k = (n + hue / 60.0).rem_euclid(6.0);
        value - value * saturation * k.min(4.0 - k).clamp(0.0, 1.0)
    };
    [channel(5.0), channel(3.0), channel(1.0)]
}

fn shown_of(color: Color) -> (Shown, Space) {
    match color {
        Color::LinearRgba(linear) => {
            let peak = linear.r.max(linear.g).max(linear.b);
            let intensity = if peak > 1.0 { peak } else { 1.0 };
            let srgb = LinearRgba::new(
                linear.r / intensity,
                linear.g / intensity,
                linear.b / intensity,
                linear.a,
            )
            .to_srgba();
            let shown = Shown {
                hsv: srgb_to_hsv([srgb.r, srgb.g, srgb.b]),
                alpha: linear.a,
                intensity,
            };
            (shown, Space::Linear)
        }
        Color::Srgba(srgb) => {
            let shown = Shown {
                hsv: srgb_to_hsv([srgb.r, srgb.g, srgb.b]),
                alpha: srgb.a,
                intensity: 1.0,
            };
            (shown, Space::Srgb)
        }
        Color::Hsla(hsla) => {
            let srgb = hsla.to_srgba();
            let mut hsv = srgb_to_hsv([srgb.r, srgb.g, srgb.b]);
            hsv[0] = hsla.h.rem_euclid(360.0);
            let shown = Shown {
                hsv,
                alpha: hsla.a,
                intensity: 1.0,
            };
            (shown, Space::Hsl)
        }
    }
}

fn color_of(shown: Shown, space: Space) -> Color {
    let srgb = shown.srgb();
    match space {
        Space::Srgb => Color::Srgba(srgb),
        Space::Hsl => {
            let mut hsla = srgb.to_hsl();
            hsla.h = shown.hsv[0].rem_euclid(360.0);
            Color::Hsla(hsla)
        }
        Space::Linear => {
            let linear = srgb.to_linear();
            Color::LinearRgba(LinearRgba::new(
                linear.r * shown.intensity,
                linear.g * shown.intensity,
                linear.b * shown.intensity,
                shown.alpha,
            ))
        }
    }
}

fn format_hex(srgb: Srgba) -> String {
    let byte = |channel: f32| (channel.clamp(0.0, 1.0) * 255.0).round() as u8;
    let rgb = format!(
        "#{:02X}{:02X}{:02X}",
        byte(srgb.r),
        byte(srgb.g),
        byte(srgb.b)
    );
    if srgb.a == 1.0 {
        rgb
    } else {
        format!("{rgb}{:02X}", byte(srgb.a))
    }
}

fn parse_hex(text: &str) -> Option<Srgba> {
    let digits = text.strip_prefix('#').unwrap_or(text);
    if !matches!(digits.len(), 6 | 8) || !digits.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    Srgba::from_hex(digits)
}

fn format_intensity(intensity: f32) -> String {
    format!("{}", (intensity * 1000.0).round() / 1000.0)
}

fn parse_intensity(text: &str) -> Option<f32> {
    let intensity: f32 = text.trim().parse().ok()?;
    (intensity.is_finite() && intensity >= 1.0).then_some(intensity)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PartKind {
    Square,
    Marker,
    HueThumb,
    AlphaStrip,
    AlphaThumb,
}

#[derive(Component)]
pub(crate) struct ColorPickerPart {
    picker: Entity,
    kind: PartKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FieldKind {
    Hex,
    Intensity,
}

#[derive(Component)]
pub(crate) struct ColorPickerField {
    picker: Entity,
    kind: FieldKind,
}

fn field_text(kind: FieldKind, shown: Shown) -> String {
    match kind {
        FieldKind::Hex => format_hex(shown.srgb()),
        FieldKind::Intensity => format_intensity(shown.intensity),
    }
}

fn entered(kind: FieldKind, text: &str, shown: Shown) -> Option<Shown> {
    match kind {
        FieldKind::Hex => parse_hex(text).map(|srgb| shown.with_srgb(srgb)),
        FieldKind::Intensity => parse_intensity(text).map(|intensity| Shown { intensity, ..shown }),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Area {
    Square,
    Hue,
    Alpha,
}

fn picked(shown: Shown, area: Area, at: Vec2) -> Shown {
    let [hue, saturation, value] = shown.hsv;
    match area {
        Area::Square => Shown {
            hsv: [hue, at.x, 1.0 - at.y],
            ..shown
        },
        Area::Hue => Shown {
            hsv: [at.x * 360.0, saturation, value],
            ..shown
        },
        Area::Alpha => Shown {
            alpha: at.x,
            ..shown
        },
    }
}

fn change(
    picker: Entity,
    pickers: &Query<&mut UIColorPicker>,
    cmd: &mut CommandQueue,
    next: Shown,
) {
    let Some(mut state) = pickers.get_entity(picker) else {
        return;
    };
    if state.shown == next {
        return;
    }
    let before = state.color();
    state.shown = next;
    let color = state.color();
    if color != before {
        cmd.entity(picker).trigger(UIColorChanged { color });
    }
}

fn pick_at(
    picker: Entity,
    area: Area,
    part: Entity,
    button: MouseButton,
    position: Vec2,
    layouts: &Query<&UILayout>,
    pickers: &Query<&mut UIColorPicker>,
    cmd: &mut CommandQueue,
) {
    if button != MouseButton::Left {
        return;
    }
    let Some(layout) = layouts.get_entity(part) else {
        return;
    };
    let size = layout.rect.size;
    if size.x <= 0.0 || size.y <= 0.0 {
        return;
    }
    let at = ((position - layout.rect.min) / size).clamp(Vec2::ZERO, Vec2::ONE);
    let Some(shown) = pickers.get_entity(picker).map(|state| state.shown) else {
        return;
    };
    change(picker, pickers, cmd, picked(shown, area, at));
}

fn press_area(picker: Entity, area: Area) -> Listener<UIPointerDown> {
    (move |on: On<UIPointerDown>,
           layouts: Query<&UILayout>,
           pickers: Query<&mut UIColorPicker>,
           mut cmd: CommandQueue| {
        let signal = on.signal();
        pick_at(
            picker,
            area,
            on.entity(),
            signal.button,
            signal.position,
            &layouts,
            &pickers,
            &mut cmd,
        );
    })
    .into_listener()
}

fn drag_area(picker: Entity, area: Area) -> Listener<UIDrag> {
    (move |on: On<UIDrag>,
           layouts: Query<&UILayout>,
           pickers: Query<&mut UIColorPicker>,
           mut cmd: CommandQueue| {
        let signal = on.signal();
        pick_at(
            picker,
            area,
            on.entity(),
            signal.button,
            signal.position,
            &layouts,
            &pickers,
            &mut cmd,
        );
    })
    .into_listener()
}

fn square_gradient(shown: Shown) -> UIGradient {
    UIGradient::SaturationValue { hue: shown.hsv[0] }
}

fn alpha_gradient(shown: Shown) -> UIGradient {
    UIGradient::Alpha {
        color: Srgba {
            a: 1.0,
            ..shown.srgb()
        },
        cell: CHECKER_CELL,
    }
}

fn marker_inset(shown: Shown) -> UIInset {
    let [_, saturation, value] = shown.hsv;
    UIInset {
        left: UIValue::Px(saturation.clamp(0.0, 1.0) * SQUARE_WIDTH - MARKER / 2.0),
        top: UIValue::Px((1.0 - value.clamp(0.0, 1.0)) * SQUARE_HEIGHT - MARKER / 2.0),
        ..Default::default()
    }
}

fn thumb_inset(fraction: f32) -> UIInset {
    UIInset {
        left: UIValue::Px(fraction.clamp(0.0, 1.0) * SQUARE_WIDTH - THUMB_WIDTH / 2.0),
        top: UIValue::Px(-THUMB_OVERHANG),
        ..Default::default()
    }
}

fn hue_thumb_inset(shown: Shown) -> UIInset {
    thumb_inset(shown.hsv[0] / 360.0)
}

fn alpha_thumb_inset(shown: Shown) -> UIInset {
    thumb_inset(shown.alpha)
}

fn thumb(picker: Entity, kind: PartKind, inset: UIInset) -> (UINode, UIMaterial, ColorPickerPart) {
    (
        UINode::default()
            .with_size(
                UIValue::Px(THUMB_WIDTH),
                UIValue::Px(STRIP_HEIGHT + 2.0 * THUMB_OVERHANG),
            )
            .with_position(Position::Absolute)
            .with_inset(inset),
        UIMaterial {
            corner_radius: 1.0,
            ..UIMaterial::with_border(Color::WHITE, Color::srgba(0.0, 0.0, 0.0, 0.6), 1.0)
        },
        ColorPickerPart { picker, kind },
    )
}

fn strip(
    picker: Entity,
    area: Area,
    gradient: UIGradient,
) -> (
    UINode,
    UIMaterial,
    Interactable,
    Listener<UIPointerDown>,
    Listener<UIDrag>,
) {
    (
        UINode::default()
            .with_size(UIValue::Px(SQUARE_WIDTH), UIValue::Px(STRIP_HEIGHT))
            .with_flex_shrink(0.0),
        UIMaterial {
            gradient,
            ..UIMaterial::flat(Color::WHITE)
        },
        Interactable,
        press_area(picker, area),
        drag_area(picker, area),
    )
}

/// The system that spawns each new `UIColorPicker`'s parts, for a world run without `UIPlugin`.
pub fn setup_color_pickers_system() -> BoxedSystem {
    setup_color_pickers.into_boxed_system()
}

pub(crate) fn setup_color_pickers(
    pickers: Query<(Entity, &UIColorPicker), Added<UIColorPicker>>,
    theme: Res<UITheme>,
    mut cmd: CommandQueue,
) {
    for (entity, picker) in pickers.iter() {
        let shown = picker.shown;
        let mut root = cmd.entity(entity);
        root.spawn_child((
            UINode::default()
                .with_size(UIValue::Px(SQUARE_WIDTH), UIValue::Px(SQUARE_HEIGHT))
                .with_flex_shrink(0.0),
            UIMaterial {
                gradient: square_gradient(shown),
                ..UIMaterial::flat(Color::WHITE)
            },
            Interactable,
            ColorPickerPart {
                picker: entity,
                kind: PartKind::Square,
            },
            press_area(entity, Area::Square),
            drag_area(entity, Area::Square),
        ))
        .add_child((
            UINode::default()
                .with_size(UIValue::Px(MARKER), UIValue::Px(MARKER))
                .with_position(Position::Absolute)
                .with_inset(marker_inset(shown)),
            UIMaterial {
                corner_radius: MARKER / 2.0,
                ..UIMaterial::with_border(Color::TRANSPARENT, Color::WHITE, 2.0)
            },
            ColorPickerPart {
                picker: entity,
                kind: PartKind::Marker,
            },
        ));
        root.spawn_child(strip(entity, Area::Hue, UIGradient::Hue))
            .add_child(thumb(entity, PartKind::HueThumb, hue_thumb_inset(shown)));
        root.spawn_child((
            strip(entity, Area::Alpha, alpha_gradient(shown)),
            ColorPickerPart {
                picker: entity,
                kind: PartKind::AlphaStrip,
            },
        ))
        .add_child(thumb(
            entity,
            PartKind::AlphaThumb,
            alpha_thumb_inset(shown),
        ));
        let mut fields = root.spawn_child(
            UINode::default()
                .with_flex_direction(FlexDirection::Row)
                .with_align_items(AlignItems::Center)
                .with_gap(Vec2::splat(theme.spacing_sm)),
        );
        fields.add_child((
            theme
                .text_field("#RRGGBB")
                .value(field_text(FieldKind::Hex, shown))
                .width(UIValue::Px(HEX_WIDTH)),
            ColorPickerField {
                picker: entity,
                kind: FieldKind::Hex,
            },
        ));
        if picker.space == Space::Linear {
            fields
                .add_child(theme.label("Intensity").small().no_wrap())
                .add_child((
                    theme
                        .text_field("1")
                        .value(field_text(FieldKind::Intensity, shown))
                        .width(UIValue::Px(INTENSITY_WIDTH)),
                    ColorPickerField {
                        picker: entity,
                        kind: FieldKind::Intensity,
                    },
                ));
        }
    }
}

fn show(input: &mut UITextInput, text: String) {
    if input.value != text {
        input.value = text;
        input.cursor = input.value.len();
        input.selection_anchor = None;
    }
}

pub(crate) fn commit_color_picker_fields(
    mut submitted: EventReader<UITextInputSubmitted>,
    mut lost: EventReader<UIFocusLost>,
    mut cancelled: EventReader<UITextInputCancelled>,
    fields: Query<(&ColorPickerField, &mut UITextInput)>,
    pickers: Query<&mut UIColorPicker>,
    mut focused: ResMut<FocusedWidget>,
    mut cmd: CommandQueue,
) {
    for event in cancelled.read() {
        let Some((field, mut input)) = fields.get_entity(event.entity) else {
            continue;
        };
        let Some(shown) = pickers.get_entity(field.picker).map(|state| state.shown) else {
            continue;
        };
        show(&mut input, field_text(field.kind, shown));
        if **focused == Some(event.entity) {
            **focused = None;
        }
    }
    let finished: Vec<Entity> = submitted
        .read()
        .map(|event| event.entity)
        .chain(lost.read().map(|event| event.0))
        .collect();
    for entity in finished {
        let Some((field, mut input)) = fields.get_entity(entity) else {
            continue;
        };
        let Some(shown) = pickers.get_entity(field.picker).map(|state| state.shown) else {
            continue;
        };
        if input.value == field_text(field.kind, shown) {
            continue;
        }
        let shown = match entered(field.kind, &input.value, shown) {
            Some(next) => {
                change(field.picker, &pickers, &mut cmd, next);
                next
            }
            None => shown,
        };
        show(&mut input, field_text(field.kind, shown));
    }
}

pub(crate) fn sync_color_picker_parts(
    pickers: Query<&UIColorPicker>,
    parts: Query<(&ColorPickerPart, &mut UINode, &mut UIMaterial)>,
    fields: Query<(Entity, &ColorPickerField, &mut UITextInput)>,
    focused: Res<FocusedWidget>,
) {
    for (part, mut node, mut material) in parts.iter() {
        let Some(picker) = pickers.get_entity(part.picker) else {
            continue;
        };
        let shown = picker.shown;
        match part.kind {
            PartKind::Square | PartKind::AlphaStrip => {
                let gradient = if part.kind == PartKind::Square {
                    square_gradient(shown)
                } else {
                    alpha_gradient(shown)
                };
                if material.gradient != gradient {
                    material.gradient = gradient;
                }
            }
            PartKind::Marker | PartKind::HueThumb | PartKind::AlphaThumb => {
                let inset = match part.kind {
                    PartKind::Marker => marker_inset(shown),
                    PartKind::HueThumb => hue_thumb_inset(shown),
                    _ => alpha_thumb_inset(shown),
                };
                if node.inset != inset {
                    node.inset = inset;
                }
            }
        }
    }
    for (entity, field, mut input) in fields.iter() {
        if **focused == Some(entity) {
            continue;
        }
        let Some(picker) = pickers.get_entity(field.picker) else {
            continue;
        };
        show(&mut input, field_text(field.kind, picker.shown));
    }
}

#[cfg(test)]
mod tests {
    use concerto_color::Hsla;
    use concerto_ecs::{
        IntoSetConfig, IntoSystemConfig, Schedule, World,
        entity::hierarchy::ChildOf,
        events::event_channel::EventChannel,
        resource::Resource,
        system::{executor::multi_thread::MultiThreadedExecutor, schedule::CompiledSchedule},
    };
    use concerto_window::input::{
        Input,
        actions::{ActionFired, ActionLabel},
    };
    use concerto_window::plugin::Window;

    use super::*;
    use crate::{
        anchor::{
            UIAnchoredPanel, UIDismissPanel, UIPanelStack, dismiss_panels, track_panel_stack,
        },
        focus::UIFocusGained,
        node::{
            UILayoutDiagnostics, UILayoutEngine, UITextMeasure, compute_ui_nodes,
            install_layout_listeners,
        },
        sets::UiSet,
        text::fonts::{UIFonts, build_font_system},
    };

    const EPS: f32 = 1e-5;

    #[derive(Resource, Default)]
    struct Heard(Vec<Color>);

    struct Harness {
        world: World,
        schedule: CompiledSchedule,
    }

    impl Harness {
        fn new() -> Self {
            let mut world = World::new();
            world.insert_resource(Window::new((800, 600), 1.0));
            world.insert_resource(UITheme::default());
            world.insert_resource(UILayoutEngine::default());
            world.insert_resource(UILayoutDiagnostics::default());
            world.insert_resource(UIPanelStack::default());
            world.insert_resource(UITextMeasure::new(build_font_system(&UIFonts::default())));
            world.insert_resource(Input::new());
            world.insert_resource(FocusedWidget::default());
            world.insert_resource(EventChannel::<ActionFired>::default());
            world.insert_resource(EventChannel::<UIFocusGained>::default());
            world.insert_resource(EventChannel::<UIFocusLost>::default());
            world.insert_resource(EventChannel::<UITextInputSubmitted>::default());
            world.insert_resource(EventChannel::<UITextInputCancelled>::default());
            world.insert_resource(Heard::default());
            install_layout_listeners(&mut world);
            let mut schedule = Schedule::new();
            schedule.configure_sets(
                (UiSet::Widgets, UiSet::Setup, UiSet::Project, UiSet::Layout).chain(),
            );
            schedule
                .add_system((commit_color_picker_fields, dismiss_panels).in_set(UiSet::Widgets));
            schedule.add_system(setup_color_pickers.in_set(UiSet::Setup));
            schedule.add_system(sync_color_picker_parts.in_set(UiSet::Project));
            schedule.add_system(
                (track_panel_stack, compute_ui_nodes.after(track_panel_stack))
                    .in_set(UiSet::Layout),
            );
            let schedule = schedule.compile::<MultiThreadedExecutor>(&mut world);
            Self { world, schedule }
        }

        fn frame(&mut self) {
            self.schedule.run(&mut self.world);
            self.world.tick();
        }

        fn theme(&self) -> UITheme {
            self.world.get_resource::<UITheme>().unwrap().clone()
        }

        fn spawn_picker(&mut self, color: Color, parent: Option<Entity>) -> Entity {
            let picker = self.theme().color_picker(color).on_change(
                |on: On<UIColorChanged>, mut heard: ResMut<Heard>| {
                    heard.0.push(on.signal().color);
                },
            );
            let picker = match parent {
                Some(parent) => self.world.spawn((picker, ChildOf::new(parent))),
                None => self.world.spawn(picker),
            };
            for _ in 0..3 {
                self.frame();
            }
            picker
        }

        fn picker(&mut self, color: Color) -> Entity {
            self.spawn_picker(color, None)
        }

        fn part(&mut self, picker: Entity, kind: PartKind) -> Entity {
            self.world
                .query::<(Entity, &ColorPickerPart), ()>()
                .iter(&mut self.world)
                .find(|(_, part)| part.picker == picker && part.kind == kind)
                .map(|(entity, _)| entity)
                .unwrap_or_else(|| panic!("{picker:?} has no {kind:?}"))
        }

        fn parent(&self, entity: Entity) -> Entity {
            **self
                .world
                .get_component_for_entity::<ChildOf>(entity)
                .unwrap()
        }

        fn hue_strip(&mut self, picker: Entity) -> Entity {
            let thumb = self.part(picker, PartKind::HueThumb);
            self.parent(thumb)
        }

        fn field(&mut self, picker: Entity, kind: FieldKind) -> Option<Entity> {
            self.world
                .query::<(Entity, &ColorPickerField), ()>()
                .iter(&mut self.world)
                .find(|(_, field)| field.picker == picker && field.kind == kind)
                .map(|(entity, _)| entity)
        }

        fn at(&self, part: Entity, fraction: Vec2) -> Vec2 {
            let rect = self
                .world
                .get_component_for_entity::<UILayout>(part)
                .unwrap()
                .rect;
            rect.min + fraction * rect.size
        }

        fn press(&mut self, part: Entity, fraction: Vec2) {
            let position = self.at(part, fraction);
            self.world.trigger_on(
                part,
                UIPointerDown {
                    position,
                    button: MouseButton::Left,
                },
            );
            self.frame();
        }

        fn drag(&mut self, part: Entity, fraction: Vec2) {
            let position = self.at(part, fraction);
            self.world.trigger_on(
                part,
                UIDrag {
                    position,
                    delta: Vec2::ZERO,
                    button: MouseButton::Left,
                },
            );
            self.frame();
        }

        fn heard(&mut self) -> Vec<Color> {
            std::mem::take(&mut self.world.get_resource_mut::<Heard>().unwrap().0)
        }

        fn color(&self, picker: Entity) -> Color {
            self.world
                .get_component_for_entity::<UIColorPicker>(picker)
                .unwrap()
                .color()
        }

        fn text(&self, field: Entity) -> String {
            self.world
                .get_component_for_entity::<UITextInput>(field)
                .unwrap()
                .value
                .clone()
        }

        fn type_into(&mut self, field: Entity, text: &str) {
            self.world
                .get_component_for_entity_mut::<UITextInput>(field)
                .unwrap()
                .value = text.to_string();
        }

        fn submit(&mut self, field: Entity, text: &str) {
            self.type_into(field, text);
            self.world
                .get_resource_mut::<EventChannel<UITextInputSubmitted>>()
                .unwrap()
                .push_event(UITextInputSubmitted {
                    entity: field,
                    value: text.to_string(),
                });
            self.frame();
        }

        fn inset(&self, part: Entity) -> UIInset {
            self.world
                .get_component_for_entity::<UINode>(part)
                .unwrap()
                .inset
        }

        fn gradient(&self, part: Entity) -> UIGradient {
            self.world
                .get_component_for_entity::<UIMaterial>(part)
                .unwrap()
                .gradient
        }

        fn press_escape(&mut self) {
            self.world
                .get_resource_mut::<EventChannel<ActionFired>>()
                .unwrap()
                .push_event(ActionFired {
                    action: UIDismissPanel.intern(),
                });
            self.frame();
        }
    }

    fn channels(color: Color) -> [f32; 4] {
        match color {
            Color::LinearRgba(linear) => linear.to_array(),
            Color::Srgba(srgb) => srgb.to_array(),
            Color::Hsla(hsla) => [hsla.h, hsla.s, hsla.l, hsla.a],
        }
    }

    fn assert_near(actual: Color, expected: Color) {
        assert_eq!(
            std::mem::discriminant(&actual),
            std::mem::discriminant(&expected),
            "{actual:?} is not in the space of {expected:?}"
        );
        let (a, b) = (channels(actual), channels(expected));
        assert!(
            a.iter().zip(b).all(|(a, b)| (a - b).abs() <= EPS),
            "{actual:?} != {expected:?}"
        );
    }

    fn assert_heard(heard: Vec<Color>, expected: &[Color]) {
        assert_eq!(heard.len(), expected.len(), "{heard:?} != {expected:?}");
        for (heard, expected) in heard.into_iter().zip(expected) {
            assert_near(heard, *expected);
        }
    }

    fn round_trip(color: Color) -> Color {
        let (shown, space) = shown_of(color);
        color_of(shown, space)
    }

    fn srgb_to_linear(channel: f32) -> f32 {
        Srgba::new(channel, 0.0, 0.0, 1.0).to_linear().r
    }

    fn px(inset: UIInset) -> (f32, f32) {
        let value = |value: UIValue| match value {
            UIValue::Px(px) => px,
            other => panic!("{other:?} is not in pixels"),
        };
        (value(inset.left), value(inset.top))
    }

    #[test]
    fn hsv_converts_to_and_from_srgb() {
        let cases = [
            ([0.0, 1.0, 1.0], [1.0, 0.0, 0.0]),
            ([120.0, 1.0, 1.0], [0.0, 1.0, 0.0]),
            ([240.0, 0.5, 0.8], [0.4, 0.4, 0.8]),
            ([30.0, 1.0, 1.0], [1.0, 0.5, 0.0]),
            ([300.0, 0.25, 0.5], [0.5, 0.375, 0.5]),
            ([0.0, 0.0, 0.5], [0.5, 0.5, 0.5]),
        ];
        for (hsv, srgb) in cases {
            let to = hsv_to_srgb(hsv);
            let from = srgb_to_hsv(srgb);
            for channel in 0..3 {
                assert!(
                    (to[channel] - srgb[channel]).abs() <= EPS,
                    "{hsv:?} -> {to:?}"
                );
                assert!(
                    (from[channel] - hsv[channel]).abs() <= 1e-4,
                    "{srgb:?} -> {from:?}"
                );
            }
        }
        assert_eq!(hsv_to_srgb([360.0, 1.0, 1.0]), hsv_to_srgb([0.0, 1.0, 1.0]));
        let steps = [0.0, 0.1, 0.25, 0.5, 0.75, 1.0];
        for r in steps {
            for g in steps {
                for b in steps {
                    let back = hsv_to_srgb(srgb_to_hsv([r, g, b]));
                    for (back, start) in back.into_iter().zip([r, g, b]) {
                        assert!((back - start).abs() <= EPS, "({r}, {g}, {b}) -> {back}");
                    }
                }
            }
        }
    }

    #[test]
    fn an_srgba_color_round_trips_through_the_picker() {
        for color in [
            Color::srgba(0.2, 0.4, 0.6, 0.8),
            Color::srgba(1.0, 0.0, 0.5, 1.0),
            Color::srgba(0.5, 0.5, 0.5, 0.0),
        ] {
            assert_near(round_trip(color), color);
            assert_eq!(UIColorPicker::new(color).color(), color);
        }
    }

    #[test]
    fn an_hsla_color_round_trips_and_keeps_its_hue_without_saturation() {
        for color in [
            Color::hsl(200.0, 0.5, 0.4, 0.7),
            Color::hsl(200.0, 0.0, 0.4, 1.0),
        ] {
            assert_near(round_trip(color), color);
            assert_eq!(UIColorPicker::new(color).color(), color);
        }
        for (color, black_or_white) in [
            (
                Color::hsl(75.0, 0.3, 0.0, 1.0),
                Color::hsl(75.0, 0.0, 0.0, 1.0),
            ),
            (
                Color::hsl(310.0, 0.8, 1.0, 1.0),
                Color::hsl(310.0, 0.0, 1.0, 1.0),
            ),
        ] {
            assert_near(round_trip(color), black_or_white);
        }
    }

    #[test]
    fn a_linear_color_round_trips_at_intensity_one_and_three() {
        for color in [
            Color::rgba(0.2, 0.4, 0.6, 1.0),
            Color::rgba(3.0, 1.5, 0.3, 0.5),
        ] {
            assert_near(round_trip(color), color);
            assert_eq!(UIColorPicker::new(color).color(), color);
        }
        assert_eq!(shown_of(Color::rgba(0.2, 0.4, 0.6, 1.0)).0.intensity, 1.0);
        assert_eq!(shown_of(Color::rgba(3.0, 1.5, 0.3, 0.5)).0.intensity, 3.0);
        assert_eq!(shown_of(Color::rgba(3.0, 1.5, 0.3, 0.5)).0.alpha, 0.5);
    }

    #[test]
    fn intensity_three_survives_a_drag_that_lowers_the_value() {
        let mut harness = Harness::new();
        let picker = harness.picker(Color::rgba(3.0, 1.5, 0.3, 1.0));
        let field = harness.field(picker, FieldKind::Intensity).unwrap();
        assert_eq!(harness.text(field), "3");
        let square = harness.part(picker, PartKind::Square);

        harness.press(square, Vec2::new(1.0, 0.5));

        let [r, g, b, a] = channels(harness.color(picker));
        assert!(matches!(harness.color(picker), Color::LinearRgba(_)));
        assert!(
            (r.max(g).max(b) - 3.0 * srgb_to_linear(0.5)).abs() <= EPS,
            "{r} {g} {b}"
        );
        assert_eq!(a, 1.0);
        assert_eq!(harness.text(field), "3");
        assert_eq!(harness.heard().len(), 1);
    }

    #[test]
    fn hex_shows_rgb_at_full_alpha_and_rgba_below_it() {
        assert_eq!(format_hex(Srgba::new(1.0, 0.5, 0.0, 1.0)), "#FF8000");
        assert_eq!(format_hex(Srgba::new(1.0, 0.5, 0.0, 0.5)), "#FF800080");
        assert_eq!(format_hex(Srgba::new(1.0, 0.5, 0.0, 0.999)), "#FF8000FF");
    }

    #[test]
    fn hex_parses_six_and_eight_digits_with_an_optional_hash_in_any_case() {
        let orange = Srgba::from_bytes(0xff, 0x80, 0x00, 0xff);
        assert_eq!(parse_hex("#FF8000"), Some(orange));
        assert_eq!(parse_hex("ff8000"), Some(orange));
        assert_eq!(
            parse_hex("#fF80a0"),
            Some(Srgba::from_bytes(0xff, 0x80, 0xa0, 0xff))
        );
        assert_eq!(
            parse_hex("Ff800080"),
            Some(Srgba::from_bytes(0xff, 0x80, 0x00, 0x80))
        );
        assert_eq!(parse_hex("#ff8000").unwrap().a, 1.0);
    }

    #[test]
    fn hex_rejects_other_lengths_and_non_hex_characters() {
        for text in [
            "#FFF", "FFFFF", "#FFFFFFF", "#GG0000", "+F0000", "#ff 000", "ééé", "", "#",
        ] {
            assert_eq!(parse_hex(text), None, "{text:?}");
        }
    }

    #[test]
    fn a_six_digit_hex_entry_sets_alpha_to_one_and_keeps_the_intensity() {
        let mut harness = Harness::new();
        let picker = harness.picker(Color::rgba(3.0, 0.0, 0.0, 0.5));
        let hex = harness.field(picker, FieldKind::Hex).unwrap();
        assert_eq!(harness.text(hex), "#FF000080");

        harness.submit(hex, "00ff00");

        assert_heard(harness.heard(), &[Color::rgba(0.0, 3.0, 0.0, 1.0)]);
        assert_eq!(harness.text(hex), "#00FF00");

        harness.submit(hex, "#12345");

        assert!(harness.heard().is_empty());
        assert_eq!(harness.text(hex), "#00FF00");
        assert_near(harness.color(picker), Color::rgba(0.0, 3.0, 0.0, 1.0));
    }

    #[test]
    fn a_hex_entry_commits_when_its_field_loses_focus() {
        let mut harness = Harness::new();
        let picker = harness.picker(Color::srgba(0.0, 0.0, 0.0, 1.0));
        let hex = harness.field(picker, FieldKind::Hex).unwrap();
        assert!(harness.field(picker, FieldKind::Intensity).is_none());

        harness.type_into(hex, "#3366CC80");
        harness
            .world
            .get_resource_mut::<EventChannel<UIFocusLost>>()
            .unwrap()
            .push_event(UIFocusLost(hex));
        harness.frame();

        assert_heard(
            harness.heard(),
            &[Color::Srgba(Srgba::from_bytes(0x33, 0x66, 0xcc, 0x80))],
        );
    }

    #[test]
    fn intensity_entry_accepts_one_and_above_and_restores_anything_else() {
        let mut harness = Harness::new();
        let picker = harness.picker(Color::rgba(0.5, 0.25, 0.1, 1.0));
        let field = harness.field(picker, FieldKind::Intensity).unwrap();
        assert_eq!(harness.text(field), "1");

        harness.submit(field, "2.5");

        let bright = Color::rgba(1.25, 0.625, 0.25, 1.0);
        assert_heard(harness.heard(), &[bright]);
        assert_eq!(harness.text(field), "2.5");

        for text in ["0.5", "0", "NaN", "inf", "bright"] {
            harness.submit(field, text);

            assert!(harness.heard().is_empty(), "{text:?}");
            assert_eq!(harness.text(field), "2.5", "{text:?}");
            assert_near(harness.color(picker), bright);
        }
    }

    #[test]
    fn a_focused_field_keeps_its_text_while_a_drag_changes_the_color() {
        let mut harness = Harness::new();
        let picker = harness.picker(Color::srgba(1.0, 0.0, 0.0, 1.0));
        let hex = harness.field(picker, FieldKind::Hex).unwrap();
        let square = harness.part(picker, PartKind::Square);
        harness.type_into(hex, "#12");
        **harness.world.get_resource_mut::<FocusedWidget>().unwrap() = Some(hex);

        harness.press(square, Vec2::new(0.0, 0.0));

        assert_eq!(harness.text(hex), "#12");
        **harness.world.get_resource_mut::<FocusedWidget>().unwrap() = None;
        harness.frame();
        assert_eq!(harness.text(hex), "#FFFFFF");
    }

    #[test]
    fn a_cancelled_entry_restores_its_field_and_drops_focus() {
        let mut harness = Harness::new();
        let picker = harness.picker(Color::srgba(1.0, 0.0, 0.0, 1.0));
        let hex = harness.field(picker, FieldKind::Hex).unwrap();
        harness.type_into(hex, "#00");
        **harness.world.get_resource_mut::<FocusedWidget>().unwrap() = Some(hex);

        harness
            .world
            .get_resource_mut::<EventChannel<UITextInputCancelled>>()
            .unwrap()
            .push_event(UITextInputCancelled { entity: hex });
        harness.frame();

        assert_eq!(harness.text(hex), "#FF0000");
        assert_eq!(
            **harness.world.get_resource::<FocusedWidget>().unwrap(),
            None
        );
        assert!(harness.heard().is_empty());
    }

    #[test]
    fn a_field_whose_picker_is_gone_is_ignored() {
        let mut harness = Harness::new();
        let picker = harness.picker(Color::srgba(1.0, 0.0, 0.0, 1.0));
        let field = harness.world.spawn((
            UITextInput::new(""),
            ColorPickerField {
                picker,
                kind: FieldKind::Hex,
            },
        ));
        harness.world.despawn(picker);

        harness.submit(field, "#00FF00");

        assert!(harness.heard().is_empty());
        assert_eq!(harness.text(field), "#00FF00");
    }

    #[test]
    fn pressing_and_dragging_the_parts_sets_saturation_value_hue_and_alpha() {
        let red = |saturation: f32, value: f32| {
            Color::srgba(
                value,
                value * (1.0 - saturation),
                value * (1.0 - saturation),
                1.0,
            )
        };
        let mut harness = Harness::new();
        let picker = harness.picker(Color::srgba(1.0, 0.0, 0.0, 1.0));
        let square = harness.part(picker, PartKind::Square);
        let marker = harness.part(picker, PartKind::Marker);

        harness.press(square, Vec2::new(0.25, 0.25));

        assert_heard(harness.heard(), &[red(0.25, 0.75)]);
        assert_eq!(px(harness.inset(marker)), (45.0, 32.5));

        for (x, y) in [(0.5, 0.5), (1.0, 0.0), (0.5, 0.0)] {
            harness.drag(square, Vec2::new(x, y));

            assert_heard(harness.heard(), &[red(x, 1.0 - y)]);
            assert_eq!(
                px(harness.inset(marker)),
                (x * SQUARE_WIDTH - 5.0, y * SQUARE_HEIGHT - 5.0)
            );
        }

        let hue = harness.hue_strip(picker);
        let hue_thumb = harness.part(picker, PartKind::HueThumb);
        harness.press(hue, Vec2::new(0.5, 0.5));

        assert_heard(harness.heard(), &[Color::srgba(0.5, 1.0, 1.0, 1.0)]);
        assert_eq!(px(harness.inset(hue_thumb)), (98.0, -THUMB_OVERHANG));
        assert_eq!(
            harness.gradient(square),
            UIGradient::SaturationValue { hue: 180.0 }
        );

        let alpha = harness.part(picker, PartKind::AlphaStrip);
        let alpha_thumb = harness.part(picker, PartKind::AlphaThumb);
        harness.drag(alpha, Vec2::new(0.75, 0.5));

        assert_heard(harness.heard(), &[Color::srgba(0.5, 1.0, 1.0, 0.75)]);
        assert_eq!(px(harness.inset(alpha_thumb)), (148.0, -THUMB_OVERHANG));
        assert_eq!(
            harness.gradient(alpha),
            UIGradient::Alpha {
                color: Srgba::new(0.5, 1.0, 1.0, 1.0),
                cell: CHECKER_CELL,
            }
        );

        for _ in 0..3 {
            harness.frame();
        }

        assert!(harness.heard().is_empty());
        assert_near(harness.color(picker), Color::srgba(0.5, 1.0, 1.0, 0.75));
        assert_eq!(px(harness.inset(marker)), (95.0, -5.0));
        assert_eq!(px(harness.inset(alpha_thumb)), (148.0, -THUMB_OVERHANG));
    }

    #[test]
    fn a_drag_that_leaves_the_color_unchanged_sends_nothing() {
        let mut harness = Harness::new();
        let picker = harness.picker(Color::srgba(1.0, 0.0, 0.0, 1.0));
        let square = harness.part(picker, PartKind::Square);
        harness.press(square, Vec2::new(0.5, 0.5));
        assert_eq!(harness.heard().len(), 1);

        for _ in 0..3 {
            harness.drag(square, Vec2::new(0.5, 0.5));
        }

        assert!(harness.heard().is_empty());

        let grey = harness.picker(Color::srgba(0.5, 0.5, 0.5, 1.0));
        let hue = harness.hue_strip(grey);
        let hue_thumb = harness.part(grey, PartKind::HueThumb);

        harness.drag(hue, Vec2::new(0.25, 0.5));

        assert!(harness.heard().is_empty());
        assert_eq!(px(harness.inset(hue_thumb)), (48.0, -THUMB_OVERHANG));
        assert_eq!(harness.color(grey), Color::srgba(0.5, 0.5, 0.5, 1.0));
    }

    fn popup_with_picker(harness: &mut Harness, color: Color) -> (Entity, Entity) {
        let theme = harness.theme();
        let popup = harness
            .world
            .spawn(theme.context_menu().open(true).anchor_target(
                crate::anchor::UIAnchorTarget::from_point(Vec2::new(100.0, 100.0)),
            ));
        let picker = harness.spawn_picker(color, Some(popup));
        harness.world.insert(restore_on_escape(picker), popup);
        harness.frame();
        (popup, picker)
    }

    fn is_open(harness: &Harness, popup: Entity) -> bool {
        harness
            .world
            .get_component_for_entity::<UIAnchoredPanel>(popup)
            .unwrap()
            .open
    }

    #[test]
    fn escape_restores_the_color_the_picker_opened_with() {
        let original = Color::rgba(0.2, 0.4, 0.6, 1.0);
        let mut harness = Harness::new();
        let (popup, picker) = popup_with_picker(&mut harness, original);
        let square = harness.part(picker, PartKind::Square);
        harness.press(square, Vec2::new(0.9, 0.1));
        assert_eq!(harness.heard().len(), 1);

        harness.press_escape();

        assert!(!is_open(&harness, popup));
        assert_eq!(harness.heard(), [original]);
        assert_eq!(harness.color(picker), original);
    }

    #[test]
    fn escape_on_an_untouched_picker_sends_nothing() {
        let mut harness = Harness::new();
        let (popup, _) = popup_with_picker(&mut harness, Color::rgba(0.2, 0.4, 0.6, 1.0));

        harness.press_escape();

        assert!(!is_open(&harness, popup));
        assert!(harness.heard().is_empty());
    }

    #[test]
    fn a_press_outside_the_popup_keeps_the_color() {
        let mut harness = Harness::new();
        let (popup, picker) = popup_with_picker(&mut harness, Color::rgba(0.2, 0.4, 0.6, 1.0));
        let square = harness.part(picker, PartKind::Square);
        harness.press(square, Vec2::new(0.9, 0.1));
        let dragged = harness.color(picker);
        harness.heard();

        let input = harness.world.get_resource_mut::<Input>().unwrap();
        input.update_mouse_position(790.0, 590.0);
        input.update_mouse_button(MouseButton::Left, true);
        harness.frame();

        assert!(!is_open(&harness, popup));
        assert!(harness.heard().is_empty());
        assert_eq!(harness.color(picker), dragged);
    }

    #[test]
    fn an_hsla_color_keeps_its_hue_through_a_drag_to_grey() {
        let mut harness = Harness::new();
        let picker = harness.picker(Color::hsl(200.0, 0.5, 0.5, 1.0));
        let square = harness.part(picker, PartKind::Square);

        harness.press(square, Vec2::new(0.0, 0.5));

        let heard = harness.heard();
        assert_eq!(heard.len(), 1);
        let Color::Hsla(Hsla { h, s, .. }) = heard[0] else {
            panic!("{:?} is not HSL", heard[0]);
        };
        assert_eq!((h, s), (200.0, 0.0));
    }
}

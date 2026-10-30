use concerto_app::{App, Plugin, schedule_groups::LateUpdate};
use concerto_ecs::{
    Component, Entity, IntoSystemConfig, Query, Res,
    command::CommandQueue,
    query::filter::{Added, Changed, Or},
    signal::{
        EntitySignal, On, Signal,
        listener::{IntoListener, Listener},
    },
};
use concerto_ui::elements::prelude::*;
use concerto_ui::{
    interaction::{Interactable, UIClick, UIDrag, UIPointerDown},
    material::UIMaterial,
    node::{FlexDirection, Position, UIBox, UIInset, UILayout, UINode, UIRect},
    sets::UiSet,
    text::UIText,
    theme::UITheme,
    transform::UIValue,
};
use concerto_window::input::MouseButton;
use glam::Vec2;

const RULER_HEIGHT: f32 = 24.0;
const READOUT_WIDTH: f32 = 280.0;
const HEAD_WIDTH: f32 = 2.0;
const SELECTION_MIN_WIDTH: f32 = 2.0;
const TICK_WIDTH: f32 = 1.0;
const MIN_TICK_GAP: f32 = 64.0;
const TICK_LABEL_OFFSET: f32 = 4.0;
const WHOLE_TICK_TOLERANCE: f32 = 1e-3;

/// A ruler and a track over a clip's length, drawn on the UI node that holds it.
#[derive(Component, PartialEq)]
pub struct Timeline {
    pub duration: f32,
    pub rate: Option<f32>,
    pub time: f32,
    pub selection: Option<(u32, u32)>,
}

impl Timeline {
    fn time_at(&self, x: f32, rect: UIBox) -> f32 {
        let time = fraction(x - rect.min.x, rect.size.x) * self.duration;
        match self.rate {
            Some(rate) => time_of_frame(frame_at(time, rate), rate).min(self.duration),
            None => time,
        }
    }

    fn selection_span(&self) -> Option<(f32, f32)> {
        let rate = self.rate?;
        let (first, last) = self.selection?;
        let left = fraction(time_of_frame(first, rate), self.duration);
        let right = fraction(time_of_frame(last, rate), self.duration);
        Some((left, right - left))
    }

    fn readout(&self) -> String {
        match self.rate {
            Some(rate) => format!(
                "Frame {} / {}   {:.2} s",
                frame_at(self.time, rate),
                frame_count(self.duration, rate),
                self.time
            ),
            None => format!("{:.2} s", self.time),
        }
    }
}

/// Sent to a [`Timeline`] when the ruler is pressed or dragged, with the time under the pointer.
pub struct TimelineScrubbed {
    pub time: f32,
}

impl Signal for TimelineScrubbed {}
impl EntitySignal for TimelineScrubbed {}

/// Sent to a [`Timeline`] when a drag on the track selects frames, inclusive, or a click clears them.
pub struct TimelineSelected {
    pub range: Option<(u32, u32)>,
}

impl Signal for TimelineSelected {}
impl EntitySignal for TimelineSelected {}

/// The frame nearest to `time`.
pub fn frame_at(time: f32, rate: f32) -> u32 {
    (time * rate).round() as u32
}

/// The time a frame sits at.
pub fn time_of_frame(frame: u32, rate: f32) -> f32 {
    frame as f32 / rate
}

/// How many frames a clip holds, counting the first and the last.
pub fn frame_count(duration: f32, rate: f32) -> u32 {
    ((duration * rate).round() as u32).saturating_add(1)
}

/// On a timeline's readout, whose text changes on every frame of playback.
#[derive(Component)]
pub(crate) struct TimelineReadout;

#[derive(Component)]
struct TimelineParts {
    readout: Entity,
    ruler: Entity,
    selection: Entity,
    head: Entity,
}

#[derive(Component, Debug, Default, PartialEq)]
struct TimelineTicks {
    width: f32,
    duration: f32,
    rate: Option<f32>,
}

#[derive(Component)]
struct TimelineSelectionStart(u32);

struct Tick {
    fraction: f32,
    label: String,
}

fn fraction(value: f32, span: f32) -> f32 {
    if span > 0.0 {
        (value / span).clamp(0.0, 1.0)
    } else {
        0.0
    }
}

fn tick_step(span: f32, width: f32) -> f32 {
    let least = span * MIN_TICK_GAP / width;
    if least <= 0.0 {
        return 1.0;
    }
    let decade = 10f32.powf(least.log10().floor());
    let mantissa = match least / decade {
        scaled if scaled <= 1.0 => 1.0,
        scaled if scaled <= 2.0 => 2.0,
        scaled if scaled <= 5.0 => 5.0,
        _ => 10.0,
    };
    mantissa * decade
}

fn ticks_of(duration: f32, rate: Option<f32>, width: f32) -> Vec<Tick> {
    let span = duration * rate.unwrap_or(1.0);
    let step = match rate {
        Some(_) => tick_step(span, width).max(1.0),
        None => tick_step(span, width),
    };
    let decimals = (-step.log10().floor()).max(0.0) as usize;
    let last = (span / step + WHOLE_TICK_TOLERANCE).floor() as u32;
    (0..=last)
        .map(|index| {
            let value = index as f32 * step;
            Tick {
                fraction: fraction(value, span),
                label: match rate {
                    Some(_) => format!("{value:.0}"),
                    None => format!("{value:.decimals$} s"),
                },
            }
        })
        .collect()
}

fn scrub(
    timeline: Entity,
    ruler: Entity,
    button: MouseButton,
    position: Vec2,
    timelines: &Query<&Timeline>,
    layouts: &Query<&UILayout>,
    cmd: &mut CommandQueue,
) {
    if button != MouseButton::Left {
        return;
    }
    let (Some(view), Some(layout)) = (timelines.get_entity(timeline), layouts.get_entity(ruler))
    else {
        return;
    };
    let time = view.time_at(position.x, layout.rect);
    cmd.entity(timeline).trigger(TimelineScrubbed { time });
}

fn press_ruler(timeline: Entity) -> Listener<UIPointerDown> {
    (move |on: On<UIPointerDown>,
           timelines: Query<&Timeline>,
           layouts: Query<&UILayout>,
           mut cmd: CommandQueue| {
        let signal = on.signal();
        scrub(
            timeline,
            on.entity(),
            signal.button,
            signal.position,
            &timelines,
            &layouts,
            &mut cmd,
        );
    })
    .into_listener()
}

fn drag_ruler(timeline: Entity) -> Listener<UIDrag> {
    (move |on: On<UIDrag>,
           timelines: Query<&Timeline>,
           layouts: Query<&UILayout>,
           mut cmd: CommandQueue| {
        let signal = on.signal();
        if signal.delta == Vec2::ZERO {
            return;
        }
        scrub(
            timeline,
            on.entity(),
            signal.button,
            signal.position,
            &timelines,
            &layouts,
            &mut cmd,
        );
    })
    .into_listener()
}

fn frame_under(
    timeline: Entity,
    track: Entity,
    position: Vec2,
    timelines: &Query<&Timeline>,
    layouts: &Query<&UILayout>,
) -> Option<u32> {
    let view = timelines.get_entity(timeline)?;
    let rate = view.rate?;
    let layout = layouts.get_entity(track)?;
    Some(frame_at(view.time_at(position.x, layout.rect), rate))
}

fn press_track(timeline: Entity) -> Listener<UIPointerDown> {
    (move |on: On<UIPointerDown>,
           timelines: Query<&Timeline>,
           layouts: Query<&UILayout>,
           mut cmd: CommandQueue| {
        let signal = on.signal();
        if signal.button != MouseButton::Left {
            return;
        }
        if let Some(frame) =
            frame_under(timeline, on.entity(), signal.position, &timelines, &layouts)
        {
            cmd.insert(TimelineSelectionStart(frame), on.entity());
        }
    })
    .into_listener()
}

fn drag_track(timeline: Entity) -> Listener<UIDrag> {
    (move |on: On<UIDrag>,
           timelines: Query<&Timeline>,
           layouts: Query<&UILayout>,
           starts: Query<&TimelineSelectionStart>,
           mut cmd: CommandQueue| {
        let signal = on.signal();
        if signal.button != MouseButton::Left || signal.delta == Vec2::ZERO {
            return;
        }
        let Some(start) = starts.get_entity(on.entity()) else {
            return;
        };
        let Some(frame) = frame_under(timeline, on.entity(), signal.position, &timelines, &layouts)
        else {
            return;
        };
        cmd.entity(timeline).trigger(TimelineSelected {
            range: Some((start.0.min(frame), start.0.max(frame))),
        });
    })
    .into_listener()
}

fn click_track(timeline: Entity) -> Listener<UIClick> {
    (move |on: On<UIClick>, timelines: Query<&Timeline>, mut cmd: CommandQueue| {
        let has_frames = timelines
            .get_entity(timeline)
            .is_some_and(|view| view.rate.is_some());
        if on.signal().button == MouseButton::Left && has_frames {
            cmd.entity(timeline)
                .trigger(TimelineSelected { range: None });
        }
    })
    .into_listener()
}

fn across_the_band() -> UIInset {
    UIInset {
        top: UIValue::Px(0.0),
        bottom: UIValue::Px(0.0),
        ..Default::default()
    }
}

fn setup_timeline(
    timelines: Query<(Entity, &mut UINode), Added<Timeline>>,
    theme: Res<UITheme>,
    mut cmd: CommandQueue,
) {
    for (timeline, mut node) in timelines.iter() {
        node.flex_direction = FlexDirection::Column;
        node.gap = Vec2::new(0.0, theme.spacing_xs);
        node.min_width = UIValue::Px(0.0);

        let mut root = cmd.entity(timeline);
        let readout = root
            .spawn_child((
                theme
                    .label("")
                    .mono()
                    .single_line()
                    .size(
                        UIValue::Px(READOUT_WIDTH),
                        UIValue::Px(theme.line_height(theme.font_size_md)),
                    )
                    .fixed(),
                TimelineReadout,
            ))
            .entity();
        let mut bands = root.spawn_child(
            UINode::default()
                .with_flex_direction(FlexDirection::Column)
                .with_flex_grow(1.0),
        );
        let ruler = bands
            .spawn_child((
                UINode::default()
                    .with_height(UIValue::Px(RULER_HEIGHT))
                    .with_flex_shrink(0.0)
                    .clipped(),
                Interactable,
                press_ruler(timeline),
                drag_ruler(timeline),
            ))
            .entity();
        let mut track = bands.spawn_child((
            theme.card().radius(theme.radius_sm).grow().clipped(),
            Interactable,
            press_track(timeline),
            drag_track(timeline),
            click_track(timeline),
        ));
        let selection = track
            .spawn_child((
                UINode::default()
                    .with_position(Position::Absolute)
                    .with_inset(across_the_band())
                    .with_min_width(UIValue::Px(SELECTION_MIN_WIDTH))
                    .with_visible(false),
                UIMaterial::flat(theme.selection()),
            ))
            .entity();
        let head = bands
            .spawn_child((
                UINode::default()
                    .with_position(Position::Absolute)
                    .with_inset(across_the_band())
                    .with_width(UIValue::Px(HEAD_WIDTH))
                    .with_margin(UIRect {
                        left: -HEAD_WIDTH / 2.0,
                        ..Default::default()
                    }),
                UIMaterial::flat(theme.accent),
            ))
            .entity();
        root.insert((
            TimelineParts {
                readout,
                ruler,
                selection,
                head,
            },
            TimelineTicks::default(),
        ));
    }
}

fn sync_timeline_parts(
    timelines: Query<(&Timeline, &TimelineParts), Or<(Changed<Timeline>, Changed<TimelineParts>)>>,
    nodes: Query<&mut UINode>,
    texts: Query<&mut UIText>,
) {
    for (timeline, parts) in timelines.iter() {
        if let Some(mut text) = texts.get_entity(parts.readout) {
            let readout = timeline.readout();
            if text.text != readout {
                text.text = readout;
            }
        }
        if let Some(mut head) = nodes.get_entity(parts.head) {
            let left = UIValue::Percent(fraction(timeline.time, timeline.duration) * 100.0);
            if head.inset.left != left {
                head.inset.left = left;
            }
        }
        if let Some(mut selection) = nodes.get_entity(parts.selection) {
            let span = timeline.selection_span();
            if selection.visible != span.is_some() {
                selection.visible = span.is_some();
            }
            if let Some((left, width)) = span {
                let left = UIValue::Percent(left * 100.0);
                let width = UIValue::Percent(width * 100.0);
                if selection.inset.left != left {
                    selection.inset.left = left;
                }
                if selection.width != width {
                    selection.width = width;
                }
            }
        }
    }
}

fn sync_timeline_ticks(
    timelines: Query<(&Timeline, &TimelineParts, &mut TimelineTicks)>,
    layouts: Query<&UILayout>,
    theme: Res<UITheme>,
    mut cmd: CommandQueue,
) {
    for (timeline, parts, mut built) in timelines.iter() {
        let Some(layout) = layouts.get_entity(parts.ruler) else {
            continue;
        };
        let wanted = TimelineTicks {
            width: layout.rect.size.x,
            duration: timeline.duration,
            rate: timeline.rate,
        };
        if wanted.width <= 0.0 || **built == wanted {
            continue;
        }
        let mut ruler = cmd.entity(parts.ruler).despawn_children();
        for tick in ticks_of(wanted.duration, wanted.rate, wanted.width) {
            let left = UIValue::Percent(tick.fraction * 100.0);
            ruler.add_child((
                UINode::default()
                    .with_position(Position::Absolute)
                    .with_inset(UIInset {
                        left,
                        ..across_the_band()
                    })
                    .with_width(UIValue::Px(TICK_WIDTH)),
                UIMaterial::flat(theme.border),
            ));
            ruler.add_child(
                theme
                    .label(tick.label)
                    .small()
                    .muted()
                    .no_wrap()
                    .size(
                        UIValue::Px(MIN_TICK_GAP - TICK_LABEL_OFFSET),
                        UIValue::Px(RULER_HEIGHT),
                    )
                    .node(|node| {
                        node.with_position(Position::Absolute)
                            .with_inset(UIInset {
                                left,
                                top: UIValue::Px(0.0),
                                ..Default::default()
                            })
                            .with_margin(UIRect {
                                left: TICK_LABEL_OFFSET,
                                ..Default::default()
                            })
                    }),
            );
        }
        **built = wanted;
    }
}

/// Builds and keeps every [`Timeline`] drawn; `EditorPlugin` registers it.
pub struct TimelinePlugin;

impl Plugin for TimelinePlugin {
    fn build(&self, app: &mut App) {
        app.add_system(LateUpdate, setup_timeline.in_set(UiSet::Setup))
            .add_system(
                LateUpdate,
                (sync_timeline_parts, sync_timeline_ticks).in_set(UiSet::Project),
            );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use concerto_app::{main_schedule::MainSchedulePlugin, plugins::TimePlugin};
    use concerto_ecs::{
        IntoSystem, ResMut, Resource, System, With, World,
        entity::hierarchy::{ChildOf, Children},
        system::BoxedSystem,
    };

    const LEFT: f32 = 100.0;
    const WIDTH: f32 = 300.0;
    const NEAR: f32 = 1e-4;

    #[derive(Resource, Default)]
    struct Heard {
        scrubbed: Vec<f32>,
        selected: Vec<Option<(u32, u32)>>,
    }

    fn at_30_fps() -> Timeline {
        Timeline {
            duration: 10.0,
            rate: Some(30.0),
            time: 0.0,
            selection: None,
        }
    }

    fn without_a_rate() -> Timeline {
        Timeline {
            rate: None,
            ..at_30_fps()
        }
    }

    struct Fixture {
        world: World,
        setup: BoxedSystem,
        parts: BoxedSystem,
        ticks: BoxedSystem,
        timeline: Entity,
    }

    impl Fixture {
        fn new(timeline: Timeline) -> Self {
            let mut world = World::new();
            world.register_component::<ChildOf>();
            world.register_component::<Children>();
            world.insert_resource(UITheme::default());
            world.insert_resource(Heard::default());
            let timeline = world.spawn((
                UINode::default(),
                timeline,
                (|on: On<TimelineScrubbed>, mut heard: ResMut<Heard>| {
                    heard.scrubbed.push(on.signal().time);
                })
                .into_listener(),
                (|on: On<TimelineSelected>, mut heard: ResMut<Heard>| {
                    heard.selected.push(on.signal().range);
                })
                .into_listener(),
            ));
            let mut setup = setup_timeline.into_boxed_system();
            let mut parts = sync_timeline_parts.into_boxed_system();
            let mut ticks = sync_timeline_ticks.into_boxed_system();
            setup.initialize(&mut world);
            parts.initialize(&mut world);
            ticks.initialize(&mut world);
            let mut fixture = Self {
                world,
                setup,
                parts,
                ticks,
                timeline,
            };
            fixture.frame();
            fixture
        }

        fn laid_out(timeline: Timeline) -> Self {
            let mut fixture = Self::new(timeline);
            fixture.lay_out(WIDTH);
            fixture.frame();
            fixture
        }

        fn frame(&mut self) {
            self.setup.run_and_apply((), &mut self.world);
            self.parts.run_and_apply((), &mut self.world);
            self.ticks.run_and_apply((), &mut self.world);
        }

        fn lay_out(&mut self, width: f32) {
            let rect = UIBox {
                min: Vec2::new(LEFT, 0.0),
                size: Vec2::new(width, 24.0),
            };
            let layout = UILayout {
                rect,
                content_rect: rect,
                clip_rect: rect,
                paint_order: 0,
            };
            let (ruler, track) = (self.parts().ruler, self.track());
            self.world.insert(layout.clone(), ruler);
            self.world.insert(layout, track);
        }

        fn parts(&self) -> &TimelineParts {
            self.world
                .get_component_for_entity::<TimelineParts>(self.timeline)
                .unwrap()
        }

        fn track(&mut self) -> Entity {
            self.world
                .query::<Entity, With<Listener<UIClick>>>()
                .iter(&mut self.world)
                .next()
                .unwrap()
        }

        fn edit(&mut self, edit: impl FnOnce(&mut Timeline)) {
            edit(
                self.world
                    .get_component_for_entity_mut::<Timeline>(self.timeline)
                    .unwrap(),
            );
        }

        fn press(&mut self, target: Entity, x: f32) {
            self.world.trigger_on(
                target,
                UIPointerDown {
                    position: Vec2::new(x, 0.0),
                    button: MouseButton::Left,
                },
            );
        }

        fn drag(&mut self, target: Entity, x: f32, moved: f32) {
            self.world.trigger_on(
                target,
                UIDrag {
                    position: Vec2::new(x, 0.0),
                    delta: Vec2::new(moved, 0.0),
                    button: MouseButton::Left,
                },
            );
        }

        fn click(&mut self, target: Entity, x: f32) {
            self.world.trigger_on(
                target,
                UIClick {
                    position: Vec2::new(x, 0.0),
                    button: MouseButton::Left,
                },
            );
        }

        fn heard(&self) -> &Heard {
            self.world.get_resource::<Heard>().unwrap()
        }

        fn node(&self, entity: Entity) -> &UINode {
            self.world
                .get_component_for_entity::<UINode>(entity)
                .unwrap()
        }

        fn readout(&self) -> &str {
            &self
                .world
                .get_component_for_entity::<UIText>(self.parts().readout)
                .unwrap()
                .text
        }

        fn ruler_children(&self) -> Vec<Entity> {
            self.world
                .get_component_for_entity::<Children>(self.parts().ruler)
                .map(|children| children.iter().copied().collect())
                .unwrap_or_default()
        }

        fn changed_nodes(&mut self) -> usize {
            self.world
                .query::<Entity, Changed<UINode>>()
                .iter(&mut self.world)
                .count()
        }

        fn changed_texts(&mut self) -> usize {
            self.world
                .query::<Entity, Changed<UIText>>()
                .iter(&mut self.world)
                .count()
        }
    }

    fn percent(value: UIValue) -> f32 {
        let UIValue::Percent(percent) = value else {
            panic!("{value:?} is not a percentage");
        };
        percent
    }

    fn frame_x(frame: u32) -> f32 {
        LEFT + frame as f32
    }

    #[test]
    fn the_plugin_builds_a_timeline_and_keeps_it_drawn() {
        let mut app = App::new();
        app.register_plugin(MainSchedulePlugin)
            .register_plugin(TimePlugin)
            .insert_resource(UITheme::default())
            .register_plugin(TimelinePlugin);
        let timeline = app
            .main_mut()
            .world_mut()
            .spawn((UINode::default(), without_a_rate()));
        app.finish_plugin_build();
        app.update();
        app.update();

        let world = app.main_mut().world_mut();
        assert_eq!(
            world
                .get_component_for_entity::<UINode>(timeline)
                .unwrap()
                .min_width,
            UIValue::Px(0.0)
        );
        let readout = world
            .get_component_for_entity::<TimelineParts>(timeline)
            .unwrap()
            .readout;
        assert!(
            world
                .get_component_for_entity::<TimelineReadout>(readout)
                .is_some()
        );
        assert_eq!(
            world
                .get_component_for_entity::<UIText>(readout)
                .unwrap()
                .text,
            "0.00 s"
        );
    }

    #[test]
    fn a_frame_count_holds_both_ends_of_the_clip() {
        assert_eq!(frame_count(1.0, 30.0), 31);
        assert_eq!(frame_count(0.0, 30.0), 1);
        assert_eq!(frame_count(1.01, 30.0), 31);
        assert_eq!(frame_count(1.02, 30.0), 32);
    }

    #[test]
    fn a_frame_count_saturates() {
        assert_eq!(frame_count(f32::MAX, 30.0), u32::MAX);
    }

    #[test]
    fn a_frame_and_its_time_convert_both_ways() {
        for frame in [0, 1, 29, 30, 123, 7839] {
            assert_eq!(frame_at(time_of_frame(frame, 30.0), 30.0), frame);
        }
        assert!((time_of_frame(15, 30.0) - 0.5).abs() < NEAR);
    }

    #[test]
    fn a_time_belongs_to_its_nearest_frame() {
        assert_eq!(frame_at(0.01, 30.0), 0);
        assert_eq!(frame_at(0.02, 30.0), 1);
        assert_eq!(frame_at(0.05, 30.0), 2);
        assert_eq!(frame_at(-1.0, 30.0), 0);
    }

    #[test]
    fn pressing_the_ruler_scrubs_to_the_nearest_frame() {
        let mut f = Fixture::laid_out(at_30_fps());
        let ruler = f.parts().ruler;

        f.press(ruler, frame_x(150) + 0.25);

        let scrubbed = &f.heard().scrubbed;
        assert_eq!(scrubbed.len(), 1);
        assert!((scrubbed[0] - 5.0).abs() < NEAR, "{scrubbed:?}");
    }

    #[test]
    fn dragging_the_ruler_scrubs_only_once_the_pointer_moved() {
        let mut f = Fixture::laid_out(at_30_fps());
        let ruler = f.parts().ruler;

        f.drag(ruler, frame_x(60), 0.0);
        assert!(f.heard().scrubbed.is_empty());

        f.drag(ruler, frame_x(60), 3.0);
        let scrubbed = &f.heard().scrubbed;
        assert_eq!(scrubbed.len(), 1);
        assert!((scrubbed[0] - 2.0).abs() < NEAR, "{scrubbed:?}");
    }

    #[test]
    fn a_scrub_past_either_end_stays_on_the_clip() {
        let mut f = Fixture::laid_out(at_30_fps());
        let ruler = f.parts().ruler;

        f.drag(ruler, LEFT - 50.0, -1.0);
        f.drag(ruler, LEFT + WIDTH + 50.0, 1.0);

        assert_eq!(f.heard().scrubbed, [0.0, 10.0]);
    }

    #[test]
    fn a_snapped_scrub_never_passes_the_end_of_the_clip() {
        let mut f = Fixture::laid_out(Timeline {
            duration: 1.02,
            ..at_30_fps()
        });
        let ruler = f.parts().ruler;

        f.press(ruler, LEFT + WIDTH);

        assert_eq!(f.heard().scrubbed, [1.02]);
    }

    #[test]
    fn without_a_rate_a_scrub_keeps_the_pointers_time() {
        let mut f = Fixture::laid_out(without_a_rate());
        let ruler = f.parts().ruler;

        f.press(ruler, LEFT + 1.0);

        let scrubbed = &f.heard().scrubbed;
        assert_eq!(scrubbed.len(), 1);
        assert!((scrubbed[0] - 10.0 / WIDTH).abs() < NEAR, "{scrubbed:?}");
    }

    #[test]
    fn only_the_left_button_scrubs_or_selects() {
        let mut f = Fixture::laid_out(at_30_fps());
        let (ruler, track) = (f.parts().ruler, f.track());
        let down = |x| UIPointerDown {
            position: Vec2::new(x, 0.0),
            button: MouseButton::Right,
        };

        f.world.trigger_on(ruler, down(frame_x(10)));
        f.world.trigger_on(track, down(frame_x(10)));
        f.world.trigger_on(
            track,
            UIClick {
                position: Vec2::new(frame_x(10), 0.0),
                button: MouseButton::Right,
            },
        );

        assert!(f.heard().scrubbed.is_empty());
        assert!(f.heard().selected.is_empty());
    }

    #[test]
    fn a_drag_on_the_track_selects_the_frames_it_crossed() {
        let mut f = Fixture::laid_out(at_30_fps());
        let track = f.track();

        f.press(track, frame_x(10));
        assert!(f.heard().selected.is_empty());

        f.drag(track, frame_x(10), 0.0);
        assert!(f.heard().selected.is_empty());

        f.drag(track, frame_x(20), 10.0);
        assert_eq!(f.heard().selected, [Some((10, 20))]);
    }

    #[test]
    fn a_backwards_drag_selects_in_order() {
        let mut f = Fixture::laid_out(at_30_fps());
        let track = f.track();

        f.press(track, frame_x(20));
        f.drag(track, frame_x(10), -10.0);

        assert_eq!(f.heard().selected, [Some((10, 20))]);
    }

    #[test]
    fn a_drag_keeps_the_frame_it_started_on() {
        let mut f = Fixture::laid_out(at_30_fps());
        let track = f.track();

        f.press(track, frame_x(20));
        f.drag(track, frame_x(30), 10.0);
        f.drag(track, frame_x(5), -25.0);

        assert_eq!(f.heard().selected, [Some((20, 30)), Some((5, 20))]);
    }

    #[test]
    fn a_click_on_the_track_clears_the_selection() {
        let mut f = Fixture::laid_out(at_30_fps());
        let track = f.track();

        f.press(track, frame_x(10));
        f.click(track, frame_x(10));

        assert_eq!(f.heard().selected, [None]);
    }

    #[test]
    fn without_a_rate_the_track_does_not_select() {
        let mut f = Fixture::laid_out(without_a_rate());
        let track = f.track();

        f.press(track, frame_x(10));
        f.drag(track, frame_x(20), 10.0);
        f.click(track, frame_x(20));

        assert!(f.heard().selected.is_empty());
    }

    #[test]
    fn a_long_take_ticks_every_500_frames() {
        let ticks = ticks_of(260.0, Some(30.0), 1000.0);

        assert_eq!(ticks.len(), 16);
        assert_eq!(ticks[1].label, "500");
        assert_eq!(ticks[15].label, "7500");
        assert!((ticks[1].fraction - 500.0 / 7800.0).abs() < NEAR);
    }

    #[test]
    fn two_seconds_without_a_rate_tick_every_fifth_of_a_second() {
        let ticks = ticks_of(2.0, None, 1000.0);

        assert_eq!(ticks.len(), 11);
        assert_eq!(ticks[0].label, "0.0 s");
        assert_eq!(ticks[3].label, "0.6 s");
        assert_eq!(ticks[10].label, "2.0 s");
        assert!((ticks[10].fraction - 1.0).abs() < NEAR);
    }

    #[test]
    fn ticks_never_step_by_less_than_a_frame() {
        let ticks = ticks_of(0.1, Some(30.0), 1000.0);

        let labels: Vec<_> = ticks.iter().map(|tick| tick.label.as_str()).collect();
        assert_eq!(labels, ["0", "1", "2", "3"]);
    }

    #[test]
    fn a_tick_step_is_one_two_or_five_of_a_power_of_ten() {
        assert_eq!(tick_step(100.0, 640.0), 10.0);
        assert_eq!(tick_step(101.0, 640.0), 20.0);
        assert_eq!(tick_step(200.0, 640.0), 20.0);
        assert_eq!(tick_step(201.0, 640.0), 50.0);
        assert_eq!(tick_step(500.0, 640.0), 50.0);
        assert_eq!(tick_step(501.0, 640.0), 100.0);
    }

    #[test]
    fn labels_stay_at_least_the_gap_apart_at_any_size() {
        for (span, width) in [(7800.0, 1000.0), (2.0, 333.0), (97.0, 1920.0), (1e5, 800.0)] {
            let step = tick_step(span, width);
            assert!(step / span * width >= MIN_TICK_GAP - NEAR, "{span} {width}");
        }
    }

    #[test]
    fn a_clip_with_no_length_has_one_tick() {
        assert_eq!(ticks_of(0.0, Some(30.0), 1000.0).len(), 1);
        assert_eq!(ticks_of(0.0, None, 1000.0).len(), 1);
    }

    #[test]
    fn ticks_are_built_once_the_ruler_has_a_width() {
        let f = Fixture::laid_out(at_30_fps());

        let expected = ticks_of(10.0, Some(30.0), WIDTH).len();
        assert_eq!(f.ruler_children().len(), expected * 2);
        assert_eq!(
            *f.world
                .get_component_for_entity::<TimelineTicks>(f.timeline)
                .unwrap(),
            TimelineTicks {
                width: WIDTH,
                duration: 10.0,
                rate: Some(30.0),
            }
        );
    }

    #[test]
    fn a_width_of_zero_builds_nothing() {
        let mut f = Fixture::new(at_30_fps());
        assert!(f.ruler_children().is_empty());

        f.lay_out(0.0);
        f.frame();

        assert!(f.ruler_children().is_empty());
        assert_eq!(
            *f.world
                .get_component_for_entity::<TimelineTicks>(f.timeline)
                .unwrap(),
            TimelineTicks::default()
        );
    }

    #[test]
    fn hiding_a_built_timeline_keeps_its_ticks() {
        let mut f = Fixture::laid_out(at_30_fps());
        let built = f.ruler_children();

        f.lay_out(0.0);
        f.frame();
        f.lay_out(WIDTH);
        f.frame();

        assert_eq!(f.ruler_children(), built);
    }

    #[test]
    fn ticks_are_not_rebuilt_while_only_the_time_changes() {
        let mut f = Fixture::laid_out(at_30_fps());
        let built = f.ruler_children();

        for time in [1.0, 2.0, 3.0] {
            f.edit(|timeline| timeline.time = time);
            f.frame();
        }

        assert_eq!(f.ruler_children(), built);
    }

    #[test]
    fn ticks_are_rebuilt_when_the_width_duration_or_rate_changes() {
        let mut f = Fixture::laid_out(at_30_fps());

        let before = f.ruler_children();
        f.lay_out(WIDTH * 2.0);
        f.frame();
        assert_ne!(f.ruler_children(), before);

        let before = f.ruler_children();
        f.edit(|timeline| timeline.duration = 20.0);
        f.frame();
        assert_ne!(f.ruler_children(), before);

        let before = f.ruler_children();
        f.edit(|timeline| timeline.rate = None);
        f.frame();
        assert_ne!(f.ruler_children(), before);
    }

    #[test]
    fn the_timeline_is_a_column_of_a_readout_and_two_bands() {
        let f = Fixture::new(at_30_fps());
        let parts = f.parts();

        assert_eq!(f.node(f.timeline).flex_direction, FlexDirection::Column);
        assert_eq!(f.node(parts.ruler).height, UIValue::Px(24.0));
        assert_eq!(f.node(parts.ruler).flex_shrink, 0.0);
        let parent = |entity| **f.world.get_component_for_entity::<ChildOf>(entity).unwrap();
        assert_eq!(parent(parts.readout), f.timeline);
        assert_eq!(parent(parts.ruler), parent(parts.head));
    }

    #[test]
    fn the_head_spans_both_bands_at_its_share_of_the_clip() {
        let mut f = Fixture::new(at_30_fps());
        f.edit(|timeline| timeline.time = 2.5);
        f.frame();

        let head = f.node(f.parts().head);
        assert_eq!(head.position, Position::Absolute);
        assert_eq!(head.inset.top, UIValue::Px(0.0));
        assert_eq!(head.inset.bottom, UIValue::Px(0.0));
        assert!((percent(head.inset.left) - 25.0).abs() < NEAR);
    }

    #[test]
    fn a_head_of_a_clip_with_no_length_rests_at_the_start() {
        let mut f = Fixture::new(Timeline {
            duration: 0.0,
            ..at_30_fps()
        });
        f.edit(|timeline| timeline.time = 1.0);
        f.frame();

        assert_eq!(f.node(f.parts().head).inset.left, UIValue::Percent(0.0));
    }

    #[test]
    fn the_selection_covers_its_frames_and_hides_without_one() {
        let mut f = Fixture::new(at_30_fps());
        assert!(!f.node(f.parts().selection).visible);

        f.edit(|timeline| timeline.selection = Some((75, 150)));
        f.frame();

        let selection = f.node(f.parts().selection);
        assert!(selection.visible);
        assert!((percent(selection.inset.left) - 25.0).abs() < NEAR);
        assert!((percent(selection.width) - 25.0).abs() < NEAR);

        f.edit(|timeline| timeline.selection = None);
        f.frame();
        assert!(!f.node(f.parts().selection).visible);
    }

    #[test]
    fn a_one_frame_selection_still_shows() {
        let mut f = Fixture::new(at_30_fps());
        f.edit(|timeline| timeline.selection = Some((75, 75)));
        f.frame();

        let selection = f.node(f.parts().selection);
        assert!(selection.visible);
        assert_eq!(selection.min_width, UIValue::Px(2.0));
    }

    #[test]
    fn a_selection_without_a_rate_stays_hidden() {
        let mut f = Fixture::new(without_a_rate());
        f.edit(|timeline| timeline.selection = Some((75, 150)));
        f.frame();

        assert!(!f.node(f.parts().selection).visible);
    }

    #[test]
    fn the_readout_shows_the_frame_the_count_and_the_time() {
        let mut f = Fixture::new(Timeline {
            duration: 7839.0 / 30.0,
            ..at_30_fps()
        });
        f.edit(|timeline| timeline.time = 4.1);
        f.frame();

        assert_eq!(f.readout(), "Frame 123 / 7840   4.10 s");
    }

    #[test]
    fn without_a_rate_the_readout_shows_the_time_alone() {
        let mut f = Fixture::new(without_a_rate());
        f.edit(|timeline| timeline.time = 4.1);
        f.frame();

        assert_eq!(f.readout(), "4.10 s");
    }

    #[test]
    fn the_readout_cannot_lay_itself_out_when_its_text_changes() {
        let f = Fixture::new(at_30_fps());
        let readout = f.node(f.parts().readout);

        assert!(matches!(readout.width, UIValue::Px(_)));
        assert!(matches!(readout.height, UIValue::Px(_)));
        assert_eq!(readout.flex_shrink, 0.0);
    }

    #[test]
    fn a_timeline_at_rest_writes_nothing() {
        let mut f = Fixture::laid_out(at_30_fps());
        f.frame();

        f.world.tick();
        f.frame();

        assert_eq!(f.changed_nodes(), 0);
        assert_eq!(f.changed_texts(), 0);
    }

    #[test]
    fn a_time_that_rounds_to_the_same_text_leaves_the_readout_alone() {
        let mut f = Fixture::laid_out(at_30_fps());
        f.edit(|timeline| timeline.time = 1.0);
        f.frame();

        f.world.tick();
        f.edit(|timeline| timeline.time = 1.0001);
        f.frame();

        assert_eq!(f.changed_texts(), 0);
    }
}

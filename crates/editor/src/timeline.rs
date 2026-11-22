use concerto_app::{App, Plugin, schedule_groups::LateUpdate};
use concerto_color::Color;
use concerto_ecs::{
    Component, Entity, IntoSystemConfig, Query, Ref, Res,
    command::CommandQueue,
    query::{
        change_detection::DetectChanges,
        filter::{Added, Changed, Or},
    },
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
    scroll::{SCROLLBAR_WIDTH, UIScrollArea},
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
const MARKER_ROW_HEIGHT: f32 = 16.0;
const TRACK_LABEL_WIDTH: f32 = 160.0;
const TRACK_ROW_HEIGHT: f32 = 18.0;
const KEY_MERGE_PX: f32 = 3.0;
const KEY_INSET: f32 = 4.0;

/// A ruler, a track, a marker lane and track rows over a clip's length, drawn on the UI node that holds it.
#[derive(Component, PartialEq)]
pub struct Timeline {
    pub duration: f32,
    pub rate: Option<f32>,
    pub time: f32,
    pub selection: Option<(u32, u32)>,
    pub selectable: bool,
    pub markers: Vec<TimelineMarker>,
    pub selected_marker: Option<usize>,
    pub tracks: Vec<TimelineTrack>,
}

/// A labelled mark at a time on a timeline's marker lane.
#[derive(Clone, Debug, PartialEq)]
pub struct TimelineMarker {
    pub time: f32,
    pub label: String,
    pub locked: bool,
}

/// A read-only row under the marker lane: a label and the times it has keys at.
#[derive(Clone, Debug, PartialEq)]
pub struct TimelineTrack {
    pub label: String,
    pub warning: bool,
    pub keys: Vec<f32>,
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
        if !self.selectable {
            return None;
        }
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

/// Sent when a marker is pressed (`Some`) or the track or the lane's empty space is clicked (`None`).
pub struct TimelineMarkerSelected {
    pub index: Option<usize>,
}

impl Signal for TimelineMarkerSelected {}
impl EntitySignal for TimelineMarkerSelected {}

/// Sent while an unlocked marker is dragged, with the time under the pointer, snapped to a frame when there is a rate.
pub struct TimelineMarkerMoved {
    pub index: usize,
    pub time: f32,
}

impl Signal for TimelineMarkerMoved {}
impl EntitySignal for TimelineMarkerMoved {}

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
    track: Entity,
    selection: Entity,
    lane_band: Entity,
    events_label: Entity,
    lane: Entity,
    rows: Entity,
    rows_content: Entity,
    overlay: Entity,
    head: Entity,
}

#[derive(Component, Default)]
struct TimelineMarkersBuilt {
    markers: Vec<BuiltMarker>,
}

struct BuiltMarker {
    label: String,
    locked: bool,
    node: Entity,
    line: Entity,
}

impl TimelineMarkersBuilt {
    fn holds(&self, markers: &[TimelineMarker]) -> bool {
        self.markers.len() == markers.len()
            && self
                .markers
                .iter()
                .zip(markers)
                .all(|(built, marker)| built.label == marker.label && built.locked == marker.locked)
    }
}

#[derive(Component, Debug, Default, PartialEq)]
struct TimelineTicks {
    width: f32,
    duration: f32,
    rate: Option<f32>,
}

#[derive(Component, Default)]
struct TimelineRows {
    width: f32,
    tracks: Vec<TimelineTrack>,
}

#[derive(Component)]
struct TimelineSelectionStart(u32);

#[derive(Debug, PartialEq)]
struct KeySpan {
    start: f32,
    end: f32,
}

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

fn key_spans(keys: &[f32], duration: f32, width: f32) -> Vec<KeySpan> {
    let mut spans: Vec<KeySpan> = Vec::new();
    for &key in keys {
        let at = fraction(key, duration);
        match spans.last_mut() {
            Some(span) if (at - span.end) * width <= KEY_MERGE_PX => span.end = span.end.max(at),
            _ => spans.push(KeySpan { start: at, end: at }),
        }
    }
    spans
}

fn band_margin(columns: bool) -> UIRect {
    if columns {
        UIRect {
            left: TRACK_LABEL_WIDTH,
            right: SCROLLBAR_WIDTH,
            ..Default::default()
        }
    } else {
        UIRect::default()
    }
}

fn beside_the_column(columns: bool) -> UIRect {
    UIRect {
        left: 0.0,
        ..band_margin(columns)
    }
}

fn overlay_inset(columns: bool) -> UIInset {
    let margin = band_margin(columns);
    UIInset {
        left: UIValue::Px(margin.left),
        right: UIValue::Px(margin.right),
        ..across_the_band()
    }
}

fn set_margin(nodes: &mut Query<&mut UINode>, entity: Entity, margin: UIRect) {
    if let Some(mut node) = nodes.get_entity_mut(entity)
        && node.margin != margin
    {
        node.margin = margin;
    }
}

fn set_visible(nodes: &mut Query<&mut UINode>, entity: Entity, visible: bool) {
    if let Some(mut node) = nodes.get_entity_mut(entity)
        && node.visible != visible
    {
        node.visible = visible;
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
    if !view.selectable {
        return None;
    }
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
        if on.signal().button != MouseButton::Left {
            return;
        }
        let selects = timelines
            .get_entity(timeline)
            .is_some_and(|view| view.selectable && view.rate.is_some());
        let mut target = cmd.entity(timeline);
        target.trigger(TimelineMarkerSelected { index: None });
        if selects {
            target.trigger(TimelineSelected { range: None });
        }
    })
    .into_listener()
}

fn press_marker(timeline: Entity, index: usize) -> Listener<UIPointerDown> {
    (move |on: On<UIPointerDown>, mut cmd: CommandQueue| {
        if on.signal().button == MouseButton::Left {
            cmd.entity(timeline)
                .trigger(TimelineMarkerSelected { index: Some(index) });
        }
    })
    .into_listener()
}

fn drag_marker(timeline: Entity, lane: Entity, index: usize) -> Listener<UIDrag> {
    (move |on: On<UIDrag>,
           timelines: Query<&Timeline>,
           layouts: Query<&UILayout>,
           mut cmd: CommandQueue| {
        let signal = on.signal();
        if signal.button != MouseButton::Left || signal.delta == Vec2::ZERO {
            return;
        }
        let (Some(view), Some(layout)) = (timelines.get_entity(timeline), layouts.get_entity(lane))
        else {
            return;
        };
        if view.markers.get(index).is_none_or(|marker| marker.locked) {
            return;
        }
        let time = view.time_at(signal.position.x, layout.rect);
        cmd.entity(timeline)
            .trigger(TimelineMarkerMoved { index, time });
    })
    .into_listener()
}

fn marker_rows(markers: &[TimelineMarker]) -> Vec<u32> {
    markers
        .iter()
        .enumerate()
        .map(|(index, marker)| {
            markers[..index]
                .iter()
                .filter(|earlier| earlier.time == marker.time)
                .count() as u32
        })
        .collect()
}

fn marker_color(marker: &TimelineMarker, selected: bool, theme: &UITheme) -> Color {
    if selected {
        theme.selection()
    } else if marker.locked {
        theme.text_muted
    } else {
        theme.accent
    }
}

fn marker_inset(timeline: &Timeline, marker: &TimelineMarker, row: u32) -> UIInset {
    UIInset {
        left: UIValue::Percent(fraction(marker.time, timeline.duration) * 100.0),
        top: UIValue::Px(row as f32 * MARKER_ROW_HEIGHT),
        ..Default::default()
    }
}

fn across_the_band() -> UIInset {
    UIInset {
        top: UIValue::Px(0.0),
        bottom: UIValue::Px(0.0),
        ..Default::default()
    }
}

fn setup_timeline(
    mut timelines: Query<(Entity, &mut UINode), Added<Timeline>>,
    theme: Res<UITheme>,
    mut cmd: CommandQueue,
) {
    for (timeline, mut node) in timelines.iter_mut() {
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
        let track = track.entity();
        let mut lane_band = bands.spawn_child(
            UINode::default()
                .with_flex_direction(FlexDirection::Row)
                .with_height(UIValue::Px(0.0))
                .with_flex_shrink(0.0)
                .with_visible(false),
        );
        let events_label = lane_band
            .spawn_child(
                theme
                    .label("Events")
                    .small()
                    .muted()
                    .single_line()
                    .size(
                        UIValue::Px(TRACK_LABEL_WIDTH),
                        UIValue::Px(MARKER_ROW_HEIGHT),
                    )
                    .fixed()
                    .clipped()
                    .hidden(),
            )
            .entity();
        let lane = lane_band
            .spawn_child((
                UINode::default().with_flex_grow(1.0).clipped(),
                Interactable,
                click_track(timeline),
            ))
            .entity();
        let lane_band = lane_band.entity();
        let mut rows = bands.spawn_child((
            UINode::default()
                .with_flex_grow(1.0)
                .with_visible(false)
                .clipped(),
            Interactable,
        ));
        let rows_content = rows
            .spawn_child(UINode::default().with_flex_direction(FlexDirection::Column))
            .entity();
        rows.insert(UIScrollArea {
            content: Some(rows_content),
            ..Default::default()
        });
        let rows = rows.entity();
        let mut overlay = bands.spawn_child(
            UINode::default()
                .with_position(Position::Absolute)
                .with_inset(overlay_inset(false)),
        );
        let head = overlay
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
        let overlay = overlay.entity();
        root.insert((
            TimelineParts {
                readout,
                ruler,
                track,
                selection,
                lane_band,
                events_label,
                lane,
                rows,
                rows_content,
                overlay,
                head,
            },
            TimelineTicks::default(),
            TimelineMarkersBuilt::default(),
            TimelineRows::default(),
        ));
    }
}

fn sync_timeline_parts(
    timelines: Query<(&Timeline, &TimelineParts), Or<(Changed<Timeline>, Changed<TimelineParts>)>>,
    mut nodes: Query<&mut UINode>,
    mut texts: Query<&mut UIText>,
) {
    for (timeline, parts) in timelines.iter() {
        if let Some(mut text) = texts.get_entity_mut(parts.readout) {
            let readout = timeline.readout();
            if text.text != readout {
                text.text = readout;
            }
        }
        let columns = !timeline.tracks.is_empty();
        set_margin(&mut nodes, parts.ruler, band_margin(columns));
        set_margin(&mut nodes, parts.track, band_margin(columns));
        set_margin(&mut nodes, parts.lane_band, beside_the_column(columns));
        set_visible(&mut nodes, parts.events_label, columns);
        set_visible(&mut nodes, parts.rows, columns);
        if let Some(mut track) = nodes.get_entity_mut(parts.track) {
            let (grow, height) = if columns {
                (0.0, UIValue::Px(TRACK_ROW_HEIGHT))
            } else {
                (1.0, UIValue::Auto)
            };
            if track.flex_grow != grow {
                track.flex_grow = grow;
            }
            if track.height != height {
                track.height = height;
            }
        }
        if let Some(mut overlay) = nodes.get_entity_mut(parts.overlay) {
            let inset = overlay_inset(columns);
            if overlay.inset != inset {
                overlay.inset = inset;
            }
        }
        if let Some(mut head) = nodes.get_entity_mut(parts.head) {
            let left = UIValue::Percent(fraction(timeline.time, timeline.duration) * 100.0);
            if head.inset.left != left {
                head.inset.left = left;
            }
        }
        if let Some(mut selection) = nodes.get_entity_mut(parts.selection) {
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

fn sync_timeline_markers(
    mut timelines: Query<
        (Entity, &Timeline, &TimelineParts, &mut TimelineMarkersBuilt),
        Or<(Changed<Timeline>, Changed<TimelineParts>)>,
    >,
    mut nodes: Query<&mut UINode>,
    mut materials: Query<&mut UIMaterial>,
    theme: Res<UITheme>,
    mut cmd: CommandQueue,
) {
    for (entity, timeline, parts, mut built) in timelines.iter_mut() {
        let rows = marker_rows(&timeline.markers);
        if let Some(mut lane) = nodes.get_entity_mut(parts.lane_band) {
            let shown = !timeline.markers.is_empty();
            if lane.visible != shown {
                lane.visible = shown;
            }
            let height = rows.iter().max().map_or(0, |row| row + 1);
            let height = UIValue::Px(height as f32 * MARKER_ROW_HEIGHT);
            if lane.height != height {
                lane.height = height;
            }
        }
        if !built.holds(&timeline.markers) {
            let mut lane = cmd.entity(parts.lane).despawn_children();
            let markers: Vec<BuiltMarker> = timeline
                .markers
                .iter()
                .zip(&rows)
                .enumerate()
                .map(|(index, (marker, &row))| {
                    let selected = timeline.selected_marker == Some(index);
                    let mut node = lane.spawn_child((
                        UINode::default()
                            .with_position(Position::Absolute)
                            .with_inset(marker_inset(timeline, marker, row))
                            .with_height(UIValue::Px(MARKER_ROW_HEIGHT))
                            .with_flex_direction(FlexDirection::Row)
                            .with_gap(Vec2::new(theme.spacing_xs, 0.0))
                            .with_margin(UIRect {
                                left: -HEAD_WIDTH / 2.0,
                                ..Default::default()
                            }),
                        Interactable,
                        press_marker(entity, index),
                        drag_marker(entity, parts.lane, index),
                    ));
                    let line = node
                        .spawn_child((
                            UINode::default()
                                .with_width(UIValue::Px(HEAD_WIDTH))
                                .with_height(UIValue::Px(MARKER_ROW_HEIGHT))
                                .with_flex_shrink(0.0),
                            UIMaterial::flat(marker_color(marker, selected, &theme)),
                        ))
                        .entity();
                    node.add_child(theme.label(marker.label.clone()).small().no_wrap().fixed());
                    BuiltMarker {
                        label: marker.label.clone(),
                        locked: marker.locked,
                        node: node.entity(),
                        line,
                    }
                })
                .collect();
            built.markers = markers;
            continue;
        }
        for (index, ((marker, &row), shown)) in timeline
            .markers
            .iter()
            .zip(&rows)
            .zip(&built.markers)
            .enumerate()
        {
            if let Some(mut node) = nodes.get_entity_mut(shown.node) {
                let inset = marker_inset(timeline, marker, row);
                if node.inset != inset {
                    node.inset = inset;
                }
            }
            if let Some(mut material) = materials.get_entity_mut(shown.line) {
                let selected = timeline.selected_marker == Some(index);
                let color = marker_color(marker, selected, &theme).to_linear();
                if material.color != color {
                    material.color = color;
                }
            }
        }
    }
}

fn sync_timeline_ticks(
    mut timelines: Query<(&Timeline, &TimelineParts, &mut TimelineTicks)>,
    layouts: Query<&UILayout>,
    theme: Res<UITheme>,
    mut cmd: CommandQueue,
) {
    for (timeline, parts, mut built) in timelines.iter_mut() {
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

fn sync_timeline_rows(
    mut timelines: Query<(Ref<Timeline>, &TimelineParts, &mut TimelineRows)>,
    layouts: Query<&UILayout>,
    mut areas: Query<&mut UIScrollArea>,
    theme: Res<UITheme>,
    mut cmd: CommandQueue,
) {
    for (timeline, parts, mut built) in timelines.iter_mut() {
        let width = layouts
            .get_entity(parts.ruler)
            .map_or(0.0, |layout| layout.rect.size.x);
        let differs = timeline.is_changed() && timeline.tracks != built.tracks;
        let resized = width > 0.0 && width != built.width && !timeline.tracks.is_empty();
        if !differs && !resized {
            continue;
        }
        let mut content = cmd.entity(parts.rows_content).despawn_children();
        for track in &timeline.tracks {
            let color = if track.warning {
                theme.warning
            } else {
                theme.text
            };
            let mut row = content.spawn_child(
                UINode::default()
                    .with_flex_direction(FlexDirection::Row)
                    .with_height(UIValue::Px(TRACK_ROW_HEIGHT))
                    .with_flex_shrink(0.0)
                    .with_margin(beside_the_column(true)),
            );
            row.add_child(
                theme
                    .label(track.label.clone())
                    .small()
                    .color(color)
                    .single_line()
                    .size(
                        UIValue::Px(TRACK_LABEL_WIDTH),
                        UIValue::Px(TRACK_ROW_HEIGHT),
                    )
                    .fixed()
                    .clipped(),
            );
            let mut keys = row.spawn_child(UINode::default().with_flex_grow(1.0).clipped());
            for span in key_spans(&track.keys, timeline.duration, width) {
                let span_width = if span.end > span.start {
                    UIValue::Percent((span.end - span.start) * 100.0)
                } else {
                    UIValue::Px(TICK_WIDTH)
                };
                keys.add_child((
                    UINode::default()
                        .with_position(Position::Absolute)
                        .with_inset(UIInset {
                            left: UIValue::Percent(span.start * 100.0),
                            top: UIValue::Px(KEY_INSET),
                            bottom: UIValue::Px(KEY_INSET),
                            ..Default::default()
                        })
                        .with_width(span_width)
                        .with_min_width(UIValue::Px(TICK_WIDTH)),
                    UIMaterial::flat(theme.text_muted),
                ));
            }
        }
        built.width = width;
        built.tracks = timeline.tracks.clone();
        let Some(mut area) = areas.get_entity_mut(parts.rows) else {
            continue;
        };
        let extent = timeline.tracks.len() as f32 * TRACK_ROW_HEIGHT;
        let offset = layouts
            .get_entity(parts.rows)
            .map_or(area.offset, |layout| {
                area.offset
                    .min((extent - layout.content_rect.size.y).max(0.0))
            });
        if area.content_extent != extent || area.offset != offset {
            area.content_extent = extent;
            area.offset = offset;
        }
    }
}

/// Builds and keeps every [`Timeline`] drawn; `EditorPlugin` registers it.
pub struct TimelinePlugin;

impl Plugin for TimelinePlugin {
    fn build(&self, app: &mut App) {
        app.add_system(LateUpdate, setup_timeline.in_set(UiSet::Setup))
            .add_system(
                LateUpdate,
                (
                    sync_timeline_parts,
                    sync_timeline_markers,
                    sync_timeline_ticks,
                    sync_timeline_rows,
                )
                    .in_set(UiSet::Project),
            );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use concerto_ecs::{
        IntoSystem, ResMut, Resource, System, World,
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
        picked: Vec<Option<usize>>,
        moved: Vec<TimelineMarkerMoved>,
    }

    fn at_30_fps() -> Timeline {
        Timeline {
            duration: 10.0,
            rate: Some(30.0),
            time: 0.0,
            selection: None,
            selectable: true,
            markers: Vec::new(),
            selected_marker: None,
            tracks: Vec::new(),
        }
    }

    fn marker(time: f32, label: &str) -> TimelineMarker {
        TimelineMarker {
            time,
            label: label.to_owned(),
            locked: false,
        }
    }

    struct Fixture {
        world: World,
        setup: BoxedSystem,
        parts: BoxedSystem,
        markers: BoxedSystem,
        timeline: Entity,
    }

    impl Fixture {
        fn laid_out(timeline: Timeline) -> Self {
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
                (|on: On<TimelineMarkerSelected>, mut heard: ResMut<Heard>| {
                    heard.picked.push(on.signal().index);
                })
                .into_listener(),
                (|on: On<TimelineMarkerMoved>, mut heard: ResMut<Heard>| {
                    let signal = on.signal();
                    heard.moved.push(TimelineMarkerMoved {
                        index: signal.index,
                        time: signal.time,
                    });
                })
                .into_listener(),
            ));
            let mut setup = setup_timeline.into_boxed_system();
            let mut parts = sync_timeline_parts.into_boxed_system();
            let mut markers = sync_timeline_markers.into_boxed_system();
            setup.initialize(&mut world);
            parts.initialize(&mut world);
            markers.initialize(&mut world);
            let mut fixture = Self {
                world,
                setup,
                parts,
                markers,
                timeline,
            };
            fixture.frame();
            fixture.lay_out();
            fixture.frame();
            fixture
        }

        fn frame(&mut self) {
            self.setup.run_and_apply((), &mut self.world);
            self.parts.run_and_apply((), &mut self.world);
            self.markers.run_and_apply((), &mut self.world);
        }

        fn lay_out(&mut self) {
            let rect = UIBox {
                min: Vec2::new(LEFT, 0.0),
                size: Vec2::new(WIDTH, 24.0),
            };
            let layout = UILayout {
                rect,
                content_rect: rect,
                clip_rect: rect,
                paint_order: 0,
            };
            let (ruler, track, lane) = (self.parts().ruler, self.track(), self.parts().lane);
            self.world.insert(layout.clone(), ruler);
            self.world.insert(layout.clone(), track);
            self.world.insert(layout, lane);
        }

        fn parts(&self) -> &TimelineParts {
            self.world
                .get_component_for_entity::<TimelineParts>(self.timeline)
                .unwrap()
        }

        fn track(&self) -> Entity {
            **self
                .world
                .get_component_for_entity::<ChildOf>(self.parts().selection)
                .unwrap()
        }

        fn marker_nodes(&self) -> Vec<Entity> {
            self.world
                .get_component_for_entity::<Children>(self.parts().lane)
                .map(|children| children.iter().copied().collect())
                .unwrap_or_default()
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
    }

    fn frame_x(frame: u32) -> f32 {
        LEFT + frame as f32
    }

    #[test]
    fn frames_and_times_convert_both_ways_and_a_count_holds_both_ends() {
        assert_eq!(frame_count(1.0, 30.0), 31);
        assert_eq!(frame_count(0.0, 30.0), 1);
        assert_eq!(frame_count(1.02, 30.0), 32);
        assert_eq!(frame_count(f32::MAX, 30.0), u32::MAX);
        for frame in [0, 1, 29, 30, 123, 7839] {
            assert_eq!(frame_at(time_of_frame(frame, 30.0), 30.0), frame);
        }
        assert_eq!(frame_at(0.01, 30.0), 0);
        assert_eq!(frame_at(0.02, 30.0), 1);
        assert_eq!(frame_at(-1.0, 30.0), 0);
    }

    #[test]
    fn a_tick_step_is_one_two_or_five_of_a_power_of_ten_and_keeps_labels_apart() {
        for [span, step] in [[100.0, 10.0], [101.0, 20.0], [201.0, 50.0], [501.0, 100.0]] {
            assert_eq!(tick_step(span, 640.0), step, "{span}");
        }
        for [span, width] in [[7800.0, 1000.0], [2.0, 333.0], [97.0, 1920.0], [1e5, 800.0]] {
            let step = tick_step(span, width);
            assert!(step / span * width >= MIN_TICK_GAP - NEAR, "{span} {width}");
        }
    }

    #[test]
    fn the_ruler_scrubs_to_the_nearest_frame_once_the_pointer_moves_and_stays_on_the_clip() {
        let mut f = Fixture::laid_out(at_30_fps());
        let ruler = f.parts().ruler;

        f.press(ruler, frame_x(150) + 0.25);
        f.drag(ruler, frame_x(60), 0.0);
        f.drag(ruler, frame_x(60), 3.0);
        f.drag(ruler, LEFT - 50.0, -1.0);
        f.drag(ruler, LEFT + WIDTH + 50.0, 1.0);

        let scrubbed = &f.heard().scrubbed;
        assert_eq!(scrubbed.len(), 4, "{scrubbed:?}");
        assert!((scrubbed[0] - 5.0).abs() < NEAR, "{scrubbed:?}");
        assert!((scrubbed[1] - 2.0).abs() < NEAR, "{scrubbed:?}");
        assert_eq!(scrubbed[2..], [0.0, 10.0]);
    }

    #[test]
    fn a_drag_on_the_track_selects_the_frames_it_crossed_and_a_click_clears_them() {
        let mut f = Fixture::laid_out(at_30_fps());
        let track = f.track();

        f.press(track, frame_x(20));
        f.drag(track, frame_x(20), 0.0);
        assert!(f.heard().selected.is_empty());

        f.drag(track, frame_x(30), 10.0);
        f.drag(track, frame_x(5), -25.0);
        f.click(track, frame_x(5));

        assert_eq!(f.heard().selected, [Some((20, 30)), Some((5, 20)), None]);
    }

    #[test]
    fn a_marker_is_picked_by_a_press_and_moved_by_a_drag_unless_it_is_locked() {
        let mut f = Fixture::laid_out(Timeline {
            markers: vec![
                marker(1.0, "footstep"),
                TimelineMarker {
                    locked: true,
                    ..marker(2.0, "Impact")
                },
            ],
            ..at_30_fps()
        });
        let (markers, track) = (f.marker_nodes(), f.track());

        f.press(markers[1], frame_x(60));
        f.click(track, frame_x(10));
        f.drag(markers[0], frame_x(90) + 0.25, 3.0);
        f.drag(markers[1], frame_x(120), 3.0);

        assert_eq!(f.heard().picked, [Some(1), None]);
        let moved = &f.heard().moved;
        assert_eq!(moved.len(), 1);
        assert_eq!(moved[0].index, 0);
        assert!((moved[0].time - 3.0).abs() < NEAR, "{}", moved[0].time);
    }
}

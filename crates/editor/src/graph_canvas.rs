pub mod layout;

use concerto_app::{
    App, Plugin,
    schedule_groups::{LateUpdate, Startup, Update},
};
use concerto_color::Color;
use concerto_ecs::{
    Component, Entity, IntoSystemConfig, Query, Ref, Res, ResMut, Resource,
    command::CommandQueue,
    events::event_reader::EventReader,
    query::{change_detection::DetectChanges, filter::Added},
    signal::{
        EntitySignal, On, Signal,
        listener::{IntoListener, Listener},
    },
};
use concerto_ui::elements::prelude::*;
use concerto_ui::{
    focus::FocusedWidget,
    interaction::{
        HoveredNode, Interactable, UIClick, UIDoubleClick, UIDrag, UIPointerDown, UIPointerUp,
    },
    material::UIMaterial,
    node::{AlignItems, FlexDirection, Position, UIBox, UIInset, UILayout, UINode, UIRect},
    sets::UiSet,
    text::UIText,
    theme::UITheme,
    transform::UIValue,
};
use concerto_window::input::{
    KeyCode, MouseButton,
    actions::{ActionFired, ActionMap, Shortcut},
};
use concerto_window::{define_action, define_context};
use glam::Vec2;

use crate::actions::{EditorContextSet, ImportDialogContext, PromptContext};
use crate::fonts::{ICON_SIZE, glyph, icon};
use layout::{
    ArrowDirection, CanvasLayout, HEADER_HEIGHT, LinkRoute, NODE_WIDTH, PIN_ROW_HEIGHT,
    SUBTITLE_HEIGHT,
};

const TEXT_INSET: f32 = 8.0;
const BADGE_WIDTH: f32 = 48.0;
const PIN_SIZE: f32 = 8.0;
const LINK_WIDTH: f32 = 2.0;
const BORDER_WIDTH: f32 = 1.0;
const MARKED_BORDER_WIDTH: f32 = 2.0;
const ARROW_OVERLAP: f32 = 4.0;
const LABEL_OFFSET: f32 = 4.0;
const FRAME_MARGIN: f32 = 24.0;

define_action!(
    /// Delete what the canvas under the pointer, or with focus, has selected.
    DeleteCanvasSelection
);
define_action!(
    /// Activate the node the canvas under the pointer, or with focus, has selected.
    ActivateCanvasSelection
);
define_context!(
    /// Active while the pointer is over a graph canvas or one has keyboard focus.
    CanvasContext
);

/// The id of a node on a canvas, chosen by the canvas's owner.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CanvasNodeId(pub u32);

/// How a canvas draws and lays out what it holds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CanvasMode {
    /// Pins on the sides and links pin to pin, laid out as trees growing leftward.
    #[default]
    Flow,
    /// No pins and links as arrows box to box, laid out in layers left to right.
    Diagram,
}

/// A mark that sets a node or a link apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CanvasEmphasis {
    Active,
    Warning,
}

/// What a canvas can have selected: a node, or a link by its index in `links`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CanvasTarget {
    Node(CanvasNodeId),
    Link(usize),
}

/// What lies under the pointer on a canvas.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CanvasPart {
    Node(CanvasNodeId),
    Input { node: CanvasNodeId, slot: usize },
    Output(CanvasNodeId),
    Link(usize),
    Empty,
}

/// A box on a canvas.
#[derive(Clone, Debug, PartialEq)]
pub struct CanvasNode {
    pub id: CanvasNodeId,
    pub title: String,
    pub subtitle: Option<String>,
    pub inputs: Vec<String>,
    pub output: bool,
    pub badge: Option<String>,
    pub emphasis: Option<CanvasEmphasis>,
}

impl CanvasNode {
    /// A node with a title and nothing else.
    pub fn new(id: CanvasNodeId, title: impl Into<String>) -> Self {
        Self {
            id,
            title: title.into(),
            subtitle: None,
            inputs: Vec::new(),
            output: false,
            badge: None,
            emphasis: None,
        }
    }

    /// A muted second line under the title.
    pub fn with_subtitle(mut self, subtitle: impl Into<String>) -> Self {
        self.subtitle = Some(subtitle.into());
        self
    }

    /// Adds one input pin, labelled, under the pins the node has.
    pub fn with_input(mut self, label: impl Into<String>) -> Self {
        self.inputs.push(label.into());
        self
    }

    /// Gives the node an output pin.
    pub fn with_output(mut self) -> Self {
        self.output = true;
        self
    }

    /// A short readout at the right of the title row.
    pub fn with_badge(mut self, badge: impl Into<String>) -> Self {
        self.badge = Some(badge.into());
        self
    }

    /// Outlines the node in the emphasis's colour.
    pub fn with_emphasis(mut self, emphasis: CanvasEmphasis) -> Self {
        self.emphasis = Some(emphasis);
        self
    }
}

/// A link from one node's output to another node.
#[derive(Clone, Debug, PartialEq)]
pub struct CanvasLink {
    pub from: CanvasNodeId,
    pub to: CanvasNodeId,
    pub slot: Option<usize>,
    pub label: Option<String>,
    pub emphasis: Option<CanvasEmphasis>,
}

impl CanvasLink {
    /// A link out of `from` and into `to`.
    pub fn new(from: CanvasNodeId, to: CanvasNodeId) -> Self {
        Self {
            from,
            to,
            slot: None,
            label: None,
            emphasis: None,
        }
    }

    /// The input pin of `to` the link ends on, in `Flow`.
    pub fn with_slot(mut self, slot: usize) -> Self {
        self.slot = Some(slot);
        self
    }

    /// A label beside the link's vertical run.
    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Draws the link in the emphasis's colour.
    pub fn with_emphasis(mut self, emphasis: CanvasEmphasis) -> Self {
        self.emphasis = Some(emphasis);
        self
    }
}

/// Nodes and the links between them, drawn on the UI node that holds it.
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct GraphCanvas {
    pub nodes: Vec<CanvasNode>,
    pub links: Vec<CanvasLink>,
    pub mode: CanvasMode,
    pub selection: Option<CanvasTarget>,
}

impl GraphCanvas {
    /// An empty canvas in `mode`.
    pub fn new(mode: CanvasMode) -> Self {
        Self {
            mode,
            ..Default::default()
        }
    }

    /// Adds one node.
    pub fn with_node(mut self, node: CanvasNode) -> Self {
        self.nodes.push(node);
        self
    }

    /// Adds one link.
    pub fn with_link(mut self, link: CanvasLink) -> Self {
        self.links.push(link);
        self
    }
}

/// Sent to a [`GraphCanvas`] when a node or a link is pressed (`Some`) or empty space is clicked (`None`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanvasSelected {
    pub target: Option<CanvasTarget>,
}

/// Sent to a [`GraphCanvas`] when a node is double-clicked, or Enter is pressed with it selected.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanvasActivated {
    pub node: CanvasNodeId,
}

/// Sent to a [`GraphCanvas`] when a drag from an output pin ends on an input pin, or in `Diagram` a drag from one box ends on another.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanvasConnected {
    pub from: CanvasNodeId,
    pub to: CanvasNodeId,
    pub slot: Option<usize>,
}

/// Sent to a [`GraphCanvas`] when a connect drag from `from` ends on empty space, with the pointer's window position.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanvasLinkDropped {
    pub from: CanvasNodeId,
    pub at: Vec2,
}

/// Sent to a [`GraphCanvas`] when Delete is pressed with something selected.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanvasDeleted {
    pub target: CanvasTarget,
}

/// Sent to a [`GraphCanvas`] on a right click, with the pointer's window position and what it is on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanvasMenuRequested {
    pub at: Vec2,
    pub on: CanvasPart,
}

/// Send to a [`GraphCanvas`] to scroll a node into view.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameCanvasNode {
    pub node: CanvasNodeId,
}

macro_rules! canvas_signals {
    ($($signal:ty),* $(,)?) => {
        $(
            impl Signal for $signal {}
            impl EntitySignal for $signal {}
        )*
    };
}

canvas_signals!(
    CanvasSelected,
    CanvasActivated,
    CanvasConnected,
    CanvasLinkDropped,
    CanvasDeleted,
    CanvasMenuRequested,
    FrameCanvasNode,
);

/// On a node's badge and subtitle, whose text an owner may rewrite on every frame of playback.
#[derive(Component)]
pub(crate) struct CanvasReadout;

#[derive(Component)]
struct CanvasParts {
    surface: Entity,
    content: Entity,
    items: Entity,
    preview: [Entity; 3],
}

#[derive(Clone, Copy, PartialEq)]
struct Connecting {
    from: CanvasNodeId,
    start: Vec2,
}

#[derive(Component, Default)]
struct CanvasView {
    offset: Vec2,
    framing: Option<CanvasNodeId>,
    connecting: Option<Connecting>,
}

struct BuiltNode {
    frame: Entity,
    badge: Entity,
    subtitle: Option<Entity>,
}

struct BuiltLink {
    link: usize,
    segments: [Entity; 3],
    arrow: Option<Entity>,
}

#[derive(Component, Default)]
struct CanvasBuilt {
    shown: Option<GraphCanvas>,
    layout: CanvasLayout,
    nodes: Vec<BuiltNode>,
    links: Vec<BuiltLink>,
}

struct Pointed {
    part: CanvasPart,
    point: Vec2,
}

impl CanvasBuilt {
    fn draws(&self, mode: CanvasMode) -> bool {
        self.shown.as_ref().is_some_and(|shown| shown.mode == mode)
    }

    fn pointed(&self, position: Vec2, surface: &UILayout, offset: Vec2) -> Pointed {
        let point = position - surface.content_rect.min - offset;
        Pointed {
            part: self.layout.part_at(point),
            point,
        }
    }
}

#[derive(Resource, Default)]
struct KeyedCanvas(Option<Entity>);

fn same_structure(shown: &GraphCanvas, wanted: &GraphCanvas) -> bool {
    shown.mode == wanted.mode
        && shown.nodes.len() == wanted.nodes.len()
        && shown.links.len() == wanted.links.len()
        && shown
            .nodes
            .iter()
            .zip(&wanted.nodes)
            .all(|(shown, wanted)| {
                shown.id == wanted.id
                    && shown.title == wanted.title
                    && shown.inputs == wanted.inputs
                    && shown.output == wanted.output
                    && shown.subtitle.is_some() == wanted.subtitle.is_some()
            })
        && shown
            .links
            .iter()
            .zip(&wanted.links)
            .all(|(shown, wanted)| {
                shown.from == wanted.from
                    && shown.to == wanted.to
                    && shown.slot == wanted.slot
                    && shown.label == wanted.label
            })
}

fn mark_color(emphasis: Option<CanvasEmphasis>, selected: bool, theme: &UITheme) -> Option<Color> {
    if selected {
        return Some(theme.accent);
    }
    emphasis.map(|emphasis| match emphasis {
        CanvasEmphasis::Active => theme.accent_secondary,
        CanvasEmphasis::Warning => theme.warning,
    })
}

fn node_border(node: &CanvasNode, selected: bool, theme: &UITheme) -> Color {
    mark_color(node.emphasis, selected, theme).unwrap_or(theme.border)
}

fn node_border_width(node: &CanvasNode, selected: bool) -> f32 {
    if selected || node.emphasis.is_some() {
        MARKED_BORDER_WIDTH
    } else {
        BORDER_WIDTH
    }
}

fn link_color(link: &CanvasLink, selected: bool, theme: &UITheme) -> Color {
    mark_color(link.emphasis, selected, theme).unwrap_or(theme.text_muted)
}

fn at_box(rect: UIBox) -> UINode {
    UINode::default()
        .with_position(Position::Absolute)
        .with_inset(UIInset {
            left: UIValue::Px(rect.min.x),
            top: UIValue::Px(rect.min.y),
            ..Default::default()
        })
        .with_size(UIValue::Px(rect.size.x), UIValue::Px(rect.size.y))
}

fn square(centre: Vec2, side: f32) -> UIBox {
    UIBox {
        min: centre - Vec2::splat(side / 2.0),
        size: Vec2::splat(side),
    }
}

fn arrow_box(route: &LinkRoute, direction: ArrowDirection, theme: &UITheme) -> UIBox {
    let height = theme.line_height(ICON_SIZE);
    let left = match direction {
        ArrowDirection::Right => route.end.x - ICON_SIZE + ARROW_OVERLAP,
        ArrowDirection::Left => route.end.x - ARROW_OVERLAP,
    };
    UIBox {
        min: Vec2::new(left, route.end.y - height / 2.0),
        size: Vec2::new(ICON_SIZE, height),
    }
}

fn show_preview(nodes: &Query<&mut UINode>, preview: [Entity; 3], route: Option<LinkRoute>) {
    let segments = route.map(|route| route.segments());
    for (index, entity) in preview.into_iter().enumerate() {
        let Some(mut node) = nodes.get_entity(entity) else {
            continue;
        };
        let wanted = match &segments {
            Some(segments) => at_box(segments[index].bounds(LINK_WIDTH)),
            None => UINode::default()
                .with_position(Position::Absolute)
                .with_visible(false),
        };
        if **node != wanted {
            **node = wanted;
        }
    }
}

fn press_surface(canvas: Entity) -> Listener<UIPointerDown> {
    (move |on: On<UIPointerDown>,
           canvases: Query<(&GraphCanvas, &CanvasBuilt, &mut CanvasView)>,
           layouts: Query<&UILayout>,
           mut cmd: CommandQueue| {
        let signal = on.signal();
        if signal.button != MouseButton::Left {
            return;
        }
        let (Some((graph, built, mut view)), Some(surface)) =
            (canvases.get_entity(canvas), layouts.get_entity(on.entity()))
        else {
            return;
        };
        let pointed = built.pointed(signal.position, surface, view.offset);
        let from = match pointed.part {
            CanvasPart::Output(node) => Some(node),
            CanvasPart::Node(node) if graph.mode == CanvasMode::Diagram => Some(node),
            _ => None,
        };
        let connecting = from.map(|from| Connecting {
            from,
            start: pointed.point,
        });
        if view.connecting != connecting {
            view.connecting = connecting;
        }
        let target = match pointed.part {
            CanvasPart::Node(node) => Some(CanvasTarget::Node(node)),
            CanvasPart::Link(link) => Some(CanvasTarget::Link(link)),
            _ => None,
        };
        if target.is_some() {
            cmd.entity(canvas).trigger(CanvasSelected { target });
        }
    })
    .into_listener()
}

fn drag_surface(canvas: Entity) -> Listener<UIDrag> {
    (move |on: On<UIDrag>,
           canvases: Query<(&CanvasParts, &CanvasBuilt, &mut CanvasView)>,
           layouts: Query<&UILayout>,
           nodes: Query<&mut UINode>| {
        let signal = on.signal();
        if signal.delta == Vec2::ZERO {
            return;
        }
        let Some((parts, built, mut view)) = canvases.get_entity(canvas) else {
            return;
        };
        match signal.button {
            MouseButton::Middle => view.offset += signal.delta,
            MouseButton::Left => {
                let (Some(connecting), Some(surface)) =
                    (view.connecting, layouts.get_entity(on.entity()))
                else {
                    return;
                };
                let pointed = built.pointed(signal.position, surface, view.offset);
                let route = LinkRoute::between(connecting.start, pointed.point);
                show_preview(&nodes, parts.preview, Some(route));
            }
            _ => {}
        }
    })
    .into_listener()
}

fn release_surface(canvas: Entity) -> Listener<UIPointerUp> {
    (move |on: On<UIPointerUp>,
           canvases: Query<(&CanvasParts, &CanvasBuilt, &mut CanvasView)>,
           layouts: Query<&UILayout>,
           nodes: Query<&mut UINode>,
           mut cmd: CommandQueue| {
        let signal = on.signal();
        if signal.button != MouseButton::Left {
            return;
        }
        let Some((parts, built, mut view)) = canvases.get_entity(canvas) else {
            return;
        };
        let Some(connecting) = view.connecting else {
            return;
        };
        view.connecting = None;
        show_preview(&nodes, parts.preview, None);
        let Some(surface) = layouts.get_entity(on.entity()) else {
            return;
        };
        let from = connecting.from;
        match built.pointed(signal.position, surface, view.offset).part {
            CanvasPart::Input { node, slot } if node != from => {
                cmd.entity(canvas).trigger(CanvasConnected {
                    from,
                    to: node,
                    slot: Some(slot),
                });
            }
            CanvasPart::Node(node) if node != from && built.draws(CanvasMode::Diagram) => {
                cmd.entity(canvas).trigger(CanvasConnected {
                    from,
                    to: node,
                    slot: None,
                });
            }
            CanvasPart::Empty => {
                cmd.entity(canvas).trigger(CanvasLinkDropped {
                    from,
                    at: signal.position,
                });
            }
            _ => {}
        }
    })
    .into_listener()
}

fn click_surface(canvas: Entity) -> Listener<UIClick> {
    (move |on: On<UIClick>,
           canvases: Query<(&CanvasBuilt, &CanvasView)>,
           layouts: Query<&UILayout>,
           mut cmd: CommandQueue| {
        let signal = on.signal();
        let (Some((built, view)), Some(surface)) =
            (canvases.get_entity(canvas), layouts.get_entity(on.entity()))
        else {
            return;
        };
        let part = built.pointed(signal.position, surface, view.offset).part;
        match signal.button {
            MouseButton::Left if part == CanvasPart::Empty => {
                cmd.entity(canvas).trigger(CanvasSelected { target: None });
            }
            MouseButton::Right => {
                cmd.entity(canvas).trigger(CanvasMenuRequested {
                    at: signal.position,
                    on: part,
                });
            }
            _ => {}
        }
    })
    .into_listener()
}

fn double_click_surface(canvas: Entity) -> Listener<UIDoubleClick> {
    (move |on: On<UIDoubleClick>,
           canvases: Query<(&CanvasBuilt, &CanvasView)>,
           layouts: Query<&UILayout>,
           mut cmd: CommandQueue| {
        let signal = on.signal();
        if signal.button != MouseButton::Left {
            return;
        }
        let (Some((built, view)), Some(surface)) =
            (canvases.get_entity(canvas), layouts.get_entity(on.entity()))
        else {
            return;
        };
        if let CanvasPart::Node(node) = built.pointed(signal.position, surface, view.offset).part {
            cmd.entity(canvas).trigger(CanvasActivated { node });
        }
    })
    .into_listener()
}

fn frame_node() -> Listener<FrameCanvasNode> {
    (|on: On<FrameCanvasNode>, views: Query<&mut CanvasView>| {
        if let Some(mut view) = views.get_entity(on.entity()) {
            view.framing = Some(on.signal().node);
        }
    })
    .into_listener()
}

fn setup_canvas(
    canvases: Query<(Entity, &mut UINode), Added<GraphCanvas>>,
    theme: Res<UITheme>,
    mut cmd: CommandQueue,
) {
    for (canvas, mut node) in canvases.iter() {
        node.min_width = UIValue::Px(0.0);
        node.min_height = UIValue::Px(0.0);

        let mut root = cmd.entity(canvas);
        let mut surface = root.spawn_child((
            UINode::default()
                .with_flex_grow(1.0)
                .with_align_self(AlignItems::Stretch)
                .clipped(),
            Interactable,
            press_surface(canvas),
            drag_surface(canvas),
            release_surface(canvas),
            click_surface(canvas),
            double_click_surface(canvas),
        ));
        let mut content = surface.spawn_child(at_box(UIBox::default()));
        let items = content.spawn_child(at_box(UIBox::default())).entity();
        let preview = [(); 3].map(|_| {
            content
                .spawn_child((
                    UINode::default()
                        .with_position(Position::Absolute)
                        .with_visible(false),
                    UIMaterial::flat(theme.accent),
                ))
                .entity()
        });
        let content = content.entity();
        let surface = surface.entity();
        root.insert((
            CanvasParts {
                surface,
                content,
                items,
                preview,
            },
            CanvasView::default(),
            CanvasBuilt::default(),
            frame_node(),
        ));
    }
}

fn build_node(
    items: &mut concerto_ecs::command::EntityCommandQueue<'_>,
    node: &CanvasNode,
    placed: &layout::NodeBox,
    selected: bool,
    theme: &UITheme,
) -> BuiltNode {
    let mut material = UIMaterial::with_border(
        theme.surface_raised,
        node_border(node, selected, theme),
        node_border_width(node, selected),
    );
    material.corner_radius = theme.radius_sm;
    let mut frame = items.spawn_child((
        at_box(placed.rect).with_flex_direction(FlexDirection::Column),
        material,
    ));
    let line = theme.line_height(theme.font_size_md);
    let small_line = theme.line_height(theme.font_size_sm);
    let inner = NODE_WIDTH - 2.0 * TEXT_INSET;
    let mut header = frame.spawn_child(
        UINode::default()
            .with_flex_direction(FlexDirection::Row)
            .with_align_items(AlignItems::Center)
            .with_height(UIValue::Px(HEADER_HEIGHT))
            .with_flex_shrink(0.0)
            .with_padding(UIRect::axes(0.0, TEXT_INSET)),
    );
    header.add_child(
        theme
            .label(node.title.clone())
            .single_line()
            .size(UIValue::Px(inner - BADGE_WIDTH), UIValue::Px(line))
            .fixed()
            .clipped(),
    );
    let badge = header
        .spawn_child((
            theme
                .label(node.badge.clone().unwrap_or_default())
                .small()
                .muted()
                .mono()
                .single_line()
                .size(UIValue::Px(BADGE_WIDTH), UIValue::Px(small_line))
                .fixed()
                .clipped(),
            CanvasReadout,
        ))
        .entity();
    let row = |text: &str, height: f32| {
        theme
            .label(text)
            .small()
            .muted()
            .single_line()
            .size(UIValue::Px(inner), UIValue::Px(height))
            .fixed()
            .clipped()
            .margin(UIRect::axes(0.0, TEXT_INSET))
            .padding(UIRect {
                top: ((height - small_line) / 2.0).max(0.0),
                ..Default::default()
            })
    };
    let subtitle = node.subtitle.as_ref().map(|subtitle| {
        frame
            .spawn_child((row(subtitle, SUBTITLE_HEIGHT), CanvasReadout))
            .entity()
    });
    for input in node.inputs.iter().take(placed.inputs.len()) {
        frame.add_child(row(input, PIN_ROW_HEIGHT));
    }
    let frame = frame.entity();
    for pin in placed.inputs.iter().chain(&placed.output) {
        let mut pin_material = UIMaterial::flat(theme.text_muted);
        pin_material.corner_radius = PIN_SIZE / 2.0;
        items.add_child((at_box(square(*pin, PIN_SIZE)), pin_material));
    }
    BuiltNode {
        frame,
        badge,
        subtitle,
    }
}

fn build_link(
    items: &mut concerto_ecs::command::EntityCommandQueue<'_>,
    link: &CanvasLink,
    route: &LinkRoute,
    selected: bool,
    theme: &UITheme,
) -> BuiltLink {
    let color = link_color(link, selected, theme);
    let segments = route.segments().map(|segment| {
        items
            .spawn_child((at_box(segment.bounds(LINK_WIDTH)), UIMaterial::flat(color)))
            .entity()
    });
    let arrow = route.arrow.map(|direction| {
        let glyph = match direction {
            ArrowDirection::Left => glyph::CARET_LEFT,
            ArrowDirection::Right => glyph::CARET_RIGHT,
        };
        items
            .spawn_child((
                at_box(arrow_box(route, direction, theme)),
                icon(theme, glyph, ICON_SIZE).color(color),
            ))
            .entity()
    });
    if let Some(label) = &link.label {
        let height = theme.line_height(theme.font_size_sm);
        let middle = (route.start.y + route.end.y) / 2.0;
        items.add_child(
            theme
                .label(label.clone())
                .small()
                .muted()
                .no_wrap()
                .node(|node| {
                    node.with_position(Position::Absolute).with_inset(UIInset {
                        left: UIValue::Px(route.bend + LABEL_OFFSET),
                        top: UIValue::Px(middle - height),
                        ..Default::default()
                    })
                }),
        );
    }
    BuiltLink {
        link: route.link,
        segments,
        arrow,
    }
}

fn repaint(
    canvas: &GraphCanvas,
    built: &CanvasBuilt,
    materials: &Query<&mut UIMaterial>,
    texts: &Query<&mut UIText>,
    theme: &UITheme,
) {
    for (node, shown) in canvas.nodes.iter().zip(&built.nodes) {
        let selected = canvas.selection == Some(CanvasTarget::Node(node.id));
        if let Some(mut material) = materials.get_entity(shown.frame) {
            let color = node_border(node, selected, theme).to_linear();
            let width = node_border_width(node, selected);
            if material.border_color != color || material.border_width != width {
                material.border_color = color;
                material.border_width = width;
            }
        }
        if let Some(mut text) = texts.get_entity(shown.badge) {
            let badge = node.badge.as_deref().unwrap_or_default();
            if text.text != badge {
                text.text = badge.to_owned();
            }
        }
        let subtitle = shown.subtitle.and_then(|entity| texts.get_entity(entity));
        if let (Some(mut text), Some(subtitle)) = (subtitle, &node.subtitle)
            && text.text != *subtitle
        {
            text.text = subtitle.clone();
        }
    }
    for shown in &built.links {
        let Some(link) = canvas.links.get(shown.link) else {
            continue;
        };
        let selected = canvas.selection == Some(CanvasTarget::Link(shown.link));
        let color = link_color(link, selected, theme);
        for segment in shown.segments {
            if let Some(mut material) = materials.get_entity(segment)
                && material.color != color.to_linear()
            {
                material.color = color.to_linear();
            }
        }
        if let Some(mut text) = shown.arrow.and_then(|entity| texts.get_entity(entity))
            && text.color != color
        {
            text.color = color;
        }
    }
}

fn sync_canvas(
    canvases: Query<(Ref<GraphCanvas>, &CanvasParts, &mut CanvasBuilt)>,
    nodes: Query<&mut UINode>,
    materials: Query<&mut UIMaterial>,
    texts: Query<&mut UIText>,
    theme: Res<UITheme>,
    mut cmd: CommandQueue,
) {
    for (canvas, parts, mut built) in canvases.iter() {
        let Some(shown) = &built.shown else {
            rebuild(&canvas, parts, &mut built, &nodes, &theme, &mut cmd);
            continue;
        };
        if !canvas.is_changed() || *shown == *canvas {
            continue;
        }
        if same_structure(shown, &canvas) {
            repaint(&canvas, &built, &materials, &texts, &theme);
            built.shown = Some(GraphCanvas::clone(&canvas));
        } else {
            rebuild(&canvas, parts, &mut built, &nodes, &theme, &mut cmd);
        }
    }
}

fn rebuild(
    canvas: &GraphCanvas,
    parts: &CanvasParts,
    built: &mut CanvasBuilt,
    nodes: &Query<&mut UINode>,
    theme: &UITheme,
    cmd: &mut CommandQueue,
) {
    let placed = layout::layout(&canvas.nodes, &canvas.links, canvas.mode);
    for entity in [parts.content, parts.items] {
        if let Some(mut node) = nodes.get_entity(entity) {
            let width = UIValue::Px(placed.size.x);
            let height = UIValue::Px(placed.size.y);
            if node.width != width || node.height != height {
                node.width = width;
                node.height = height;
            }
        }
    }
    let mut items = cmd.entity(parts.items).despawn_children();
    built.links = placed
        .links
        .iter()
        .map(|route| {
            let selected = canvas.selection == Some(CanvasTarget::Link(route.link));
            build_link(
                &mut items,
                &canvas.links[route.link],
                route,
                selected,
                theme,
            )
        })
        .collect();
    built.nodes = canvas
        .nodes
        .iter()
        .zip(&placed.nodes)
        .map(|(node, node_box)| {
            let selected = canvas.selection == Some(CanvasTarget::Node(node.id));
            build_node(&mut items, node, node_box, selected, theme)
        })
        .collect();
    built.layout = placed;
    built.shown = Some(canvas.clone());
}

fn framed(offset: f32, min: f32, size: f32, view: f32) -> f32 {
    let near = FRAME_MARGIN.min(((view - size) / 2.0).max(0.0));
    if min + offset < near {
        near - min
    } else if min + size + offset > view - near {
        view - near - min - size
    } else {
        offset
    }
}

fn sync_canvas_view(
    canvases: Query<(&CanvasParts, &CanvasBuilt, &mut CanvasView)>,
    layouts: Query<&UILayout>,
    nodes: Query<&mut UINode>,
) {
    for (parts, built, mut view) in canvases.iter() {
        if let (Some(id), Some(surface)) = (view.framing, layouts.get_entity(parts.surface))
            && built.shown.is_some()
        {
            let visible = surface.content_rect.size;
            let offset = built.layout.node(id).map_or(view.offset, |node| {
                Vec2::new(
                    framed(view.offset.x, node.rect.min.x, node.rect.size.x, visible.x),
                    framed(view.offset.y, node.rect.min.y, node.rect.size.y, visible.y),
                )
            });
            view.framing = None;
            if view.offset != offset {
                view.offset = offset;
            }
        }
        if let Some(mut content) = nodes.get_entity(parts.content) {
            let inset = UIInset {
                left: UIValue::Px(view.offset.x),
                top: UIValue::Px(view.offset.y),
                ..Default::default()
            };
            if content.inset != inset {
                content.inset = inset;
            }
        }
    }
}

fn sync_canvas_context(
    hovered: Res<HoveredNode>,
    focus: Res<FocusedWidget>,
    canvases: Query<(Entity, &CanvasParts)>,
    mut keyed: ResMut<KeyedCanvas>,
    mut actions: ResMut<ActionMap>,
) {
    let canvas_of = |surface: Option<Entity>| {
        canvases
            .iter()
            .find(|(_, parts)| Some(parts.surface) == surface)
            .map(|(canvas, _)| canvas)
    };
    let modal = actions.is_active(PromptContext) || actions.is_active(ImportDialogContext);
    let canvas = canvas_of(**hovered)
        .or_else(|| canvas_of(**focus))
        .filter(|_| !modal);
    if keyed.0 != canvas {
        keyed.0 = canvas;
    }
    if canvas.is_some() {
        actions.push_context(CanvasContext);
    } else if actions.is_active(CanvasContext) {
        actions.pop_context(CanvasContext);
    }
}

fn canvas_shortcuts(
    mut fired: EventReader<ActionFired>,
    keyed: Res<KeyedCanvas>,
    canvases: Query<&GraphCanvas>,
    mut cmd: CommandQueue,
) {
    for action in fired.read() {
        let Some(canvas) = keyed.0 else {
            continue;
        };
        let Some(target) = canvases
            .get_entity(canvas)
            .and_then(|canvas| canvas.selection)
        else {
            continue;
        };
        if action.is(DeleteCanvasSelection) {
            cmd.entity(canvas).trigger(CanvasDeleted { target });
        } else if action.is(ActivateCanvasSelection)
            && let CanvasTarget::Node(node) = target
        {
            cmd.entity(canvas).trigger(CanvasActivated { node });
        }
    }
}

fn bind_canvas_keys(mut actions: ResMut<ActionMap>) {
    actions.bind(
        DeleteCanvasSelection,
        Shortcut::key(KeyCode::Delete),
        CanvasContext,
    );
    actions.bind(
        ActivateCanvasSelection,
        Shortcut::key(KeyCode::Enter),
        CanvasContext,
    );
}

/// Builds and keeps every [`GraphCanvas`] drawn; `EditorPlugin` registers it.
pub struct GraphCanvasPlugin;

impl Plugin for GraphCanvasPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(KeyedCanvas::default())
            .add_system(Startup, bind_canvas_keys)
            .add_system(Update, canvas_shortcuts)
            .add_system(LateUpdate, setup_canvas.in_set(UiSet::Setup))
            .add_system(
                LateUpdate,
                (sync_canvas, sync_canvas_view)
                    .chain()
                    .in_set(UiSet::Project),
            )
            .add_system(LateUpdate, sync_canvas_context.after(EditorContextSet));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use concerto_ecs::{
        IntoSystem, System, World,
        entity::hierarchy::{ChildOf, Children},
        events::event_channel::EventChannel,
        system::BoxedSystem,
    };

    const ORIGIN: Vec2 = Vec2::new(100.0, 50.0);
    const VIEW: Vec2 = Vec2::new(600.0, 400.0);
    const CLIP_A: CanvasNodeId = CanvasNodeId(1);
    const CLIP_B: CanvasNodeId = CanvasNodeId(2);
    const BLEND: CanvasNodeId = CanvasNodeId(3);
    const IDLE: CanvasNodeId = CanvasNodeId(1);
    const WALK: CanvasNodeId = CanvasNodeId(2);

    #[derive(Resource, Default)]
    struct Heard {
        selected: Vec<Option<CanvasTarget>>,
        activated: Vec<CanvasNodeId>,
        connected: Vec<CanvasConnected>,
        dropped: Vec<CanvasLinkDropped>,
        deleted: Vec<CanvasTarget>,
        menus: Vec<CanvasMenuRequested>,
    }

    fn flow() -> GraphCanvas {
        GraphCanvas::new(CanvasMode::Flow)
            .with_node(CanvasNode::new(CLIP_A, "Walk").with_output())
            .with_node(CanvasNode::new(CLIP_B, "Run").with_output())
            .with_node(
                CanvasNode::new(BLEND, "Blend")
                    .with_input("a")
                    .with_input("b")
                    .with_output(),
            )
            .with_link(CanvasLink::new(CLIP_A, BLEND).with_slot(0))
    }

    fn diagram() -> GraphCanvas {
        GraphCanvas::new(CanvasMode::Diagram)
            .with_node(CanvasNode::new(IDLE, "Idle"))
            .with_node(CanvasNode::new(WALK, "Walk"))
            .with_link(CanvasLink::new(IDLE, WALK))
    }

    struct Fixture {
        world: World,
        setup: BoxedSystem,
        sync: BoxedSystem,
        view: BoxedSystem,
        context: BoxedSystem,
        shortcuts: BoxedSystem,
        canvas: Entity,
    }

    impl Fixture {
        fn new(graph: GraphCanvas) -> Self {
            let mut world = World::new();
            world.register_component::<ChildOf>();
            world.register_component::<Children>();
            world.insert_resource(UITheme::default());
            world.insert_resource(Heard::default());
            world.insert_resource(HoveredNode::default());
            world.insert_resource(FocusedWidget::default());
            world.insert_resource(KeyedCanvas::default());
            world.insert_resource(ActionMap::default());
            world.insert_resource(EventChannel::<ActionFired>::default());
            let canvas = world.spawn((
                UINode::default(),
                graph,
                (|on: On<CanvasSelected>, mut heard: ResMut<Heard>| {
                    heard.selected.push(on.signal().target);
                })
                .into_listener(),
                (|on: On<CanvasActivated>, mut heard: ResMut<Heard>| {
                    heard.activated.push(on.signal().node);
                })
                .into_listener(),
                (|on: On<CanvasConnected>, mut heard: ResMut<Heard>| {
                    heard.connected.push(*on.signal());
                })
                .into_listener(),
                (|on: On<CanvasLinkDropped>, mut heard: ResMut<Heard>| {
                    heard.dropped.push(*on.signal());
                })
                .into_listener(),
                (|on: On<CanvasDeleted>, mut heard: ResMut<Heard>| {
                    heard.deleted.push(on.signal().target);
                })
                .into_listener(),
                (|on: On<CanvasMenuRequested>, mut heard: ResMut<Heard>| {
                    heard.menus.push(*on.signal());
                })
                .into_listener(),
            ));
            let mut bind = bind_canvas_keys.into_boxed_system();
            let mut setup = setup_canvas.into_boxed_system();
            let mut sync = sync_canvas.into_boxed_system();
            let mut view = sync_canvas_view.into_boxed_system();
            let mut context = sync_canvas_context.into_boxed_system();
            let mut shortcuts = canvas_shortcuts.into_boxed_system();
            bind.initialize(&mut world);
            setup.initialize(&mut world);
            sync.initialize(&mut world);
            view.initialize(&mut world);
            context.initialize(&mut world);
            shortcuts.initialize(&mut world);
            bind.run_and_apply((), &mut world);
            let mut fixture = Self {
                world,
                setup,
                sync,
                view,
                context,
                shortcuts,
                canvas,
            };
            fixture.frame();
            let rect = UIBox {
                min: ORIGIN,
                size: VIEW,
            };
            let surface = fixture.parts().surface;
            fixture.world.insert(
                UILayout {
                    rect,
                    content_rect: rect,
                    clip_rect: rect,
                    paint_order: 0,
                },
                surface,
            );
            fixture
        }

        fn frame(&mut self) {
            self.shortcuts.run_and_apply((), &mut self.world);
            self.setup.run_and_apply((), &mut self.world);
            self.sync.run_and_apply((), &mut self.world);
            self.view.run_and_apply((), &mut self.world);
            self.context.run_and_apply((), &mut self.world);
        }

        fn parts(&self) -> &CanvasParts {
            self.world
                .get_component_for_entity::<CanvasParts>(self.canvas)
                .unwrap()
        }

        fn placed(&self) -> CanvasLayout {
            self.world
                .get_component_for_entity::<CanvasBuilt>(self.canvas)
                .unwrap()
                .layout
                .clone()
        }

        fn offset(&self) -> Vec2 {
            self.world
                .get_component_for_entity::<CanvasView>(self.canvas)
                .unwrap()
                .offset
        }

        fn on_screen(&self, point: Vec2) -> Vec2 {
            ORIGIN + self.offset() + point
        }

        fn press(&mut self, point: Vec2) {
            let position = self.on_screen(point);
            let surface = self.parts().surface;
            self.world.trigger_on(
                surface,
                UIPointerDown {
                    position,
                    button: MouseButton::Left,
                },
            );
        }

        fn drag(&mut self, point: Vec2, delta: Vec2, button: MouseButton) {
            let position = self.on_screen(point);
            let surface = self.parts().surface;
            self.world.trigger_on(
                surface,
                UIDrag {
                    position,
                    delta,
                    button,
                },
            );
        }

        fn release(&mut self, point: Vec2) {
            let position = self.on_screen(point);
            let surface = self.parts().surface;
            self.world.trigger_on(
                surface,
                UIPointerUp {
                    position,
                    button: MouseButton::Left,
                },
            );
        }

        fn click(&mut self, point: Vec2, button: MouseButton) {
            let position = self.on_screen(point);
            let surface = self.parts().surface;
            self.world.trigger_on(surface, UIClick { position, button });
        }

        fn double_click(&mut self, point: Vec2) {
            let position = self.on_screen(point);
            let surface = self.parts().surface;
            self.world.trigger_on(
                surface,
                UIDoubleClick {
                    position,
                    button: MouseButton::Left,
                },
            );
        }

        fn heard(&self) -> &Heard {
            self.world.get_resource::<Heard>().unwrap()
        }

        fn delete_means(&self) -> bool {
            self.world
                .get_resource::<ActionMap>()
                .unwrap()
                .resolve(Shortcut::key(KeyCode::Delete))
                .is_some()
        }
    }

    fn centre(rect: UIBox) -> Vec2 {
        rect.min + rect.size / 2.0
    }

    struct Spot {
        name: &'static str,
        at: Vec2,
        on: CanvasPart,
    }

    #[test]
    fn the_pointer_finds_nodes_pins_links_and_empty_space() {
        let mut f = Fixture::new(flow());
        let placed = f.placed();
        let blend = placed.node(BLEND).unwrap().clone();
        let clip = placed.node(CLIP_A).unwrap().clone();
        let run = placed.links[0].segments()[1];
        let on_link = (run.from + run.to) / 2.0 + Vec2::new(layout::LINK_REACH - 1.0, 0.0);
        let spots = [
            Spot {
                name: "a node",
                at: centre(blend.rect),
                on: CanvasPart::Node(BLEND),
            },
            Spot {
                name: "an input pin",
                at: blend.inputs[1] + Vec2::new(layout::PIN_REACH - 1.0, 0.0),
                on: CanvasPart::Input {
                    node: BLEND,
                    slot: 1,
                },
            },
            Spot {
                name: "an output pin",
                at: clip.output.unwrap() - Vec2::new(layout::PIN_REACH - 1.0, 0.0),
                on: CanvasPart::Output(CLIP_A),
            },
            Spot {
                name: "a link",
                at: on_link,
                on: CanvasPart::Link(0),
            },
            Spot {
                name: "beside a link",
                at: on_link + Vec2::new(2.0, 0.0),
                on: CanvasPart::Empty,
            },
            Spot {
                name: "empty space",
                at: Vec2::new(2.0, 2.0),
                on: CanvasPart::Empty,
            },
        ];
        for spot in &spots {
            f.click(spot.at, MouseButton::Right);
            let menu = *f.heard().menus.last().unwrap();
            assert_eq!(menu.on, spot.on, "{}", spot.name);
            assert_eq!(menu.at, ORIGIN + spot.at, "{}", spot.name);
        }
        assert!(f.heard().selected.is_empty());

        f.press(centre(blend.rect));
        f.press(on_link);
        f.press(blend.inputs[1]);
        f.click(Vec2::new(2.0, 2.0), MouseButton::Left);
        f.click(centre(blend.rect), MouseButton::Left);
        assert_eq!(
            f.heard().selected,
            [
                Some(CanvasTarget::Node(BLEND)),
                Some(CanvasTarget::Link(0)),
                None
            ]
        );

        f.double_click(on_link);
        f.double_click(centre(clip.rect));
        assert_eq!(f.heard().activated, [CLIP_A]);
    }

    struct Gesture {
        press: Vec2,
        release: Vec2,
    }

    struct ConnectCase {
        canvas: GraphCanvas,
        connect: Gesture,
        misses: Vec<Gesture>,
        connected: CanvasConnected,
        dropped: Vec<CanvasLinkDropped>,
    }

    #[test]
    fn a_drag_connects_what_it_starts_on_to_what_it_ends_on_in_either_mode() {
        let flow_placed = Fixture::new(flow()).placed();
        let output = flow_placed.node(CLIP_B).unwrap().output.unwrap();
        let input = flow_placed.node(BLEND).unwrap().inputs[1];
        let diagram_placed = Fixture::new(diagram()).placed();
        let idle = centre(diagram_placed.node(IDLE).unwrap().rect);
        let walk = centre(diagram_placed.node(WALK).unwrap().rect);
        let empty = Vec2::new(2.0, 2.0);
        let cases = [
            ConnectCase {
                canvas: flow(),
                connect: Gesture {
                    press: output,
                    release: input,
                },
                misses: vec![
                    Gesture {
                        press: output,
                        release: empty,
                    },
                    Gesture {
                        press: output,
                        release: centre(flow_placed.node(BLEND).unwrap().rect),
                    },
                    Gesture {
                        press: centre(flow_placed.node(CLIP_B).unwrap().rect),
                        release: input,
                    },
                ],
                connected: CanvasConnected {
                    from: CLIP_B,
                    to: BLEND,
                    slot: Some(1),
                },
                dropped: vec![CanvasLinkDropped {
                    from: CLIP_B,
                    at: ORIGIN + empty,
                }],
            },
            ConnectCase {
                canvas: diagram(),
                connect: Gesture {
                    press: walk,
                    release: idle,
                },
                misses: vec![Gesture {
                    press: walk,
                    release: walk + Vec2::new(6.0, 0.0),
                }],
                connected: CanvasConnected {
                    from: WALK,
                    to: IDLE,
                    slot: None,
                },
                dropped: Vec::new(),
            },
        ];

        for (row, case) in cases.into_iter().enumerate() {
            let mut f = Fixture::new(case.canvas);
            let preview = f.parts().preview[0];
            let shown = |f: &Fixture| {
                f.world
                    .get_component_for_entity::<UINode>(preview)
                    .unwrap()
                    .visible
            };
            let Gesture { press, release } = case.connect;

            f.press(press);
            f.drag(release, release - press, MouseButton::Left);
            assert!(shown(&f), "row {row}");
            assert!(f.heard().connected.is_empty(), "row {row}");
            f.release(release);
            assert!(!shown(&f), "row {row}");
            for miss in case.misses {
                f.press(miss.press);
                f.release(miss.release);
            }

            assert_eq!(f.heard().connected, [case.connected], "row {row}");
            assert_eq!(f.heard().dropped, case.dropped, "row {row}");
        }
    }

    #[test]
    fn a_middle_drag_pans_the_content_and_framing_brings_a_node_back() {
        let mut f = Fixture::new(flow());
        let blend = f.placed().node(BLEND).unwrap().clone();
        let pan = Vec2::new(-900.0, 40.0);

        f.drag(Vec2::ZERO, Vec2::ZERO, MouseButton::Middle);
        f.drag(Vec2::ZERO, pan, MouseButton::Left);
        assert_eq!(f.offset(), Vec2::ZERO);
        f.drag(Vec2::ZERO, pan, MouseButton::Middle);
        f.frame();

        assert_eq!(f.offset(), pan);
        let content = f.parts().content;
        let inset = f
            .world
            .get_component_for_entity::<UINode>(content)
            .unwrap()
            .inset;
        assert_eq!(inset.left, UIValue::Px(pan.x));
        assert_eq!(inset.top, UIValue::Px(pan.y));
        f.click(centre(blend.rect), MouseButton::Right);
        let menu = *f.heard().menus.last().unwrap();
        assert_eq!(menu.on, CanvasPart::Node(BLEND));
        assert_eq!(menu.at, ORIGIN + pan + centre(blend.rect));

        f.world
            .trigger_on(f.canvas, FrameCanvasNode { node: BLEND });
        f.frame();

        let min = f.offset() + blend.rect.min;
        let max = min + blend.rect.size;
        assert!(min.x >= 0.0 && min.y >= 0.0, "{min}");
        assert!(max.x <= VIEW.x && max.y <= VIEW.y, "{max}");
    }

    #[test]
    fn delete_reaches_the_selection_of_the_canvas_under_the_pointer_or_with_focus() {
        let mut f = Fixture::new(flow());
        let surface = f.parts().surface;
        let fire = |f: &mut Fixture| {
            f.world
                .get_resource_mut::<EventChannel<ActionFired>>()
                .unwrap()
                .push_event(ActionFired::new(DeleteCanvasSelection));
            f.frame();
        };
        assert!(!f.delete_means());

        **f.world.get_resource_mut::<HoveredNode>().unwrap() = Some(surface);
        f.frame();
        assert!(f.delete_means());
        fire(&mut f);
        assert!(f.heard().deleted.is_empty());

        f.world
            .get_component_for_entity_mut::<GraphCanvas>(f.canvas)
            .unwrap()
            .selection = Some(CanvasTarget::Link(0));
        **f.world.get_resource_mut::<HoveredNode>().unwrap() = None;
        **f.world.get_resource_mut::<FocusedWidget>().unwrap() = Some(surface);
        f.frame();
        fire(&mut f);
        assert_eq!(f.heard().deleted, [CanvasTarget::Link(0)]);

        **f.world.get_resource_mut::<FocusedWidget>().unwrap() = None;
        f.frame();
        assert!(!f.delete_means());
    }
}

//! The shell's skeleton: tiled cards with splitters in the gaps between them, and the sizes they show.
use std::collections::HashMap;

use concerto_app::{
    App, Plugin,
    schedule_groups::{Startup, Update},
};
use concerto_color::Color;
use concerto_ecs::{
    Component, Entity, Query, Res, ResMut, Resource,
    command::CommandQueue,
    query::change_detection::DetectChanges,
    signal::{
        On, Signal,
        listener::{IntoListener, Listener},
    },
};
use concerto_ui::{
    elements::{Surface, prelude::*},
    interaction::Interactable,
    node::{UINode, UIRect},
    scroll::UISplitAxis,
    splitter::{SPLITTER_HIT_SIZE, UISplitterDragged},
    theme::UITheme,
    transform::UIValue,
};
use concerto_window::plugin::Window;
use glam::Vec2;
use taffy::{AlignItems, FlexDirection};

/// Where a panel sits in the shell.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Region {
    Brand,
    Stats,
    Left,
    Centre,
    Right,
    Bottom,
    Status,
}

impl Region {
    fn is_titled(self) -> bool {
        matches!(self, Region::Left | Region::Right)
    }

    fn is_card(self) -> bool {
        matches!(self, Region::Left | Region::Right | Region::Bottom)
    }
}

/// A panel's declaration.
#[derive(Clone, Copy)]
pub struct PanelDescriptor {
    pub id: &'static str,
    pub title: &'static str,
    pub region: Region,
}

#[derive(Resource, Default)]
pub struct PanelRegistry {
    panels: Vec<PanelDescriptor>,
    bodies: HashMap<&'static str, Entity>,
    regions: HashMap<Region, Entity>,
    root: Option<Entity>,
}

impl PanelRegistry {
    pub fn register(&mut self, panel: PanelDescriptor) {
        if self.panels.iter().any(|existing| existing.id == panel.id) {
            log::warn!(
                "Panel '{}' is already registered; ignoring the duplicate",
                panel.id
            );
            return;
        }
        self.panels.push(panel);
    }

    /// The node that holds every region, once the dock has run.
    pub fn root(&self) -> Option<Entity> {
        self.root
    }

    /// The node of a region, which holds its panels' bodies, once the dock has run.
    pub fn region(&self, region: Region) -> Option<Entity> {
        self.regions.get(&region).copied()
    }

    /// The entity a panel builds its contents into, once the dock has run.
    pub fn body(&self, id: &str) -> Option<Entity> {
        self.bodies.get(id).copied()
    }

    pub fn panels(&self) -> &[PanelDescriptor] {
        &self.panels
    }

    fn in_region(&self, region: Region) -> Vec<PanelDescriptor> {
        self.panels
            .iter()
            .copied()
            .filter(|panel| panel.region == region)
            .collect()
    }
}

/// Registers a panel. Call from a panel plugin's `build`.
pub trait DockedApp {
    fn add_panel(&mut self, panel: PanelDescriptor) -> &mut Self;
}

impl DockedApp for App {
    fn add_panel(&mut self, panel: PanelDescriptor) -> &mut Self {
        self.get_resource_mut::<PanelRegistry>()
            .expect("DockPlugin must be registered before any panel")
            .register(panel);
        self
    }
}

pub struct DockPlugin;

impl Plugin for DockPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(EditorLayout::default());
        app.insert_resource(PanelStates::default());
        app.main_mut().world_mut().add_listener(resize_panel);
        app.add_system(Startup, build_dock)
            .add_system(Update, apply_layout);
    }
}

/// A panel whose size the layout holds.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum LayoutPanel {
    Left,
    Right,
    Bottom,
    Browser,
}

/// How a panel takes its space on the front tab.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PanelState {
    /// It shows its stored size and can be resized.
    Sized,
    /// It is collapsed, with its splitter.
    Hidden,
    /// It takes the space that is left, with no splitter.
    Fill,
}

/// What each panel is on the front tab; the workspace decides it and the dock lays it out.
#[derive(Resource, Clone, Copy, PartialEq, Eq, Debug)]
pub struct PanelStates {
    pub left: PanelState,
    pub right: PanelState,
    pub bottom: PanelState,
    pub browser: PanelState,
}

impl Default for PanelStates {
    fn default() -> Self {
        Self {
            left: PanelState::Sized,
            right: PanelState::Sized,
            bottom: PanelState::Sized,
            browser: PanelState::Fill,
        }
    }
}

impl PanelStates {
    pub fn of(&self, panel: LayoutPanel) -> PanelState {
        match panel {
            LayoutPanel::Left => self.left,
            LayoutPanel::Right => self.right,
            LayoutPanel::Bottom => self.bottom,
            LayoutPanel::Browser => self.browser,
        }
    }
}

/// The stored sizes of the resizable panels, in logical pixels, for the session and every tab.
#[derive(Resource, Clone, Copy, PartialEq, Debug)]
pub struct EditorLayout {
    pub left: f32,
    pub right: f32,
    pub bottom: f32,
    pub browser: f32,
}

impl Default for EditorLayout {
    fn default() -> Self {
        Self {
            left: 260.0,
            right: 320.0,
            bottom: 220.0,
            browser: 420.0,
        }
    }
}

const LEFT_MIN: f32 = 180.0;
const RIGHT_MIN: f32 = 220.0;
const BOTTOM_MIN: f32 = 120.0;
const BROWSER_MIN: f32 = 200.0;
const CENTRE_MIN_WIDTH: f32 = 320.0;
const CENTRE_MIN_HEIGHT: f32 = 240.0;
const BOTTOM_HOST_MIN: f32 = 240.0;

const TOP: f32 = 34.0;
/// Height of the top bar.
pub const TOP_STRIP: f32 = 40.0;
const STATUS: f32 = 28.0;
const GAP: f32 = SPLITTER_HIT_SIZE;

impl EditorLayout {
    /// The stored size of `panel`.
    pub fn size(&self, panel: LayoutPanel) -> f32 {
        match panel {
            LayoutPanel::Left => self.left,
            LayoutPanel::Right => self.right,
            LayoutPanel::Bottom => self.bottom,
            LayoutPanel::Browser => self.browser,
        }
    }

    fn set(&mut self, panel: LayoutPanel, size: f32) {
        match panel {
            LayoutPanel::Left => self.left = size,
            LayoutPanel::Right => self.right = size,
            LayoutPanel::Bottom => self.bottom = size,
            LayoutPanel::Browser => self.browser = size,
        }
    }

    /// The sizes shown in a window of `window` logical pixels: the stored ones, or smaller where they do not fit.
    pub fn shown(&self, states: &PanelStates, window: Vec2) -> EditorLayout {
        let room = row_room(states, window);
        let mut shown = *self;
        if states.right == PanelState::Sized {
            let left = sized(states.left, self.left);
            shown.right = self.right.min(room - left).max(RIGHT_MIN);
        }
        if states.left == PanelState::Sized {
            let right = sized(states.right, shown.right);
            shown.left = self.left.min(room - right).max(LEFT_MIN);
        }
        shown.bottom = self.bottom.min(column_room(window)).max(BOTTOM_MIN);
        shown.browser = self.browser.min(strip_room(window)).max(BROWSER_MIN);
        shown
    }

    /// `size` held within what `panel` may take in a window of `window` logical pixels.
    pub fn clamp(&self, panel: LayoutPanel, size: f32, states: &PanelStates, window: Vec2) -> f32 {
        let shown = self.shown(states, window);
        let room = row_room(states, window);
        let range = match panel {
            LayoutPanel::Left => Range {
                min: LEFT_MIN,
                max: room - sized(states.right, shown.right),
            },
            LayoutPanel::Right => Range {
                min: RIGHT_MIN,
                max: room - sized(states.left, shown.left),
            },
            LayoutPanel::Bottom => Range {
                min: BOTTOM_MIN,
                max: column_room(window),
            },
            LayoutPanel::Browser => Range {
                min: BROWSER_MIN,
                max: strip_room(window),
            },
        };
        size.min(range.max).max(range.min)
    }
}

struct Range {
    min: f32,
    max: f32,
}

fn sized(state: PanelState, size: f32) -> f32 {
    if state == PanelState::Sized {
        size
    } else {
        0.0
    }
}

fn row_room(states: &PanelStates, window: Vec2) -> f32 {
    let splitters = [states.left, states.right]
        .into_iter()
        .filter(|state| *state == PanelState::Sized)
        .count() as f32
        * GAP;
    let rest = if states.right == PanelState::Fill {
        RIGHT_MIN
    } else {
        CENTRE_MIN_WIDTH
    };
    window.x - GAP * 2.0 - splitters - rest
}

fn column_room(window: Vec2) -> f32 {
    window.y - TOP_STRIP - STATUS - GAP - CENTRE_MIN_HEIGHT
}

fn strip_room(window: Vec2) -> f32 {
    window.x - GAP * 3.0 - BOTTOM_HOST_MIN
}

/// Sets the stored size of a panel, clamped to what the window allows; ignored for a panel that is hidden or filling.
#[derive(Clone, Copy, Debug)]
pub struct ResizePanel {
    pub panel: LayoutPanel,
    /// The wanted size, in logical pixels.
    pub size: f32,
}

impl Signal for ResizePanel {}

fn resize_panel(
    on: On<ResizePanel>,
    mut layout: ResMut<EditorLayout>,
    states: Res<PanelStates>,
    window: Res<Window>,
) {
    let ResizePanel { panel, size } = *on.signal();
    if states.of(panel) != PanelState::Sized {
        return;
    }
    let size = layout.clamp(panel, size, &states, window.logical_size());
    if layout.size(panel) != size {
        layout.set(panel, size);
    }
}

/// A region's card: the theme's panel, outlined so it reads on the backdrop.
pub fn card(theme: &UITheme) -> Surface {
    let border = theme.border.to_srgba();
    theme
        .panel()
        .border(Color::srgba(border.r, border.g, border.b, 0.55), 1.0)
}

#[derive(Component, Clone, Copy)]
pub(crate) enum LayoutPart {
    Panel(LayoutPanel),
    Splitter(LayoutPanel),
}

fn drag_panel(panel: LayoutPanel) -> Listener<UISplitterDragged> {
    (move |on: On<UISplitterDragged>,
           layout: Res<EditorLayout>,
           states: Res<PanelStates>,
           window: Res<Window>,
           mut cmd: CommandQueue| {
        let shown = layout.shown(&states, window.logical_size()).size(panel);
        let delta = on.signal().delta;
        let size = match panel {
            LayoutPanel::Left | LayoutPanel::Browser => shown + delta,
            LayoutPanel::Right | LayoutPanel::Bottom => shown - delta,
        };
        cmd.trigger(ResizePanel { panel, size });
    })
    .into_listener()
}

/// Writes the shown sizes, and which panels are collapsed or filling, to the region nodes.
pub(crate) fn apply_layout(
    layout: Res<EditorLayout>,
    states: Res<PanelStates>,
    window: Res<Window>,
    parts: Query<(&LayoutPart, &mut UINode)>,
) {
    if !(layout.is_changed() || states.is_changed() || window.is_changed()) {
        return;
    }
    let shown = layout.shown(&states, window.logical_size());
    for (part, mut node) in parts.iter() {
        let panel = match *part {
            LayoutPart::Splitter(panel) => {
                let visible = states.of(panel) == PanelState::Sized;
                if node.visible != visible {
                    node.visible = visible;
                }
                continue;
            }
            LayoutPart::Panel(panel) => panel,
        };
        let state = states.of(panel);
        let visible = state != PanelState::Hidden;
        if node.visible != visible {
            node.visible = visible;
        }
        if !visible {
            continue;
        }
        let fill = state == PanelState::Fill;
        let extent = if fill {
            UIValue::Auto
        } else {
            UIValue::Px(shown.size(panel))
        };
        let flex = if fill { 1.0 } else { 0.0 };
        if panel == LayoutPanel::Bottom {
            if node.height != extent {
                node.height = extent;
            }
        } else if node.width != extent {
            node.width = extent;
        }
        if node.flex_grow != flex {
            node.flex_grow = flex;
        }
        if node.flex_shrink != flex {
            node.flex_shrink = flex;
        }
    }
}

fn build_dock(
    mut cmd: CommandQueue,
    mut registry: ResMut<PanelRegistry>,
    theme: Res<UITheme>,
    mut window: ResMut<Window>,
) {
    window.settings.title = "Concerto".to_string();
    window.settings.min_size = Some(Vec2::new(900.0, 600.0));

    let backdrop = cmd
        .spawn(
            theme
                .canvas()
                .size(UIValue::Percent(100.0), UIValue::Percent(100.0))
                .column()
                .clipped(),
        )
        .entity();
    registry.root = Some(backdrop);
    let root = cmd
        .entity(backdrop)
        .spawn_child(
            UINode::default()
                .with_flex_grow(1.0)
                .with_min_height(UIValue::Px(0.0))
                .with_flex_direction(FlexDirection::Column)
                .with_padding(UIRect::axes(0.0, GAP))
                .clipped(),
        )
        .entity();

    let bar = cmd
        .entity(root)
        .spawn_child(
            UINode::default()
                .with_height(UIValue::Px(TOP_STRIP))
                .with_flex_shrink(0.0)
                .with_flex_direction(FlexDirection::Row)
                .with_align_items(AlignItems::Center),
        )
        .entity();
    let chrome = |grow: f32| {
        UINode::default()
            .with_height(UIValue::Px(TOP))
            .with_flex_grow(grow)
            .with_flex_shrink(grow)
            .with_min_width(UIValue::Px(0.0))
            .with_flex_direction(FlexDirection::Row)
            .clipped()
    };
    let brand = cmd.entity(bar).spawn_child(chrome(1.0)).entity();
    let stats = cmd.entity(bar).spawn_child(chrome(0.0)).entity();

    let row = cmd
        .entity(root)
        .spawn_child(
            UINode::default()
                .with_flex_grow(1.0)
                .with_min_height(UIValue::Px(0.0))
                .with_flex_direction(FlexDirection::Row),
        )
        .entity();
    let side = || {
        UINode::default()
            .with_flex_shrink(0.0)
            .with_flex_direction(FlexDirection::Column)
            .with_gap(Vec2::new(0.0, GAP))
            .clipped()
    };
    let left = cmd
        .entity(row)
        .spawn_child((side(), LayoutPart::Panel(LayoutPanel::Left)))
        .entity();
    spawn_splitter(&mut cmd, row, LayoutPanel::Left, &theme);
    let centre = cmd
        .entity(row)
        .spawn_child(
            UINode::default()
                .with_flex_grow(1.0)
                .with_min_width(UIValue::Px(0.0))
                .with_min_height(UIValue::Px(0.0))
                .with_flex_direction(FlexDirection::Column)
                .clipped(),
        )
        .entity();
    spawn_splitter(&mut cmd, row, LayoutPanel::Right, &theme);
    let right = cmd
        .entity(row)
        .spawn_child((side(), LayoutPart::Panel(LayoutPanel::Right)))
        .entity();

    spawn_splitter(&mut cmd, root, LayoutPanel::Bottom, &theme);
    let bottom = cmd
        .entity(root)
        .spawn_child((
            UINode::default()
                .with_flex_shrink(0.0)
                .with_flex_direction(FlexDirection::Row)
                .clipped(),
            LayoutPart::Panel(LayoutPanel::Bottom),
        ))
        .entity();
    let status = cmd
        .entity(root)
        .spawn_child(
            UINode::default()
                .with_height(UIValue::Px(STATUS))
                .with_flex_shrink(0.0)
                .with_flex_direction(FlexDirection::Column)
                .clipped(),
        )
        .entity();

    for (region, container) in [
        (Region::Brand, brand),
        (Region::Stats, stats),
        (Region::Left, left),
        (Region::Centre, centre),
        (Region::Right, right),
        (Region::Bottom, bottom),
        (Region::Status, status),
    ] {
        registry.regions.insert(region, container);
        let panels = registry.in_region(region);
        for (id, body) in fill_region(&mut cmd, container, region, &panels, &theme) {
            registry.bodies.insert(id, body);
        }
    }
    spawn_splitter(&mut cmd, bottom, LayoutPanel::Browser, &theme);
}

fn spawn_splitter(cmd: &mut CommandQueue, parent: Entity, panel: LayoutPanel, theme: &UITheme) {
    let axis = match panel {
        LayoutPanel::Bottom => UISplitAxis::Vertical,
        LayoutPanel::Left | LayoutPanel::Right | LayoutPanel::Browser => UISplitAxis::Horizontal,
    };
    cmd.entity(parent).add_child((
        theme.splitter(axis).on_drag(drag_panel(panel)),
        LayoutPart::Splitter(panel),
    ));
}

fn fill_region(
    cmd: &mut CommandQueue,
    container: Entity,
    region: Region,
    panels: &[PanelDescriptor],
    theme: &UITheme,
) -> Vec<(&'static str, Entity)> {
    let mut bodies = Vec::with_capacity(panels.len());
    for panel in panels {
        let node = UINode::default()
            .with_flex_grow(1.0)
            .with_flex_direction(FlexDirection::Column)
            .clipped();
        let node = if region.is_titled() {
            node.with_padding(UIRect::all(theme.spacing_md))
        } else {
            node
        };
        let mut container_queue = cmd.entity(container);
        let mut body_queue = if region.is_card() {
            container_queue.spawn_child(card(theme).node(|_| node))
        } else {
            container_queue.spawn_child(node)
        };
        let body = body_queue.entity();
        if region != Region::Centre {
            body_queue.insert(Interactable);
        }
        if region == Region::Bottom {
            body_queue.insert(LayoutPart::Panel(LayoutPanel::Browser));
        }
        if region.is_titled() {
            body_queue.add_child(
                theme
                    .label(panel.title.to_uppercase())
                    .small()
                    .muted()
                    .weight(crate::fonts::MEDIUM)
                    .height(UIValue::Px(16.0))
                    .fixed(),
            );
        }
        bodies.push((panel.id, body));
    }
    bodies
}

#[cfg(test)]
mod tests {
    use concerto_ecs::World;

    use super::*;

    #[derive(Clone, Copy, Debug)]
    struct WindowSize {
        width: u32,
        height: u32,
    }

    impl WindowSize {
        fn window(self) -> Window {
            Window::new((self.width, self.height), 1.0)
        }
    }

    struct Resize {
        window: WindowSize,
        browser: PanelState,
        panel: LayoutPanel,
        size: f32,
        stored: f32,
    }

    #[test]
    fn a_resize_stores_the_size_clamped_to_what_the_window_leaves() {
        use LayoutPanel::{Bottom, Browser, Left, Right};
        use PanelState::{Fill, Sized};
        let wide = WindowSize {
            width: 1280,
            height: 720,
        };
        let small = WindowSize {
            width: 900,
            height: 600,
        };
        let case = |window, browser, panel, size, stored| Resize {
            window,
            browser,
            panel,
            size,
            stored,
        };
        let cases = [
            case(wide, Fill, Left, 300.0, 300.0),
            case(wide, Fill, Left, 100.0, 180.0),
            case(wide, Fill, Left, 2000.0, 608.0),
            case(wide, Fill, Bottom, 2000.0, 404.0),
            case(wide, Sized, Browser, 5000.0, 1016.0),
            case(small, Fill, Right, 2000.0, 288.0),
        ];
        for (index, case) in cases.iter().enumerate() {
            let mut world = World::new();
            world.insert_resource(case.window.window());
            world.insert_resource(EditorLayout::default());
            world.insert_resource(PanelStates {
                browser: case.browser,
                ..Default::default()
            });
            world.add_listener(resize_panel);

            world.trigger(ResizePanel {
                panel: case.panel,
                size: case.size,
            });

            let layout = *world.get_resource::<EditorLayout>().unwrap();
            assert_eq!(
                layout.size(case.panel),
                case.stored,
                "case {index}: {:?} to {} in {:?}",
                case.panel,
                case.size,
                case.window
            );
            let untouched = EditorLayout::default();
            for other in [Left, Right, Bottom, Browser] {
                if other != case.panel {
                    assert_eq!(layout.size(other), untouched.size(other), "case {index}");
                }
            }
        }

        let mut world = World::new();
        world.insert_resource(
            WindowSize {
                width: 800,
                height: 600,
            }
            .window(),
        );
        world.insert_resource(EditorLayout::default());
        world.insert_resource(PanelStates::default());
        world.add_listener(resize_panel);
        world.trigger(ResizePanel {
            panel: Bottom,
            size: 200.0,
        });
        let stored = *world.get_resource::<EditorLayout>().unwrap();
        let window = world.get_resource::<Window>().unwrap().logical_size();
        let shown = stored.shown(&PanelStates::default(), window);
        assert_eq!(shown.left, 228.0);
        assert_eq!(shown.right, 220.0);
        assert_eq!(stored.left, 260.0);
        assert_eq!(stored.right, 320.0);
    }
}

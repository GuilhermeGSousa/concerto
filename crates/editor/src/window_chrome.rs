//! What a title bar would have given us, now that the window has none.
//!
//! Move, minimise, maximise, close and resize are all the window manager's job
//! normally; undecorated, the application has to offer them itself. Moving and
//! resizing are started by the event loop the moment the button goes down
//! (see [`WindowGestureRegion`]); all this module does is publish where the
//! frame's edges and title band currently are.
use app::{
    schedule_groups::{LateUpdate, Startup},
    App, Plugin,
};
use color::Color;
use ecs::{
    command::CommandQueue, events::event_reader::EventReader, Component, Query, Res, ResMut,
    Resource,
};
use taffy::{FlexDirection, Position};
use ui::{
    interaction::{Interactable, UIClick, UIInteractionStyle},
    material::UIMaterial,
    node::{UIInset, UILayout, UINode, UIRect},
    text::TextComponent,
    theme::UITheme,
    transform::UIValue,
};
use window::plugin::{CloseRequest, Window, WindowGesture, WindowGestureRegion, WindowGestureZone};
use winit::window::ResizeDirection;

use crate::dock::{DockedApp, PanelDescriptor, PanelRegistry, Region, TOP_STRIP};
use crate::fonts::{glyph, icon};

pub const PANEL_ID: &str = "rabbithole.window";

/// Width of the invisible strip along each window edge that starts a resize.
const GRIP: f32 = 6.0;
/// Corners are square and larger, because hitting one is fiddly otherwise.
const CORNER: f32 = 14.0;
/// Grips sit above every panel; a card must not swallow the window's edge.
const GRIP_LAYER: i32 = 100;
/// The title band sits above the scene but below the buttons standing in it.
const DRAG_LAYER: i32 = 50;
const CONTROL_LAYER: i32 = 60;

/// Marks an interactive title-strip control that must block window dragging.
/// Tab buttons and their close controls use this marker too.
#[derive(Component, Clone, Copy, Default)]
pub struct WindowChromeControl;

#[derive(Component, Clone, Copy)]
enum Control {
    Minimise,
    Maximise,
    Close,
}

/// The area you drag to move the window: the band across the top, where a
/// title bar would be. It covers the brand and the stats readout, both of which
/// are labels rather than controls; the window buttons sit above it.
#[derive(Component)]
struct DragHandle;

#[derive(Component, Clone, Copy)]
struct ResizeGrip(ResizeDirection);

/// Marks the glyph that has to follow whether the window is maximised.
#[derive(Component)]
struct MaximiseGlyph;

/// Whether the window manager draws the frame.
///
/// Undecorated is what the design asks for, but moving and resizing an
/// undecorated window is a request to the window manager
/// (`_NET_WM_MOVERESIZE`, `xdg_toplevel::move`) that a compositor is free to
/// ignore. `--decorated` hands the whole job back to the window manager for
/// anyone whose desktop does.
pub struct WindowChromePlugin {
    pub decorated: bool,
}

impl Plugin for WindowChromePlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(WindowStyle {
            decorated: self.decorated,
        });
        if !self.decorated {
            app.add_panel(PanelDescriptor {
                id: PANEL_ID,
                title: "Window",
                region: Region::Stats,
            });
        }
        app.add_system(Startup, build_controls)
            .add_system(LateUpdate, publish_window_gestures)
            .add_system(LateUpdate, handle_controls)
            .add_system(LateUpdate, sync_maximise_glyph);
    }
}

#[derive(Resource)]
struct WindowStyle {
    decorated: bool,
}

fn build_controls(
    mut cmd: CommandQueue,
    registry: Res<PanelRegistry>,
    style: Res<WindowStyle>,
    window: Res<Window>,
    theme: Res<UITheme>,
) {
    window.window_handle.set_decorations(style.decorated);
    // An undecorated window is not always given focus by the window manager,
    // and without focus it receives no keys at all.
    window.window_handle.focus_window();
    if style.decorated {
        return;
    }

    if let Some(root) = registry.root() {
        let bar = cmd
            .spawn((
                UINode {
                    height: UIValue::Px(TOP_STRIP),
                    position: Position::Absolute,
                    inset: UIInset {
                        top: UIValue::Px(0.0),
                        left: UIValue::Px(0.0),
                        right: UIValue::Px(0.0),
                        ..Default::default()
                    },
                    z_index: DRAG_LAYER,
                    ..Default::default()
                },
                Interactable,
                DragHandle,
            ))
            .entity();
        cmd.add_child(root, bar);
    }

    if let Some(body) = registry.body(PANEL_ID) {
        let bar = cmd
            .spawn(UINode {
                flex_direction: FlexDirection::Row,
                align_items: Some(taffy::AlignItems::Center),
                gap: glam::Vec2::new(2.0, 0.0),
                padding: UIRect::axes(0.0, theme.spacing_md),
                ..Default::default()
            })
            .entity();
        cmd.add_child(body, bar);

        for (control, mark) in [
            (Control::Minimise, glyph::MINUS),
            (Control::Maximise, glyph::CORNERS_OUT),
            (Control::Close, glyph::X),
        ] {
            let button = cmd
                .spawn((
                    UINode {
                        // Big enough that the glyph's line box fits: a line
                        // taller than its box is dropped, not clipped.
                        width: UIValue::Px(30.0),
                        height: UIValue::Px(26.0),
                        flex_shrink: 0.0,
                        padding: UIRect::axes(3.0, 8.0),
                        // Above the title band, which spans the whole strip.
                        z_index: CONTROL_LAYER,
                        ..Default::default()
                    },
                    UIMaterial {
                        corner_radius: theme.radius_sm,
                        ..UIMaterial::flat(Color::srgba(0.0, 0.0, 0.0, 0.0))
                    },
                    TextComponent {
                        color: theme.text_muted,
                        ..icon(&theme, mark, theme.font_size_lg)
                    },
                    Interactable,
                    UIInteractionStyle {
                        normal: Color::srgba(0.0, 0.0, 0.0, 0.0),
                        hovered: theme.surface_hovered,
                        // Closing is the one that wants to look dangerous.
                        pressed: match control {
                            Control::Close => theme.error,
                            _ => theme.accent,
                        },
                        disabled: Color::srgba(0.0, 0.0, 0.0, 0.0),
                    },
                    control,
                    WindowChromeControl,
                ))
                .entity();
            cmd.add_child(bar, button);
            if matches!(control, Control::Maximise) {
                cmd.insert(MaximiseGlyph, button);
            }
        }
    }

    if let Some(root) = registry.root() {
        for grip in grips() {
            let entity = cmd.spawn(grip).entity();
            cmd.add_child(root, entity);
        }
    }
}

/// The eight edge and corner strips, as (node, marker, hit-test opt-in) triples.
fn grips() -> Vec<(UINode, ResizeGrip, Interactable)> {
    let px = UIValue::Px;
    let edge = |inset: UIInset, width: UIValue, height: UIValue, direction| {
        (
            UINode {
                width,
                height,
                position: Position::Absolute,
                inset,
                z_index: GRIP_LAYER,
                ..Default::default()
            },
            ResizeGrip(direction),
            Interactable,
        )
    };
    let corner = |inset: UIInset, direction| {
        (
            UINode {
                width: px(CORNER),
                height: px(CORNER),
                position: Position::Absolute,
                inset,
                // Above the edges, so a corner is a corner and not the edge it
                // overlaps.
                z_index: GRIP_LAYER + 1,
                ..Default::default()
            },
            ResizeGrip(direction),
            Interactable,
        )
    };
    let side = |top, right, bottom, left| UIInset {
        top,
        right,
        bottom,
        left,
    };
    let auto = UIValue::Auto;
    vec![
        edge(
            side(px(0.0), px(0.0), auto, px(0.0)),
            auto,
            px(GRIP),
            ResizeDirection::North,
        ),
        edge(
            side(auto, px(0.0), px(0.0), px(0.0)),
            auto,
            px(GRIP),
            ResizeDirection::South,
        ),
        edge(
            side(px(0.0), auto, px(0.0), px(0.0)),
            px(GRIP),
            auto,
            ResizeDirection::West,
        ),
        edge(
            side(px(0.0), px(0.0), px(0.0), auto),
            px(GRIP),
            auto,
            ResizeDirection::East,
        ),
        corner(
            side(px(0.0), auto, auto, px(0.0)),
            ResizeDirection::NorthWest,
        ),
        corner(
            side(px(0.0), px(0.0), auto, auto),
            ResizeDirection::NorthEast,
        ),
        corner(
            side(auto, auto, px(0.0), px(0.0)),
            ResizeDirection::SouthWest,
        ),
        corner(
            side(auto, px(0.0), px(0.0), auto),
            ResizeDirection::SouthEast,
        ),
    ]
}

/// Publishes the window's own frame — the edges and the title band — for the
/// event loop to hit-test when a press arrives.
///
/// Rectangles rather than "what is hovered": the press is acted on as it
/// happens, and the hover is always a frame behind it, so a press that lands
/// as the pointer reaches a grip would find nothing there.
fn publish_window_gestures(
    grips: Query<(&ResizeGrip, &UILayout)>,
    handles: Query<(&DragHandle, &UILayout)>,
    controls: Query<(&Control, &UILayout)>,
    chrome_controls: Query<(&WindowChromeControl, &UILayout)>,
    mut region: ResMut<WindowGestureRegion>,
) {
    let mut zones = Vec::new();
    for (grip, layout) in grips.iter() {
        push_zone(
            &mut zones,
            layout,
            Some(WindowGesture::Resize { direction: grip.0 }),
        );
    }
    for (_, layout) in handles.iter() {
        push_zone(&mut zones, layout, Some(WindowGesture::Move));
    }
    // A control standing in the title band is a button first.
    for (_, layout) in controls.iter() {
        push_zone(&mut zones, layout, None);
    }
    for (_, layout) in chrome_controls.iter() {
        push_zone(&mut zones, layout, None);
    }
    // Topmost first, so a corner beats the edge it overlaps and a button beats
    // the band it stands in — the order the UI itself paints them in.
    zones.sort_by_key(|(paint_order, _)| std::cmp::Reverse(*paint_order));
    region.zones = zones.into_iter().map(|(_, zone)| zone).collect();
}

/// Adds a node's visible rectangle, skipping it when clipping leaves nothing.
fn push_zone(
    zones: &mut Vec<(i64, WindowGestureZone)>,
    layout: &UILayout,
    gesture: Option<WindowGesture>,
) {
    let visible = layout.rect.intersection(layout.clip_rect);
    if visible.size.x <= 0.0 || visible.size.y <= 0.0 {
        return;
    }
    zones.push((
        layout.paint_order,
        WindowGestureZone {
            min: visible.min,
            max: visible.max(),
            gesture,
        },
    ));
}

fn handle_controls(
    mut clicks: EventReader<UIClick>,
    controls: Query<&Control>,
    window: Res<Window>,
    mut close: ResMut<CloseRequest>,
) {
    for click in clicks.read() {
        let Some(control) = controls.get_entity(click.entity) else {
            continue;
        };
        match control {
            Control::Minimise => window.window_handle.set_minimized(true),
            Control::Maximise => window
                .window_handle
                .set_maximized(!window.window_handle.is_maximized()),
            Control::Close => close.0 = true,
        }
    }
}

fn sync_maximise_glyph(window: Res<Window>, glyphs: Query<(&MaximiseGlyph, &mut TextComponent)>) {
    let mark = if window.window_handle.is_maximized() {
        glyph::CORNERS_IN
    } else {
        glyph::CORNERS_OUT
    };
    for (_, mut text) in glyphs.iter() {
        let mark = mark.to_string();
        if text.text != mark {
            text.text = mark;
        }
    }
}

//! What a title bar would have given us, now that the window has none.
use concerto_app::{
    App, Plugin,
    schedule_groups::{LateUpdate, Startup},
};
use concerto_ecs::{
    Component, Query, Res, ResMut, Resource,
    command::CommandQueue,
    signal::{
        On,
        listener::{IntoListener, Listener},
    },
};
use concerto_ui::{
    elements::prelude::*,
    interaction::{Interactable, UIClick},
    node::{UIInset, UILayout, UINode, UIRect},
    text::UIText,
    theme::UITheme,
    transform::UIValue,
};
use concerto_window::input::MouseButton;
use concerto_window::plugin::{
    ResizeDirection, Window, WindowCommand, WindowGesture, WindowGestureRegion, WindowGestureZone,
};
use taffy::Position;

use crate::dock::{DockedApp, PanelDescriptor, PanelRegistry, Region, TOP_STRIP};
use crate::fonts::{glyph, icon};

pub const PANEL_ID: &str = "concerto.window";

const GRIP: f32 = 6.0;
const CORNER: f32 = 14.0;
const GRIP_LAYER: i32 = 100;
const DRAG_LAYER: i32 = 50;
pub(crate) const CONTROL_LAYER: i32 = 60;

/// Marks an interactive title-strip control that must block window dragging.
#[derive(Component, Clone, Copy, Default)]
pub struct WindowChromeControl;

#[derive(Clone, Copy)]
enum Control {
    Minimise,
    Maximise,
    Close,
}

#[derive(Component)]
struct DragHandle;

#[derive(Component, Clone, Copy)]
struct ResizeGrip(ResizeDirection);

#[derive(Component)]
struct MaximiseGlyph;

/// Whether the window manager draws the frame.
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
    mut window: ResMut<Window>,
    theme: Res<UITheme>,
) {
    window.settings.decorations = style.decorated;
    window.send(WindowCommand::Focus);
    if style.decorated {
        return;
    }

    if let Some(root) = registry.root() {
        cmd.entity(root).add_child((
            UINode::default()
                .with_height(UIValue::Px(TOP_STRIP))
                .with_position(Position::Absolute)
                .with_inset(UIInset {
                    top: UIValue::Px(0.0),
                    left: UIValue::Px(0.0),
                    right: UIValue::Px(0.0),
                    ..Default::default()
                })
                .with_z_index(DRAG_LAYER),
            Interactable,
            DragHandle,
        ));
    }

    if let Some(body) = registry.body(PANEL_ID) {
        let mut body_queue = cmd.entity(body);
        let mut bar = body_queue.spawn_child(
            theme
                .row()
                .gap(2.0)
                .padding(UIRect::axes(0.0, theme.spacing_md)),
        );

        for (control, mark) in [
            (Control::Minimise, glyph::MINUS),
            (Control::Maximise, glyph::CORNERS_OUT),
            (Control::Close, glyph::X),
        ] {
            bar.add_child_with(
                (
                    theme
                        .pressable()
                        .pressed(match control {
                            Control::Close => theme.error,
                            _ => theme.accent,
                        })
                        .size(UIValue::Px(30.0), UIValue::Px(26.0))
                        .padding(UIRect::axes(3.0, 8.0))
                        .z_index(CONTROL_LAYER)
                        .on_click(press_control(control)),
                    icon(&theme, mark, theme.font_size_lg).muted(),
                    WindowChromeControl,
                ),
                |mut button| {
                    if matches!(control, Control::Maximise) {
                        button.insert(MaximiseGlyph);
                    }
                },
            );
        }
    }

    if let Some(root) = registry.root() {
        let mut root_queue = cmd.entity(root);
        for grip in grips() {
            root_queue.add_child(grip);
        }
    }
}

fn grips() -> Vec<(UINode, ResizeGrip, Interactable)> {
    let px = UIValue::Px;
    let edge = |inset: UIInset, width: UIValue, height: UIValue, direction| {
        (
            UINode::default()
                .with_size(width, height)
                .with_position(Position::Absolute)
                .with_inset(inset)
                .with_z_index(GRIP_LAYER),
            ResizeGrip(direction),
            Interactable,
        )
    };
    let corner = |inset: UIInset, direction| {
        (
            UINode::default()
                .with_size(px(CORNER), px(CORNER))
                .with_position(Position::Absolute)
                .with_inset(inset)
                .with_z_index(GRIP_LAYER + 1),
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

fn publish_window_gestures(
    grips: Query<(&ResizeGrip, &UILayout)>,
    handles: Query<(&DragHandle, &UILayout)>,
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
    for (_, layout) in chrome_controls.iter() {
        push_zone(&mut zones, layout, None);
    }
    zones.sort_by_key(|(paint_order, _)| std::cmp::Reverse(*paint_order));
    region.zones = zones.into_iter().map(|(_, zone)| zone).collect();
}

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

fn press_control(control: Control) -> Listener<UIClick> {
    (move |on: On<UIClick>,
           mut window: ResMut<Window>,
           mut guard: ResMut<crate::guard::UnsavedGuard>| {
        if on.signal().button != MouseButton::Left {
            return;
        }
        match control {
            Control::Minimise => window.send(WindowCommand::Minimise),
            Control::Maximise => {
                let maximised = window.is_maximised();
                window.send(WindowCommand::SetMaximised(!maximised));
            }
            Control::Close => guard.request_quit(),
        }
    })
    .into_listener()
}

fn sync_maximise_glyph(window: Res<Window>, glyphs: Query<(&MaximiseGlyph, &mut UIText)>) {
    let mark = if window.is_maximised() {
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

#[cfg(test)]
mod tests {
    use super::*;
    use concerto_ecs::World;

    fn clicked(control: Control) -> Vec<WindowCommand> {
        let mut world = World::new();
        world.insert_resource(Window::new((1280, 720), 1.0));
        world.insert_resource(crate::guard::UnsavedGuard::default());
        let button = world.spawn(press_control(control));
        world.trigger_on(
            button,
            UIClick {
                position: Default::default(),
                button: MouseButton::Left,
            },
        );
        world.get_resource_mut::<Window>().unwrap().drain_commands()
    }

    #[test]
    fn each_control_sends_its_own_window_command() {
        assert_eq!(clicked(Control::Minimise), [WindowCommand::Minimise]);
        assert_eq!(
            clicked(Control::Maximise),
            [WindowCommand::SetMaximised(true)]
        );
    }
}

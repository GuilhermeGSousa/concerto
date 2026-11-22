use concerto_color::Color;
use concerto_ecs::{
    command::CommandQueue,
    component::Component,
    entity::{Entity, hierarchy::ChildOf},
    query::{
        Query,
        filter::{Added, With},
    },
    resource::Res,
    signal::{EntitySignal, On, Signal},
};
use concerto_window::input::MouseButton;

use crate::{
    interaction::{HoveredNode, UIDrag, UIInputState},
    material::UIMaterial,
    node::UINode,
    scroll::UISplitAxis,
    transform::UIValue,
};

/// The width of a [`UISplitter`]'s hit area across its axis, in logical pixels.
pub const SPLITTER_HIT_SIZE: f32 = 8.0;

/// A draggable gap between two siblings that reports drags along its axis and moves nothing itself.
#[derive(Component)]
pub struct UISplitter {
    pub axis: UISplitAxis,
    /// Colour of the line at rest.
    pub color: Color,
    /// Colour of the line while hovered or dragged.
    pub active_color: Color,
}

impl UISplitter {
    pub fn new(axis: UISplitAxis) -> Self {
        Self {
            axis,
            color: Color::TRANSPARENT,
            active_color: Color::rgba(0.25, 0.55, 0.95, 1.0),
        }
    }
}

/// Marker placed on the line child entity spawned by `setup_splitter_visuals`.
#[derive(Component)]
pub(crate) struct UISplitterLine;

/// Sent to a [`UISplitter`] while it is dragged, with the pointer's movement along its axis since the last one, in logical pixels.
pub struct UISplitterDragged {
    pub delta: f32,
}

impl Signal for UISplitterDragged {}
impl EntitySignal for UISplitterDragged {}

/// Spawns the one-pixel line child of each newly added [`UISplitter`].
pub(crate) fn setup_splitter_visuals(
    new_splitters: Query<(Entity, &UISplitter), Added<UISplitter>>,
    mut cmd: CommandQueue,
) {
    for (entity, splitter) in new_splitters.iter() {
        let node = match splitter.axis {
            UISplitAxis::Horizontal => UINode::default().with_width(UIValue::Px(1.0)),
            UISplitAxis::Vertical => UINode::default().with_height(UIValue::Px(1.0)),
        };
        let line = cmd
            .spawn((
                UISplitterLine,
                node.with_flex_shrink(0.0),
                UIMaterial::flat(splitter.color),
            ))
            .entity();
        cmd.add_child(entity, line);
    }
}

/// Drag listener that reports the movement along the axis of the [`UISplitter`] it sits on.
pub fn drag_splitter(on: On<UIDrag>, splitters: Query<&UISplitter>, mut cmd: CommandQueue) {
    let drag = on.signal();
    if drag.button != MouseButton::Left {
        return;
    }
    let Some(splitter) = splitters.get_entity(on.entity()) else {
        return;
    };
    let delta = match splitter.axis {
        UISplitAxis::Horizontal => drag.delta.x,
        UISplitAxis::Vertical => drag.delta.y,
    };
    if delta != 0.0 {
        cmd.entity(on.entity()).trigger(UISplitterDragged { delta });
    }
}

/// Lights each splitter's line while the splitter is hovered or dragged.
pub(crate) fn sync_splitter_lines(
    mut lines: Query<(&mut UIMaterial, &ChildOf), With<UISplitterLine>>,
    splitters: Query<&UISplitter>,
    hovered: Res<HoveredNode>,
    input: Res<UIInputState>,
) {
    let dragged = input.captured(MouseButton::Left);
    for (mut material, child_of) in lines.iter_mut() {
        let owner = **child_of;
        let Some(splitter) = splitters.get_entity(owner) else {
            continue;
        };
        let active = dragged == Some(owner) || (dragged.is_none() && **hovered == Some(owner));
        let color = if active {
            splitter.active_color
        } else {
            splitter.color
        }
        .to_linear();
        if material.color != color {
            material.color = color;
        }
    }
}

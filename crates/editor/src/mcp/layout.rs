use concerto_app::{App, schedule_groups::First};
use concerto_ecs::{
    Component, Entity, IntoSystemConfig, Query, Res, command::CommandQueue,
    system::input::StaticSystemInput, system::input::SystemInputData,
};
use concerto_mcp::{AppMcpExt, McpTool, PendingCall, ToolCalls, ToolError};
use concerto_ui::node::UILayout;
use concerto_window::plugin::Window;
use schemars::JsonSchema;
use serde::Deserialize;

use crate::{
    dock::{EditorLayout, LayoutPanel, LayoutPart, PanelState, PanelStates, ResizePanel},
    hosted::input::ViewportShown,
    viewport::publish_viewport_view,
};

/// Sets the size of one of the panels around the viewport, held within what the window allows, and answers the layout and the viewport's new size.
#[derive(Deserialize, JsonSchema)]
pub struct SetPanelSize {
    /// The panel to size: left, right, bottom or browser.
    pub panel: PanelName,
    /// The wanted size in logical pixels: the width of left, right and browser, the height of bottom.
    pub size: f32,
}

/// A panel `set_panel_size` sizes.
#[derive(Deserialize, JsonSchema, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub enum PanelName {
    /// The panels left of the viewport.
    Left,
    /// The panels right of the viewport.
    Right,
    /// The strip under the viewport.
    Bottom,
    /// The Asset Browser inside the strip, while an asset editor shares the strip with it.
    Browser,
}

impl From<PanelName> for LayoutPanel {
    fn from(name: PanelName) -> Self {
        match name {
            PanelName::Left => Self::Left,
            PanelName::Right => Self::Right,
            PanelName::Bottom => Self::Bottom,
            PanelName::Browser => Self::Browser,
        }
    }
}

#[derive(Component)]
struct AwaitingLayout(LayoutPanel);

pub(super) type LayoutInputs = (
    Res<'static, EditorLayout>,
    Res<'static, PanelStates>,
    Res<'static, Window>,
    Res<'static, ViewportShown>,
);

const PANELS: [LayoutPanel; 4] = [
    LayoutPanel::Left,
    LayoutPanel::Right,
    LayoutPanel::Bottom,
    LayoutPanel::Browser,
];

impl LayoutPanel {
    fn name(self) -> &'static str {
        match self {
            LayoutPanel::Left => "left",
            LayoutPanel::Right => "right",
            LayoutPanel::Bottom => "bottom",
            LayoutPanel::Browser => "browser",
        }
    }
}

pub(super) fn layout_line((layout, states, window, _): &SystemInputData<LayoutInputs>) -> String {
    let shown = layout.shown(states, window.logical_size());
    let panels: Vec<String> = PANELS
        .into_iter()
        .map(|panel| match states.of(panel) {
            PanelState::Sized => format!("{} {}", panel.name(), shown.size(panel).round()),
            PanelState::Hidden => format!("{} hidden", panel.name()),
            PanelState::Fill => format!("{} fill", panel.name()),
        })
        .collect();
    format!("layout: {}", panels.join(" · "))
}

pub(super) fn viewport_line((_, _, _, viewport): &SystemInputData<LayoutInputs>) -> String {
    match viewport.0 {
        Some(view) => format!("viewport: {}x{}", view.size.0, view.size.1),
        None => "viewport: not shown".to_owned(),
    }
}

fn resized(inputs: &SystemInputData<LayoutInputs>) -> String {
    let layout = layout_line(inputs);
    if inputs.3.0.is_some() {
        format!("{layout}\n{}", viewport_line(inputs))
    } else {
        layout
    }
}

fn refused(panel: LayoutPanel, state: PanelState) -> ToolError {
    let is = match state {
        PanelState::Hidden => "is hidden",
        _ => "fills the space left",
    };
    let sized_on = match panel {
        LayoutPanel::Browser => {
            "a tab whose asset editor shares the bottom strip, such as a clip or a rig"
        }
        _ => "a scene or a preview tab",
    };
    ToolError::new(
        "refused",
        format!(
            "The {} panel {is} on the front tab and has no size to set; it is sized on {sized_on}.",
            panel.name()
        ),
    )
}

pub(super) fn build(app: &mut App) {
    app.add_tool::<SetPanelSize, _>(McpTool::new("set_panel_size"), set_panel_size)
        .add_system(First, answer_resized.after(publish_viewport_view));
}

fn set_panel_size(
    mut calls: ToolCalls<SetPanelSize>,
    states: Res<PanelStates>,
    mut cmd: CommandQueue,
) {
    for (arguments, reply) in calls.read() {
        let panel = LayoutPanel::from(arguments.panel);
        let state = states.of(panel);
        if state != PanelState::Sized {
            reply.err(refused(panel, state));
            continue;
        }
        cmd.trigger(ResizePanel {
            panel,
            size: arguments.size,
        });
        cmd.spawn((
            PendingCall::new(reply, "the panels to lay out"),
            AwaitingLayout(panel),
        ));
    }
}

fn answer_resized(
    waiting: Query<(Entity, &PendingCall, &AwaitingLayout)>,
    inputs: StaticSystemInput<LayoutInputs>,
    parts: Query<(&LayoutPart, &UILayout)>,
    mut cmd: CommandQueue,
) {
    let (layout, states, window, _) = &*inputs;
    let shown = layout.shown(states, window.logical_size());
    for (entity, pending, awaiting) in waiting.iter() {
        let panel = awaiting.0;
        let laid_out = states.of(panel) != PanelState::Sized
            || parts.iter().any(|(part, laid)| {
                let extent = match panel {
                    LayoutPanel::Bottom => laid.rect.size.y,
                    _ => laid.rect.size.x,
                };
                matches!(part, LayoutPart::Panel(part) if *part == panel)
                    && (extent - shown.size(panel)).abs() < 0.5
            });
        if laid_out {
            pending.reply.ok(resized(&inputs));
            cmd.despawn(entity);
        }
    }
}

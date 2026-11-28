//! Whether every edit `App`'s viewport draws its scene lit or unlit: the viewport's Lit/Unlit button and the `viewport_shading` MCP tool.
use concerto_app::{App, Plugin, schedule_groups::LateUpdate};
use concerto_ecs::{
    Component, IntoSystemConfig, Query, Res, ResMut, Resource,
    command::CommandQueue,
    query::filter::With,
    signal::{On, lifecycle::Add},
    system::NonSendMarker,
};
use concerto_mcp::{AppMcpExt, McpTool, Needs, ToolCalls, ToolRegistry};
use concerto_ui::{
    elements::prelude::*, interaction::UIClick, node::UINode, sets::UiSet, text::UIText,
    theme::UITheme,
};
use schemars::JsonSchema;
use serde::Deserialize;

use crate::{
    hosted::{EditTarget, HostRole, HostedApp, ParkedEditApp, PendingApp},
    tools::environment::{SetViewportUnlit, ViewportShading},
    viewport::ViewportToolbar,
};

/// Whether the editor draws every scene it edits unlit, for every edit `App`, kept for the session.
#[derive(Resource, Default)]
pub struct ShadingChoice {
    /// `true` draws every edited scene by base colour and emission alone.
    pub unlit: bool,
}

impl ShadingChoice {
    fn apply_to(&self, app: &mut HostedApp) {
        if app.role() != HostRole::Edit {
            return;
        }
        let shown = app
            .world()
            .get_resource::<ViewportShading>()
            .map(|shading| shading.unlit);
        if shown.is_some_and(|unlit| unlit != self.unlit) {
            app.trigger(SetViewportUnlit(self.unlit));
        }
    }
}

/// Keeps every edit `App` drawing as [`ShadingChoice`] says, and brings the viewport's Lit/Unlit button and the `viewport_shading` tool.
pub struct ViewportShadingPlugin;

impl Plugin for ViewportShadingPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(ShadingChoice::default())
            .add_listener(add_shading_button)
            .add_system(LateUpdate, apply_shading_choice)
            .add_system(LateUpdate, show_shading_button.before(UiSet::Materials));
    }

    fn finish(&self, app: &mut App) {
        if app.get_resource::<ToolRegistry>().is_none() {
            return;
        }
        app.add_tool::<ViewportShadingTool, _>(
            McpTool::new("viewport_shading").needs(Needs::Scene),
            viewport_shading,
        );
    }
}

/// Answers whether the viewport draws the edited scene lit or unlit, or sets it for every scene with unlit.
#[derive(Deserialize, JsonSchema)]
pub struct ViewportShadingTool {
    /// true draws by base colour and emission alone, without lights, shadows or fog; false draws lit.
    pub unlit: Option<bool>,
}

#[derive(Component)]
struct ShadingButton;

fn label(unlit: bool) -> &'static str {
    if unlit { "Unlit" } else { "Lit" }
}

fn apply_shading_choice(
    _: NonSendMarker,
    choice: Res<ShadingChoice>,
    mut hosted: Query<&mut HostedApp>,
    mut parked: Query<&mut ParkedEditApp>,
    mut pending: Query<&mut PendingApp>,
) {
    for mut app in hosted.iter_mut() {
        choice.apply_to(&mut app);
    }
    for mut app in parked.iter_mut() {
        choice.apply_to(&mut app.0);
    }
    for mut app in pending.iter_mut() {
        if let Some(held) = app.held_mut() {
            choice.apply_to(held);
        }
    }
}

fn add_shading_button(on: On<Add<ViewportToolbar>>, theme: Res<UITheme>, mut cmd: CommandQueue) {
    cmd.entity(on.signal().entity).add_child((
        theme
            .button(label(false))
            .small()
            .node(|node| node.with_visible(false))
            .on_click(toggle_shading),
        ShadingButton,
    ));
}

fn toggle_shading(_: On<UIClick>, mut choice: ResMut<ShadingChoice>) {
    choice.unlit = !choice.unlit;
}

fn show_shading_button(
    targets: Query<&HostedApp, With<EditTarget>>,
    choice: Res<ShadingChoice>,
    mut buttons: Query<(&mut UINode, &mut UIText), With<ShadingButton>>,
) {
    let editing = targets
        .iter()
        .next()
        .is_some_and(|app| app.role() == HostRole::Edit);
    let text = label(choice.unlit);
    for (mut node, mut shown) in buttons.iter_mut() {
        if node.visible != editing {
            node.visible = editing;
        }
        if shown.text != text {
            shown.text = text.to_owned();
        }
    }
}

fn viewport_shading(mut calls: ToolCalls<ViewportShadingTool>, mut choice: ResMut<ShadingChoice>) {
    for (arguments, reply) in calls.read() {
        if let Some(unlit) = arguments.unlit
            && choice.unlit != unlit
        {
            choice.unlit = unlit;
        }
        reply.ok(format!(
            "The viewport draws scenes {}.",
            label(choice.unlit).to_lowercase()
        ));
    }
}

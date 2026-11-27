//! Which component gizmo kinds every edit and play `App` draws always: the viewport's Gizmos menu and the `gizmos` MCP tool.
use std::collections::BTreeSet;

use concerto_app::{App, Plugin, schedule_groups::LateUpdate};
use concerto_ecs::{
    Component, Entity, IntoSystemConfig, Query, Res, ResMut, Resource,
    command::CommandQueue,
    query::filter::With,
    signal::{
        On,
        lifecycle::Add,
        listener::{IntoListener, Listener},
    },
    system::NonSendMarker,
};
use concerto_mcp::{AppMcpExt, McpTool, Needs, ToolCalls, ToolError, ToolRegistry};
use concerto_ui::{
    anchor::{UIAnchorAlign, UIAnchoredPanel, toggle_owned_panels},
    checkbox::{UICheckbox, UICheckboxChanged},
    elements::prelude::*,
    node::UINode,
    sets::UiSet,
    theme::UITheme,
    transform::UIValue,
};
use schemars::JsonSchema;
use serde::Deserialize;

use crate::{
    hosted::{
        EditTarget, EditWorld, HostRole, HostedApp, ParkedEditApp, PendingApp, PendingPlayApp,
    },
    tools::component_gizmos::{ComponentGizmos, SetGizmoAlways},
    viewport::ViewportToolbar,
};

const MENU_WIDTH: f32 = 180.0;

/// The gizmo kinds set to Always, for every edit and play `App` the editor hosts; every other kind is drawn for the selection.
#[derive(Resource, Default)]
pub struct GizmoChoices {
    always: BTreeSet<String>,
}

impl GizmoChoices {
    /// Whether the kind `kind` is drawn for every entity.
    pub fn always(&self, kind: &str) -> bool {
        self.always.contains(kind)
    }

    /// Draws the kind `kind` for every entity, or only for the selection and its descendants.
    pub fn set(&mut self, kind: &str, always: bool) {
        if always {
            self.always.insert(kind.to_owned());
        } else {
            self.always.remove(kind);
        }
    }
}

/// Keeps every edit and play `App` drawing the kinds in [`GizmoChoices`] always, and brings the viewport's Gizmos menu and the `gizmos` tool.
pub struct ComponentGizmosPlugin;

impl Plugin for ComponentGizmosPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(GizmoChoices::default())
            .add_listener(add_gizmos_button)
            .add_system(LateUpdate, apply_gizmo_choices)
            .add_system(
                LateUpdate,
                (show_gizmos_button, fill_gizmo_menu).before(UiSet::Materials),
            );
    }

    fn finish(&self, app: &mut App) {
        if app.get_resource::<ToolRegistry>().is_none() {
            return;
        }
        app.add_tool::<Gizmos, _>(McpTool::new("gizmos").needs(Needs::Scene), gizmos);
    }
}

/// Lists the gizmo kinds of the open scene, each drawn for the selection or always, or sets one kind for every scene and answers the list.
#[derive(Deserialize, JsonSchema)]
pub struct Gizmos {
    /// The kind to set, such as Bounds, as the list names it; give it with always.
    pub kind: Option<String>,
    /// true draws the kind for every entity, false only for the selection and its descendants.
    pub always: Option<bool>,
}

#[derive(Component)]
struct GizmosButton;

#[derive(Component)]
struct GizmoMenu {
    shown: Option<Vec<&'static str>>,
}

#[derive(Component)]
struct GizmoRow(&'static str);

fn kind_names(kinds: Option<&ComponentGizmos>) -> Vec<&'static str> {
    kinds
        .map(|kinds| kinds.kinds().iter().map(|kind| kind.name).collect())
        .unwrap_or_default()
}

fn apply_to(app: &mut HostedApp, choices: &GizmoChoices) {
    if app.role() == HostRole::Preview {
        return;
    }
    let changes: Vec<SetGizmoAlways> = app
        .world()
        .get_resource::<ComponentGizmos>()
        .map(|kinds| {
            kinds
                .kinds()
                .iter()
                .filter(|kind| kind.always != choices.always(kind.name))
                .map(|kind| SetGizmoAlways {
                    kind: kind.name.to_owned(),
                    always: !kind.always,
                })
                .collect()
        })
        .unwrap_or_default();
    for change in changes {
        app.trigger(change);
    }
}

fn differs(app: &HostedApp, choices: &GizmoChoices) -> bool {
    app.role() != HostRole::Preview
        && app
            .world()
            .get_resource::<ComponentGizmos>()
            .is_some_and(|kinds| {
                kinds
                    .kinds()
                    .iter()
                    .any(|kind| kind.always != choices.always(kind.name))
            })
}

pub(crate) fn apply_gizmo_choices(
    _: NonSendMarker,
    choices: Res<GizmoChoices>,
    mut hosted: Query<&mut HostedApp>,
    mut parked: Query<&mut ParkedEditApp>,
    mut pending: Query<&mut PendingApp>,
    mut starting: Query<&mut PendingPlayApp>,
) {
    for mut app in hosted.iter_mut() {
        if differs(&app, &choices) {
            apply_to(&mut app, &choices);
        }
    }
    for mut app in parked.iter_mut() {
        if differs(&app.0, &choices) {
            apply_to(&mut app.0, &choices);
        }
    }
    for mut app in pending.iter_mut() {
        if app.held().is_some_and(|held| differs(held, &choices))
            && let Some(held) = app.held_mut()
        {
            apply_to(held, &choices);
        }
    }
    for mut app in starting.iter_mut() {
        if app.0.held().is_some_and(|held| differs(held, &choices))
            && let Some(held) = app.0.held_mut()
        {
            apply_to(held, &choices);
        }
    }
}

fn add_gizmos_button(on: On<Add<ViewportToolbar>>, theme: Res<UITheme>, mut cmd: CommandQueue) {
    let button = cmd
        .entity(on.signal().entity)
        .spawn_child((
            theme
                .button("Gizmos")
                .small()
                .node(|node| node.with_visible(false))
                .on_click(toggle_owned_panels),
            GizmosButton,
        ))
        .entity();
    cmd.spawn((
        theme
            .dropdown(button)
            .anchor_align(UIAnchorAlign::End)
            .width(UIValue::Px(MENU_WIDTH))
            .padding(theme.spacing_xs),
        GizmoMenu { shown: None },
    ));
}

fn show_gizmos_button(
    targets: Query<Entity, With<EditTarget>>,
    mut buttons: Query<&mut UINode, With<GizmosButton>>,
    mut menus: Query<&mut UIAnchoredPanel, With<GizmoMenu>>,
) {
    let shown = targets.iter().next().is_some();
    for mut node in buttons.iter_mut() {
        if node.visible != shown {
            node.visible = shown;
        }
    }
    if !shown {
        for mut panel in menus.iter_mut() {
            if panel.open {
                panel.open = false;
            }
        }
    }
}

fn fill_gizmo_menu(
    kinds: EditWorld<Option<Res<ComponentGizmos>>>,
    choices: Res<GizmoChoices>,
    mut menus: Query<(Entity, &mut GizmoMenu)>,
    mut rows: Query<(&GizmoRow, &mut UICheckbox)>,
    theme: Res<UITheme>,
    mut cmd: CommandQueue,
) {
    let names = kind_names(kinds.as_deref());
    for (entity, mut menu) in menus.iter_mut() {
        if menu.shown.as_ref() == Some(&names) {
            continue;
        }
        let mut menu_children = cmd.entity(entity).despawn_children();
        if names.is_empty() {
            menu_children.add_child(theme.label("No gizmos").font_size(12.5).no_wrap());
        }
        for &name in &names {
            menu_children.add_child((
                theme
                    .checkbox(name, choices.always(name))
                    .on_change(choose_gizmo(name)),
                GizmoRow(name),
            ));
        }
        menu.shown = Some(names.clone());
    }
    for (row, mut checkbox) in rows.iter_mut() {
        let checked = choices.always(row.0);
        if checkbox.checked != checked {
            checkbox.checked = checked;
        }
    }
}

fn choose_gizmo(kind: &'static str) -> Listener<UICheckboxChanged> {
    (move |on: On<UICheckboxChanged>, mut choices: ResMut<GizmoChoices>| {
        choices.set(kind, on.signal().checked);
    })
    .into_listener()
}

fn listing(names: &[&'static str], choices: &GizmoChoices) -> String {
    if names.is_empty() {
        return "The scene has no gizmos.".into();
    }
    names
        .iter()
        .map(|name| {
            let shown = if choices.always(name) {
                "always"
            } else {
                "selected"
            };
            format!("{name} · {shown}")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn unknown_gizmo(kind: &str, names: &[&'static str]) -> ToolError {
    let message = if names.is_empty() {
        format!("The scene has no gizmo kind {kind}; it has no gizmos.")
    } else {
        format!(
            "The scene has no gizmo kind {kind}; give one of {}.",
            names.join(", ")
        )
    };
    ToolError::new("unknown_gizmo", message)
}

impl Gizmos {
    fn chosen(
        &self,
        names: &[&'static str],
        choices: &mut GizmoChoices,
    ) -> Result<String, ToolError> {
        match (self.kind.as_deref(), self.always) {
            (None, None) => {}
            (Some(kind), Some(always)) => {
                let name = names
                    .iter()
                    .find(|name| name.eq_ignore_ascii_case(kind))
                    .ok_or_else(|| unknown_gizmo(kind, names))?;
                choices.set(name, always);
            }
            _ => {
                return Err(ToolError::new(
                    "invalid_arguments",
                    "Give kind and always together to set a kind, or neither to list the kinds.",
                ));
            }
        }
        Ok(listing(names, choices))
    }
}

fn gizmos(
    mut calls: ToolCalls<Gizmos>,
    kinds: EditWorld<Option<Res<ComponentGizmos>>>,
    mut choices: ResMut<GizmoChoices>,
) {
    let names = kind_names(kinds.as_deref());
    for (arguments, reply) in calls.read() {
        match arguments.chosen(&names, &mut choices) {
            Ok(answer) => reply.ok(answer),
            Err(error) => reply.err(error),
        }
    }
}

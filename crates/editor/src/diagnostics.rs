//! UI cost counters, as a docked panel.
//!
//! The engine has recorded these every frame for a while and nothing read
//! them. They are also what proves the dock's tabbing: this panel shares the
//! bottom slot with Content.
use app::{
    schedule_groups::{LateUpdate, Startup},
    App, Plugin,
};
use ecs::{command::CommandQueue, Component, Query, Res, ResMut, Resource};
use essential::time::Time;
use taffy::FlexDirection;
use ui::{
    node::{UILayoutDiagnostics, UINode},
    text::{FontFamily, TextComponent},
    theme::UITheme,
    UIRenderDiagnostics,
};

use crate::dock::{DockedApp, PanelDescriptor, PanelRegistry, Region};

pub const PANEL_ID: &str = "rabbithole.stats";

#[derive(Component)]
struct Readout;

/// How often the readout refreshes. A number that changes every frame is both
/// unreadable and, now that text participates in layout, a relayout per frame.
const REFRESH: f32 = 0.25;

#[derive(Resource, Default)]
struct Sampler {
    elapsed: f32,
    frames: u32,
}

pub struct DiagnosticsPlugin;

impl Plugin for DiagnosticsPlugin {
    fn build(&self, app: &mut App) {
        app.add_panel(PanelDescriptor {
            id: PANEL_ID,
            title: "Stats",
            region: Region::Stats,
        });
        app.insert_resource(Sampler::default());
        app.add_system(Startup, build_panel)
            .add_system(LateUpdate, refresh_panel);
    }
}

fn build_panel(mut cmd: CommandQueue, registry: Res<PanelRegistry>, theme: Res<UITheme>) {
    let Some(body) = registry.body(PANEL_ID) else {
        return;
    };
    let panel = cmd
        .spawn(
            UINode {
                flex_grow: 1.0,
                flex_direction: FlexDirection::Column,
                // The strip is only as tall as one line; padding would clip it.
                ..Default::default()
            }
            .clipped(),
        )
        .entity();
    cmd.add_child(body, panel);

    let readout = cmd
        .spawn((
            UINode {
                flex_grow: 1.0,
                ..Default::default()
            },
            TextComponent {
                text: String::new(),
                font_family: FontFamily::Monospace,
                font_size: theme.font_size_sm,
                line_height: theme.line_height(theme.font_size_sm),
                color: theme.text_muted,
                ..Default::default()
            },
            Readout,
        ))
        .entity();
    cmd.add_child(panel, readout);
}

fn refresh_panel(
    layout: Res<UILayoutDiagnostics>,
    render: Res<UIRenderDiagnostics>,
    time: Res<Time>,
    mut sampler: ResMut<Sampler>,
    readouts: Query<(&Readout, &mut TextComponent)>,
) {
    sampler.elapsed += time.delta().as_secs_f32();
    sampler.frames += 1;
    if sampler.elapsed < REFRESH {
        return;
    }
    let fps = sampler.frames as f32 / sampler.elapsed;
    sampler.elapsed = 0.0;
    sampler.frames = 0;

    // One line, like the design's stats strip: the running cost at a glance
    // rather than a table nobody reads mid-edit.
    let value = format!(
        "{fps:.0} fps · {} layouts · {} quads · {} shapes",
        layout.layout_passes,
        render.geometry_rebuilds(),
        render.text_reshapes(),
    );
    for (_, mut component) in readouts.iter() {
        if component.text != value {
            component.text = value.clone();
        }
    }
}

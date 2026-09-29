//! UI cost counters, as a docked panel.
use concerto_app::{
    App, Plugin,
    schedule_groups::{LateUpdate, Startup},
};
use concerto_ecs::{Component, Query, Res, ResMut, Resource, command::CommandQueue};
use concerto_foundation::time::Time;
use concerto_ui::{
    UIRenderDiagnostics,
    node::{UILayoutDiagnostics, UINode},
    text::{FontFamily, TextComponent},
    theme::UITheme,
};
use taffy::FlexDirection;

use crate::dock::{DockedApp, PanelDescriptor, PanelRegistry, Region};

pub const PANEL_ID: &str = "concerto.stats";

#[derive(Component)]
struct Readout;

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
    cmd.entity(body).add_child_with(
        UINode {
            flex_grow: 1.0,
            flex_direction: FlexDirection::Column,
            ..Default::default()
        }
        .clipped(),
        |panel| {
            panel.add_child((
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
            ));
        },
    );
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

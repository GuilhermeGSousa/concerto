use concerto_app::{
    plugins::Plugin,
    schedule_groups::{Startup, Update},
};
use concerto_color::Color;
use concerto_ecs::{
    command::CommandQueue,
    component::Component,
    query::{Query, filter::With},
    resource::{Res, ResMut, Resource},
};
use concerto_foundation::time::{FrameStats, Time};

use crate::{
    material::UIMaterial,
    node::{UINode, UIRect},
    text::{FontFamily, UIText},
    transform::UIValue,
};

#[derive(Component)]
struct FrameStatsText;

const REFRESH_INTERVAL: f32 = 0.25;

#[derive(Resource)]
struct OverlayRefreshTimer(f32);

/// Small always-on-top frame-time readout in the window's top-left corner.
pub struct FrameStatsOverlayPlugin;

impl Plugin for FrameStatsOverlayPlugin {
    fn build(&self, app: &mut concerto_app::App) {
        app.insert_resource(OverlayRefreshTimer(0.0));
        app.add_system(Startup, spawn_overlay);
        app.add_system(Update, update_overlay_text);
    }
}

fn spawn_overlay(mut cmd: CommandQueue) {
    cmd.spawn((
        UINode::default()
            .with_size(UIValue::Px(230.0), UIValue::Px(46.0))
            .with_padding(UIRect::axes(6.0, 10.0))
            .with_margin(UIRect::all(8.0)),
        UIMaterial::flat(Color::rgba(0.0, 0.0, 0.0, 0.6)),
    ))
    .add_child((
        UINode::default().with_flex_grow(1.0),
        UIText {
            text: "-- FPS".to_string(),
            font_size: 12.0,
            line_height: 16.0,
            font_family: FontFamily::Monospace,
            ..Default::default()
        },
        FrameStatsText,
    ));
}

fn update_overlay_text(
    time: Res<Time>,
    stats: Res<FrameStats>,
    mut timer: ResMut<OverlayRefreshTimer>,
    text_nodes: Query<&mut UIText, With<FrameStatsText>>,
) {
    timer.0 += time.delta().as_secs_f32();
    if timer.0 < REFRESH_INTERVAL || stats.is_empty() {
        return;
    }
    timer.0 = 0.0;

    for mut text in text_nodes.iter() {
        text.text = format!(
            "{:>6.0} FPS  {:>6.2} ms\np99 {:>5.2} ms  max {:>5.2} ms",
            stats.fps(),
            stats.average_ms(),
            stats.percentile_ms(0.99),
            stats.max_ms(),
        );
    }
}

use concerto_app::extractor::Extracted;
use concerto_color::Color;
use concerto_ecs::{
    World,
    command::CommandQueue,
    component::Component,
    query::{
        Query,
        filter::{Added, Changed, Or},
    },
    resource::{Res, ResMut},
    system::input::SystemLocal,
    world::FromWorld,
};
use concerto_render::components::render_entity::RenderEntity;
use concerto_window::plugin::Window;
use glyphon::{Attrs, Buffer, Family, FontSystem, Metrics, Shaping, Style, Weight, Wrap};
use std::hash::{Hash, Hasher};

use crate::{node::UILayout, resources::UIRenderDiagnostics, text::resources::TextFontSystem};

pub mod fonts;
pub(crate) mod resources;

/// Font family for a text node.
#[derive(Clone, Default)]
pub enum FontFamily {
    #[default]
    SansSerif,
    Serif,
    Monospace,
    /// Any font family by name (e.g. `"JetBrains Mono"`).
    Name(String),
}

/// Font style (normal or italic).
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum FontStyle {
    #[default]
    Normal,
    Italic,
}

/// Text node component.
#[derive(Component, Clone)]
pub struct UIText {
    pub text: String,
    pub font_size: f32,
    pub line_height: f32,
    /// Font family. Defaults to [`FontFamily::SansSerif`].
    pub font_family: FontFamily,
    /// Font weight (100–900). 400 = regular, 700 = bold. Defaults to 400.
    pub font_weight: u16,
    /// Font style. Defaults to [`FontStyle::Normal`].
    pub font_style: FontStyle,
    /// Semantic foreground color.
    pub color: Color,
    /// Whether text may wrap at word boundaries inside the content rectangle.
    pub wrap: bool,
    /// Whether text too long for its box is cut short with an ellipsis.
    pub ellipsis: bool,
}

impl Default for UIText {
    fn default() -> Self {
        Self {
            text: String::new(),
            font_size: 14.0,
            line_height: 18.0,
            font_family: FontFamily::default(),
            font_weight: 400,
            font_style: FontStyle::default(),
            color: Color::srgba(0.945, 0.925, 0.885, 1.0),
            wrap: true,
            ellipsis: false,
        }
    }
}

pub(crate) fn physical_metrics(font_size: f32, line_height: f32, scale: f32) -> Metrics {
    Metrics {
        font_size: font_size * scale,
        line_height: line_height * scale,
    }
}

fn truncate_to_width(
    font_system: &mut FontSystem,
    metrics: Metrics,
    attrs: Attrs<'_>,
    text: &str,
    width: f32,
) -> Option<String> {
    if width <= 0.0 || measure_line(font_system, metrics, attrs, text) <= width {
        return None;
    }

    const ELLIPSIS: char = '…';
    let boundaries: Vec<usize> = text
        .char_indices()
        .map(|(index, _)| index)
        .chain(std::iter::once(text.len()))
        .collect();

    let mut low = 0;
    let mut high = boundaries.len() - 1;
    while low < high {
        let mid = (low + high).div_ceil(2);
        let mut candidate = text[..boundaries[mid]].to_string();
        candidate.push(ELLIPSIS);
        if measure_line(font_system, metrics, attrs, &candidate) <= width {
            low = mid;
        } else {
            high = mid - 1;
        }
    }

    let mut truncated = text[..boundaries[low]].to_string();
    truncated.push(ELLIPSIS);
    Some(truncated)
}

/// Truncation with its own font system, for tests.
#[doc(hidden)]
pub fn truncate_for_test(text: &str, width: f32, metrics: (f32, f32)) -> Option<String> {
    let mut font_system = fonts::build_font_system(&fonts::UIFonts::default());
    truncate_to_width(
        &mut font_system,
        Metrics {
            font_size: metrics.0,
            line_height: metrics.1,
        },
        Attrs::new(),
        text,
        width,
    )
}

fn measure_line(
    font_system: &mut FontSystem,
    metrics: Metrics,
    attrs: Attrs<'_>,
    text: &str,
) -> f32 {
    let mut buffer = Buffer::new(font_system, metrics);
    buffer.set_size(font_system, None, None);
    buffer.set_wrap(font_system, Wrap::None);
    buffer.set_text(font_system, text, attrs, Shaping::Advanced);
    buffer.shape_until_scroll(font_system, false);
    buffer
        .layout_runs()
        .map(|run| run.line_w)
        .fold(0.0_f32, f32::max)
}

#[derive(Component)]
pub(crate) struct RenderTextComponent {
    pub(crate) buffer: glyphon::Buffer,
    pub(crate) location: glam::Vec2,
    pub(crate) clip_min: glam::Vec2,
    pub(crate) clip_max: glam::Vec2,
    pub(crate) color: glyphon::Color,
    /// Explicit z-layer, recovered from the node's paint order.
    pub(crate) layer: i32,
    shaping: u64,
}

pub(crate) struct ExtractedScale {
    scale: Option<u32>,
}

impl FromWorld for ExtractedScale {
    fn from_world(_: &World) -> Self {
        Self { scale: None }
    }
}

pub(crate) fn extract_text_nodes(
    changed_text_nodes: Extracted<
        Query<
            (&UIText, &UILayout, &RenderEntity),
            Or<(Changed<UIText>, Changed<UILayout>, Added<RenderEntity>)>,
        >,
    >,
    all_text_nodes: Extracted<Query<(&UIText, &UILayout, &RenderEntity)>>,
    window: Extracted<Res<Window>>,
    mut font_system: ResMut<TextFontSystem>,
    render_text_nodes: Query<&mut RenderTextComponent>,
    diagnostics: Res<UIRenderDiagnostics>,
    mut extracted: SystemLocal<ExtractedScale>,
    mut cmd: CommandQueue,
) {
    let scale = window.scale_factor() as f32;
    let rescaled = extracted.scale.replace(scale.to_bits()) != Some(scale.to_bits());

    let mut extract = |text_component: &UIText, layout: &UILayout, render_entity: &RenderEntity| {
        let clip = layout.clip_rect.intersection(layout.content_rect);
        if !layout.is_shown() || !clip.has_area() {
            if render_text_nodes.contains_entity(**render_entity) {
                cmd.remove::<RenderTextComponent>(**render_entity);
            }
            return;
        }
        let location = layout.content_rect.min * scale;
        let clip_min = clip.min * scale;
        let clip_max = clip.max() * scale;
        let rgba = text_component.color.to_srgba();
        let color = glyphon::Color::rgba(
            (rgba.r * 255.0).round() as u8,
            (rgba.g * 255.0).round() as u8,
            (rgba.b * 255.0).round() as u8,
            (rgba.a * 255.0).round() as u8,
        );
        let layer = text_layer(layout);
        let shaping = shaping_signature(text_component, layout, scale);

        if let Some(mut render_text) = render_text_nodes.get_entity(**render_entity)
            && render_text.shaping == shaping
        {
            if render_text.location != location
                || render_text.clip_min != clip_min
                || render_text.clip_max != clip_max
                || render_text.color != color
                || render_text.layer != layer
            {
                render_text.location = location;
                render_text.clip_min = clip_min;
                render_text.clip_max = clip_max;
                render_text.color = color;
                render_text.layer = layer;
            }
            return;
        }

        diagnostics.record_text_reshape();
        let buffer = shape_text(
            &mut font_system,
            text_component,
            layout.content_rect.size * scale,
            scale,
        );
        cmd.insert(
            RenderTextComponent {
                buffer,
                location,
                clip_min,
                clip_max,
                color,
                layer,
                shaping,
            },
            **render_entity,
        );
    };

    if rescaled {
        for (text_component, layout, render_entity) in all_text_nodes.iter() {
            extract(text_component, layout, render_entity);
        }
    } else {
        for (text_component, layout, render_entity) in changed_text_nodes.iter() {
            extract(text_component, layout, render_entity);
        }
    }
}

fn shape_text(
    font_system: &mut FontSystem,
    text_component: &UIText,
    content_size: glam::Vec2,
    scale: f32,
) -> Buffer {
    let metrics = physical_metrics(text_component.font_size, text_component.line_height, scale);
    let mut text_buffer = Buffer::new(font_system, metrics);

    text_buffer.set_size(
        font_system,
        Some(content_size.x.max(0.0)),
        Some(content_size.y.max(0.0)),
    );
    text_buffer.set_wrap(
        font_system,
        if text_component.wrap {
            Wrap::Word
        } else {
            Wrap::None
        },
    );

    let family = match &text_component.font_family {
        FontFamily::SansSerif => Family::SansSerif,
        FontFamily::Serif => Family::Serif,
        FontFamily::Monospace => Family::Monospace,
        FontFamily::Name(name) => Family::Name(name.as_str()),
    };

    let style = match text_component.font_style {
        FontStyle::Normal => Style::Normal,
        FontStyle::Italic => Style::Italic,
    };

    let attrs = Attrs::new()
        .family(family)
        .weight(Weight(text_component.font_weight))
        .style(style);

    let display = if text_component.ellipsis && !text_component.wrap {
        truncate_to_width(
            font_system,
            metrics,
            attrs,
            &text_component.text,
            content_size.x.max(0.0),
        )
    } else {
        None
    };
    text_buffer.set_text(
        font_system,
        display.as_deref().unwrap_or(&text_component.text),
        attrs,
        Shaping::Advanced,
    );
    text_buffer.shape_until_scroll(font_system, false);
    text_buffer
}

/// The explicit `z_index` a node was given, recovered from its paint order.
pub(crate) fn text_layer(layout: &UILayout) -> i32 {
    (layout.paint_order >> 32) as i32
}

fn shaping_signature(text: &UIText, layout: &UILayout, scale: f32) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.text.hash(&mut hasher);
    text.font_size.to_bits().hash(&mut hasher);
    text.line_height.to_bits().hash(&mut hasher);
    text.font_weight.hash(&mut hasher);
    (text.font_style as u8).hash(&mut hasher);
    text.wrap.hash(&mut hasher);
    text.ellipsis.hash(&mut hasher);
    match &text.font_family {
        FontFamily::SansSerif => 0_u8.hash(&mut hasher),
        FontFamily::Serif => 1_u8.hash(&mut hasher),
        FontFamily::Monospace => 2_u8.hash(&mut hasher),
        FontFamily::Name(name) => {
            3_u8.hash(&mut hasher);
            name.hash(&mut hasher);
        }
    }
    for value in [
        layout.content_rect.size.x,
        layout.content_rect.size.y,
        scale,
    ] {
        value.to_bits().hash(&mut hasher);
    }
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn widest(font_system: &mut FontSystem, metrics: Metrics, text: &str) -> f32 {
        measure_line(font_system, metrics, Attrs::new(), text)
    }

    #[test]
    fn physical_metrics_scale_both_size_and_line_height() {
        let metrics = physical_metrics(14.0, 20.0, 2.0);
        assert_eq!(metrics.font_size, 28.0);
        assert_eq!(metrics.line_height, 40.0);
    }

    #[test]
    fn text_shaped_at_scale_two_is_twice_as_wide() {
        let mut font_system = fonts::build_font_system(&fonts::UIFonts::default());
        let one = widest(
            &mut font_system,
            physical_metrics(14.0, 20.0, 1.0),
            "Forest",
        );
        let two = widest(
            &mut font_system,
            physical_metrics(14.0, 20.0, 2.0),
            "Forest",
        );
        assert!((two - one * 2.0).abs() < 1.0, "{one} vs {two}");
    }

    #[test]
    fn truncation_at_scale_two_matches_its_logical_width() {
        let mut font_system = fonts::build_font_system(&fonts::UIFonts::default());
        let text = "bunny_porcelain_subsurface.mat";
        let at_one = truncate_to_width(
            &mut font_system,
            physical_metrics(14.0, 20.0, 1.0),
            Attrs::new(),
            text,
            60.0,
        )
        .expect("must truncate");
        let at_two = truncate_to_width(
            &mut font_system,
            physical_metrics(14.0, 20.0, 2.0),
            Attrs::new(),
            text,
            120.0,
        )
        .expect("must truncate");
        assert_eq!(at_one, at_two);
        let width = widest(&mut font_system, physical_metrics(14.0, 20.0, 2.0), &at_two);
        assert!(width <= 120.0, "{width}");
    }

    fn extract_text(main: &mut concerto_ecs::World, render: &mut concerto_ecs::World) {
        if render
            .get_resource::<concerto_ecs::system::schedule::CompiledSchedules>()
            .is_none()
        {
            let mut schedules = concerto_ecs::system::schedule::Schedules::default();
            schedules.add_system(concerto_app::schedule_groups::Extract, extract_text_nodes);
            let compiled = schedules
                .compile::<concerto_ecs::system::executor::single_thread::SingleThreadedExecutor>(
                render,
            );
            render.insert_resource(compiled);
        }
        concerto_app::extractor::extract(main, render);
    }

    #[test]
    fn text_whose_clip_is_empty_is_not_drawn() {
        let mut main = concerto_ecs::World::new();
        main.insert_resource(Window::new((800, 600), 1.5));
        let mut render = concerto_ecs::World::new();
        render.insert_resource(TextFontSystem(fonts::build_font_system(
            &fonts::UIFonts::default(),
        )));
        render.insert_resource(UIRenderDiagnostics::default());
        let target = render.spawn(());
        let rect = crate::node::UIBox {
            min: glam::Vec2::new(10.5, 20.5),
            size: glam::Vec2::new(100.0, 20.0),
        };
        let node = main.spawn((
            UIText {
                text: "Culled".into(),
                ..Default::default()
            },
            UILayout {
                rect,
                content_rect: rect,
                clip_rect: rect,
                paint_order: 0,
            },
            RenderEntity::new(target),
        ));

        extract_text(&mut main, &mut render);
        assert!(
            render
                .get_component_for_entity::<RenderTextComponent>(target)
                .is_some()
        );

        main.get_component_for_entity_mut::<UILayout>(node)
            .unwrap()
            .clip_rect = crate::node::UIBox {
            min: rect.min,
            size: glam::Vec2::ZERO,
        };
        extract_text(&mut main, &mut render);
        assert!(
            render
                .get_component_for_entity::<RenderTextComponent>(target)
                .is_none(),
            "an empty clip must leave nothing to draw"
        );
    }

    fn text_worlds(
        labels: &[&str],
    ) -> (
        concerto_ecs::World,
        concerto_ecs::World,
        Vec<(concerto_ecs::entity::Entity, concerto_ecs::entity::Entity)>,
    ) {
        let mut main = concerto_ecs::World::new();
        main.insert_resource(Window::new((800, 600), 1.0));
        let mut render = concerto_ecs::World::new();
        render.insert_resource(TextFontSystem(fonts::build_font_system(
            &fonts::UIFonts::default(),
        )));
        render.insert_resource(UIRenderDiagnostics::default());
        let nodes = labels
            .iter()
            .enumerate()
            .map(|(index, label)| {
                let target = render.spawn(());
                let rect = crate::node::UIBox {
                    min: glam::Vec2::new(10.0, 20.0 + index as f32 * 30.0),
                    size: glam::Vec2::new(100.0, 20.0),
                };
                let node = main.spawn((
                    UIText {
                        text: (*label).into(),
                        ..Default::default()
                    },
                    UILayout {
                        rect,
                        content_rect: rect,
                        clip_rect: rect,
                        paint_order: 0,
                    },
                    RenderEntity::new(target),
                ));
                (node, target)
            })
            .collect();
        (main, render, nodes)
    }

    fn text_reshapes(render: &concerto_ecs::World) -> u64 {
        render
            .get_resource::<UIRenderDiagnostics>()
            .unwrap()
            .text_reshapes()
    }

    #[test]
    fn idle_text_extractions_reshape_nothing() {
        let (mut main, mut render, _) = text_worlds(&["One", "Two"]);
        extract_text(&mut main, &mut render);
        assert_eq!(text_reshapes(&render), 2);

        extract_text(&mut main, &mut render);
        extract_text(&mut main, &mut render);
        assert_eq!(text_reshapes(&render), 2);
    }

    #[test]
    fn writing_identical_text_reshapes_nothing() {
        let (mut main, mut render, nodes) = text_worlds(&["Same"]);
        extract_text(&mut main, &mut render);
        let before = text_reshapes(&render);

        main.get_component_for_entity_mut::<UIText>(nodes[0].0)
            .unwrap()
            .text = "Same".into();
        extract_text(&mut main, &mut render);
        assert_eq!(text_reshapes(&render), before);
    }

    #[test]
    fn changing_the_scale_factor_reshapes_every_text_node() {
        let (mut main, mut render, nodes) = text_worlds(&["One", "Two", "Three"]);
        extract_text(&mut main, &mut render);
        let before = text_reshapes(&render);

        main.insert_resource(Window::new((800, 600), 2.0));
        extract_text(&mut main, &mut render);
        assert_eq!(text_reshapes(&render), before + nodes.len() as u64);
        for (_, target) in nodes {
            let text = render
                .get_component_for_entity::<RenderTextComponent>(target)
                .unwrap();
            assert_eq!(text.buffer.metrics().font_size, 28.0);
        }
    }

    #[test]
    fn moving_a_text_node_without_resizing_it_keeps_its_shaping() {
        let (mut main, mut render, nodes) = text_worlds(&["Scrolled"]);
        let (node, target) = nodes[0];
        extract_text(&mut main, &mut render);
        let before = text_reshapes(&render);

        {
            let layout = main.get_component_for_entity_mut::<UILayout>(node).unwrap();
            let offset = glam::Vec2::new(0.0, -15.0);
            layout.rect.min += offset;
            layout.content_rect.min += offset;
            layout.clip_rect.min += offset;
        }
        extract_text(&mut main, &mut render);
        assert_eq!(text_reshapes(&render), before);
        let text = render
            .get_component_for_entity::<RenderTextComponent>(target)
            .unwrap();
        assert_eq!(text.location, glam::Vec2::new(10.0, 5.0));
        assert_eq!(text.clip_min, glam::Vec2::new(10.0, 5.0));
        assert_eq!(text.clip_max, glam::Vec2::new(110.0, 25.0));
    }

    #[test]
    fn a_nodes_shaped_text_fits_its_layout_box_at_scale_two() {
        let mut main = concerto_ecs::World::new();
        main.insert_resource(Window::new((800, 600), 2.0));
        let mut render = concerto_ecs::World::new();
        render.insert_resource(TextFontSystem(fonts::build_font_system(
            &fonts::UIFonts::default(),
        )));
        render.insert_resource(UIRenderDiagnostics::default());
        let target = render.spawn(());
        let rect = crate::node::UIBox {
            min: glam::Vec2::new(10.0, 20.0),
            size: glam::Vec2::new(100.0, 20.0),
        };
        main.spawn((
            UIText {
                text: "bunny_porcelain_subsurface.mat".into(),
                font_size: 14.0,
                line_height: 18.0,
                wrap: false,
                ellipsis: true,
                ..Default::default()
            },
            UILayout {
                rect,
                content_rect: rect,
                clip_rect: rect,
                paint_order: 0,
            },
            RenderEntity::new(target),
        ));

        extract_text(&mut main, &mut render);
        let text = render
            .get_component_for_entity::<RenderTextComponent>(target)
            .expect("the text must be drawn");
        let metrics = text.buffer.metrics();
        assert_eq!(metrics.font_size, 28.0);
        assert_eq!(metrics.line_height, 36.0);
        let physical = rect.size * 2.0;
        let width = text
            .buffer
            .layout_runs()
            .map(|run| run.line_w)
            .fold(0.0_f32, f32::max);
        assert!(width > 0.0);
        assert!(
            width <= physical.x,
            "shaped width {width} overflows the physical box {}",
            physical.x
        );
        assert!(metrics.line_height <= physical.y);
    }
}

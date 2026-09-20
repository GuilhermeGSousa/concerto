use app::extractor::Extracted;
use color::Color;
use ecs::{
    command::CommandQueue,
    component::Component,
    query::Query,
    resource::{Res, ResMut},
};
use glyphon::{Attrs, Buffer, Family, FontSystem, Metrics, Shaping, Style, Weight, Wrap};
use render::components::render_entity::RenderEntity;
use std::hash::{Hash, Hasher};
use window::plugin::Window;

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
///
/// Changing any field re-builds the glyphon buffer on the next frame.
#[derive(Component)]
pub struct TextComponent {
    pub text: String,
    pub font_size: f32,
    pub line_height: f32,
    /// Font family.  Defaults to [`FontFamily::SansSerif`].
    pub font_family: FontFamily,
    /// Font weight (100–900).  400 = regular, 700 = bold.  Defaults to 400.
    pub font_weight: u16,
    /// Font style.  Defaults to [`FontStyle::Normal`].
    pub font_style: FontStyle,
    /// Semantic foreground color. The default is warm off-white rather than
    /// renderer-owned pure white so widgets can be themed consistently.
    pub color: Color,
    /// Whether text may wrap at word boundaries inside the content rectangle.
    pub wrap: bool,
    /// Whether text too long for its box is cut short with an ellipsis rather
    /// than simply clipped. Only meaningful for unwrapped single-line text.
    pub ellipsis: bool,
}

impl Default for TextComponent {
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

/// The longest prefix of `text` that fits `width`, with an ellipsis appended.
///
/// Shaping is the only honest way to know where a string stops fitting, so this
/// binary-searches character boundaries rather than guessing from a character
/// count — proportional fonts make any such guess wrong for exactly the names
/// that need truncating.
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

    // Largest prefix that still fits once the ellipsis is accounted for.
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

/// Width of `text` shaped on one unwrapped line.
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
pub struct RenderTextComponent {
    pub(crate) buffer: glyphon::Buffer,
    pub(crate) location: glam::Vec2,
    pub(crate) clip_min: glam::Vec2,
    pub(crate) clip_max: glam::Vec2,
    pub(crate) color: glyphon::Color,
    /// Explicit z-layer, recovered from the node's paint order. Text is batched
    /// by this so it can be interleaved with the quads of the same layer.
    pub(crate) layer: i32,
    signature: u64,
}

pub(crate) fn extract_text_nodes(
    text_nodes: Extracted<Query<(&TextComponent, &UILayout, &RenderEntity)>>,
    window: Extracted<Res<Window>>,
    mut font_system: ResMut<TextFontSystem>,
    render_text_nodes: Query<&RenderTextComponent>,
    diagnostics: Res<UIRenderDiagnostics>,
    mut cmd: CommandQueue,
) {
    for (text_component, layout, render_entity) in text_nodes.iter() {
        let scale = window.scale_factor() as f32;
        let content_size = layout.content_rect.size * scale;
        let signature = text_signature(text_component, layout, scale);
        if render_text_nodes
            .get_entity(**render_entity)
            .is_some_and(|render_text| render_text.signature == signature)
        {
            continue;
        }
        diagnostics.record_text_reshape();
        let mut text_buffer = Buffer::new(
            &mut font_system,
            Metrics {
                font_size: text_component.font_size,
                line_height: text_component.line_height,
            },
        );

        text_buffer.set_size(
            &mut font_system,
            Some(content_size.x.max(0.0)),
            Some(content_size.y.max(0.0)),
        );
        text_buffer.set_wrap(
            &mut font_system,
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

        // Truncation happens here, where the font system is: the source text
        // stays intact so the panel that owns it never has to know.
        let display = if text_component.ellipsis && !text_component.wrap {
            truncate_to_width(
                &mut font_system,
                Metrics {
                    font_size: text_component.font_size,
                    line_height: text_component.line_height,
                },
                attrs,
                &text_component.text,
                content_size.x.max(0.0),
            )
        } else {
            None
        };
        text_buffer.set_text(
            &mut font_system,
            display.as_deref().unwrap_or(&text_component.text),
            attrs,
            // Advanced, not Basic: basic shaping has no font fallback, so any
            // character the UI font lacks is drawn as a .notdef box rather than
            // borrowed from another face.
            Shaping::Advanced,
        );
        text_buffer.shape_until_scroll(&mut font_system, false);

        let clip = layout.clip_rect.intersection(layout.content_rect);
        let rgba = text_component.color.to_srgba();
        cmd.insert(
            RenderTextComponent {
                buffer: text_buffer,
                location: layout.content_rect.min * scale,
                clip_min: clip.min * scale,
                clip_max: clip.max() * scale,
                color: glyphon::Color::rgba(
                    (rgba.r * 255.0).round() as u8,
                    (rgba.g * 255.0).round() as u8,
                    (rgba.b * 255.0).round() as u8,
                    (rgba.a * 255.0).round() as u8,
                ),
                layer: text_layer(layout),
                signature,
            },
            **render_entity,
        );
    }
}

/// The explicit `z_index` a node was given, recovered from its paint order.
/// `paint_order` is `(z_index << 32) + tree sequence`, and the sequence is
/// always non-negative, so an arithmetic shift returns the z_index unchanged.
pub(crate) fn text_layer(layout: &UILayout) -> i32 {
    (layout.paint_order >> 32) as i32
}

fn text_signature(text: &TextComponent, layout: &UILayout, scale: f32) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.text.hash(&mut hasher);
    text.font_size.to_bits().hash(&mut hasher);
    text.line_height.to_bits().hash(&mut hasher);
    text.font_weight.hash(&mut hasher);
    (text.font_style as u8).hash(&mut hasher);
    text.wrap.hash(&mut hasher);
    text.ellipsis.hash(&mut hasher);
    text_layer(layout).hash(&mut hasher);
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
        layout.content_rect.min.x,
        layout.content_rect.min.y,
        layout.content_rect.size.x,
        layout.content_rect.size.y,
        layout.clip_rect.min.x,
        layout.clip_rect.min.y,
        layout.clip_rect.size.x,
        layout.clip_rect.size.y,
        scale,
    ] {
        value.to_bits().hash(&mut hasher);
    }
    for value in text.color.to_srgba().to_array() {
        value.to_bits().hash(&mut hasher);
    }
    hasher.finish()
}

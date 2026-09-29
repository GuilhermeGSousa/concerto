use concerto_color::Color;
use concerto_ecs::resource::Resource;

use crate::interaction::UIInteractionStyle;

/// Semantic colors and metrics for the Concerto editor's Nocturne UI.
#[derive(Resource, Clone)]
pub struct UITheme {
    pub canvas: Color,
    pub surface: Color,
    pub surface_raised: Color,
    pub surface_hovered: Color,
    pub text: Color,
    pub text_muted: Color,
    pub border: Color,
    pub accent: Color,
    pub accent_hovered: Color,
    /// The palette's second accent, for marks that must read as a different kind of thing.
    pub accent_secondary: Color,
    pub focus: Color,
    pub error: Color,
    pub warning: Color,
    pub spacing_xs: f32,
    pub spacing_sm: f32,
    pub spacing_md: f32,
    pub spacing_lg: f32,
    pub row_height: f32,
    pub control_height: f32,
    /// Nocturne's radius scale, in logical pixels.
    pub radius_sm: f32,
    pub radius_md: f32,
    pub radius_lg: f32,
    /// Type scale.
    pub font_size_sm: f32,
    pub font_size_md: f32,
    pub font_size_lg: f32,
}

/// The colour scheme an interactive element follows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonVariant {
    /// A raised control: buttons, menu rows.
    Solid,
    /// No fill until hovered: list rows, icon controls.
    Ghost,
    /// A document tab: sits on the surface, raised when selected.
    Tab,
}

/// A chip's fill, outline and text colour.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChipColors {
    pub fill: Color,
    pub border: Color,
    pub text: Color,
}

const DISABLED_SOLID: Color = Color::srgba(0.09, 0.075, 0.11, 0.55);

impl UITheme {
    /// Leading for a given size.
    pub fn line_height(&self, font_size: f32) -> f32 {
        (font_size * 1.4).round()
    }

    /// The wash of accent behind a selected row.
    pub fn selection(&self) -> Color {
        let accent = self.accent.to_srgba();
        Color::srgba(accent.r, accent.g, accent.b, 0.2)
    }

    /// The interaction colours for a variant, raised to its selected fill when `selected`.
    pub fn interaction(&self, variant: ButtonVariant, selected: bool) -> UIInteractionStyle {
        match variant {
            ButtonVariant::Solid => UIInteractionStyle {
                normal: if selected { self.surface_hovered } else { self.surface_raised },
                hovered: self.surface_hovered,
                pressed: self.accent,
                disabled: DISABLED_SOLID,
            },
            ButtonVariant::Ghost => UIInteractionStyle {
                normal: if selected { self.selection() } else { Color::TRANSPARENT },
                hovered: self.surface_hovered,
                pressed: self.selection(),
                disabled: Color::TRANSPARENT,
            },
            ButtonVariant::Tab => UIInteractionStyle {
                normal: if selected { self.surface_raised } else { self.surface },
                hovered: self.surface_hovered,
                pressed: self.accent,
                disabled: self.surface,
            },
        }
    }

    /// A chip's colours, marked with the accent when `selected`.
    pub fn chip_colors(&self, selected: bool) -> ChipColors {
        if selected {
            ChipColors { fill: self.selection(), border: self.accent, text: self.text }
        } else {
            ChipColors { fill: Color::TRANSPARENT, border: self.border, text: self.text_muted }
        }
    }

    /// Nocturne: a quiet, compact dark interface.
    #[allow(clippy::approx_constant)]
    pub fn nocturne() -> Self {
        Self {
            canvas: Color::srgba(0.063, 0.071, 0.125, 1.0),
            surface: Color::srgba(0.078, 0.086, 0.133, 0.72),
            surface_raised: Color::srgba(0.137, 0.145, 0.196, 0.85),
            surface_hovered: Color::srgba(0.212, 0.204, 0.318, 0.9),
            text: Color::srgba(0.914, 0.914, 0.929, 1.0),
            text_muted: Color::srgba(0.576, 0.592, 0.671, 1.0),
            border: Color::srgba(0.247, 0.259, 0.302, 1.0),
            accent: Color::srgba(0.569, 0.518, 0.851, 1.0),
            accent_secondary: Color::srgba(0.588, 0.565, 0.788, 1.0),
            accent_hovered: Color::srgba(0.710, 0.671, 0.988, 1.0),
            focus: Color::srgba(0.710, 0.671, 0.988, 1.0),
            error: Color::srgba(0.851, 0.404, 0.451, 1.0),
            warning: Color::srgba(0.878, 0.694, 0.400, 1.0),
            spacing_xs: 4.0,
            spacing_sm: 7.0,
            spacing_md: 10.0,
            spacing_lg: 18.0,
            row_height: 26.0,
            control_height: 28.0,
            radius_sm: 4.0,
            radius_md: 8.0,
            radius_lg: 14.0,
            font_size_sm: 10.5,
            font_size_md: 12.5,
            font_size_lg: 14.0,
        }
    }
}

impl Default for UITheme {
    fn default() -> Self {
        Self::nocturne()
    }
}

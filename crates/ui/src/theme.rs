use color::Color;
use ecs::resource::Resource;

/// Semantic colors and metrics for the Rabbithole editor's Nocturne UI.
///
/// Applications can replace this resource to reskin reusable UI without
/// coupling widgets to editor-specific concepts.
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
    /// The palette's second accent, for marks that must read as a different
    /// kind of thing rather than as a different state.
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
    /// Type scale. Panels read these rather than hard-coding sizes, so
    /// replacing the theme actually replaces the typography.
    pub font_size_sm: f32,
    pub font_size_md: f32,
    pub font_size_lg: f32,
}

impl UITheme {
    /// Leading for a given size. One ratio for the whole scale keeps vertical
    /// rhythm consistent between panels.
    pub fn line_height(&self, font_size: f32) -> f32 {
        (font_size * 1.4).round()
    }

    /// Nocturne: a quiet, compact dark interface. A near-neutral blue-grey
    /// ground, one blurple accent used as a line rather than a flood, and
    /// panels that sit on the scene as translucent cards.
    // Channel values are sampled colours, not maths: one of them lands near
    // 1/π and clippy would rather it were the constant.
    #[allow(clippy::approx_constant)]
    pub fn nocturne() -> Self {
        Self {
            // The scene ground shows through everything; panels float on it.
            canvas: Color::srgba(0.063, 0.071, 0.125, 1.0),
            // Cards are translucent so the scene reads behind them.
            surface: Color::srgba(0.078, 0.086, 0.133, 0.72),
            surface_raised: Color::srgba(0.137, 0.145, 0.196, 0.85),
            surface_hovered: Color::srgba(0.212, 0.204, 0.318, 0.9),
            text: Color::srgba(0.914, 0.914, 0.929, 1.0),
            text_muted: Color::srgba(0.576, 0.592, 0.671, 1.0),
            border: Color::srgba(0.247, 0.259, 0.302, 1.0),
            accent: Color::srgba(0.569, 0.518, 0.851, 1.0),
            // Nocturne's accent-2-500, #9690c9.
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

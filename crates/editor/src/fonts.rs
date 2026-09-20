//! The editor's typefaces.
//!
//! Inter carries the text — it is what the Nocturne design system asks for —
//! and Phosphor carries the icons, as glyphs rather than images so they shape,
//! measure and clip exactly like any other text.
use app::{App, Plugin};
use ui::text::fonts::UIFontsApp;
use ui::text::{FontFamily, TextComponent};
use ui::theme::UITheme;

/// Family name Inter's faces share; weight picks between them.
pub const INTER: &str = "Inter";
/// Family name the icon face registers under.
pub const PHOSPHOR: &str = "Phosphor";

/// Weight for headings and anything Nocturne sets in caps.
pub const MEDIUM: u16 = 500;
/// Weight for the few labels that need to sit above `MEDIUM`.
pub const SEMIBOLD: u16 = 600;

pub struct FontsPlugin;

impl Plugin for FontsPlugin {
    fn build(&self, app: &mut App) {
        // Registration has to happen during `build`: the UI builds its font
        // systems once every plugin has finished building.
        app.add_ui_font(include_bytes!("../fonts/Inter/Inter-Regular.ttf").as_slice());
        app.add_ui_font(include_bytes!("../fonts/Inter/Inter-Medium.ttf").as_slice());
        app.add_ui_font(include_bytes!("../fonts/Inter/Inter-SemiBold.ttf").as_slice());
        app.add_ui_font(include_bytes!("../fonts/Phosphor/Phosphor.ttf").as_slice());
        app.set_ui_sans_serif(INTER);
    }
}

/// A text node holding one icon glyph.
///
/// Sized in the same units as text, because that is what it is: `size` is the
/// glyph's em box, so an icon next to a label wants the label's font size.
pub fn icon(theme: &UITheme, glyph: char, size: f32) -> TextComponent {
    TextComponent {
        text: glyph.to_string(),
        font_family: FontFamily::Name(PHOSPHOR.into()),
        font_size: size,
        line_height: theme.line_height(size),
        wrap: false,
        ..Default::default()
    }
}

/// Phosphor codepoints, by the name they carry upstream.
///
/// Taken from Phosphor 2.1's own mapping; the faces under `fonts/Phosphor` are
/// that release, so the two agree. Adding one means looking its codepoint up
/// there rather than guessing.
pub mod glyph {
    /// `rabbit` — the brand mark.
    pub const RABBIT: char = '\u{EAC2}';
    /// `caret-right` — a collapsed tree row.
    pub const CARET_RIGHT: char = '\u{E13A}';
    /// `caret-down` — an expanded tree row.
    pub const CARET_DOWN: char = '\u{E136}';
    /// `cube` — a mesh, and the fallback entity glyph.
    pub const CUBE: char = '\u{E1DA}';
    /// `camera` — a camera.
    pub const CAMERA: char = '\u{E10E}';
    /// `lightbulb` — a light.
    pub const LIGHTBULB: char = '\u{E2DC}';
    /// `magnifying-glass` — a filter field.
    pub const MAGNIFYING_GLASS: char = '\u{E30C}';
    /// `stack` — a scene.
    pub const STACK: char = '\u{E466}';
    /// `image` — a texture.
    pub const IMAGE: char = '\u{E2CA}';
    /// `file` — an asset of no particular kind.
    pub const FILE: char = '\u{E230}';
    /// `waveform` — an animation.
    pub const WAVEFORM: char = '\u{E802}';
    /// `circles-three` — a material.
    pub const CIRCLES_THREE: char = '\u{E192}';
    /// `warning` — something the editor is unhappy about.
    pub const WARNING: char = '\u{E4E0}';
    /// `info` — an ordinary status line.
    pub const INFO: char = '\u{E2CE}';
    /// `frame-corners` — framing the view.
    pub const FRAME_CORNERS: char = '\u{E626}';
    /// `plus` — adding something.
    pub const PLUS: char = '\u{E3D4}';
    /// `minus` — minimising the window.
    pub const MINUS: char = '\u{E32A}';
    /// `corners-out` — maximising the window.
    pub const CORNERS_OUT: char = '\u{E1D0}';
    /// `corners-in` — restoring a maximised window.
    pub const CORNERS_IN: char = '\u{E1CE}';
    /// `x` — dismissing, and the window's close button.
    pub const X: char = '\u{E4F6}';
    /// `dot` — a leaf row with nothing to expand.
    pub const DOT: char = '\u{ECDE}';
}

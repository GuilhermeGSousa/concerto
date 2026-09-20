//! The faces the UI can shape text with.
//!
//! Two font systems exist — one measures during layout, one renders — and they
//! must agree, so both are built from this one registry when plugin building
//! finishes. That is also why fonts have to be registered from a plugin's
//! `build`: a face added later is invisible to both.
use std::borrow::Cow;

use app::App;
use ecs::resource::Resource;
use glyphon::FontSystem;

/// Font faces to load into every UI font system.
#[derive(Resource, Default)]
pub struct UIFonts {
    faces: Vec<Cow<'static, [u8]>>,
    /// Family that [`FontFamily::SansSerif`](crate::text::FontFamily::SansSerif)
    /// resolves to, overriding the platform's default.
    sans_serif: Option<String>,
}

impl UIFonts {
    pub fn add_face(&mut self, data: impl Into<Cow<'static, [u8]>>) {
        self.faces.push(data.into());
    }

    pub fn set_sans_serif(&mut self, family: impl Into<String>) {
        self.sans_serif = Some(family.into());
    }
}

/// Registers fonts with the UI. Call from a plugin's `build`.
pub trait UIFontsApp {
    /// Adds one face. Faces sharing a family name are resolved by weight and
    /// style, so an app ships regular and bold as two calls.
    fn add_ui_font(&mut self, data: impl Into<Cow<'static, [u8]>>) -> &mut Self;
    /// Points the default sans-serif family at `family`, which one of the
    /// registered faces must provide.
    fn set_ui_sans_serif(&mut self, family: impl Into<String>) -> &mut Self;
}

impl UIFontsApp for App {
    fn add_ui_font(&mut self, data: impl Into<Cow<'static, [u8]>>) -> &mut Self {
        fonts_mut(self).add_face(data);
        self
    }

    fn set_ui_sans_serif(&mut self, family: impl Into<String>) -> &mut Self {
        fonts_mut(self).set_sans_serif(family);
        self
    }
}

/// The registry, created on first use so plugin registration order does not
/// decide whether fonts can be added.
fn fonts_mut(app: &mut App) -> &mut UIFonts {
    if app.get_resource::<UIFonts>().is_none() {
        app.insert_resource(UIFonts::default());
    }
    app.get_resource_mut::<UIFonts>()
        .expect("UIFonts was just inserted")
}

/// Builds a [`FontSystem`] holding every registered face.
///
/// Public so tests can build the very system the editor will run with.
///
/// Native also resolves against the platform's installed fonts. Browsers expose
/// no enumerable system fonts, so cosmic-text's database is empty there and
/// shaping panics with "no default font found"; ship a font and point every
/// generic family at it.
#[doc(hidden)]
pub fn build_font_system(fonts: &UIFonts) -> FontSystem {
    let mut font_system = FontSystem::new();
    let db = font_system.db_mut();

    #[cfg(target_arch = "wasm32")]
    {
        db.load_font_data(include_bytes!("../../fonts/DejaVuSansMono.ttf").to_vec());
        const FALLBACK: &str = "DejaVu Sans Mono";
        db.set_sans_serif_family(FALLBACK);
        db.set_serif_family(FALLBACK);
        db.set_monospace_family(FALLBACK);
        db.set_cursive_family(FALLBACK);
        db.set_fantasy_family(FALLBACK);
    }

    for face in &fonts.faces {
        db.load_font_data(face.to_vec());
    }
    if let Some(family) = &fonts.sans_serif {
        db.set_sans_serif_family(family);
    }

    font_system
}

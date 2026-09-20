//! The editor's faces have to carry what the editor asks them for: a wrong
//! Phosphor codepoint draws a blank box, which is easy to miss on screen and
//! hard to trace back to a constant.
use glyphon::cosmic_text::fontdb;
use glyphon::{Attrs, Buffer, Family, Metrics, Shaping, Wrap};
use ui::text::fonts::{build_font_system, UIFonts};

fn fonts() -> UIFonts {
    let mut fonts = UIFonts::default();
    fonts.add_face(include_bytes!("../fonts/Inter/Inter-Regular.ttf").as_slice());
    fonts.add_face(include_bytes!("../fonts/Inter/Inter-Medium.ttf").as_slice());
    fonts.add_face(include_bytes!("../fonts/Inter/Inter-SemiBold.ttf").as_slice());
    fonts.add_face(include_bytes!("../fonts/Phosphor/Phosphor.ttf").as_slice());
    fonts.set_sans_serif(editor::fonts::INTER);
    fonts
}

/// Glyph ids of `text` shaped in `family`. A zero is `.notdef` — the font has
/// no such character.
fn glyph_ids(text: &str, family: &str) -> Vec<u16> {
    let mut font_system = build_font_system(&fonts());
    let mut buffer = Buffer::new(
        &mut font_system,
        Metrics {
            font_size: 16.0,
            line_height: 20.0,
        },
    );
    buffer.set_size(&mut font_system, None, None);
    buffer.set_wrap(&mut font_system, Wrap::None);
    buffer.set_text(
        &mut font_system,
        text,
        Attrs::new().family(Family::Name(family)),
        Shaping::Advanced,
    );
    buffer.shape_until_scroll(&mut font_system, false);
    buffer
        .layout_runs()
        .flat_map(|run| run.glyphs.iter().map(|glyph| glyph.glyph_id))
        .collect()
}

#[test]
fn every_icon_constant_exists_in_the_phosphor_face() {
    use editor::fonts::glyph::*;
    let icons = [
        ("RABBIT", RABBIT),
        ("CARET_RIGHT", CARET_RIGHT),
        ("CARET_DOWN", CARET_DOWN),
        ("CUBE", CUBE),
        ("CAMERA", CAMERA),
        ("LIGHTBULB", LIGHTBULB),
        ("MAGNIFYING_GLASS", MAGNIFYING_GLASS),
        ("STACK", STACK),
        ("IMAGE", IMAGE),
        ("FILE", FILE),
        ("WAVEFORM", WAVEFORM),
        ("CIRCLES_THREE", CIRCLES_THREE),
        ("WARNING", WARNING),
        ("INFO", INFO),
        ("FRAME_CORNERS", FRAME_CORNERS),
        ("PLUS", PLUS),
        ("MINUS", MINUS),
        ("CORNERS_OUT", CORNERS_OUT),
        ("CORNERS_IN", CORNERS_IN),
        ("X", X),
        ("DOT", DOT),
    ];
    for (name, icon) in icons {
        let ids = glyph_ids(&icon.to_string(), editor::fonts::PHOSPHOR);
        assert_eq!(ids.len(), 1, "{name} shaped to {} glyphs", ids.len());
        assert_ne!(
            ids[0], 0,
            "{name} (U+{:04X}) is not in the Phosphor face",
            icon as u32
        );
    }
}

#[test]
fn text_resolves_to_inter_rather_than_whatever_the_platform_offers() {
    let font_system = build_font_system(&fonts());
    assert_eq!(
        font_system.db().family_name(&fontdb::Family::SansSerif),
        editor::fonts::INTER,
    );
}

#[test]
fn inter_carries_the_weights_the_editor_asks_for() {
    let font_system = build_font_system(&fonts());
    let weights: Vec<u16> = font_system
        .db()
        .faces()
        .filter(|face| {
            face.families
                .iter()
                .any(|(name, _)| name == editor::fonts::INTER)
        })
        .map(|face| face.weight.0)
        .collect();
    for weight in [400, editor::fonts::MEDIUM, editor::fonts::SEMIBOLD] {
        assert!(
            weights.contains(&weight),
            "no Inter face at weight {weight}"
        );
    }
}

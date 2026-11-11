//! The editor's faces have to carry what the editor asks them for.
use concerto_ui::text::fonts::{UIFonts, build_font_system};
use glyphon::cosmic_text::fontdb;
use glyphon::{Attrs, Buffer, Family, Metrics, Shaping, Wrap};

fn fonts() -> UIFonts {
    let mut fonts = UIFonts::default();
    fonts.add_face(include_bytes!("../../fonts/Inter/Inter-Regular.ttf").as_slice());
    fonts.add_face(include_bytes!("../../fonts/Inter/Inter-Medium.ttf").as_slice());
    fonts.add_face(include_bytes!("../../fonts/Inter/Inter-SemiBold.ttf").as_slice());
    fonts.add_face(include_bytes!("../../fonts/Phosphor/Phosphor.ttf").as_slice());
    fonts.set_sans_serif(concerto_editor::fonts::INTER);
    fonts
}

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
    use concerto_editor::fonts::glyph::*;
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
        ("DOTS_THREE", DOTS_THREE),
        ("CIRCLE", CIRCLE),
        ("PLAY", PLAY),
        ("PAUSE", PAUSE),
        ("SKIP_BACK", SKIP_BACK),
        ("SKIP_FORWARD", SKIP_FORWARD),
        ("CROSSHAIR", CROSSHAIR),
        ("BONE", BONE),
        ("FOLDER", FOLDER),
        ("FOLDER_OPEN", FOLDER_OPEN),
        ("PERSON_ARMS_SPREAD", PERSON_ARMS_SPREAD),
        ("MASK_HAPPY", MASK_HAPPY),
        ("GRAPH", GRAPH),
        ("DATABASE", DATABASE),
    ];
    for (name, icon) in icons {
        let ids = glyph_ids(&icon.to_string(), concerto_editor::fonts::PHOSPHOR);
        assert_eq!(ids.len(), 1, "{name} shaped to {} glyphs", ids.len());
        assert_ne!(
            ids[0], 0,
            "{name} (U+{:04X}) is not in the Phosphor face",
            icon as u32
        );
    }
}

struct IconCase {
    kind: &'static str,
    glyph: char,
}

#[test]
fn an_asset_kind_shows_the_glyph_registered_last_and_the_file_when_none_is() {
    use concerto_editor::fonts::{AssetIcons, AssetIconsAppExt, FontsPlugin, glyph};
    let mut app = concerto_app::App::new();
    app.register_asset_icon("Dialogue", glyph::IMAGE);
    app.register_plugin(FontsPlugin);
    app.register_asset_icon("AnimationClip", glyph::WAVEFORM)
        .register_asset_icon("Dialogue", glyph::GRAPH);
    let icons = app.get_resource::<AssetIcons>().unwrap();
    for case in [
        IconCase {
            kind: "Scene",
            glyph: glyph::STACK,
        },
        IconCase {
            kind: "Mesh",
            glyph: glyph::CUBE,
        },
        IconCase {
            kind: "Texture",
            glyph: glyph::IMAGE,
        },
        IconCase {
            kind: "StandardMaterial",
            glyph: glyph::CIRCLES_THREE,
        },
        IconCase {
            kind: "Skeleton",
            glyph: glyph::BONE,
        },
        IconCase {
            kind: "AnimationClip",
            glyph: glyph::WAVEFORM,
        },
        IconCase {
            kind: "Dialogue",
            glyph: glyph::GRAPH,
        },
        IconCase {
            kind: "NavMesh",
            glyph: glyph::FILE,
        },
    ] {
        assert_eq!(icons.glyph(case.kind), case.glyph, "{}", case.kind);
    }
}

#[test]
fn text_resolves_to_inter_rather_than_whatever_the_platform_offers() {
    let font_system = build_font_system(&fonts());
    assert_eq!(
        font_system.db().family_name(&fontdb::Family::SansSerif),
        concerto_editor::fonts::INTER,
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
                .any(|(name, _)| name == concerto_editor::fonts::INTER)
        })
        .map(|face| face.weight.0)
        .collect();
    for weight in [
        400,
        concerto_editor::fonts::MEDIUM,
        concerto_editor::fonts::SEMIBOLD,
    ] {
        assert!(
            weights.contains(&weight),
            "no Inter face at weight {weight}"
        );
    }
}

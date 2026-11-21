//! The editor's faces have to carry what the editor asks them for.
use concerto_app::App;
use concerto_editor::fonts::{AssetIcons, AssetIconsAppExt, FontsPlugin, glyph};
use concerto_ui::text::fonts::{UIFonts, build_font_system};
use glyphon::cosmic_text::fontdb;
use glyphon::{Attrs, Buffer, Family, FontSystem, Metrics, Shaping, Wrap};

fn font_system() -> FontSystem {
    let mut app = App::new();
    app.register_plugin(FontsPlugin);
    build_font_system(app.get_resource::<UIFonts>().unwrap())
}

fn glyph_ids(font_system: &mut FontSystem, text: &str, family: &str) -> Vec<u16> {
    let mut buffer = Buffer::new(
        font_system,
        Metrics {
            font_size: 16.0,
            line_height: 20.0,
        },
    );
    buffer.set_size(None, None);
    buffer.set_wrap(Wrap::None);
    buffer.set_text(
        text,
        &Attrs::new().family(Family::Name(family)),
        Shaping::Advanced,
        None,
    );
    buffer.shape_until_scroll(font_system, false);
    buffer
        .layout_runs()
        .flat_map(|run| run.glyphs.iter().map(|glyph| glyph.glyph_id))
        .collect()
}

#[test]
fn the_editor_sets_its_text_in_inter_and_its_faces_hold_every_icon_and_the_mark() {
    use concerto_editor::fonts::{BRAND, BRAND_MARK, INTER, MEDIUM, PHOSPHOR, SEMIBOLD};
    use glyph::*;
    let mut font_system = font_system();
    assert_eq!(
        font_system.db().family_name(&fontdb::Family::SansSerif),
        INTER
    );
    let weights: Vec<u16> = font_system
        .db()
        .faces()
        .filter(|face| face.families.iter().any(|(name, _)| name == INTER))
        .map(|face| face.weight.0)
        .collect();
    for weight in [400, MEDIUM, SEMIBOLD] {
        assert!(
            weights.contains(&weight),
            "no Inter face at weight {weight}"
        );
    }

    let icons = [
        CARET_RIGHT,
        CARET_LEFT,
        CARET_DOWN,
        CUBE,
        CAMERA,
        LIGHTBULB,
        MAGNIFYING_GLASS,
        STACK,
        IMAGE,
        FILE,
        WAVEFORM,
        CIRCLES_THREE,
        WARNING,
        INFO,
        FRAME_CORNERS,
        PLUS,
        MINUS,
        CORNERS_OUT,
        CORNERS_IN,
        X,
        DOTS_THREE,
        CIRCLE,
        PLAY,
        STOP,
        PAUSE,
        SKIP_BACK,
        SKIP_FORWARD,
        CROSSHAIR,
        BONE,
        FOLDER,
        FOLDER_OPEN,
        PERSON_ARMS_SPREAD,
        MASK_HAPPY,
        GRAPH,
        DATABASE,
    ];
    for icon in icons {
        let ids = glyph_ids(&mut font_system, &icon.to_string(), PHOSPHOR);
        assert!(
            ids.len() == 1 && ids[0] != 0,
            "U+{:04X} is not in the Phosphor face",
            icon as u32
        );
    }
    let ids = glyph_ids(&mut font_system, &BRAND_MARK.to_string(), BRAND);
    assert!(
        ids.len() == 1 && ids[0] != 0,
        "the mark is not in the {BRAND} face"
    );
}

#[test]
fn an_asset_kind_shows_the_glyph_registered_last_even_before_the_plugin_and_the_file_otherwise() {
    let mut app = App::new();
    app.register_asset_icon("Cue", glyph::WAVEFORM);
    app.register_plugin(FontsPlugin);
    app.register_asset_icon("Dialogue", glyph::IMAGE)
        .register_asset_icon("Dialogue", glyph::GRAPH);
    let icons = app.get_resource::<AssetIcons>().unwrap();
    assert_eq!(icons.glyph("Dialogue"), glyph::GRAPH);
    assert_eq!(icons.glyph("Cue"), glyph::WAVEFORM);
    assert_eq!(icons.glyph("Scene"), glyph::STACK);
    assert_eq!(icons.glyph("NavMesh"), glyph::FILE);
}

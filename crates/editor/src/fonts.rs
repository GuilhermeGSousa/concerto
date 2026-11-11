//! The editor's typefaces.
use concerto_app::{App, Plugin, schedule_groups::LateUpdate};
use concerto_color::Color;
use concerto_ecs::{
    IntoSystemConfig,
    component::Component,
    entity::{Entity, hierarchy::Children},
    query::{Query, change_detection::Mut, filter::With},
    resource::{Res, Resource},
    system::input::SystemLocal,
    world::{FromWorld, World},
};
use concerto_foundation::assets::Asset;
use concerto_mesh::{Mesh, skeleton::Skeleton};
use concerto_render::assets::{material::StandardMaterial, texture::Texture};
use concerto_scene::scene::Scene;
use concerto_ui::elements::{Pressable, Text, prelude::*};
use concerto_ui::interaction::HoveredNode;
use concerto_ui::items::UISelected;
use concerto_ui::node::{UINode, UIRect};
use concerto_ui::sets::UiSet;
use concerto_ui::text::fonts::UIFontsApp;
use concerto_ui::text::{FontFamily, UIText};
use concerto_ui::theme::UITheme;
use concerto_ui::transform::UIValue;
use std::collections::HashMap;

/// Family name Inter's faces share; weight picks between them.
pub const INTER: &str = "Inter";
/// Family name the icon face registers under.
pub const PHOSPHOR: &str = "Phosphor";
/// Family name the one-glyph face holding the Concerto mark registers under.
pub const BRAND: &str = "Concerto Mark";
/// The Concerto mark's codepoint in the `BRAND` face.
pub const BRAND_MARK: char = '\u{E000}';

/// Weight for headings and anything Nocturne sets in caps.
pub const MEDIUM: u16 = 500;
/// Weight for the few labels that need to sit above `MEDIUM`.
pub const SEMIBOLD: u16 = 600;

/// Size of every icon glyph, in logical pixels.
pub const ICON_SIZE: f32 = 16.0;
/// Width of the column a row icon sits in, in logical pixels.
pub const ICON_COLUMN: f32 = 20.0;

pub struct FontsPlugin;

impl Plugin for FontsPlugin {
    fn build(&self, app: &mut App) {
        app.add_ui_font(include_bytes!("../fonts/Inter/Inter-Regular.ttf").as_slice());
        app.add_ui_font(include_bytes!("../fonts/Inter/Inter-Medium.ttf").as_slice());
        app.add_ui_font(include_bytes!("../fonts/Inter/Inter-SemiBold.ttf").as_slice());
        app.add_ui_font(include_bytes!("../fonts/Phosphor/Phosphor.ttf").as_slice());
        app.add_ui_font(include_bytes!("../fonts/Concerto/ConcertoMark.ttf").as_slice());
        app.set_ui_sans_serif(INTER);
        app.register_asset_icon(Scene::name(), glyph::STACK)
            .register_asset_icon(Mesh::name(), glyph::CUBE)
            .register_asset_icon(Texture::name(), glyph::IMAGE)
            .register_asset_icon(StandardMaterial::name(), glyph::CIRCLES_THREE)
            .register_asset_icon(Skeleton::name(), glyph::BONE);
        app.add_system(LateUpdate, light_hovered_row_icon.in_set(UiSet::Materials));
    }
}

/// The glyph each asset kind shows in the Asset Browser and on its tab, keyed by `Asset::name()`.
#[derive(Resource, Default)]
pub struct AssetIcons {
    glyphs: HashMap<String, char>,
}

impl AssetIcons {
    /// Shows `glyph` for assets of `kind`, replacing the glyph the kind had.
    pub fn register(&mut self, kind: impl Into<String>, glyph: char) {
        self.glyphs.insert(kind.into(), glyph);
    }

    /// The glyph of `kind`, or `file` for a kind nothing registered.
    pub fn glyph(&self, kind: &str) -> char {
        self.glyphs.get(kind).copied().unwrap_or(glyph::FILE)
    }
}

pub trait AssetIconsAppExt {
    /// Shows `glyph` for assets whose `Asset::name()` is `kind`, in the Asset Browser and on their tabs.
    fn register_asset_icon(&mut self, kind: &str, glyph: char) -> &mut Self;
}

impl AssetIconsAppExt for App {
    fn register_asset_icon(&mut self, kind: &str, glyph: char) -> &mut Self {
        if self.get_resource::<AssetIcons>().is_none() {
            self.insert_resource(AssetIcons::default());
        }
        self.get_resource_mut::<AssetIcons>()
            .expect("AssetIcons was just inserted")
            .register(kind, glyph);
        self
    }
}

/// Marks the icon of a row, which takes the text colour while its row is hovered or selected.
#[derive(Component)]
pub struct RowIcon;

/// A row's icon: one glyph centred in the icon column, muted until [`draw_row_icon`] or a hover says otherwise.
pub fn row_icon(theme: &UITheme, glyph: char) -> (UINode, Text, RowIcon) {
    (
        UINode::default()
            .with_width(UIValue::Px(ICON_COLUMN))
            .with_flex_shrink(0.0)
            .with_align_self(taffy::AlignItems::Center)
            .with_padding(UIRect::axes(0.0, (ICON_COLUMN - ICON_SIZE) / 2.0)),
        icon(theme, glyph, ICON_SIZE).muted(),
        RowIcon,
    )
}

/// The colour of a row's icon: the text colour when its row is hovered or selected, muted at rest.
pub fn row_icon_color(theme: &UITheme, lit: bool) -> Color {
    if lit { theme.text } else { theme.text_muted }
}

/// Shows `glyph` in a [`row_icon`] in the colour of its row's state, writing only what differs.
pub fn draw_row_icon(text: &mut Mut<UIText>, theme: &UITheme, glyph: char, lit: bool) {
    let color = row_icon_color(theme, lit);
    if !text.text.chars().eq([glyph]) {
        text.text = glyph.to_string();
    }
    if text.color != color {
        text.color = color;
    }
}

/// A square button of the theme's control height, padded to centre the [`button_icon`] spawned with it.
pub fn icon_button(theme: &UITheme) -> Pressable {
    let side = theme.control_height;
    theme
        .pressable()
        .size(UIValue::Px(side), UIValue::Px(side))
        .padding(UIRect::axes(
            (side - theme.line_height(ICON_SIZE)) / 2.0,
            (side - ICON_SIZE) / 2.0,
        ))
        .fixed()
}

/// The glyph of an [`icon_button`], muted.
pub fn button_icon(theme: &UITheme, glyph: char) -> Text {
    icon(theme, glyph, ICON_SIZE).muted()
}

struct HoveredRow {
    row: Option<Entity>,
}

impl FromWorld for HoveredRow {
    fn from_world(_world: &World) -> Self {
        Self { row: None }
    }
}

fn light_hovered_row_icon(
    hovered: Option<Res<HoveredNode>>,
    rows: Query<(&Children, Option<&UISelected>)>,
    icons: Query<&mut UIText, With<RowIcon>>,
    theme: Res<UITheme>,
    mut last: SystemLocal<HoveredRow>,
) {
    let hovered = hovered.and_then(|hovered| **hovered);
    let tint = |row: Entity, lit: bool| {
        let Some((children, selected)) = rows.get_entity(row) else {
            return;
        };
        let color = row_icon_color(&theme, lit || selected.is_some());
        for child in children.iter() {
            if let Some(mut text) = icons.get_entity(*child)
                && text.color != color
            {
                text.color = color;
            }
        }
    };
    if last.row != hovered
        && let Some(row) = last.row
    {
        tint(row, false);
    }
    if let Some(row) = hovered {
        tint(row, true);
    }
    last.row = hovered;
}

/// A text node holding one icon glyph.
pub fn icon(theme: &UITheme, glyph: char, size: f32) -> Text {
    theme
        .text(glyph)
        .family(FontFamily::Name(PHOSPHOR.into()))
        .font_size(size)
        .no_wrap()
}

/// A text node holding the Concerto mark.
pub fn brand_mark(theme: &UITheme, size: f32) -> Text {
    theme
        .text(BRAND_MARK)
        .family(FontFamily::Name(BRAND.into()))
        .font_size(size)
        .no_wrap()
}

/// Phosphor codepoints, by the name they carry upstream.
pub mod glyph {
    /// `caret-right` — a collapsed tree row.
    pub const CARET_RIGHT: char = '\u{E13A}';
    /// `caret-down` — an expanded tree row.
    pub const CARET_DOWN: char = '\u{E136}';
    /// `cube` — a mesh.
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
    /// `dots-three` — a menu of further actions.
    pub const DOTS_THREE: char = '\u{E1FE}';
    /// `circle` — an entity that is no camera, light or mesh.
    pub const CIRCLE: char = '\u{E18A}';
    /// `play` — starting playback.
    pub const PLAY: char = '\u{E3D0}';
    /// `stop` — ending a running game.
    pub const STOP: char = '\u{E46C}';
    /// `pause` — pausing playback.
    pub const PAUSE: char = '\u{E39E}';
    /// `skip-back` — one frame back.
    pub const SKIP_BACK: char = '\u{E5A4}';
    /// `skip-forward` — one frame forward.
    pub const SKIP_FORWARD: char = '\u{E5A6}';
    /// `crosshair` — selecting the entity a field names.
    pub const CROSSHAIR: char = '\u{E1D6}';
    /// `bone` — a skeleton.
    pub const BONE: char = '\u{E7F2}';
    /// `folder` — a collapsed folder of assets.
    pub const FOLDER: char = '\u{E24A}';
    /// `folder-open` — an expanded folder of assets.
    pub const FOLDER_OPEN: char = '\u{E256}';
    /// `person-arms-spread` — a retarget rig.
    pub const PERSON_ARMS_SPREAD: char = '\u{ECFE}';
    /// `mask-happy` — a bone mask.
    pub const MASK_HAPPY: char = '\u{E9F4}';
    /// `graph` — an animation graph.
    pub const GRAPH: char = '\u{EB58}';
    /// `database` — a motion database.
    pub const DATABASE: char = '\u{E1DE}';
}

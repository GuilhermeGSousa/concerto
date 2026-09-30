use concerto_color::Color;
use concerto_ecs::{Component, Entity, World, component::bundle::IntoBundle};
use concerto_ui::{
    anchor::{UIAnchorAlign, UIAnchorSide, UIAnchorTarget, UIAnchoredPanel},
    checkbox::UICheckbox,
    elements::prelude::*,
    focus::UIFocusable,
    interaction::{Interactable, UIInteractionStyle},
    material::UIMaterial,
    node::{AlignContent, AlignItems, FlexDirection, Overflow, UINode, UIRect},
    slider::UISlider,
    text::{FontFamily, UIText},
    text_input::UITextInput,
    theme::{ButtonVariant, ChipColors, UITheme},
    transform::UIValue,
    widgets::UIButton,
};
use glam::Vec2;

struct Spawned {
    world: World,
    entity: Entity,
}

impl Spawned {
    fn get<T: Component>(&self) -> &T {
        self.world
            .get_component_for_entity::<T>(self.entity)
            .unwrap_or_else(|| panic!("missing {}", std::any::type_name::<T>()))
    }

    fn has<T: Component>(&self) -> bool {
        self.world
            .get_component_for_entity::<T>(self.entity)
            .is_some()
    }
}

fn spawn(bundle: impl IntoBundle) -> Spawned {
    let mut world = World::default();
    let entity = world.spawn(bundle);
    Spawned { world, entity }
}

fn theme() -> UITheme {
    UITheme::nocturne()
}

fn colors(style: &UIInteractionStyle) -> [Color; 4] {
    [style.normal, style.hovered, style.pressed, style.disabled]
}

#[derive(Component)]
struct Marker;

#[test]
fn a_single_value_pads_every_side() {
    assert_eq!(UIRect::from(3.0), UIRect::all(3.0));
}

#[test]
fn selection_is_the_accent_at_a_fifth_alpha() {
    let theme = theme();
    let accent = theme.accent.to_srgba();
    assert_eq!(
        theme.selection(),
        Color::srgba(accent.r, accent.g, accent.b, 0.2)
    );
}

#[test]
fn interaction_follows_the_variant_table() {
    let t = theme();
    let disabled_solid = Color::srgba(0.09, 0.075, 0.11, 0.55);
    let cases = [
        (
            ButtonVariant::Solid,
            false,
            [
                t.surface_raised,
                t.surface_hovered,
                t.accent,
                disabled_solid,
            ],
        ),
        (
            ButtonVariant::Solid,
            true,
            [
                t.surface_hovered,
                t.surface_hovered,
                t.accent,
                disabled_solid,
            ],
        ),
        (
            ButtonVariant::Ghost,
            false,
            [
                Color::TRANSPARENT,
                t.surface_hovered,
                t.selection(),
                Color::TRANSPARENT,
            ],
        ),
        (
            ButtonVariant::Ghost,
            true,
            [
                t.selection(),
                t.surface_hovered,
                t.selection(),
                Color::TRANSPARENT,
            ],
        ),
        (
            ButtonVariant::Tab,
            false,
            [t.surface, t.surface_hovered, t.accent, t.surface],
        ),
        (
            ButtonVariant::Tab,
            true,
            [t.surface_raised, t.surface_hovered, t.accent, t.surface],
        ),
    ];
    for (variant, selected, expected) in cases {
        assert_eq!(
            colors(&t.interaction(variant, selected)),
            expected,
            "{variant:?} selected={selected}"
        );
    }
}

#[test]
fn chip_colours_mark_the_selected_chip() {
    let t = theme();
    assert_eq!(
        t.chip_colors(false),
        ChipColors {
            fill: Color::TRANSPARENT,
            border: t.border,
            text: t.text_muted
        }
    );
    assert_eq!(
        t.chip_colors(true),
        ChipColors {
            fill: t.selection(),
            border: t.accent,
            text: t.text
        }
    );
}

#[test]
fn text_defaults_follow_the_theme() {
    let t = theme();
    let spawned = spawn(t.text("Warren"));
    let text = spawned.get::<UIText>();
    assert_eq!(text.text, "Warren");
    assert_eq!(text.font_size, t.font_size_md);
    assert_eq!(text.line_height, t.line_height(t.font_size_md));
    assert_eq!(
        text.color, t.text,
        "body text uses the theme colour, not UIText's default"
    );
    assert!(text.wrap);
    assert!(!text.ellipsis);
}

#[test]
fn a_label_is_a_node_with_text() {
    let spawned = spawn(theme().label("Curiosities"));
    assert_eq!(spawned.get::<UINode>(), &UINode::default());
    assert_eq!(spawned.get::<UIText>().text, "Curiosities");
}

#[test]
fn font_size_always_pairs_its_line_height() {
    let t = theme();
    let spawned = spawn(t.label("x").font_size(20.0));
    let text = spawned.get::<UIText>();
    assert_eq!(text.font_size, 20.0);
    assert_eq!(text.line_height, t.line_height(20.0));
}

#[test]
fn typography_modifiers_compose_in_any_order() {
    let t = theme();
    let a = spawn(t.text("x").small().muted().mono().weight(500));
    let b = spawn(t.text("x").mono().weight(500).muted().small());
    for spawned in [&a, &b] {
        let text = spawned.get::<UIText>();
        assert_eq!(text.font_size, t.font_size_sm);
        assert_eq!(text.line_height, t.line_height(t.font_size_sm));
        assert_eq!(text.color, t.text_muted);
        assert!(matches!(text.font_family, FontFamily::Monospace));
        assert_eq!(text.font_weight, 500);
    }
}

#[test]
fn large_color_and_family_set_their_fields() {
    let t = theme();
    let spawned = spawn(
        t.text("x")
            .large()
            .color(t.accent)
            .family(FontFamily::Name("Inter".into())),
    );
    let text = spawned.get::<UIText>();
    assert_eq!(text.font_size, t.font_size_lg);
    assert_eq!(text.color, t.accent);
    assert!(matches!(&text.font_family, FontFamily::Name(name) if name == "Inter"));
}

#[test]
fn single_line_ellipsises_and_no_wrap_does_not() {
    let t = theme();
    let single = spawn(t.text("x").single_line());
    assert!(!single.get::<UIText>().wrap);
    assert!(single.get::<UIText>().ellipsis);
    let plain = spawn(t.text("x").no_wrap());
    assert!(!plain.get::<UIText>().wrap);
    assert!(!plain.get::<UIText>().ellipsis);
}

#[test]
fn layout_modifiers_shape_the_node() {
    let spawned = spawn(
        theme()
            .label("x")
            .width(UIValue::Px(10.0))
            .height(UIValue::Px(12.0))
            .min_width(UIValue::Px(1.0))
            .max_height(UIValue::Px(40.0))
            .row()
            .gap(3.0)
            .padding(2.0)
            .margin(UIRect::axes(1.0, 4.0))
            .grow()
            .fixed()
            .align_self(AlignItems::End)
            .clipped()
            .hidden()
            .z_index(5),
    );
    let node = spawned.get::<UINode>();
    assert_eq!(node.width, UIValue::Px(10.0));
    assert_eq!(node.height, UIValue::Px(12.0));
    assert_eq!(node.min_width, UIValue::Px(1.0));
    assert_eq!(node.max_height, UIValue::Px(40.0));
    assert_eq!(node.flex_direction, FlexDirection::Row);
    assert_eq!(
        node.align_items,
        Some(AlignItems::Center),
        "row() centres its children"
    );
    assert_eq!(node.gap, Vec2::splat(3.0));
    assert_eq!(node.padding, UIRect::all(2.0));
    assert_eq!(node.margin, UIRect::axes(1.0, 4.0));
    assert_eq!(node.flex_grow, 1.0);
    assert_eq!(node.flex_shrink, 0.0);
    assert_eq!(node.align_self, Some(AlignItems::End));
    assert_eq!(node.overflow_x, Overflow::Hidden);
    assert!(!node.visible);
    assert_eq!(node.z_index, 5);
}

#[test]
fn the_node_escape_hatch_reaches_any_uinode_setting() {
    let spawned = spawn(
        theme()
            .label("x")
            .node(|node| node.with_overflow_y(Overflow::Scroll)),
    );
    assert_eq!(spawned.get::<UINode>().overflow_y, Overflow::Scroll);
}

#[test]
fn label_nests_with_extra_components() {
    let spawned = spawn((theme().label("x").muted(), Marker));
    assert!(spawned.has::<Marker>());
    assert!(spawned.has::<UINode>());
    assert_eq!(spawned.get::<UIText>().color, theme().text_muted);
}

#[test]
fn text_reinserted_replaces_existing_text() {
    let t = theme();
    let mut spawned = spawn((UINode::default(), t.text("before")));
    spawned
        .world
        .insert(t.text("after").muted(), spawned.entity);
    assert_eq!(spawned.get::<UIText>().text, "after");
    assert_eq!(spawned.get::<UIText>().color, t.text_muted);
}

#[test]
fn text_converts_into_a_plain_uitext() {
    let t = theme();
    let text: UIText = t.text("x").small().into();
    assert_eq!(text.font_size, t.font_size_sm);
}

fn material_of(spawned: &Spawned) -> &UIMaterial {
    spawned.get::<UIMaterial>()
}

#[test]
fn each_surface_takes_its_fill_and_radius_from_the_theme() {
    let t = theme();
    let cases = [
        (spawn(t.canvas()), t.canvas, 0.0),
        (spawn(t.panel()), t.surface, t.radius_lg),
        (spawn(t.card()), t.surface_raised, t.radius_md),
    ];
    for (spawned, fill, radius) in &cases {
        let material = material_of(spawned);
        assert_eq!(material.color, fill.to_linear());
        assert_eq!(material.corner_radius, *radius);
        assert_eq!(material.border_width, 0.0);
    }
}

#[test]
fn a_popup_is_a_bordered_clipped_column_card() {
    let t = theme();
    let spawned = spawn(t.popup());
    let material = material_of(&spawned);
    assert_eq!(material.color, t.surface_raised.to_linear());
    assert_eq!(material.corner_radius, t.radius_md);
    assert_eq!(material.border_color, t.border.to_linear());
    assert_eq!(material.border_width, 1.0);
    let node = spawned.get::<UINode>();
    assert_eq!(node.flex_direction, FlexDirection::Column);
    assert_eq!(node.overflow_x, Overflow::Hidden);
    assert_eq!(node.overflow_y, Overflow::Hidden);
}

#[test]
fn shape_modifiers_restyle_a_surface() {
    let t = theme();
    let spawned = spawn(
        t.panel()
            .fill(t.accent)
            .radius_sm()
            .border(t.focus, 2.0)
            .rotation(0.5),
    );
    let material = material_of(&spawned);
    assert_eq!(material.color, t.accent.to_linear());
    assert_eq!(material.corner_radius, t.radius_sm);
    assert_eq!(material.border_color, t.focus.to_linear());
    assert_eq!(material.border_width, 2.0);
    assert_eq!(material.rotation, 0.5);
}

#[test]
fn stacks_space_their_children_by_the_theme() {
    let t = theme();
    let row = spawn(t.row());
    let node = row.get::<UINode>();
    assert_eq!(node.flex_direction, FlexDirection::Row);
    assert_eq!(node.align_items, Some(AlignItems::Center));
    assert_eq!(node.gap, Vec2::splat(t.spacing_sm));

    let column = spawn(t.column().gap(0.0));
    let node = column.get::<UINode>();
    assert_eq!(node.flex_direction, FlexDirection::Column);
    assert_eq!(node.gap, Vec2::ZERO);
}

#[test]
fn dividers_are_one_border_coloured_pixel() {
    let t = theme();
    let horizontal = spawn(t.divider());
    assert_eq!(horizontal.get::<UINode>().height, UIValue::Px(1.0));
    assert_eq!(horizontal.get::<UINode>().flex_shrink, 0.0);
    assert_eq!(
        horizontal.get::<UINode>().align_self,
        Some(AlignItems::Stretch)
    );
    assert_eq!(material_of(&horizontal).color, t.border.to_linear());

    let vertical = spawn(t.divider().vertical());
    assert_eq!(vertical.get::<UINode>().width, UIValue::Px(1.0));
    assert_eq!(vertical.get::<UINode>().height, UIValue::Auto);
}

#[test]
fn a_pressable_is_a_ghost_row_by_default() {
    let t = theme();
    let spawned = spawn(t.pressable());
    assert!(spawned.has::<Interactable>());
    assert_eq!(
        colors(spawned.get::<UIInteractionStyle>()),
        colors(&t.interaction(ButtonVariant::Ghost, false))
    );
    let material = spawned.get::<UIMaterial>();
    assert_eq!(material.corner_radius, t.radius_sm);
    assert_eq!(
        material.color,
        Color::TRANSPARENT.to_linear(),
        "the first frame already shows the normal colour"
    );
    assert_eq!(spawned.get::<UINode>().flex_shrink, 0.0);
}

#[test]
fn a_button_is_a_solid_bordered_control() {
    let t = theme();
    let spawned = spawn(t.button("Save"));
    assert!(spawned.has::<UIButton>());
    assert!(spawned.has::<Interactable>());
    assert_eq!(
        colors(spawned.get::<UIInteractionStyle>()),
        colors(&t.interaction(ButtonVariant::Solid, false))
    );
    let material = spawned.get::<UIMaterial>();
    assert_eq!(material.color, t.surface_raised.to_linear());
    assert_eq!(material.border_color, t.border.to_linear());
    assert_eq!(material.border_width, 1.0);
    assert_eq!(material.corner_radius, t.radius_sm);
    let node = spawned.get::<UINode>();
    assert_eq!(node.height, UIValue::Px(t.control_height));
    assert_eq!(node.flex_shrink, 0.0);
    assert_eq!(node.padding, UIRect::axes(t.spacing_sm, t.spacing_md));
    let text = spawned.get::<UIText>();
    assert_eq!(text.text, "Save");
    assert_eq!(text.color, t.text);
    assert!(!text.wrap && text.ellipsis);
}

#[test]
fn interaction_modifiers_compose_in_any_order() {
    let t = theme();
    let a = spawn(t.button("x").pressed(t.error).ghost().selected(true));
    let b = spawn(t.button("x").selected(true).ghost().pressed(t.error));
    let mut expected = t.interaction(ButtonVariant::Ghost, true);
    expected.pressed = t.error;
    assert_eq!(colors(a.get::<UIInteractionStyle>()), colors(&expected));
    assert_eq!(colors(b.get::<UIInteractionStyle>()), colors(&expected));
    assert_eq!(a.get::<UIMaterial>().color, t.selection().to_linear());
}

#[test]
fn a_selected_tab_is_raised_and_disabled_colour_overrides() {
    let t = theme();
    let spawned = spawn(t.pressable().tab().selected(true).disabled_color(t.error));
    let style = spawned.get::<UIInteractionStyle>();
    assert_eq!(style.normal, t.surface_raised);
    assert_eq!(style.disabled, t.error);
}

#[test]
fn button_nests_with_extra_components() {
    let spawned = spawn((theme().button("Close").ghost().large(), Marker));
    assert!(spawned.has::<Marker>());
    assert!(spawned.has::<UIButton>());
    assert_eq!(spawned.get::<UIText>().font_size, theme().font_size_lg);
}

#[test]
fn a_char_labels_a_button() {
    assert_eq!(spawn(theme().button('x')).get::<UIText>().text, "x");
}

#[test]
fn text_field_defaults() {
    let t = theme();
    let spawned = spawn(t.text_field("Search entities…"));
    assert!(spawned.has::<Interactable>());
    assert!(spawned.has::<UIFocusable>());
    assert_eq!(spawned.get::<UITextInput>().placeholder, "Search entities…");
    let node = spawned.get::<UINode>();
    assert_eq!(node.height, UIValue::Px(t.control_height));
    assert_eq!(node.flex_shrink, 0.0);
    assert_eq!(node.padding, UIRect::axes(t.spacing_xs, t.spacing_sm));
    let material = spawned.get::<UIMaterial>();
    assert_eq!(material.color, t.canvas.to_linear());
    assert_eq!(material.border_color, t.border.to_linear());
    assert_eq!(material.border_width, 1.0);
    assert_eq!(material.corner_radius, t.radius_md);
    let text = spawned.get::<UIText>();
    assert!(!text.wrap, "fields stay on one line");
    assert!(
        !text.ellipsis,
        "a field shows its whole value while editing"
    );
    assert_eq!(text.color, t.text);
}

#[test]
fn a_bare_field_has_no_fill_or_border_and_fill_overrides() {
    let t = theme();
    let bare = spawn(t.text_field("").bare());
    assert_eq!(
        bare.get::<UIMaterial>().color,
        Color::TRANSPARENT.to_linear()
    );
    assert_eq!(bare.get::<UIMaterial>().border_width, 0.0);
    let filled = spawn(t.text_field("").fill(t.surface));
    assert_eq!(filled.get::<UIMaterial>().color, t.surface.to_linear());
}

#[test]
fn chip_colours_follow_its_selection() {
    let t = theme();
    for selected in [false, true] {
        let spawned = spawn(t.chip("mesh").selected(selected));
        let ChipColors { fill, border, text } = t.chip_colors(selected);
        let material = spawned.get::<UIMaterial>();
        assert_eq!(material.color, fill.to_linear());
        assert_eq!(material.border_color, border.to_linear());
        assert_eq!(material.border_width, 1.0);
        assert_eq!(material.corner_radius, t.radius_sm);
        assert_eq!(spawned.get::<UIText>().color, text);
        assert_eq!(spawned.get::<UIText>().font_size, t.font_size_sm);
        assert!(spawned.has::<Interactable>());
    }
}

#[test]
fn chip_selection_overrides_text_colour() {
    let t = theme();
    let spawned = spawn(t.chip("mesh").muted().selected(true));
    assert_eq!(
        spawned.get::<UIText>().color,
        t.text,
        "a chip's text colour always comes from chip_colors"
    );
}

#[test]
fn a_checkbox_uses_the_accent_when_checked() {
    let t = theme();
    let spawned = spawn(t.checkbox("CHECK", true));
    let checkbox = spawned.get::<UICheckbox>();
    assert!(checkbox.checked);
    assert_eq!(checkbox.checked_color, t.accent);
    assert_eq!(checkbox.unchecked_color, t.surface_raised);
    assert_eq!(spawned.get::<UIText>().text, "CHECK");
    assert_eq!(
        spawned.get::<UINode>().height,
        UIValue::Px(t.control_height)
    );
    assert!(spawned.has::<Interactable>());
}

#[test]
fn a_slider_holds_its_range() {
    let t = theme();
    let spawned = spawn(t.slider(0.62, 0.0, 1.0).width(UIValue::Px(140.0)));
    let slider = spawned.get::<UISlider>();
    assert_eq!((slider.value, slider.min, slider.max), (0.62, 0.0, 1.0));
    assert_eq!(
        spawned.get::<UIMaterial>().color,
        t.surface_raised.to_linear()
    );
    assert_eq!(spawned.get::<UINode>().width, UIValue::Px(140.0));
    assert!(spawned.has::<Interactable>());
}

#[test]
fn a_dropdown_hangs_below_its_trigger_and_toggles_with_it() {
    let t = theme();
    let mut world = World::default();
    let trigger = world.spawn(Marker);
    let entity = world.spawn(t.dropdown(trigger));
    let panel = world
        .get_component_for_entity::<UIAnchoredPanel>(entity)
        .unwrap();
    assert_eq!(panel.target, UIAnchorTarget::Node { entity: trigger });
    assert_eq!(panel.owner, Some(trigger));
    assert!(panel.toggled_by_owner);
    assert_eq!(panel.side, UIAnchorSide::Below);
    assert_eq!(panel.align, UIAnchorAlign::Start);
    assert_eq!(panel.gap, t.spacing_xs);
    assert!(!panel.open);
    let material = world
        .get_component_for_entity::<UIMaterial>(entity)
        .unwrap();
    assert_eq!(material.color, t.surface_raised.to_linear());
    assert_eq!(material.border_width, 1.0);
}

#[test]
fn popup_anchor_modifiers_set_the_panel() {
    let t = theme();
    let mut world = World::default();
    let trigger = world.spawn(Marker);
    let search = world.spawn(Marker);
    let entity = world.spawn(
        t.dropdown(trigger)
            .anchor_side(UIAnchorSide::Above)
            .anchor_align(UIAnchorAlign::End)
            .anchor_gap(6.0)
            .focus_on_open(search)
            .open(true)
            .fill(t.canvas)
            .width(UIValue::Px(272.0)),
    );
    let panel = world
        .get_component_for_entity::<UIAnchoredPanel>(entity)
        .unwrap();
    assert_eq!(panel.side, UIAnchorSide::Above);
    assert_eq!(panel.align, UIAnchorAlign::End);
    assert_eq!(panel.gap, 6.0);
    assert_eq!(panel.focus_on_open, Some(search));
    assert!(panel.open);
    assert_eq!(
        world
            .get_component_for_entity::<UIMaterial>(entity)
            .unwrap()
            .color,
        t.canvas.to_linear()
    );
    assert_eq!(
        world
            .get_component_for_entity::<UINode>(entity)
            .unwrap()
            .width,
        UIValue::Px(272.0)
    );
}

#[test]
fn a_context_menu_floats_at_a_point_with_no_owner() {
    let t = theme();
    let spawned = spawn((
        t.context_menu().anchor_target(UIAnchorTarget::Point {
            position: Vec2::new(4.0, 5.0),
        }),
        t.text("Rename"),
    ));
    let panel = spawned.get::<UIAnchoredPanel>();
    assert_eq!(
        panel.target,
        UIAnchorTarget::Point {
            position: Vec2::new(4.0, 5.0)
        }
    );
    assert_eq!(panel.owner, None);
    assert!(!panel.toggled_by_owner);
    assert_eq!(spawned.get::<UIText>().text, "Rename");
}

#[test]
fn normal_and_hovered_override_the_variant_colours() {
    let t = theme();
    let spawned = spawn(t.button("Go").normal(t.accent).hovered(t.accent_hovered));
    let style = spawned.get::<UIInteractionStyle>();
    let base = t.interaction(ButtonVariant::Solid, false);
    assert_eq!(style.normal, t.accent);
    assert_eq!(style.hovered, t.accent_hovered);
    assert_eq!(style.pressed, base.pressed);
    assert_eq!(style.disabled, base.disabled);
    assert_eq!(spawned.get::<UIMaterial>().color, t.accent.to_linear());
}

#[test]
fn colour_overrides_survive_any_modifier_order() {
    let t = theme();
    let a = spawn(t.button("x").normal(t.error).ghost());
    let b = spawn(t.button("x").ghost().normal(t.error));
    assert_eq!(a.get::<UIInteractionStyle>().normal, t.error);
    assert_eq!(
        colors(a.get::<UIInteractionStyle>()),
        colors(b.get::<UIInteractionStyle>())
    );
    assert_eq!(a.get::<UIMaterial>().color, t.error.to_linear());
    assert_eq!(a.get::<UIMaterial>().color, b.get::<UIMaterial>().color);
}

#[test]
fn a_field_value_moves_the_cursor_to_its_end() {
    let spawned = spawn(theme().text_field("Search…").value("héllo"));
    let input = spawned.get::<UITextInput>();
    assert_eq!(input.value, "héllo");
    assert_eq!(input.cursor, "héllo".len());
    assert_eq!(input.placeholder, "Search…");
}

#[test]
fn line_height_overrides_the_size_derived_leading() {
    let spawned = spawn(theme().text("x").font_size(9.0).line_height(18.0));
    let text = spawned.get::<UIText>();
    assert_eq!(text.font_size, 9.0);
    assert_eq!(text.line_height, 18.0);
}

#[test]
fn borderless_drops_the_border() {
    let spawned = spawn(theme().button("x").borderless());
    let material = material_of(&spawned);
    assert_eq!(material.border_width, 0.0);
    assert_eq!(material.border_color, Color::TRANSPARENT.to_linear());
}

#[test]
fn remaining_layout_modifiers_set_their_fields() {
    let spawned = spawn(
        theme()
            .label("x")
            .size(UIValue::Px(8.0), UIValue::Px(9.0))
            .min_height(UIValue::Px(2.0))
            .max_width(UIValue::Px(50.0))
            .shrink(0.5)
            .align_items(AlignItems::End)
            .justify(AlignContent::SpaceBetween),
    );
    let node = spawned.get::<UINode>();
    assert_eq!(node.width, UIValue::Px(8.0));
    assert_eq!(node.height, UIValue::Px(9.0));
    assert_eq!(node.min_height, UIValue::Px(2.0));
    assert_eq!(node.max_width, UIValue::Px(50.0));
    assert_eq!(node.flex_shrink, 0.5);
    assert_eq!(node.align_items, Some(AlignItems::End));
    assert_eq!(node.justify_content, Some(AlignContent::SpaceBetween));
}

#[test]
fn align_items_written_after_row_wins() {
    let node = spawn(theme().label("x").row().align_items(AlignItems::Start));
    assert_eq!(node.get::<UINode>().align_items, Some(AlignItems::Start));
}

#[test]
fn checkbox_and_slider_defaults() {
    let t = theme();
    let checkbox = spawn(t.checkbox("x", false));
    assert_eq!(
        checkbox.get::<UINode>().padding,
        UIRect::axes(t.spacing_xs, t.spacing_sm)
    );
    assert_eq!(checkbox.get::<UINode>().flex_shrink, 0.0);
    let slider = spawn(t.slider(0.0, 0.0, 1.0));
    assert_eq!(slider.get::<UINode>().height, UIValue::Px(t.control_height));
}

#[test]
fn a_chip_pads_two_by_seven() {
    assert_eq!(
        spawn(theme().chip("x")).get::<UINode>().padding,
        UIRect::axes(2.0, 7.0)
    );
}

#[test]
fn a_context_menu_is_a_bordered_raised_card() {
    let t = theme();
    let spawned = spawn(t.context_menu());
    let material = material_of(&spawned);
    assert_eq!(material.color, t.surface_raised.to_linear());
    assert_eq!(material.border_width, 1.0);
}

#[test]
fn a_vertical_divider_keeps_its_shrink_and_stretch() {
    let spawned = spawn(theme().divider().vertical());
    assert_eq!(spawned.get::<UINode>().flex_shrink, 0.0);
    assert_eq!(
        spawned.get::<UINode>().align_self,
        Some(AlignItems::Stretch)
    );
}

#[test]
fn a_small_bare_field_on_a_surface() {
    let t = theme();
    let spawned = spawn(t.text_field("").small().bare().fill(t.surface));
    assert_eq!(spawned.get::<UIText>().font_size, t.font_size_sm);
    assert_eq!(material_of(&spawned).color, t.surface.to_linear());
    assert_eq!(material_of(&spawned).border_width, 0.0);
}

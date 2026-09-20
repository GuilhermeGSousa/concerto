//! Covers ellipsis truncation: a name too long for its box is cut where it
//! actually stops fitting, not where a character count guesses.
use ui::text::{TextComponent, truncate_for_test};

const METRICS: (f32, f32) = (14.0, 20.0);

#[test]
fn text_that_fits_is_left_alone() {
    assert_eq!(truncate_for_test("bunny.glb", 400.0, METRICS), None);
}

#[test]
fn text_that_does_not_fit_gains_an_ellipsis() {
    let out = truncate_for_test("bunny_porcelain_subsurface.mat", 60.0, METRICS)
        .expect("a long name in a narrow box must truncate");
    assert!(out.ends_with('…'), "{out}");
    assert!(
        out.chars().count() < "bunny_porcelain_subsurface.mat".chars().count(),
        "{out}"
    );
}

#[test]
fn a_narrower_box_keeps_less() {
    let wide = truncate_for_test("white_rabbit_waistcoat.glb", 120.0, METRICS).unwrap();
    let narrow = truncate_for_test("white_rabbit_waistcoat.glb", 60.0, METRICS).unwrap();
    assert!(
        narrow.chars().count() < wide.chars().count(),
        "{narrow} vs {wide}"
    );
}

#[test]
fn a_box_with_no_room_does_not_panic() {
    let out = truncate_for_test("anything", 0.0, METRICS);
    assert_eq!(out, None, "nothing fits, and nothing should be attempted");
}

#[test]
fn multibyte_names_are_cut_on_character_boundaries() {
    // Slicing by byte would panic here; the search walks char boundaries.
    let out =
        truncate_for_test("ünïcödé_mësh_wíth_áccents.glb", 50.0, METRICS).expect("must truncate");
    assert!(out.ends_with('…'), "{out}");
}

#[test]
fn ellipsis_is_off_by_default() {
    assert!(!TextComponent::default().ellipsis);
}

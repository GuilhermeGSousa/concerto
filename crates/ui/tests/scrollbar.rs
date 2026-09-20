//! Covers scrollbar thumb geometry: where the thumb sits and how big it is for
//! a given scroll offset, and when the bar should not be shown at all.
use ui::scroll::{ThumbGeometry, thumb_geometry};

const TRACK: f32 = 200.0;

#[test]
fn a_list_that_fits_needs_no_bar() {
    assert_eq!(
        thumb_geometry(0.0, 300.0, 500.0, TRACK),
        None,
        "content shorter than its viewport must not draw a scrollbar"
    );
    assert_eq!(
        thumb_geometry(0.0, 500.0, 500.0, TRACK),
        None,
        "content exactly filling its viewport must not either"
    );
}

#[test]
fn the_thumb_is_the_visible_fraction_of_the_track() {
    // A quarter of the content is on screen, so the thumb is a quarter of the
    // track: the bar reads as "how much of this am I seeing".
    let geometry = thumb_geometry(0.0, 1000.0, 250.0, TRACK).expect("content overflows");
    assert_eq!(geometry.height, 50.0);
    assert_eq!(geometry.offset, 0.0);
}

#[test]
fn the_thumb_reaches_the_bottom_at_maximum_scroll() {
    let content = 1000.0;
    let viewport = 250.0;
    let max = content - viewport;

    let geometry = thumb_geometry(max, content, viewport, TRACK).expect("content overflows");
    assert_eq!(
        geometry.offset + geometry.height,
        TRACK,
        "at the end of the list the thumb must sit flush with the end of the track"
    );
}

#[test]
fn the_thumb_moves_proportionally() {
    let geometry = thumb_geometry(375.0, 1000.0, 250.0, TRACK).expect("content overflows");
    // Half of the 750px scrollable range, so half of the 150px thumb travel.
    assert_eq!(geometry.offset, 75.0);
}

#[test]
fn a_very_long_list_still_has_a_grabbable_thumb() {
    let geometry = thumb_geometry(0.0, 1_000_000.0, 250.0, TRACK).expect("content overflows");
    assert!(
        geometry.height >= 24.0,
        "the thumb must stay big enough to see and grab, got {}",
        geometry.height
    );
    assert!(geometry.height <= TRACK, "and must never exceed the track");
}

#[test]
fn a_clamped_thumb_still_reaches_both_ends() {
    let content = 1_000_000.0;
    let viewport = 250.0;
    let ThumbGeometry { offset, height } =
        thumb_geometry(content - viewport, content, viewport, TRACK).expect("content overflows");
    assert!(
        (offset + height - TRACK).abs() < 0.01,
        "even a minimum-size thumb must land flush at the end: {offset} + {height}"
    );
}

#[test]
fn an_offset_past_the_end_does_not_push_the_thumb_off_the_track() {
    let geometry = thumb_geometry(99_999.0, 1000.0, 250.0, TRACK).expect("content overflows");
    assert!(
        geometry.offset + geometry.height <= TRACK + 0.01,
        "a stale offset must clamp rather than overflow the track"
    );
}

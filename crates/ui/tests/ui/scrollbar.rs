//! Covers scrollbar thumb geometry: where the thumb sits and how big it is.
use concerto_ui::scroll::thumb_geometry;
use proptest::prelude::*;

const TRACK: f32 = 200.0;

#[test]
fn the_thumb_is_the_visible_fraction_of_the_track_at_the_scrolled_fraction() {
    struct Case {
        name: &'static str,
        offset: f32,
        content: f32,
        thumb: Option<[f32; 2]>,
    }
    for case in [
        Case {
            name: "content shorter than its viewport draws no bar",
            offset: 0.0,
            content: 300.0,
            thumb: None,
        },
        Case {
            name: "content exactly filling its viewport draws none either",
            offset: 0.0,
            content: 500.0,
            thumb: None,
        },
        Case {
            name: "a quarter visible is a quarter of the track",
            offset: 0.0,
            content: 2000.0,
            thumb: Some([0.0, 50.0]),
        },
        Case {
            name: "the thumb moves proportionally",
            offset: 750.0,
            content: 2000.0,
            thumb: Some([75.0, 50.0]),
        },
    ] {
        let thumb = thumb_geometry(case.offset, case.content, 500.0, TRACK)
            .map(|geometry| [geometry.offset, geometry.height]);
        assert_eq!(thumb, case.thumb, "{}", case.name);
    }
}

proptest! {
    #[test]
    fn the_thumb_stays_grabbable_inside_the_track_and_reaches_its_end_at_the_last_offset(
        viewport in 50.0f32..1_000.0,
        extra in 1.0f32..1_000_000.0,
        scrolled in 0.0f32..2.0,
    ) {
        let content = viewport + extra;
        let geometry = thumb_geometry(extra * scrolled, content, viewport, TRACK).unwrap();
        prop_assert!(geometry.offset >= 0.0);
        prop_assert!(geometry.height >= 24.0 && geometry.height <= TRACK, "{}", geometry.height);
        prop_assert!(geometry.offset + geometry.height <= TRACK + 0.01);

        let end = thumb_geometry(extra, content, viewport, TRACK).unwrap();
        prop_assert!((end.offset + end.height - TRACK).abs() < 0.01);
    }
}

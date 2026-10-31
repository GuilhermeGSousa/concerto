use concerto_app::App;

/// Adds the motion database editor to the editor's `App`.
pub fn install(_app: &mut App) {}

#[cfg(test)]
mod tests {
    use concerto_editor::hosted::testing::SceneFixture;

    #[test]
    fn the_testing_feature_opens_a_scene_in_an_extension_crate() {
        let mut fixture = SceneFixture::new();

        let document = fixture.open();

        assert!(
            fixture
                .scene_names(document)
                .contains(&"second".to_string())
        );
    }
}

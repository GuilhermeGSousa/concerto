//! The read-only editor of an animation clip.
pub mod preview;

use concerto_animation::clip::AnimationClip;
use concerto_app::{App, Plugin};

use crate::{
    asset_editor::{AssetEditor, AssetEditorAppExt, PreviewSpec},
    hosted::HostedEventsAppExt,
};
use preview::{ClipPreviewBones, ClipPreviewProgress, clip_preview};

/// Opens an `AnimationClip` in a tab that plays it in a preview.
#[derive(Default)]
pub struct AnimationClipEditor;

impl AssetEditor for AnimationClipEditor {
    fn preview(&self) -> Option<PreviewSpec> {
        Some(PreviewSpec {
            build: clip_preview,
        })
    }
}

/// Registers the clip editor and forwards what its preview reports; `EditorPlugin` registers it.
pub struct ClipEditorPlugin;

impl Plugin for ClipEditorPlugin {
    fn build(&self, app: &mut App) {
        app.register_asset_editor::<AnimationClip, AnimationClipEditor>()
            .expect("ClipEditorPlugin requires the asset-editor registry")
            .forward_hosted_event::<ClipPreviewProgress>()
            .forward_hosted_event::<ClipPreviewBones>();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        asset_editor::AssetEditorRegistry,
        hosted::{HostRole, Hosted, HostedApp, testing::*},
    };
    use concerto_app::{
        main_schedule::MainSchedulePlugin,
        plugins::{AssetManagerPlugin, TimePlugin},
    };
    use concerto_ecs::events::event_channel::EventChannel;
    use concerto_foundation::assets::Asset;
    use concerto_mesh::skeleton::SkeletonComponent;
    use concerto_render::ExclusiveCamera;
    use concerto_window::plugin::WindowPlugin;

    #[test]
    fn the_editor_plugin_registers_the_clip_editor_and_forwards_its_reports() {
        let mut app = App::new();
        app.register_plugin(MainSchedulePlugin)
            .register_plugin(AssetManagerPlugin)
            .register_plugin(TimePlugin)
            .register_plugin(WindowPlugin)
            .register_plugin(crate::EditorPlugin {
                project: None,
                decorated: false,
                recipe: plain_recipe,
            });

        assert!(app.is_plugin_added::<ClipEditorPlugin>());
        let editors = app.get_resource::<AssetEditorRegistry>().unwrap();
        assert!(editors.preview(AnimationClip::name()).is_some());
        assert!(
            app.get_resource::<EventChannel<Hosted<ClipPreviewProgress>>>()
                .is_some()
        );
        assert!(
            app.get_resource::<EventChannel<Hosted<ClipPreviewBones>>>()
                .is_some()
        );
    }

    #[test]
    fn opening_a_clip_opens_a_tab_with_an_empty_stage() {
        let mut f = SceneFixture::new();
        f.world
            .get_resource_mut::<AssetEditorRegistry>()
            .unwrap()
            .register::<AnimationClip, AnimationClipEditor>()
            .unwrap();

        let document = f.request_kind(AnimationClip::name());
        f.frames_until("the preview was never built", |f| {
            f.world
                .get_component_for_entity::<HostedApp>(document)
                .is_some()
        });
        f.frame();

        assert_eq!(
            f.world
                .get_component_for_entity::<HostedApp>(document)
                .unwrap()
                .role(),
            HostRole::Preview
        );
        assert!(
            f.scene_world(document)
                .get_resource::<ExclusiveCamera>()
                .is_some()
        );
        let stage = f.scene_world_mut(document);
        assert_eq!(
            stage.query::<&SkeletonComponent, ()>().iter(stage).count(),
            0
        );
    }
}

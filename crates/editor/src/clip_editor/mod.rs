//! The read-only editor of an animation clip.
pub mod preview;

use std::{path::Path, sync::Arc};

use concerto_animation::clip::AnimationClip;
use concerto_app::{App, Plugin, schedule_groups::Update};
use concerto_ecs::{
    Component, Entity, IntoSystemConfig, Query, Res, command::EntityCommandQueue,
    events::event_reader::EventReader,
};
use concerto_foundation::assets::{
    Asset, AssetId,
    content::read_content_asset,
    registry::{AssetEntry, AssetRegistry},
};
use concerto_scene::scene::Scene;

use crate::{
    asset_editor::{
        AssetEditor, AssetEditorAppExt, EditorDocument, PreviewSpec, finish_asset_request,
    },
    hosted::{Hosted, HostedEventsAppExt},
    preview::PreviewSignals,
    project::ProjectState,
    scene::load_scene,
    timeline::frame_count,
};
use preview::{
    ClipPreviewBones, ClipPreviewProgress, SetClipTransport, ShowClip, ShowClipScene, clip_preview,
};

/// What a clip's tab shows and how its preview plays, kept on the document.
#[derive(Component)]
pub struct ClipDocument {
    pub clip: Option<Arc<AnimationClip>>,
    pub facts: ClipFacts,
    pub time: f32,
    pub playing: bool,
    pub looping: bool,
    pub rate: f32,
    pub root_motion: bool,
    pub skeleton_gizmo: bool,
    pub picked_scene: Option<AssetId>,
    pub shown_scene: Option<AssetId>,
    pub scene_error: Option<String>,
    pub selection: Option<(u32, u32)>,
}

impl Default for ClipDocument {
    fn default() -> Self {
        Self {
            clip: None,
            facts: ClipFacts::default(),
            time: 0.0,
            playing: true,
            looping: true,
            rate: 1.0,
            root_motion: false,
            skeleton_gizmo: true,
            picked_scene: None,
            shown_scene: None,
            scene_error: None,
            selection: None,
        }
    }
}

/// What a clip holds, and how much of it the shown skeleton uses.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ClipFacts {
    pub duration: f32,
    pub rate: Option<f32>,
    pub frames: Option<u32>,
    pub clip_bones: usize,
    pub skeleton_bones: usize,
    pub driven: usize,
}

impl ClipFacts {
    fn of(clip: &AnimationClip) -> Self {
        let duration = clip.duration();
        let rate = clip.detected_frame_rate();
        Self {
            duration,
            rate,
            frames: rate.map(|rate| frame_count(duration, rate)),
            clip_bones: clip.target_ids().len(),
            skeleton_bones: 0,
            driven: 0,
        }
    }
}

/// Opens an `AnimationClip` in a tab that plays it in a preview.
#[derive(Default)]
pub struct AnimationClipEditor;

impl AssetEditor for AnimationClipEditor {
    fn build(&self, editor: &mut EntityCommandQueue) {
        editor.insert(ClipDocument::default());
    }

    fn preview(&self) -> Option<PreviewSpec> {
        Some(PreviewSpec {
            build: clip_preview,
        })
    }

    fn bottom_host(&self) -> bool {
        true
    }
}

pub(crate) fn sync_clip_preview(
    mut progress: EventReader<Hosted<ClipPreviewProgress>>,
    mut bones: EventReader<Hosted<ClipPreviewBones>>,
    documents: Query<&mut ClipDocument>,
) {
    for Hosted { document, event } in progress.read() {
        if let Some(mut state) = documents.get_entity(*document) {
            state.time = event.time;
            state.playing = event.playing;
        }
    }
    for Hosted { document, event } in bones.read() {
        if let Some(mut state) = documents.get_entity(*document) {
            state.facts.skeleton_bones = event.bones;
            state.facts.driven = event.driven;
        }
    }
}

fn read_clip(root: &Path, asset: &AssetEntry) -> anyhow::Result<AnimationClip> {
    let bytes = std::fs::read(root.join(&asset.address))?;
    let (header, payload) = read_content_asset(&bytes)?;
    anyhow::ensure!(
        header.asset_id == asset.id && header.kind == AnimationClip::name(),
        "Asset identity or type mismatch"
    );
    Ok(bincode::deserialize(payload)?)
}

fn default_scene<'a>(clip: &AssetEntry, assets: &'a AssetRegistry) -> Option<&'a AssetEntry> {
    assets
        .of_type::<Scene>()
        .find(|entry| entry.name() == "scene" && entry.folder() == clip.folder())
}

fn read_shown_scene(
    root: &Path,
    clip: &AssetEntry,
    picked: Option<AssetId>,
    assets: &AssetRegistry,
) -> anyhow::Result<Option<(AssetId, Scene)>> {
    let entry = match picked {
        Some(id) => Some(
            assets
                .get(id)
                .ok_or_else(|| anyhow::anyhow!("The picked scene is not in the project"))?,
        ),
        None => default_scene(clip, assets),
    };
    entry
        .map(|entry| {
            let scene = load_scene(root.to_owned(), entry.address.clone(), entry.id)?;
            Ok((entry.id, scene))
        })
        .transpose()
}

fn scene_signal(
    state: &mut ClipDocument,
    root: &Path,
    clip: &AssetEntry,
    assets: &AssetRegistry,
) -> ShowClipScene {
    let shown = read_shown_scene(root, clip, state.picked_scene, assets);
    state.scene_error = shown.as_ref().err().map(|error| format!("{error:#}"));
    let shown = shown.ok().flatten();
    state.shown_scene = shown.as_ref().map(|(id, _)| *id);
    ShowClipScene(shown.map(|(_, scene)| scene))
}

pub(crate) fn load_clip(
    documents: Query<(Entity, &mut EditorDocument, &mut ClipDocument)>,
    project: Res<ProjectState>,
    assets: Res<AssetRegistry>,
    mut previews: PreviewSignals,
) {
    let Some(root) = project.project.as_ref().map(|project| project.root.clone()) else {
        return;
    };
    for (entity, mut document, mut state) in documents.iter() {
        let Some(asset) = document.pending.clone() else {
            continue;
        };
        let clip = match read_clip(&root, &asset) {
            Ok(clip) => Arc::new(clip),
            Err(error) => {
                finish_asset_request(&mut document, &asset, Err(error));
                continue;
            }
        };
        state.facts = ClipFacts {
            skeleton_bones: state.facts.skeleton_bones,
            driven: state.facts.driven,
            ..ClipFacts::of(&clip)
        };
        state.clip = Some(Arc::clone(&clip));
        state.time = 0.0;
        state.selection = None;
        state.playing = true;
        previews.trigger(entity, ShowClip(clip));
        previews.trigger(
            entity,
            SetClipTransport {
                playing: true,
                looping: state.looping,
                rate: state.rate,
            },
        );
        previews.trigger(entity, scene_signal(&mut state, &root, &asset, &assets));
        finish_asset_request(&mut document, &asset, Ok(()));
    }
}

/// Registers the clip editor and forwards what its preview reports; `EditorPlugin` registers it.
pub struct ClipEditorPlugin;

impl Plugin for ClipEditorPlugin {
    fn build(&self, app: &mut App) {
        app.register_asset_editor::<AnimationClip, AnimationClipEditor>()
            .expect("ClipEditorPlugin requires the asset-editor registry")
            .forward_hosted_event::<ClipPreviewProgress>()
            .forward_hosted_event::<ClipPreviewBones>()
            .add_system(Update, (sync_clip_preview, load_clip).chain());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        asset_editor::{ActiveEditor, AssetEditorRegistry},
        hosted::{
            HostRole, Hosted, HostedApp, HostedPlugin,
            events::forward_events,
            input::{ViewportShown, ViewportView},
            testing::*,
        },
        preview::{PreviewHostPlugin, PreviewSignals},
        project::Project,
        scene::SceneRecipe,
    };
    use concerto_animation::clip::{AnimationChanelOutput, AnimationChannel};
    use concerto_app::{
        main_schedule::MainSchedulePlugin,
        plugins::{AssetManagerPlugin, TimePlugin},
    };
    use concerto_ecs::{IntoSystem, System, events::event_channel::EventChannel};
    use concerto_foundation::assets::{Asset, content::save_content_asset_into};
    use concerto_mesh::skeleton::SkeletonComponent;
    use concerto_render::ExclusiveCamera;
    use concerto_window::plugin::{Window, WindowPlugin};
    use glam::Vec2;
    use preview::SetClipTransport;
    use std::time::{Duration, Instant};
    use uuid::Uuid;

    #[test]
    fn a_clip_tab_gets_a_bottom_host() {
        let mut editors = AssetEditorRegistry::default();
        editors
            .register::<AnimationClip, AnimationClipEditor>()
            .unwrap();
        assert!(editors.bottom_host(AnimationClip::name()));
        assert!(!editors.bottom_host("no editor"));
    }

    fn keys(seconds: usize) -> AnimationChannel {
        let times: Vec<f32> = (0..=seconds * 30).map(|key| key as f32 / 30.0).collect();
        let translations = vec![[0.0; 3]; times.len()];
        AnimationChannel::new(
            times,
            AnimationChanelOutput::from_translation(translations.into_iter()),
        )
    }

    fn clip_of(seconds: usize) -> AnimationClip {
        let mut clip = AnimationClip::default();
        clip.add_channel(Uuid::from_u128(1), keys(seconds));
        clip
    }

    fn clip_fixture() -> SceneFixture {
        let mut f = SceneFixture::new();
        f.world
            .get_resource_mut::<AssetEditorRegistry>()
            .unwrap()
            .register::<AnimationClip, AnimationClipEditor>()
            .unwrap();
        f.world
            .insert_resource(EventChannel::<Hosted<ClipPreviewProgress>>::new());
        f.world
            .insert_resource(EventChannel::<Hosted<ClipPreviewBones>>::new());
        f.add_delivery(forward_events::<ClipPreviewProgress>.into_boxed_system());
        f.add_delivery(forward_events::<ClipPreviewBones>.into_boxed_system());
        f.add_delivery(sync_clip_preview.into_boxed_system());
        f.add_delivery(load_clip.into_boxed_system());
        f
    }

    fn write_clip(f: &mut SceneFixture, address: &str, clip: &AnimationClip) -> AssetEntry {
        let root = f.project_root.clone();
        let assets = f.world.get_resource_mut::<AssetRegistry>().unwrap();
        let id = save_content_asset_into(clip, &root, address, assets).unwrap();
        assets.get(id).unwrap().clone()
    }

    fn scene_at(f: &mut SceneFixture, address: &str) -> AssetEntry {
        let asset = AssetEntry {
            address: address.into(),
            ..scene_entry("fixture")
        };
        let path = f.project_root.join(address);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        f.add_to_project(&asset);
        asset
    }

    fn write_scene(f: &mut SceneFixture, address: &str, node: &str) -> AssetEntry {
        let asset = scene_at(f, address);
        write_scene_file(&f.project_root, &asset, &[node]);
        asset
    }

    fn write_broken_scene(f: &mut SceneFixture, address: &str) -> AssetEntry {
        let asset = scene_at(f, address);
        std::fs::write(f.project_root.join(address), b"not a scene").unwrap();
        asset
    }

    fn open_clip(f: &mut SceneFixture, clip: &AssetEntry) -> Entity {
        f.request(clip);
        let document = f.world.get_resource::<ActiveEditor>().unwrap().0.unwrap();
        f.frames_until("the clip never finished opening", |f| {
            f.document(document).pending.is_none()
                && f.world
                    .get_component_for_entity::<HostedApp>(document)
                    .is_some()
        });
        document
    }

    fn state(f: &SceneFixture, document: Entity) -> &ClipDocument {
        f.world
            .get_component_for_entity::<ClipDocument>(document)
            .expect("the document is a clip's")
    }

    fn state_mut(f: &mut SceneFixture, document: Entity) -> &mut ClipDocument {
        f.world
            .get_component_for_entity_mut::<ClipDocument>(document)
            .expect("the document is a clip's")
    }

    fn pause(f: &mut SceneFixture, document: Entity) {
        let mut system = (move |mut previews: PreviewSignals| {
            previews.trigger(
                document,
                SetClipTransport {
                    playing: false,
                    looping: true,
                    rate: 1.0,
                },
            );
        })
        .into_system();
        system.initialize(&mut f.world);
        system.run_and_apply((), &mut f.world);
        f.frames_until("the pause never came back", |f| !state(f, document).playing);
    }

    fn report_bones(f: &mut SceneFixture, document: Entity, bones: usize, driven: usize) {
        f.scene_world_mut(document)
            .get_resource_mut::<EventChannel<ClipPreviewBones>>()
            .unwrap()
            .push_event(ClipPreviewBones { bones, driven });
        f.frames_until("the bones never came back", |f| {
            state(f, document).facts.skeleton_bones == bones
        });
    }

    #[test]
    fn a_new_document_plays_and_loops_at_full_rate_with_the_gizmo_on() {
        let document = ClipDocument::default();

        assert!(document.clip.is_none());
        assert_eq!(document.time, 0.0);
        assert!(document.playing);
        assert!(document.looping);
        assert_eq!(document.rate, 1.0);
        assert!(!document.root_motion);
        assert!(document.skeleton_gizmo);
        assert_eq!(document.picked_scene, None);
        assert_eq!(document.shown_scene, None);
        assert_eq!(document.scene_error, None);
        assert_eq!(document.selection, None);
    }

    #[test]
    fn the_facts_come_from_the_clip_and_leave_the_skeletons_counts_at_zero() {
        let mut clip = clip_of(1);
        clip.add_channel(Uuid::from_u128(2), keys(1));
        clip.add_channel(Uuid::from_u128(2), keys(1));

        assert_eq!(
            ClipFacts::of(&clip),
            ClipFacts {
                duration: 1.0,
                rate: Some(30.0),
                frames: Some(31),
                clip_bones: 2,
                skeleton_bones: 0,
                driven: 0,
            }
        );
    }

    #[test]
    fn the_frames_fact_is_the_timelines_frame_count() {
        let facts = ClipFacts::of(&clip_of(10));

        assert_eq!(
            facts.frames,
            Some(frame_count(facts.duration, facts.rate.unwrap()))
        );
    }

    #[test]
    fn a_clip_with_no_detected_rate_has_no_frames() {
        let facts = ClipFacts::of(&AnimationClip::default());

        assert_eq!(facts.rate, None);
        assert_eq!(facts.frames, None);
    }

    #[test]
    fn a_clip_opens_with_the_scene_of_its_folder() {
        let mut f = clip_fixture();
        let scene = write_scene(&mut f, "content/hero/scene.gasset", "sibling");
        write_scene(&mut f, "content/other/scene.gasset", "stranger");
        write_scene(&mut f, "content/hero/level.gasset", "level");
        let walk = write_clip(&mut f, "content/hero/walk.gasset", &clip_of(10));

        let document = open_clip(&mut f, &walk);

        f.frames_until("the scene never reached the preview", |f| {
            f.scene_names(document).contains(&"sibling".to_owned())
        });
        let names = f.scene_names(document);
        assert!(!names.contains(&"stranger".to_owned()));
        assert!(!names.contains(&"level".to_owned()));
        let opened = state(&f, document);
        assert_eq!(opened.shown_scene, Some(scene.id));
        assert_eq!(opened.scene_error, None);
        assert_eq!(opened.facts, ClipFacts::of(&clip_of(10)));
        assert!(opened.clip.is_some());
        assert_eq!(f.document(document).current, Some(walk));
        assert_eq!(f.status(document), "");
    }

    #[test]
    fn a_folder_with_no_scene_shows_none_and_clears_the_stage() {
        let mut f = clip_fixture();
        write_scene(&mut f, "content/hero/scene.gasset", "sibling");
        let walk = write_clip(&mut f, "content/hero/walk.gasset", &clip_of(10));
        let lone = write_clip(&mut f, "content/empty/lone.gasset", &clip_of(10));

        let document = open_clip(&mut f, &walk);
        f.frames_until("the scene never reached the preview", |f| {
            f.scene_names(document).contains(&"sibling".to_owned())
        });
        open_clip(&mut f, &lone);

        f.frames_until("the old scene stayed on the stage", |f| {
            !f.scene_names(document).contains(&"sibling".to_owned())
        });
        assert_eq!(state(&f, document).shown_scene, None);
        assert_eq!(state(&f, document).scene_error, None);
    }

    #[test]
    fn a_picked_scene_wins_over_the_folders() {
        let mut f = clip_fixture();
        write_scene(&mut f, "content/hero/scene.gasset", "sibling");
        let level = write_scene(&mut f, "content/world/level.gasset", "level");
        let walk = write_clip(&mut f, "content/hero/walk.gasset", &clip_of(10));

        f.request(&walk);
        let document = f.world.get_resource::<ActiveEditor>().unwrap().0.unwrap();
        state_mut(&mut f, document).picked_scene = Some(level.id);
        open_clip(&mut f, &walk);

        f.frames_until("the picked scene never reached the preview", |f| {
            f.scene_names(document).contains(&"level".to_owned())
        });
        assert!(!f.scene_names(document).contains(&"sibling".to_owned()));
        assert_eq!(state(&f, document).shown_scene, Some(level.id));
    }

    #[test]
    fn a_scene_that_fails_to_load_is_reported_and_the_clip_still_loads() {
        let mut f = clip_fixture();
        write_scene(&mut f, "content/hero/scene.gasset", "sibling");
        write_broken_scene(&mut f, "content/broken/scene.gasset");
        let walk = write_clip(&mut f, "content/hero/walk.gasset", &clip_of(10));
        let broken = write_clip(&mut f, "content/broken/walk.gasset", &clip_of(10));

        let document = open_clip(&mut f, &walk);
        f.frames_until("the scene never reached the preview", |f| {
            f.scene_names(document).contains(&"sibling".to_owned())
        });
        open_clip(&mut f, &broken);

        f.frames_until("the old scene stayed on the stage", |f| {
            !f.scene_names(document).contains(&"sibling".to_owned())
        });
        let opened = state(&f, document);
        assert!(opened.scene_error.is_some());
        assert_eq!(opened.shown_scene, None);
        assert_eq!(f.document(document).current, Some(broken));
        assert_eq!(f.status(document), "");

        open_clip(&mut f, &walk);

        assert_eq!(state(&f, document).scene_error, None);
    }

    #[test]
    fn a_picked_scene_that_left_the_project_is_reported() {
        let mut f = clip_fixture();
        write_scene(&mut f, "content/hero/scene.gasset", "sibling");
        let walk = write_clip(&mut f, "content/hero/walk.gasset", &clip_of(10));

        f.request(&walk);
        let document = f.world.get_resource::<ActiveEditor>().unwrap().0.unwrap();
        state_mut(&mut f, document).picked_scene = Some(AssetId::new());
        open_clip(&mut f, &walk);

        let opened = state(&f, document);
        assert!(opened.scene_error.is_some());
        assert_eq!(opened.shown_scene, None);
        assert!(opened.clip.is_some());
    }

    #[test]
    fn a_broken_clip_fails_its_request() {
        let mut f = clip_fixture();
        let walk = write_clip(&mut f, "content/walk.gasset", &clip_of(1));
        std::fs::write(f.project_root.join(&walk.address), b"not a clip").unwrap();

        let document = open_clip(&mut f, &walk);

        assert!(f.status(document).contains("content asset"));
        assert!(state(&f, document).clip.is_none());
        assert_eq!(f.document(document).current, None);
    }

    #[test]
    fn a_file_of_another_kind_fails_its_request() {
        let mut f = clip_fixture();
        let scene = write_scene(&mut f, "content/scene.gasset", "sibling");
        let mistaken = AssetEntry {
            kind: AnimationClip::name().into(),
            ..scene
        };
        f.add_to_project(&mistaken);

        let document = open_clip(&mut f, &mistaken);

        assert!(f.status(document).contains("mismatch"));
        assert!(state(&f, document).clip.is_none());
    }

    #[test]
    fn a_broken_second_clip_leaves_the_first_where_it_was() {
        let mut f = clip_fixture();
        let walk = write_clip(&mut f, "content/walk.gasset", &clip_of(10));
        let broken = write_clip(&mut f, "content/broken.gasset", &clip_of(1));
        std::fs::write(f.project_root.join(&broken.address), b"not a clip").unwrap();

        let document = open_clip(&mut f, &walk);
        f.frames_until("the head never moved", |f| state(f, document).time > 0.0);
        pause(&mut f, document);
        let paused_at = state(&f, document).time;
        state_mut(&mut f, document).selection = Some((1, 2));
        open_clip(&mut f, &broken);

        assert_ne!(f.status(document), "");
        assert_eq!(f.document(document).current, Some(walk));
        let kept = state(&f, document);
        assert_eq!(kept.time, paused_at);
        assert_eq!(kept.selection, Some((1, 2)));
        assert!(!kept.playing);
        assert_eq!(kept.facts, ClipFacts::of(&clip_of(10)));
    }

    #[test]
    fn the_document_follows_what_the_preview_reports() {
        let mut f = clip_fixture();
        let walk = write_clip(&mut f, "content/walk.gasset", &clip_of(10));

        let document = open_clip(&mut f, &walk);

        f.frames_until("the head never moved", |f| state(f, document).time > 0.0);
        assert!(state(&f, document).playing);
        pause(&mut f, document);
        report_bones(&mut f, document, 65, 60);
        assert_eq!(state(&f, document).facts.driven, 60);
    }

    #[test]
    fn a_second_clip_resets_time_and_selection_plays_and_keeps_the_rest() {
        let mut f = clip_fixture();
        write_scene(&mut f, "content/hero/scene.gasset", "sibling");
        let level = write_scene(&mut f, "content/world/level.gasset", "level");
        let walk = write_clip(&mut f, "content/hero/walk.gasset", &clip_of(10));
        let run = write_clip(&mut f, "content/hero/run.gasset", &clip_of(20));

        let document = open_clip(&mut f, &walk);
        f.frames_until("the head never moved", |f| state(f, document).time > 0.0);
        pause(&mut f, document);
        report_bones(&mut f, document, 65, 60);
        let chosen = state_mut(&mut f, document);
        chosen.picked_scene = Some(level.id);
        chosen.selection = Some((3, 9));
        chosen.rate = 2.0;
        chosen.looping = false;
        chosen.root_motion = true;
        chosen.skeleton_gizmo = false;

        open_clip(&mut f, &run);

        let reset = state(&f, document);
        assert_eq!(reset.time, 0.0);
        assert_eq!(reset.selection, None);
        assert!(reset.playing);
        assert_eq!(reset.picked_scene, Some(level.id));
        assert_eq!(reset.shown_scene, Some(level.id));
        assert_eq!(reset.rate, 2.0);
        assert!(!reset.looping);
        assert!(reset.root_motion);
        assert!(!reset.skeleton_gizmo);
        assert_eq!(reset.facts.duration, 20.0);
        assert_eq!(reset.facts.skeleton_bones, 65);
        assert_eq!(reset.facts.driven, 60);
        assert_eq!(f.document(document).current, Some(run));

        f.frames_until("the second clip never played", |f| {
            state(f, document).time > 0.0
        });
        assert!(state(&f, document).playing);
        assert!(f.scene_names(document).contains(&"level".to_owned()));
        assert!(!f.unsaved(document));
    }

    #[test]
    fn the_plugin_loads_a_clip_and_follows_its_preview_in_the_editors_schedule() {
        let directory = tempfile::tempdir().unwrap();
        let mut assets = AssetRegistry::default();
        let id = save_content_asset_into(
            &clip_of(10),
            directory.path(),
            "content/walk.gasset",
            &mut assets,
        )
        .unwrap();
        let walk = assets.get(id).unwrap().clone();
        let mut project = ProjectState::default();
        project.project = Some(Project {
            root: directory.path().into(),
            content_root: "content".into(),
        });
        let mut app = App::new();
        app.register_plugin(MainSchedulePlugin)
            .register_plugin(TimePlugin)
            .register_plugin(WindowPlugin)
            .register_plugin(HostedPlugin)
            .register_plugin(PreviewHostPlugin)
            .insert_resource(Window::new((640, 480), 1.0))
            .insert_resource(ActiveEditor::default())
            .insert_resource(AssetEditorRegistry::default())
            .insert_resource(project)
            .insert_resource(assets)
            .insert_resource(SceneRecipe {
                recipe: scene_recipe,
                gpu: None,
                clipboard: None,
            })
            .insert_resource(ViewportShown(Some(ViewportView {
                origin: Vec2::ZERO,
                size: (64, 64),
                over: false,
                typing: false,
            })))
            .register_plugin(ClipEditorPlugin);
        app.finish_plugin_build();
        let world = app.main_mut().world_mut();
        let document = world.spawn((
            EditorDocument {
                asset_type: AnimationClip::name(),
                title: "walk".into(),
                current: None,
                pending: Some(walk),
                order: 1,
                status: String::new(),
            },
            ClipDocument::default(),
        ));
        world.get_resource_mut::<ActiveEditor>().unwrap().0 = Some(document);

        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            app.update();
            let followed = app
                .main()
                .world()
                .get_component_for_entity::<ClipDocument>(document)
                .unwrap();
            if followed.time > 0.0 {
                break;
            }
            assert!(Instant::now() < deadline, "the head never moved");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

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

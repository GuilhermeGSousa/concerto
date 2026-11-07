//! Saving a clip tab's events into the clip's `.gasset`, and putting them back on a discard.
use std::{path::Path, sync::Arc};

use anyhow::Context;
use concerto_ecs::{
    Entity, Query, Res, ResMut, With, command::CommandQueue, events::event_writer::EventWriter,
    signal::On,
};
use concerto_editor::{
    asset_editor::{EditorDocument, SaveRequested, Unsaved},
    guard::DiscardEdits,
    preview::PreviewSignals,
    project::ProjectState,
    scene::{SaveFailed, SceneSaved},
};
use concerto_foundation::assets::{
    content::resave_content_asset,
    registry::{AssetEntry, AssetRegistry},
};

use super::{
    ClipDocument, ClipFacts,
    events::{TabEvent, TabEventKind, read_events, write_events},
    preview::{SeekClip, SetClipTransport, ShowClip},
    read_clip,
};

fn event_name(event: &TabEvent) -> &str {
    match &event.kind {
        TabEventKind::Named(name) => name.as_str(),
        TabEventKind::Typed { type_name, .. } => type_name.as_str(),
    }
}

fn check_times(events: &[TabEvent], duration: f32) -> Result<(), String> {
    match events.iter().find(|event| event.time > duration) {
        Some(event) => Err(format!(
            "`{}` at {:.2} s is past the clip's {duration:.2} s",
            event_name(event),
            event.time
        )),
        None => Ok(()),
    }
}

fn save_clip_document(
    document: &mut EditorDocument,
    state: &mut ClipDocument,
    root: &Path,
    assets: &mut AssetRegistry,
) -> anyhow::Result<AssetEntry> {
    let current = document.current.clone().context("No clip is open")?;
    let mut clip = read_clip(root, &current)?;
    check_times(&state.events, clip.duration()).map_err(anyhow::Error::msg)?;
    let keys = write_events(&state.events).map_err(anyhow::Error::msg)?;
    clip.replace_events(keys).map_err(anyhow::Error::msg)?;
    let id = resave_content_asset(&clip, root, &current.address, assets)?;
    let saved = assets
        .get(id)
        .cloned()
        .context("The saved clip was not registered")?;
    document.current = Some(saved.clone());
    state.facts = ClipFacts {
        skeleton_bones: state.facts.skeleton_bones,
        driven: state.facts.driven,
        ..ClipFacts::of(&clip)
    };
    state.clip = Some(Arc::new(clip));
    Ok(saved)
}

fn show_saved_clip(entity: Entity, state: &ClipDocument, previews: &mut PreviewSignals) {
    let Some(clip) = state.clip.as_ref() else {
        return;
    };
    previews.trigger(entity, ShowClip(Arc::clone(clip)));
    previews.trigger(
        entity,
        SetClipTransport {
            playing: state.playing,
            looping: state.looping,
            rate: state.rate,
        },
    );
    previews.trigger(entity, SeekClip(state.time));
}

pub(crate) fn save_clip(
    requested: Query<(Entity, &mut EditorDocument, &mut ClipDocument), With<SaveRequested>>,
    project: Res<ProjectState>,
    mut assets: ResMut<AssetRegistry>,
    mut saved: EventWriter<SceneSaved>,
    mut failed: EventWriter<SaveFailed>,
    mut previews: PreviewSignals,
    mut cmd: CommandQueue,
) {
    let root = project
        .project
        .as_ref()
        .map(|project| project.root.as_path());
    for (entity, mut document, mut state) in requested.iter() {
        cmd.remove::<SaveRequested>(entity);
        let written = root
            .context("Open a project before saving")
            .and_then(|root| save_clip_document(&mut document, &mut state, root, &mut assets));
        match written {
            Ok(asset) => {
                document.status = format!("Saved {}", asset.address);
                show_saved_clip(entity, &state, &mut previews);
                saved.write(SceneSaved {
                    document: entity,
                    asset,
                });
                cmd.remove::<Unsaved>(entity);
            }
            Err(error) => {
                let message = format!("Save failed: {error:#}");
                document.status = message.clone();
                failed.write(SaveFailed {
                    document: entity,
                    message,
                });
            }
        }
    }
}

pub(crate) fn restore_clip_events(
    on: On<DiscardEdits>,
    documents: Query<(&mut EditorDocument, &mut ClipDocument)>,
    mut cmd: CommandQueue,
) {
    let document = on.signal().0;
    let Some((mut editor, mut state)) = documents.get_entity(document) else {
        return;
    };
    let Some(clip) = state.clip.clone() else {
        return;
    };
    match read_events(&clip) {
        Ok(events) => {
            state.events = events;
            state.selected_event = None;
        }
        Err(message) => {
            editor.status = message;
            cmd.insert(Unsaved, document);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{
        FOOTSTEP, clip_fixture, clip_of, footstep_value, named_key, open_clip, report_bones, state,
        state_mut, typed_key, write_clip,
    };
    use concerto_animation::clip::AnimationClip;
    use concerto_ecs::IntoSystem;
    use concerto_editor::hosted::testing::SceneFixture;
    use concerto_foundation::assets::{
        Asset, AssetId,
        content::{
            ContentAssetHeader, ImportProvenance, read_content_asset_header, write_content_asset,
        },
    };

    fn save_fixture() -> SceneFixture {
        let mut f = clip_fixture();
        f.add_delivery(save_clip.into_boxed_system());
        f.world.add_listener(restore_clip_events);
        f
    }

    fn write_imported_clip(
        f: &mut SceneFixture,
        address: &str,
        clip: &AnimationClip,
    ) -> AssetEntry {
        let provenance = Some(ImportProvenance {
            source: "assets/hero.glb".to_owned(),
            sub_asset: "animation/walk".to_owned(),
        });
        let header = ContentAssetHeader {
            asset_id: AssetId::new(),
            references: Vec::new(),
            kind: AnimationClip::name().to_owned(),
            kind_version: AnimationClip::FORMAT_VERSION,
            provenance: provenance.clone(),
            derived: None,
        };
        let path = f.project_root.join(address);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let payload = bincode::serialize(clip).unwrap();
        std::fs::write(&path, write_content_asset(&header, &payload).unwrap()).unwrap();
        let entry = AssetEntry {
            id: header.asset_id,
            address: address.to_owned(),
            kind: header.kind,
            kind_version: Some(header.kind_version),
            provenance,
        };
        f.world
            .get_resource_mut::<AssetRegistry>()
            .unwrap()
            .insert(entry.clone());
        entry
    }

    fn save(f: &mut SceneFixture, document: Entity) {
        f.world.insert(SaveRequested, document);
        f.deliver();
    }

    #[test]
    fn save_writes_the_events_sorted_keeps_the_tabs_order_and_the_provenance() {
        let mut f = save_fixture();
        let mut eventful = clip_of(2);
        eventful.replace_events(vec![typed_key(1.0)]).unwrap();
        let walk = write_imported_clip(&mut f, "content/walk.gasset", &eventful);
        let document = open_clip(&mut f, &walk);
        report_bones(&mut f, document, 65, 60);
        let edited = state_mut(&mut f, document);
        edited.add_event("land", 1.5).unwrap();
        edited.add_event("footstep", 0.5).unwrap();
        let order = edited.events.clone();
        f.world.insert(Unsaved, document);

        save(&mut f, document);

        let written = read_clip(&f.project_root, &walk).unwrap();
        assert_eq!(
            read_events(&written).unwrap(),
            vec![order[2].clone(), order[0].clone(), order[1].clone()]
        );
        let header = read_content_asset_header(&f.project_root.join(&walk.address)).unwrap();
        assert_eq!(header.asset_id, walk.id);
        assert_eq!(header.provenance, walk.provenance);
        let saved = state(&f, document);
        assert_eq!(saved.events, order);
        assert_eq!(
            read_events(saved.clip.as_ref().unwrap()).unwrap(),
            read_events(&written).unwrap()
        );
        assert_eq!(
            saved.facts,
            ClipFacts {
                skeleton_bones: 65,
                driven: 60,
                ..ClipFacts::of(&written)
            }
        );
        assert_eq!(f.status(document), "Saved content/walk.gasset");
        assert_eq!(
            f.written::<SceneSaved>(),
            [SceneSaved {
                document,
                asset: walk.clone(),
            }]
        );
        assert!(f.written::<SaveFailed>().is_empty());
        assert!(!f.unsaved(document));
        assert!(
            f.world
                .get_component_for_entity::<SaveRequested>(document)
                .is_none()
        );
        assert_eq!(f.document(document).current, Some(walk));
    }

    #[test]
    fn a_save_after_the_files_clip_got_shorter_names_the_event_and_stays_unsaved() {
        let mut f = save_fixture();
        let walk = write_clip(&mut f, "content/walk.gasset", &clip_of(2));
        let document = open_clip(&mut f, &walk);
        state_mut(&mut f, document)
            .add_event("footstep", 1.5)
            .unwrap();
        f.world.insert(Unsaved, document);
        write_clip(&mut f, "content/walk.gasset", &clip_of(1));

        save(&mut f, document);

        let message = "Save failed: `footstep` at 1.50 s is past the clip's 1.00 s";
        assert_eq!(f.status(document), message);
        assert_eq!(
            f.written::<SaveFailed>(),
            [SaveFailed {
                document,
                message: message.to_owned(),
            }]
        );
        assert!(f.written::<SceneSaved>().is_empty());
        assert!(f.unsaved(document));
        assert!(
            read_clip(&f.project_root, &walk)
                .unwrap()
                .events()
                .is_empty()
        );
        assert_eq!(state(&f, document).facts.duration, 2.0);
    }

    #[test]
    fn a_typed_event_past_the_end_is_named_by_its_type_name() {
        let typed = TabEvent {
            time: 1.5,
            kind: TabEventKind::Typed {
                type_name: FOOTSTEP.to_owned(),
                value: footstep_value(),
            },
        };

        assert_eq!(
            check_times(&[typed], 1.0),
            Err(format!("`{FOOTSTEP}` at 1.50 s is past the clip's 1.00 s"))
        );
    }

    #[test]
    fn discard_puts_back_the_events_as_last_saved_and_clears_the_selection() {
        let mut f = save_fixture();
        let mut eventful = clip_of(2);
        eventful
            .replace_events(vec![typed_key(1.0), named_key(0.5, "footstep")])
            .unwrap();
        let walk = write_clip(&mut f, "content/walk.gasset", &eventful);
        let document = open_clip(&mut f, &walk);
        state_mut(&mut f, document).add_event("land", 1.5).unwrap();
        f.world.insert(Unsaved, document);
        save(&mut f, document);
        let saved = read_events(&read_clip(&f.project_root, &walk).unwrap()).unwrap();
        let edited = state_mut(&mut f, document);
        edited.rename_event(0, "jump").unwrap();
        edited.add_event("step", 0.25).unwrap();
        f.world.insert(Unsaved, document);

        f.world.remove_component::<Unsaved>(document);
        f.world.trigger(DiscardEdits(document));
        f.frame();

        let discarded = state(&f, document);
        assert_eq!(discarded.events, saved);
        assert_eq!(discarded.selected_event, None);
        assert!(!f.unsaved(document));
    }
}

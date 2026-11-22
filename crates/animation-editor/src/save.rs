//! Saving a clip tab's events into the clip's `.gasset`, and putting them back on a discard.
use std::{path::Path, sync::Arc};

use anyhow::Context;
use concerto_ecs::{
    Entity, Query, Res, ResMut, With, command::CommandQueue, events::event_writer::EventWriter,
    signal::On,
};
use concerto_editor::{
    asset_editor::{EditorDocument, SaveRequested, Unsaved, report_save},
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
    measure_transport,
    preview::{SeekClip, SetClipTransport, ShowClip},
    read_clip,
    transport::ClipTransport,
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
    state.rebuild_rows();
    Ok(saved)
}

fn show_saved_clip(
    entity: Entity,
    state: &ClipDocument,
    transport: &ClipTransport,
    previews: &mut PreviewSignals,
) {
    let Some(clip) = state.clip.as_ref() else {
        return;
    };
    previews.trigger(entity, ShowClip(Arc::clone(clip)));
    previews.trigger(
        entity,
        SetClipTransport {
            playing: transport.playing,
            looping: transport.looping,
            rate: transport.speed,
        },
    );
    previews.trigger(entity, SeekClip(transport.time));
}

pub(crate) fn save_clip(
    mut requested: Query<
        (
            Entity,
            &mut EditorDocument,
            &mut ClipDocument,
            &mut ClipTransport,
        ),
        With<SaveRequested>,
    >,
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
    for (entity, mut document, mut state, mut transport) in requested.iter_mut() {
        let written = root
            .context("Open a project before saving")
            .and_then(|root| save_clip_document(&mut document, &mut state, root, &mut assets));
        if written.is_ok() {
            measure_transport(&mut transport, &state.facts);
            show_saved_clip(entity, &state, &transport, &mut previews);
        }
        report_save(
            entity,
            &mut document,
            written.map(Into::into),
            &mut saved,
            &mut failed,
            &mut cmd,
        );
    }
}

pub(crate) fn restore_clip_events(
    on: On<DiscardEdits>,
    mut documents: Query<(&mut EditorDocument, &mut ClipDocument)>,
    mut cmd: CommandQueue,
) {
    let document = on.signal().0;
    let Some((mut editor, mut state)) = documents.get_entity_mut(document) else {
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
    use crate::tests::{clip_fixture, clip_of, open_clip, state, state_mut, typed_key};
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
            uncooked: None,
            derived: None,
            embedded: Vec::new(),
        };
        let path = f.project_root.join(address);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let payload = bincode::serialize(clip).unwrap();
        std::fs::write(&path, write_content_asset(&header, &payload).unwrap()).unwrap();
        let entry = AssetEntry {
            embedded_in: None,
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
    fn save_writes_the_events_sorted_over_the_clip_with_its_provenance_and_discard_puts_them_back()
    {
        let mut f = save_fixture();
        let mut eventful = clip_of(2);
        eventful.replace_events(vec![typed_key(1.0)]).unwrap();
        let walk = write_imported_clip(&mut f, "content/walk.gasset", &eventful);
        let document = open_clip(&mut f, &walk);
        let edited = state_mut(&mut f, document);
        edited.add_event("land", 1.5).unwrap();
        edited.add_event("footstep", 0.5).unwrap();
        let order = edited.events.clone();
        f.world.insert(Unsaved, document);

        save(&mut f, document);

        let saved = read_events(&read_clip(&f.project_root, &walk).unwrap()).unwrap();
        assert_eq!(
            saved,
            vec![order[2].clone(), order[0].clone(), order[1].clone()]
        );
        let header = read_content_asset_header(&f.project_root.join(&walk.address)).unwrap();
        assert_eq!(header.asset_id, walk.id);
        assert_eq!(header.provenance, walk.provenance);
        assert_eq!(state(&f, document).events, order);
        assert_eq!(
            f.written::<SceneSaved>(),
            [SceneSaved {
                document,
                asset: walk.clone(),
                written: vec![walk.clone()],
            }]
        );
        assert!(!f.unsaved(document));

        let edited = state_mut(&mut f, document);
        edited.rename_event(1, "jump").unwrap();
        edited.add_event("step", 0.25).unwrap();
        f.world.trigger(DiscardEdits(document));
        f.frame();

        assert_eq!(state(&f, document).events, saved);
        assert_eq!(state(&f, document).selected_event, None);
    }
}

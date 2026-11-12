//! The editor's side of a graph tab's preview: the `Preview` and `Parameters` values and what is sent to the preview when they or the graphs change.
use concerto_ecs::{Changed, Component, Entity, Query, Res, World, reflect::AppTypeRegistry};
use concerto_editor::{
    asset_editor::EditorDocument,
    hosted::HostedApp,
    inspector::{PropertyTarget, TargetWorld},
    preview::PreviewSignals,
    project::ProjectState,
};
use concerto_foundation::assets::{AssetId, handle::AssetHandle, registry::AssetRegistry};
use concerto_reflect::{Reflect, ReflectDefault, to_value};
use concerto_scene::scene::Scene;

use super::{
    copy::GraphNodeId,
    document::{GraphDocument, Sources},
    preview::{GraphBytes, SetGraphParam, SetGraphPlayback, ShowGraphs},
};
use crate::{
    preview::{SetSkeletonGizmo, ShowClipScene},
    read_shown_scene,
};

/// How a graph tab's preview shows and plays the graph, the document value `Preview`; never saved.
#[derive(Clone, Debug, PartialEq, Reflect)]
#[reflect(Default)]
pub struct GraphPreview {
    /// The scene the character comes from; unset, the `Scene` named `scene` in the graph's folder.
    pub scene: AssetHandle<Scene>,
    /// Whether the skeleton's bones are drawn.
    pub skeleton: bool,
    pub playing: bool,
    /// The preview's time scale, any finite value above 0.
    pub speed: f32,
    /// The node of the pose graph in view whose subtree plays, or none for the whole graph in view.
    pub from: Option<u32>,
}

impl Default for GraphPreview {
    fn default() -> Self {
        Self {
            scene: AssetHandle::default(),
            skeleton: true,
            playing: true,
            speed: 1.0,
            from: None,
        }
    }
}

fn hosted_types(world: &World, document: Entity) -> Option<AppTypeRegistry> {
    world
        .get_component_for_entity::<HostedApp>(document)
        .and_then(HostedApp::type_registry)
}

/// A graph tab's [`GraphPreview`], shown as the `Preview` value; an edit never marks the tab unsaved, and a speed of 0 or below, or not finite, is refused.
pub struct PreviewValue {
    pub document: Entity,
}

impl PropertyTarget for PreviewValue {
    fn name(&self) -> String {
        "Preview".to_owned()
    }

    fn world(&self) -> TargetWorld {
        TargetWorld::Editor
    }

    fn read(&self, world: &World, read: &mut dyn FnMut(&dyn Reflect)) -> bool {
        let Some(held) = world.get_component_for_entity::<GraphDocument>(self.document) else {
            return false;
        };
        read(&held.preview);
        true
    }

    fn edit(&self, world: &mut World, edit: &mut dyn FnMut(&mut dyn Reflect)) -> bool {
        let Some(held) = world.get_component_for_entity::<GraphDocument>(self.document) else {
            return false;
        };
        let mut preview = held.preview.clone();
        edit(&mut preview);
        if !preview.speed.is_finite() || preview.speed <= 0.0 {
            return false;
        }
        if preview != held.preview
            && let Some(held) = world.get_component_for_entity_mut::<GraphDocument>(self.document)
        {
            held.preview = preview;
        }
        true
    }

    fn types(&self, world: &World) -> Option<AppTypeRegistry> {
        hosted_types(world, self.document)
    }

    fn document(&self) -> Option<Entity> {
        Some(self.document)
    }

    fn saved(&self) -> bool {
        false
    }
}

/// A graph tab's parameters, shown as the `Parameters` value, one field per parameter the graph in view names; an edit never marks the tab unsaved.
pub struct ParametersValue {
    pub document: Entity,
}

impl PropertyTarget for ParametersValue {
    fn name(&self) -> String {
        "Parameters".to_owned()
    }

    fn world(&self) -> TargetWorld {
        TargetWorld::Editor
    }

    fn read(&self, world: &World, read: &mut dyn FnMut(&dyn Reflect)) -> bool {
        let Some(held) = world.get_component_for_entity::<GraphDocument>(self.document) else {
            return false;
        };
        read(&held.parameters);
        true
    }

    fn edit(&self, world: &mut World, edit: &mut dyn FnMut(&mut dyn Reflect)) -> bool {
        let Some(held) = world.get_component_for_entity_mut::<GraphDocument>(self.document) else {
            return false;
        };
        edit(&mut held.parameters);
        true
    }

    fn types(&self, world: &World) -> Option<AppTypeRegistry> {
        hosted_types(world, self.document)
    }

    fn document(&self) -> Option<Entity> {
        Some(self.document)
    }

    fn saved(&self) -> bool {
        false
    }
}

#[derive(Clone, Copy, PartialEq)]
struct SceneKey {
    picked: Option<AssetId>,
    graph: Option<AssetId>,
}

#[derive(Clone, Copy, PartialEq)]
struct Playback {
    playing: bool,
    speed: f32,
}

/// What a graph tab last sent its preview, kept on the document so only a difference is sent.
#[derive(Component)]
pub struct SentPreview {
    own: AssetId,
    graphs: Vec<GraphBytes>,
    play: Option<AssetId>,
    scene: Option<SceneKey>,
    skeleton: Option<bool>,
    playback: Option<Playback>,
    params: Vec<SetGraphParam>,
}

impl Default for SentPreview {
    fn default() -> Self {
        Self {
            own: AssetId::new(),
            graphs: Vec::new(),
            play: None,
            scene: None,
            skeleton: None,
            playback: None,
            params: Vec::new(),
        }
    }
}

struct Played {
    graphs: Vec<GraphBytes>,
    play: AssetId,
}

fn played(state: &GraphDocument, own: AssetId) -> anyhow::Result<Option<Played>> {
    let Some(shown) = state.shown_asset() else {
        return Ok(None);
    };
    let mut graphs = Vec::new();
    for held in state.graphs() {
        graphs.push(GraphBytes::of(held.asset.id, &held.copy.build()?)?);
    }
    let mut play = shown;
    if let (Some(from), Some(copy)) = (state.preview.from, state.graph()) {
        graphs.push(GraphBytes::of(own, &copy.build_from(GraphNodeId(from))?)?);
        play = own;
    }
    Ok(Some(Played { graphs, play }))
}

fn wanted_params(state: &GraphDocument) -> Vec<SetGraphParam> {
    state
        .parameters
        .shown()
        .filter_map(|param| {
            Some(SetGraphParam {
                name: param.name,
                type_name: param.type_name.to_owned(),
                value: to_value(param.value()).ok()?,
            })
        })
        .collect()
}

pub(crate) fn sync_graph_preview(
    documents: Query<
        (
            Entity,
            &mut EditorDocument,
            &mut GraphDocument,
            &mut SentPreview,
            &HostedApp,
        ),
        Changed<GraphDocument>,
    >,
    assets: Res<AssetRegistry>,
    project: Res<ProjectState>,
    mut previews: PreviewSignals,
) {
    let Some(root) = project
        .project
        .as_ref()
        .map(|project| project.root.as_path())
    else {
        return;
    };
    for (document, mut editor, mut state, mut sent, hosted) in documents.iter() {
        if state.shown_asset().is_none() {
            continue;
        }
        let types = hosted.type_registry();
        let refusal = state
            .preview
            .from
            .and_then(|from| state.preview_refusal(GraphNodeId(from)));
        if let Some(reason) = refusal {
            state.preview.from = None;
            editor.status = reason;
        }

        let found = state.found_params(&Sources {
            types: types.as_ref(),
            assets: &assets,
            root: Some(root),
        });
        if !state.parameters.shows(&found) {
            state.parameters.show(&found);
        }

        let picked = Some(state.preview.scene.id()).filter(|id| !id.is_nil());
        let key = SceneKey {
            picked,
            graph: editor.current.as_ref().map(|asset| asset.id),
        };
        if sent.scene != Some(key) {
            sent.scene = Some(key);
            let shown = read_shown_scene(root, editor.current.as_ref(), picked, &assets);
            let error = shown.as_ref().err().map(|error| format!("{error:#}"));
            let shown = shown.ok().flatten();
            let id = shown.as_ref().map(|(id, _)| *id);
            if state.scene_error != error {
                state.scene_error = error;
            }
            if state.shown_scene != id {
                state.shown_scene = id;
            }
            previews.trigger(document, ShowClipScene(shown.map(|(_, scene)| scene)));
        }

        if sent.skeleton != Some(state.preview.skeleton) {
            sent.skeleton = Some(state.preview.skeleton);
            previews.trigger(document, SetSkeletonGizmo(state.preview.skeleton));
        }

        let playback = Playback {
            playing: state.preview.playing,
            speed: state.preview.speed,
        };
        if sent.playback != Some(playback) {
            sent.playback = Some(playback);
            previews.trigger(
                document,
                SetGraphPlayback {
                    playing: playback.playing,
                    speed: playback.speed,
                },
            );
        }

        match played(&state, sent.own) {
            Ok(Some(played)) => {
                if sent.graphs != played.graphs || sent.play != Some(played.play) {
                    sent.graphs = played.graphs.clone();
                    sent.play = Some(played.play);
                    sent.params.clear();
                    previews.trigger(
                        document,
                        ShowGraphs {
                            graphs: played.graphs,
                            play: played.play,
                        },
                    );
                }
            }
            Ok(None) => {}
            Err(error) => {
                let status = format!("The preview cannot play the graph: {error:#}");
                if editor.status != status {
                    editor.status = status;
                }
            }
        }

        let params = wanted_params(&state);
        for param in &params {
            if !sent.params.contains(param) {
                previews.trigger(document, param.clone());
            }
        }
        if sent.params != params {
            sent.params = params;
        }
    }
}

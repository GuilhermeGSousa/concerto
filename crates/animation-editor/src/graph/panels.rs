use concerto_animation::node::{InputSlots, state_machine::AnimationStateMachine};
use concerto_ecs::{
    Added, Component, Entity, Query, Res, ResMut, With,
    command::{CommandQueue, EntityCommandQueue},
    events::event_reader::EventReader,
    signal::{
        On,
        listener::{IntoListener, Listener},
    },
};
use concerto_editor::{
    asset_editor::{EditorDocument, EditorHosts},
    asset_picker::{AssetPickerMenu, asset_name, open_asset_menu},
    graph_canvas::{
        CanvasActivated, CanvasConnected, CanvasDeleted, CanvasEmphasis, CanvasLink,
        CanvasLinkDropped, CanvasMenuRequested, CanvasMode, CanvasNode, CanvasNodeId, CanvasPart,
        CanvasSelected, CanvasTarget, FrameCanvasNode, GraphCanvas,
    },
    hosted::HostedApp,
    inspector::WriteValue,
    name_menu::{NameMenu, close_menu, open_name_menu_at},
};
use concerto_foundation::assets::{
    Asset, AssetId, handle::ReflectAssetHandle, registry::AssetRegistry,
};
use concerto_name::Name;
use concerto_reflect::{PropertyVisitor, Reflect, TypeRegistry};
use concerto_scene::scene::Scene;
use concerto_ui::elements::prelude::*;
use concerto_ui::{
    anchor::UIAnchoredPanel,
    checkbox::{UICheckbox, UICheckboxChanged},
    focus::FocusedWidget,
    interaction::{UIClick, UIInteractionStyle},
    items::UIItemActivated,
    node::{UINode, UIRect},
    text::UIText,
    text_input::{UITextInput, UITextInputSubmitted},
    theme::{ButtonVariant, UITheme},
    transform::UIValue,
};
use concerto_window::input::MouseButton;
use glam::Vec2;
use serde_json::{Value, json};

use super::{
    copy::{GraphCopy, GraphNodeId, HeldNode},
    document::{
        EditGraph, GraphDocument, GraphEdit, GraphLevel, GraphSelection, OpenTarget, node_types,
        shown_transitions,
    },
    preview::MachineView,
    stage::{GraphLive, LiveView, SentPreview},
    tools::param_line,
};
use crate::{
    panels::{NO_VALUE, StageControls, check_row, error_line, stage_controls, title},
    transport::SPEEDS,
};

/// The widgets of a graph's tab that follow its document, kept on the document.
#[derive(Component)]
pub(crate) struct GraphPanels {
    left: LeftPanel,
    breadcrumb: Entity,
    name: Entity,
    target: Entity,
    reset: Entity,
    canvas: Entity,
    status: Entity,
    crumbs: Vec<String>,
    params: Vec<ParamRow>,
    framed: Vec<GraphLevel>,
    selected: Option<GraphSelection>,
}

struct LeftPanel {
    stage: StageControls,
    playing: Entity,
    speeds: [Entity; SPEEDS.len()],
    params: Entity,
}

#[derive(PartialEq)]
struct ParamRow {
    text: String,
    shared: bool,
}

fn write_preview(document: Entity, path: &'static str, json: Value) -> Listener<UIClick> {
    (move |on: On<UIClick>,
           documents: Query<&GraphDocument>,
           mut menus: Query<&mut UIAnchoredPanel, With<NameMenu>>,
           mut cmd: CommandQueue| {
        if on.signal().button == MouseButton::Left {
            send_preview(document, path, &json, &documents, &mut menus, &mut cmd);
        }
    })
    .into_listener()
}

fn pick_preview(document: Entity, path: &'static str, json: Value) -> Listener<UIItemActivated> {
    (move |_: On<UIItemActivated>,
           documents: Query<&GraphDocument>,
           mut menus: Query<&mut UIAnchoredPanel, With<NameMenu>>,
           mut cmd: CommandQueue| {
        send_preview(document, path, &json, &documents, &mut menus, &mut cmd);
    })
    .into_listener()
}

fn send_preview(
    document: Entity,
    path: &str,
    json: &Value,
    documents: &Query<&GraphDocument>,
    menus: &mut Query<&mut UIAnchoredPanel, With<NameMenu>>,
    cmd: &mut CommandQueue,
) {
    close_menu(menus);
    let target = documents
        .get_entity(document)
        .and_then(GraphDocument::preview_value);
    if let Some(target) = target {
        cmd.trigger(WriteValue {
            target,
            path: path.to_owned(),
            json: json.clone(),
        });
    }
}

fn check_preview(document: Entity, path: &'static str) -> Listener<UICheckboxChanged> {
    (move |on: On<UICheckboxChanged>, documents: Query<&GraphDocument>, mut cmd: CommandQueue| {
        let target = documents
            .get_entity(document)
            .and_then(GraphDocument::preview_value);
        if let Some(target) = target {
            cmd.trigger(WriteValue {
                target,
                path: path.to_owned(),
                json: on.signal().checked.into(),
            });
        }
    })
    .into_listener()
}

fn toggle_scene_picker(document: Entity) -> Listener<UIClick> {
    (move |on: On<UIClick>,
           documents: Query<&GraphDocument>,
           mut menus: Query<(Entity, &mut AssetPickerMenu, &mut UIAnchoredPanel)>,
           assets: Res<AssetRegistry>,
           theme: Res<UITheme>,
           mut cmd: CommandQueue| {
        if on.signal().button != MouseButton::Left {
            return;
        }
        let current = documents
            .get_entity(document)
            .and_then(|state| state.shown_scene);
        open_asset_menu(
            on.entity(),
            Scene::name(),
            None,
            current,
            pick_scene(document),
            &mut menus,
            &assets,
            &theme,
            &mut cmd,
        );
    })
    .into_listener()
}

fn pick_scene(document: Entity) -> impl Fn(AssetId) -> Listener<UIItemActivated> {
    move |id| {
        (move |_: On<UIItemActivated>,
               mut menus: Query<&mut UIAnchoredPanel, With<AssetPickerMenu>>,
               documents: Query<&GraphDocument>,
               mut cmd: CommandQueue| {
            for mut panel in menus.iter_mut() {
                panel.open = false;
            }
            let target = documents
                .get_entity(document)
                .and_then(GraphDocument::preview_value);
            if let Some(target) = target {
                cmd.trigger(WriteValue {
                    target,
                    path: "scene".to_owned(),
                    json: id.simple_hex().into(),
                });
            }
        })
        .into_listener()
    }
}

fn build_left(host: &mut EntityCommandQueue, document: Entity, theme: &UITheme) -> LeftPanel {
    let mut panel = host.spawn_child(theme.column().gap(theme.spacing_sm));
    panel.add_child(title(theme, "Preview"));
    let stage = stage_controls(
        &mut panel,
        toggle_scene_picker(document),
        check_preview(document, "skeleton"),
        theme,
    );
    let playing = check_row(
        &mut panel,
        "Play",
        check_preview(document, "playing"),
        theme,
    );
    let speeds = {
        let mut row = panel.spawn_child(theme.row().gap(theme.spacing_xs).fixed());
        SPEEDS.map(|speed| {
            row.spawn_child(theme.button(speed.label).ghost().on_click(write_preview(
                document,
                "speed",
                json!(speed.rate),
            )))
            .entity()
        })
    };
    panel.add_child(title(theme, "Parameters"));
    let params = panel
        .spawn_child(theme.column().gap(theme.spacing_xs).fixed())
        .entity();
    LeftPanel {
        stage,
        playing,
        speeds,
        params,
    }
}

struct FirstHandle<'a> {
    types: &'a TypeRegistry,
    found: Option<AssetId>,
}

impl PropertyVisitor for FirstHandle<'_> {
    fn field(&mut self, _name: &str, value: &dyn Reflect) {
        if self.found.is_some() {
            return;
        }
        let id = std::any::Any::type_id(value);
        self.found = self
            .types
            .get_type_data::<ReflectAssetHandle>(id)
            .and_then(|handle| handle.id(value));
    }

    fn element(&mut self, _index: usize, _value: &dyn Reflect) {}
}

/// The name of the asset the node's first handle field names, when the project has it.
pub(crate) fn subtitle(
    node: &HeldNode,
    types: Option<&TypeRegistry>,
    assets: &AssetRegistry,
) -> Option<String> {
    let mut first = FirstHandle {
        types: types?,
        found: None,
    };
    node.live()?.as_reflect().visit(&mut first);
    let asset = assets.get(first.found?)?;
    Some(asset.name().into_owned())
}

fn canvas_id(id: GraphNodeId) -> CanvasNodeId {
    CanvasNodeId(id.0)
}

fn node_id(id: CanvasNodeId) -> GraphNodeId {
    GraphNodeId(id.0)
}

fn pins(node: &HeldNode) -> Vec<String> {
    let mut pins: Vec<String> = match node.slots() {
        InputSlots::None => Vec::new(),
        InputSlots::Fixed(names) => names.iter().map(|name| name.as_str().to_owned()).collect(),
        InputSlots::Variable => (0..=node.inputs.len())
            .map(|index| index.to_string())
            .collect(),
    };
    for index in pins.len()..node.inputs.len() {
        pins.push(index.to_string());
    }
    pins
}

fn state_id(index: usize) -> CanvasNodeId {
    CanvasNodeId(index as u32)
}

fn shown_diagram(
    machine: &AnimationStateMachine,
    selection: Option<GraphSelection>,
    playing: Option<MachineView>,
    assets: &AssetRegistry,
) -> GraphCanvas {
    let mut canvas = GraphCanvas::new(CanvasMode::Diagram);
    let names: Vec<Name> = machine.states().map(|state| state.name).collect();
    let id_of = |name: Name| names.iter().position(|known| *known == name).map(state_id);
    let initial = machine.initial_state();
    for (index, state) in machine.states().enumerate() {
        let mut shown = CanvasNode::new(state_id(index), state.name.as_str());
        match assets.get(state.graph.id()) {
            Some(asset) => shown = shown.with_subtitle(asset.name()),
            None => {
                shown = shown
                    .with_subtitle("no graph")
                    .with_emphasis(CanvasEmphasis::Warning);
            }
        }
        if initial == Some(state.name) {
            shown = shown.with_badge("initial");
        }
        if playing.is_some_and(|playing| playing.current == Some(state.name)) {
            shown = shown.with_emphasis(CanvasEmphasis::Active);
        }
        canvas.nodes.push(shown);
    }
    let blending = playing.and_then(|playing| playing.transition);
    let transitions = shown_transitions(machine);
    for shown in &transitions {
        if let (Some(from), Some(to)) = (id_of(shown.named.from), id_of(shown.named.to)) {
            let mut link = CanvasLink::new(from, to).with_label(shown.trigger.clone());
            if blending.is_some_and(|blending| {
                blending.from == shown.named.from && blending.to == shown.named.to
            }) {
                link = link.with_emphasis(CanvasEmphasis::Active);
            }
            canvas.links.push(link);
        }
    }
    canvas.selection = match selection {
        Some(GraphSelection::State(name)) => id_of(name).map(CanvasTarget::Node),
        Some(GraphSelection::Transition(named)) => transitions
            .iter()
            .position(|shown| shown.named == named)
            .map(CanvasTarget::Link),
        _ => None,
    };
    canvas
}

/// A weight as a node's badge and the live block show it.
pub(crate) fn weight_text(weight: f32) -> String {
    format!("{weight:.2}")
}

/// A clip node's time as its subtitle and the live block show it.
pub(crate) fn time_text(time: f32) -> String {
    format!("{time:.2}s")
}

fn shown_canvas(
    state: &GraphDocument,
    live: Option<LiveView>,
    types: Option<&TypeRegistry>,
    assets: &AssetRegistry,
) -> GraphCanvas {
    if let Some(machine) = state.machine() {
        let playing = state
            .shown_machine()
            .and_then(|node| live?.node(node)?.machine);
        return shown_diagram(machine, state.selection, playing, assets);
    }
    let mut canvas = GraphCanvas::new(CanvasMode::Flow);
    let Some(copy) = state.graph() else {
        return canvas;
    };
    for node in copy.nodes() {
        let mut shown = CanvasNode::new(canvas_id(node.id), node.short_name());
        let playing = live.and_then(|live| live.node(node.id));
        if let Some(playing) = playing {
            shown = shown.with_badge(weight_text(playing.weight));
        }
        if let Some(subtitle) = subtitle(node, types, assets) {
            shown = shown.with_subtitle(match playing.and_then(|playing| playing.clip_time) {
                Some(time) => format!("{subtitle} · {}", time_text(time)),
                None => subtitle,
            });
        }
        for pin in pins(node) {
            shown = shown.with_input(pin);
        }
        if node.id != copy.result() {
            shown = shown.with_output();
        }
        if copy.editable() && node.live().is_none() {
            shown = shown.with_emphasis(CanvasEmphasis::Warning);
        }
        canvas.nodes.push(shown);
        for (slot, input) in node.inputs.iter().enumerate() {
            canvas
                .links
                .push(CanvasLink::new(canvas_id(*input), canvas_id(node.id)).with_slot(slot));
        }
    }
    canvas.selection = selected(copy, state.selection, &canvas.links);
    canvas
}

fn selected(
    copy: &GraphCopy,
    selection: Option<GraphSelection>,
    links: &[CanvasLink],
) -> Option<CanvasTarget> {
    match selection? {
        GraphSelection::Node(id) => copy
            .node(id)
            .map(|node| CanvasTarget::Node(canvas_id(node.id))),
        GraphSelection::Link { child } => links
            .iter()
            .position(|link| link.from == canvas_id(child))
            .map(CanvasTarget::Link),
        GraphSelection::State(_) | GraphSelection::Transition(_) => None,
    }
}

fn state_at(machine: &AnimationStateMachine, id: CanvasNodeId) -> Option<Name> {
    machine.states().nth(id.0 as usize).map(|state| state.name)
}

fn picked(
    state: &GraphDocument,
    canvas: Option<&GraphCanvas>,
    target: CanvasTarget,
) -> Option<GraphSelection> {
    match (state.machine(), target) {
        (Some(machine), CanvasTarget::Node(id)) => state_at(machine, id).map(GraphSelection::State),
        (Some(machine), CanvasTarget::Link(link)) => shown_transitions(machine)
            .get(link)
            .map(|shown| GraphSelection::Transition(shown.named)),
        (None, CanvasTarget::Node(id)) => Some(GraphSelection::Node(node_id(id))),
        (None, CanvasTarget::Link(link)) => {
            canvas?.links.get(link).map(|link| GraphSelection::Link {
                child: node_id(link.from),
            })
        }
    }
}

fn select_on_canvas(document: Entity) -> Listener<CanvasSelected> {
    (move |on: On<CanvasSelected>,
           documents: Query<&GraphDocument>,
           canvases: Query<&GraphCanvas>,
           mut cmd: CommandQueue| {
        let Some(state) = documents.get_entity(document) else {
            return;
        };
        let selection = match on.signal().target {
            None => None,
            Some(target) => {
                let Some(picked) = picked(state, canvases.get_entity(on.entity()), target) else {
                    return;
                };
                Some(picked)
            }
        };
        cmd.trigger(EditGraph {
            document,
            edit: GraphEdit::Select(selection),
        });
    })
    .into_listener()
}

fn connect_on_canvas(document: Entity) -> Listener<CanvasConnected> {
    (move |on: On<CanvasConnected>, documents: Query<&GraphDocument>, mut cmd: CommandQueue| {
        let CanvasConnected { from, to, slot } = *on.signal();
        let Some(state) = documents.get_entity(document) else {
            return;
        };
        let edit = match state.machine() {
            Some(machine) => {
                let (Some(from), Some(to)) = (state_at(machine, from), state_at(machine, to))
                else {
                    return;
                };
                GraphEdit::AddTransition { from, to }
            }
            None => GraphEdit::Connect {
                child: node_id(from),
                parent: node_id(to),
                slot,
            },
        };
        cmd.trigger(EditGraph { document, edit });
    })
    .into_listener()
}

fn delete_on_canvas(document: Entity) -> Listener<CanvasDeleted> {
    (move |on: On<CanvasDeleted>,
           documents: Query<&GraphDocument>,
           canvases: Query<&GraphCanvas>,
           mut cmd: CommandQueue| {
        let Some(state) = documents.get_entity(document) else {
            return;
        };
        let target = on.signal().target;
        let edit = match picked(state, canvases.get_entity(on.entity()), target) {
            Some(GraphSelection::Node(id)) => GraphEdit::RemoveNode(id),
            Some(GraphSelection::Link { child }) => GraphEdit::RemoveLink { child },
            Some(GraphSelection::State(name)) => GraphEdit::RemoveState(name),
            Some(GraphSelection::Transition(named)) => GraphEdit::RemoveTransition(named),
            None => return,
        };
        cmd.trigger(EditGraph { document, edit });
    })
    .into_listener()
}

fn open_on_canvas(document: Entity) -> Listener<CanvasActivated> {
    (move |on: On<CanvasActivated>, documents: Query<&GraphDocument>, mut cmd: CommandQueue| {
        let Some(state) = documents.get_entity(document) else {
            return;
        };
        let node = on.signal().node;
        let target = match state.machine() {
            Some(machine) => {
                let Some(name) = state_at(machine, node) else {
                    return;
                };
                OpenTarget::State(name)
            }
            None => OpenTarget::Node(node_id(node)),
        };
        cmd.trigger(EditGraph {
            document,
            edit: GraphEdit::Open(target),
        });
    })
    .into_listener()
}

fn run_edit(document: Entity, edit: Option<GraphEdit>) -> Listener<UIItemActivated> {
    (move |_: On<UIItemActivated>,
           mut menus: Query<&mut UIAnchoredPanel, With<NameMenu>>,
           mut cmd: CommandQueue| {
        close_menu(&mut menus);
        if let Some(edit) = &edit {
            cmd.trigger(EditGraph {
                document,
                edit: edit.clone(),
            });
        }
    })
    .into_listener()
}

fn focus_field(field: Entity) -> Listener<UIItemActivated> {
    (move |_: On<UIItemActivated>,
           mut menus: Query<&mut UIAnchoredPanel, With<NameMenu>>,
           mut focused: ResMut<FocusedWidget>| {
        close_menu(&mut menus);
        **focused = Some(field);
    })
    .into_listener()
}

fn go_up(document: Entity, levels: usize) -> Listener<UIClick> {
    (move |on: On<UIClick>, mut cmd: CommandQueue| {
        if on.signal().button == MouseButton::Left {
            cmd.trigger(EditGraph {
                document,
                edit: GraphEdit::Up(levels),
            });
        }
    })
    .into_listener()
}

struct ListedType {
    label: String,
    type_name: String,
}

fn listed_types(types: &TypeRegistry) -> Vec<ListedType> {
    let types = node_types(types);
    types
        .iter()
        .map(|registration| {
            let short = registration.short_name();
            let shared = types
                .iter()
                .filter(|other| other.short_name() == short)
                .count()
                > 1;
            ListedType {
                label: if shared {
                    registration.type_path().to_owned()
                } else {
                    short.to_owned()
                },
                type_name: registration.type_path().to_owned(),
            }
        })
        .collect()
}

fn open_type_menu(
    at: Vec2,
    document: Entity,
    hosted: &HostedApp,
    edit: impl Fn(String) -> GraphEdit,
    menus: &mut Query<(Entity, &mut UIAnchoredPanel), With<NameMenu>>,
    theme: &UITheme,
    cmd: &mut CommandQueue,
) {
    let Some(types) = hosted.type_registry() else {
        return;
    };
    let listed = listed_types(&types.read());
    let labels: Vec<&str> = listed.iter().map(|listed| listed.label.as_str()).collect();
    open_name_menu_at(
        at,
        &labels,
        |label| {
            let picked = listed
                .iter()
                .find(|listed| listed.label == label)
                .map(|listed| edit(listed.type_name.clone()));
            run_edit(document, picked)
        },
        menus,
        theme,
        cmd,
    );
}

const NEW_STATE: &str = "New state";
const TO_SELF: &str = "Transition to self";
const PREVIEW_FROM: &str = "Preview from here";

fn open_canvas_menu(document: Entity, name: Entity) -> Listener<CanvasMenuRequested> {
    (move |on: On<CanvasMenuRequested>,
           documents: Query<(&GraphDocument, &HostedApp)>,
           mut menus: Query<(Entity, &mut UIAnchoredPanel), With<NameMenu>>,
           theme: Res<UITheme>,
           mut cmd: CommandQueue| {
        let Some((state, hosted)) = documents.get_entity(document) else {
            return;
        };
        let CanvasMenuRequested { at, on: part } = *on.signal();
        if let (None, CanvasPart::Node(id)) = (state.machine(), part) {
            let from = json!({ "Some": { "0": id.0 } });
            open_name_menu_at(
                at,
                &[PREVIEW_FROM],
                |_| pick_preview(document, "from", from.clone()),
                &mut menus,
                &theme,
                &mut cmd,
            );
            return;
        }
        if !state.graph().is_some_and(GraphCopy::editable) {
            return;
        }
        match (state.machine(), part) {
            (Some(_), CanvasPart::Empty) => open_name_menu_at(
                at,
                &[NEW_STATE],
                |_| focus_field(name),
                &mut menus,
                &theme,
                &mut cmd,
            ),
            (Some(machine), CanvasPart::Node(id)) => {
                let Some(state) = state_at(machine, id) else {
                    return;
                };
                let edit = GraphEdit::AddTransition {
                    from: state,
                    to: state,
                };
                open_name_menu_at(
                    at,
                    &[TO_SELF],
                    |_| run_edit(document, Some(edit.clone())),
                    &mut menus,
                    &theme,
                    &mut cmd,
                );
            }
            (None, CanvasPart::Empty) => open_type_menu(
                at,
                document,
                hosted,
                |type_name| GraphEdit::AddNode {
                    type_name,
                    parent: None,
                    slot: None,
                },
                &mut menus,
                &theme,
                &mut cmd,
            ),
            _ => {}
        }
    })
    .into_listener()
}

fn add_parent_on_drop(document: Entity) -> Listener<CanvasLinkDropped> {
    (move |on: On<CanvasLinkDropped>,
           documents: Query<(&GraphDocument, &HostedApp)>,
           mut menus: Query<(Entity, &mut UIAnchoredPanel), With<NameMenu>>,
           theme: Res<UITheme>,
           mut cmd: CommandQueue| {
        let Some((state, hosted)) = documents.get_entity(document) else {
            return;
        };
        if state.machine().is_some() || !state.graph().is_some_and(GraphCopy::editable) {
            return;
        }
        let CanvasLinkDropped { from, at } = *on.signal();
        open_type_menu(
            at,
            document,
            hosted,
            |type_name| GraphEdit::AddParent {
                type_name,
                child: node_id(from),
            },
            &mut menus,
            &theme,
            &mut cmd,
        );
    })
    .into_listener()
}

fn build_inspector(host: &mut EntityCommandQueue, theme: &UITheme) -> Entity {
    let mut panel = host.spawn_child(theme.column().gap(theme.spacing_sm));
    panel.add_child(title(theme, "Graph"));
    panel.spawn_child(error_line(theme)).entity()
}

struct BottomPanel {
    breadcrumb: Entity,
    name: Entity,
    target: Entity,
    reset: Entity,
    canvas: Entity,
}

fn build_bottom(host: &mut EntityCommandQueue, document: Entity, theme: &UITheme) -> BottomPanel {
    let mut panel = host.spawn_child(
        theme
            .column()
            .gap(theme.spacing_xs)
            .grow()
            .padding(UIRect::all(theme.spacing_sm)),
    );
    let mut row = panel.spawn_child(theme.row().gap(theme.spacing_sm).fixed());
    let breadcrumb = row
        .spawn_child(theme.row().gap(theme.spacing_xs).fixed())
        .entity();
    let name = row
        .spawn_child(
            theme
                .text_field("New state name")
                .width(UIValue::Px(180.0))
                .hidden(),
        )
        .entity();
    let target = row
        .spawn_child(theme.label("").small().muted().no_wrap().fixed().hidden())
        .entity();
    let reset = row
        .spawn_child(theme.button("Whole graph").ghost().on_click(write_preview(
            document,
            "from",
            json!("None"),
        )))
        .entity();
    let canvas = panel
        .spawn_child((
            UINode::default().with_flex_grow(1.0),
            GraphCanvas::new(CanvasMode::Flow),
            select_on_canvas(document),
            connect_on_canvas(document),
            delete_on_canvas(document),
            open_on_canvas(document),
            open_canvas_menu(document, name),
            add_parent_on_drop(document),
        ))
        .entity();
    BottomPanel {
        breadcrumb,
        name,
        target,
        reset,
        canvas,
    }
}

pub(crate) fn build_graph_panels(
    documents: Query<(Entity, &EditorHosts), (With<GraphDocument>, Added<EditorHosts>)>,
    theme: Res<UITheme>,
    mut cmd: CommandQueue,
) {
    for (document, hosts) in documents.iter() {
        let (Some(left), Some(inspector), Some(bottom)) =
            (hosts.left, hosts.inspector, hosts.bottom)
        else {
            continue;
        };
        let left = build_left(&mut cmd.entity(left), document, &theme);
        let status = build_inspector(&mut cmd.entity(inspector), &theme);
        let bottom = build_bottom(&mut cmd.entity(bottom), document, &theme);
        cmd.insert(
            GraphPanels {
                left,
                breadcrumb: bottom.breadcrumb,
                name: bottom.name,
                target: bottom.target,
                reset: bottom.reset,
                canvas: bottom.canvas,
                status,
                crumbs: Vec::new(),
                params: Vec::new(),
                framed: Vec::new(),
                selected: None,
            },
            document,
        );
    }
}

pub(crate) fn name_new_states(
    mut submitted: EventReader<UITextInputSubmitted>,
    documents: Query<(Entity, &GraphPanels)>,
    mut fields: Query<&mut UITextInput>,
    mut cmd: CommandQueue,
) {
    for event in submitted.read() {
        let Some((document, _)) = documents
            .iter()
            .find(|(_, panels)| panels.name == event.entity)
        else {
            continue;
        };
        cmd.trigger(EditGraph {
            document,
            edit: GraphEdit::AddState {
                name: Name::new(event.value.trim()),
                graph: None,
            },
        });
        if let Some(mut field) = fields.get_entity_mut(event.entity) {
            field.value.clear();
            field.cursor = 0;
            field.selection_anchor = None;
        }
    }
}

fn build_breadcrumb(
    row: &mut EntityCommandQueue,
    document: Entity,
    crumbs: &[String],
    theme: &UITheme,
) {
    for (index, crumb) in crumbs.iter().enumerate() {
        if index > 0 {
            row.add_child(theme.label("›").small().muted().no_wrap().fixed());
        }
        let above = crumbs.len() - 1 - index;
        if above == 0 {
            row.add_child(theme.label(crumb.as_str()).small().no_wrap().fixed());
            continue;
        }
        row.add_child_with(
            theme
                .pressable()
                .radius_sm()
                .fixed()
                .padding(UIRect::axes(0.0, theme.spacing_xs))
                .on_click(go_up(document, above)),
            |mut entry| {
                entry.add_child(
                    theme
                        .label(crumb.as_str())
                        .small()
                        .muted()
                        .no_wrap()
                        .fixed(),
                );
            },
        );
    }
}

pub(crate) fn sync_graph_panels(
    mut documents: Query<(
        Entity,
        &EditorDocument,
        &GraphDocument,
        &GraphLive,
        &SentPreview,
        &mut GraphPanels,
        Option<&HostedApp>,
    )>,
    mut canvases: Query<&mut GraphCanvas>,
    mut texts: Query<&mut UIText>,
    mut nodes: Query<&mut UINode>,
    mut checkboxes: Query<&mut UICheckbox>,
    mut styles: Query<&mut UIInteractionStyle>,
    assets: Res<AssetRegistry>,
    theme: Res<UITheme>,
    mut cmd: CommandQueue,
) {
    let mut show = |entity: Entity, visible: bool| {
        if let Some(mut node) = nodes.get_entity_mut(entity)
            && node.visible != visible
        {
            node.visible = visible;
        }
    };
    let mut write_text = |entity: Entity, value: &str| {
        if let Some(mut text) = texts.get_entity_mut(entity)
            && text.text != value
        {
            text.text = value.to_owned();
        }
    };
    let mut write_checked = |entity: Entity, checked: bool| {
        if let Some(mut checkbox) = checkboxes.get_entity_mut(entity)
            && checkbox.checked != checked
        {
            checkbox.checked = checked;
        }
    };
    let mut write_selected = |entity: Entity, selected: bool| {
        let wanted = theme.interaction(ButtonVariant::Ghost, selected);
        if let Some(mut style) = styles.get_entity_mut(entity)
            && style.normal != wanted.normal
        {
            **style = wanted;
        }
    };
    for (document, editor, state, live, sent, mut panels, hosted) in documents.iter_mut() {
        let types = hosted.and_then(HostedApp::type_registry);
        let types = types.as_ref().map(|types| types.read());
        let live = LiveView::latest(live, sent);
        let wanted = shown_canvas(state, live, types.as_deref(), &assets);
        if let Some(mut canvas) = canvases.get_entity_mut(panels.canvas)
            && **canvas != wanted
        {
            **canvas = wanted;
        }
        let crumbs = state.breadcrumb();
        if panels.crumbs != crumbs {
            let mut row = cmd.entity(panels.breadcrumb).despawn_children();
            build_breadcrumb(&mut row, document, &crumbs, &theme);
            panels.crumbs = crumbs;
        }
        show(panels.name, state.machine().is_some());
        write_text(panels.status, &editor.status);
        show(panels.status, !editor.status.is_empty());

        let preview = &state.preview;
        let scene = state
            .shown_scene
            .map_or_else(|| NO_VALUE.to_owned(), |id| asset_name(&assets, id));
        write_text(panels.left.stage.scene, &scene);
        write_checked(panels.left.stage.skeleton, preview.skeleton);
        let scene_error = state.scene_error.as_deref().unwrap_or("");
        write_text(panels.left.stage.scene_error, scene_error);
        show(panels.left.stage.scene_error, !scene_error.is_empty());
        write_checked(panels.left.playing, preview.playing);
        for (button, speed) in panels.left.speeds.into_iter().zip(SPEEDS) {
            write_selected(button, speed.rate == preview.speed);
        }
        let from = preview.from.map(GraphNodeId).and_then(|from| {
            let node = state.graph()?.node(from)?;
            Some(format!("Previewing node {from}, {}", node.described()))
        });
        write_text(panels.target, from.as_deref().unwrap_or(""));
        show(panels.target, from.is_some());
        show(panels.reset, from.is_some());

        let params: Vec<ParamRow> = state
            .parameters
            .shown()
            .map(|param| ParamRow {
                text: match types.as_deref() {
                    Some(types) => param_line(
                        param.name,
                        param.type_name,
                        param.value(),
                        param.shared,
                        types,
                    ),
                    None => format!("{} · {}", param.name, param.short_type()),
                },
                shared: param.shared,
            })
            .collect();
        if panels.params != params {
            let mut list = cmd.entity(panels.left.params).despawn_children();
            for param in &params {
                let line = theme.label(param.text.as_str()).small().fixed();
                if param.shared {
                    list.add_child(line.color(theme.warning));
                } else {
                    list.add_child(line);
                }
            }
            if params.is_empty() {
                list.add_child(theme.label(NO_VALUE).small().muted().fixed());
            }
            panels.params = params;
        }
        if panels.framed != state.levels() {
            let first = match (state.machine(), state.graph()) {
                (Some(machine), _) => machine.states().next().map(|_| state_id(0)),
                (None, Some(copy)) => Some(canvas_id(copy.result())),
                (None, None) => None,
            };
            if let Some(node) = first {
                panels.framed = state.levels().to_vec();
                cmd.entity(panels.canvas).trigger(FrameCanvasNode { node });
            }
        }
        if panels.selected != state.selection {
            panels.selected = state.selection;
            let node = match (state.machine(), state.selection) {
                (Some(machine), Some(GraphSelection::State(name))) => machine
                    .states()
                    .position(|state| state.name == name)
                    .map(state_id),
                (None, Some(GraphSelection::Node(id))) => Some(canvas_id(id)),
                _ => None,
            };
            if let Some(node) = node {
                cmd.entity(panels.canvas).trigger(FrameCanvasNode { node });
            }
        }
    }
}

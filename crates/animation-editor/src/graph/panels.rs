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
    graph_canvas::{
        CanvasActivated, CanvasConnected, CanvasDeleted, CanvasEmphasis, CanvasLink,
        CanvasLinkDropped, CanvasMenuRequested, CanvasMode, CanvasNode, CanvasNodeId, CanvasPart,
        CanvasSelected, CanvasTarget, FrameCanvasNode, GraphCanvas,
    },
    hosted::HostedApp,
    name_menu::{NameMenu, close_menu, open_name_menu_at},
};
use concerto_foundation::assets::{AssetId, handle::ReflectAssetHandle, registry::AssetRegistry};
use concerto_name::Name;
use concerto_reflect::{PropertyVisitor, Reflect, TypeRegistry};
use concerto_ui::elements::prelude::*;
use concerto_ui::{
    anchor::UIAnchoredPanel,
    focus::FocusedWidget,
    interaction::UIClick,
    node::{UINode, UIRect},
    text::UIText,
    text_input::{UITextInput, UITextInputSubmitted},
    theme::UITheme,
    transform::UIValue,
};
use concerto_window::input::MouseButton;
use glam::Vec2;

use super::{
    copy::{GraphCopy, GraphNodeId, HeldNode},
    document::{
        EditGraph, GraphDocument, GraphEdit, GraphLevel, GraphSelection, OpenTarget, node_types,
        shown_transitions,
    },
};
use crate::panels::{error_line, title};

/// The widgets of a graph's tab that follow its document, kept on the document.
#[derive(Component)]
pub(crate) struct GraphPanels {
    breadcrumb: Entity,
    name: Entity,
    canvas: Entity,
    status: Entity,
    crumbs: Vec<String>,
    framed: Vec<GraphLevel>,
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
    Some(asset.name().to_owned())
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
        canvas.nodes.push(shown);
    }
    let transitions = shown_transitions(machine);
    for shown in &transitions {
        if let (Some(from), Some(to)) = (id_of(shown.named.from), id_of(shown.named.to)) {
            canvas
                .links
                .push(CanvasLink::new(from, to).with_label(shown.trigger.clone()));
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

fn shown_canvas(
    state: &GraphDocument,
    types: Option<&TypeRegistry>,
    assets: &AssetRegistry,
) -> GraphCanvas {
    if let Some(machine) = state.machine() {
        return shown_diagram(machine, state.selection, assets);
    }
    let mut canvas = GraphCanvas::new(CanvasMode::Flow);
    let Some(copy) = state.graph() else {
        return canvas;
    };
    for node in copy.nodes() {
        let mut shown = CanvasNode::new(canvas_id(node.id), node.short_name());
        if let Some(subtitle) = subtitle(node, types, assets) {
            shown = shown.with_subtitle(subtitle);
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

fn run_edit(document: Entity, edit: Option<GraphEdit>) -> Listener<UIClick> {
    (move |on: On<UIClick>,
           menus: Query<&mut UIAnchoredPanel, With<NameMenu>>,
           mut cmd: CommandQueue| {
        if on.signal().button != MouseButton::Left {
            return;
        }
        close_menu(&menus);
        if let Some(edit) = &edit {
            cmd.trigger(EditGraph {
                document,
                edit: edit.clone(),
            });
        }
    })
    .into_listener()
}

fn focus_field(field: Entity) -> Listener<UIClick> {
    (move |on: On<UIClick>,
           menus: Query<&mut UIAnchoredPanel, With<NameMenu>>,
           mut focused: ResMut<FocusedWidget>| {
        if on.signal().button != MouseButton::Left {
            return;
        }
        close_menu(&menus);
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
    menus: &Query<(Entity, &mut UIAnchoredPanel), With<NameMenu>>,
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

fn open_canvas_menu(document: Entity, name: Entity) -> Listener<CanvasMenuRequested> {
    (move |on: On<CanvasMenuRequested>,
           documents: Query<(&GraphDocument, &HostedApp)>,
           menus: Query<(Entity, &mut UIAnchoredPanel), With<NameMenu>>,
           theme: Res<UITheme>,
           mut cmd: CommandQueue| {
        let Some((state, hosted)) = documents.get_entity(document) else {
            return;
        };
        if !state.graph().is_some_and(GraphCopy::editable) {
            return;
        }
        let CanvasMenuRequested { at, on: part } = *on.signal();
        match (state.machine(), part) {
            (Some(_), CanvasPart::Empty) => open_name_menu_at(
                at,
                &[NEW_STATE],
                |_| focus_field(name),
                &menus,
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
                    &menus,
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
                &menus,
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
           menus: Query<(Entity, &mut UIAnchoredPanel), With<NameMenu>>,
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
            &menus,
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
        canvas,
    }
}

pub(crate) fn build_graph_panels(
    documents: Query<(Entity, &EditorHosts), (With<GraphDocument>, Added<EditorHosts>)>,
    theme: Res<UITheme>,
    mut cmd: CommandQueue,
) {
    for (document, hosts) in documents.iter() {
        let (Some(inspector), Some(bottom)) = (hosts.inspector, hosts.bottom) else {
            continue;
        };
        let status = build_inspector(&mut cmd.entity(inspector), &theme);
        let bottom = build_bottom(&mut cmd.entity(bottom), document, &theme);
        cmd.insert(
            GraphPanels {
                breadcrumb: bottom.breadcrumb,
                name: bottom.name,
                canvas: bottom.canvas,
                status,
                crumbs: Vec::new(),
                framed: Vec::new(),
            },
            document,
        );
    }
}

pub(crate) fn name_new_states(
    mut submitted: EventReader<UITextInputSubmitted>,
    documents: Query<(Entity, &GraphPanels)>,
    fields: Query<&mut UITextInput>,
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
        if let Some(mut field) = fields.get_entity(event.entity) {
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
    documents: Query<(
        Entity,
        &EditorDocument,
        &GraphDocument,
        &mut GraphPanels,
        Option<&HostedApp>,
    )>,
    canvases: Query<&mut GraphCanvas>,
    texts: Query<&mut UIText>,
    nodes: Query<&mut UINode>,
    assets: Res<AssetRegistry>,
    theme: Res<UITheme>,
    mut cmd: CommandQueue,
) {
    let show = |entity: Entity, visible: bool| {
        if let Some(mut node) = nodes.get_entity(entity)
            && node.visible != visible
        {
            node.visible = visible;
        }
    };
    for (document, editor, state, mut panels, hosted) in documents.iter() {
        let types = hosted.and_then(HostedApp::type_registry);
        let types = types.as_ref().map(|types| types.read());
        let wanted = shown_canvas(state, types.as_deref(), &assets);
        if let Some(mut canvas) = canvases.get_entity(panels.canvas)
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
        if let Some(mut text) = texts.get_entity(panels.status)
            && text.text != editor.status
        {
            text.text = editor.status.clone();
        }
        show(panels.status, !editor.status.is_empty());
        if panels.framed != state.levels() {
            panels.framed = state.levels().to_vec();
            let first = match (state.machine(), state.graph()) {
                (Some(machine), _) => machine.states().next().map(|_| state_id(0)),
                (None, Some(copy)) => Some(canvas_id(copy.result())),
                (None, None) => None,
            };
            if let Some(node) = first {
                cmd.entity(panels.canvas).trigger(FrameCanvasNode { node });
            }
        }
    }
}

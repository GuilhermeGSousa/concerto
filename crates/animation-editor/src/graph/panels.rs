use concerto_animation::node::InputSlots;
use concerto_ecs::{
    Added, Component, Entity, Query, Res, With,
    command::{CommandQueue, EntityCommandQueue},
    signal::{
        On,
        listener::{IntoListener, Listener},
    },
};
use concerto_editor::{
    asset_editor::{EditorDocument, EditorHosts},
    graph_canvas::{
        CanvasConnected, CanvasDeleted, CanvasEmphasis, CanvasLink, CanvasMenuRequested,
        CanvasMode, CanvasNode, CanvasNodeId, CanvasPart, CanvasSelected, CanvasTarget,
        FrameCanvasNode, GraphCanvas,
    },
    hosted::HostedApp,
    name_menu::{NameMenu, close_menu, open_name_menu_at},
};
use concerto_foundation::assets::{AssetId, handle::ReflectAssetHandle, registry::AssetRegistry};
use concerto_reflect::{PropertyVisitor, Reflect, TypeRegistry};
use concerto_ui::elements::prelude::*;
use concerto_ui::{
    anchor::UIAnchoredPanel,
    interaction::UIClick,
    node::{UINode, UIRect},
    text::UIText,
    theme::UITheme,
};
use concerto_window::input::MouseButton;

use super::{
    copy::{GraphCopy, GraphNodeId, HeldNode},
    document::{EditGraph, GraphDocument, GraphEdit, GraphSelection, node_types},
};
use crate::panels::{error_line, title};

/// The widgets of a graph's tab that follow its document, kept on the document.
#[derive(Component)]
pub(crate) struct GraphPanels {
    breadcrumb: Entity,
    canvas: Entity,
    status: Entity,
    framed: Option<AssetId>,
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

fn shown_canvas(
    state: &GraphDocument,
    types: Option<&TypeRegistry>,
    assets: &AssetRegistry,
) -> GraphCanvas {
    let mut canvas = GraphCanvas::new(CanvasMode::Flow);
    let Some(copy) = &state.graph else {
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
    }
}

fn link_child(canvases: &Query<&GraphCanvas>, canvas: Entity, link: usize) -> Option<GraphNodeId> {
    let canvas = canvases.get_entity(canvas)?;
    canvas.links.get(link).map(|link| node_id(link.from))
}

fn select_on_canvas(document: Entity) -> Listener<CanvasSelected> {
    (move |on: On<CanvasSelected>, canvases: Query<&GraphCanvas>, mut cmd: CommandQueue| {
        let selection = match on.signal().target {
            None => None,
            Some(CanvasTarget::Node(id)) => Some(GraphSelection::Node(node_id(id))),
            Some(CanvasTarget::Link(link)) => {
                let Some(child) = link_child(&canvases, on.entity(), link) else {
                    return;
                };
                Some(GraphSelection::Link { child })
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
    (move |on: On<CanvasConnected>, mut cmd: CommandQueue| {
        let CanvasConnected { from, to, slot } = *on.signal();
        cmd.trigger(EditGraph {
            document,
            edit: GraphEdit::Connect {
                child: node_id(from),
                parent: node_id(to),
                slot,
            },
        });
    })
    .into_listener()
}

fn delete_on_canvas(document: Entity) -> Listener<CanvasDeleted> {
    (move |on: On<CanvasDeleted>, canvases: Query<&GraphCanvas>, mut cmd: CommandQueue| {
        let edit = match on.signal().target {
            CanvasTarget::Node(id) => GraphEdit::RemoveNode(node_id(id)),
            CanvasTarget::Link(link) => {
                let Some(child) = link_child(&canvases, on.entity(), link) else {
                    return;
                };
                GraphEdit::RemoveLink { child }
            }
        };
        cmd.trigger(EditGraph { document, edit });
    })
    .into_listener()
}

fn add_node(document: Entity, type_name: Option<String>) -> Listener<UIClick> {
    (move |on: On<UIClick>,
           menus: Query<&mut UIAnchoredPanel, With<NameMenu>>,
           mut cmd: CommandQueue| {
        if on.signal().button != MouseButton::Left {
            return;
        }
        close_menu(&menus);
        if let Some(type_name) = &type_name {
            cmd.trigger(EditGraph {
                document,
                edit: GraphEdit::AddNode {
                    type_name: type_name.clone(),
                    parent: None,
                    slot: None,
                },
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

fn open_node_menu(document: Entity) -> Listener<CanvasMenuRequested> {
    (move |on: On<CanvasMenuRequested>,
           documents: Query<(&GraphDocument, &HostedApp)>,
           menus: Query<(Entity, &mut UIAnchoredPanel), With<NameMenu>>,
           theme: Res<UITheme>,
           mut cmd: CommandQueue| {
        if on.signal().on != CanvasPart::Empty {
            return;
        }
        let Some((state, hosted)) = documents.get_entity(document) else {
            return;
        };
        if !state.graph.as_ref().is_some_and(GraphCopy::editable) {
            return;
        }
        let Some(types) = hosted.type_registry() else {
            return;
        };
        let listed = listed_types(&types.read());
        let labels: Vec<&str> = listed.iter().map(|listed| listed.label.as_str()).collect();
        open_name_menu_at(
            on.signal().at,
            &labels,
            |label| {
                let type_name = listed
                    .iter()
                    .find(|listed| listed.label == label)
                    .map(|listed| listed.type_name.clone());
                add_node(document, type_name)
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
    let breadcrumb = panel
        .spawn_child(theme.label("").small().muted().no_wrap().fixed())
        .entity();
    let canvas = panel
        .spawn_child((
            UINode::default().with_flex_grow(1.0),
            GraphCanvas::new(CanvasMode::Flow),
            select_on_canvas(document),
            connect_on_canvas(document),
            delete_on_canvas(document),
            open_node_menu(document),
        ))
        .entity();
    BottomPanel { breadcrumb, canvas }
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
                canvas: bottom.canvas,
                status,
                framed: None,
            },
            document,
        );
    }
}

pub(crate) fn sync_graph_panels(
    documents: Query<(
        &EditorDocument,
        &GraphDocument,
        &mut GraphPanels,
        Option<&HostedApp>,
    )>,
    canvases: Query<&mut GraphCanvas>,
    texts: Query<&mut UIText>,
    nodes: Query<&mut UINode>,
    assets: Res<AssetRegistry>,
    mut cmd: CommandQueue,
) {
    let write_text = |entity: Entity, value: &str| {
        if let Some(mut text) = texts.get_entity(entity)
            && text.text != value
        {
            text.text = value.to_owned();
        }
    };
    let write_line = |entity: Entity, value: &str| {
        write_text(entity, value);
        if let Some(mut node) = nodes.get_entity(entity)
            && node.visible == value.is_empty()
        {
            node.visible = !value.is_empty();
        }
    };
    for (editor, state, mut panels, hosted) in documents.iter() {
        let types = hosted.and_then(HostedApp::type_registry);
        let types = types.as_ref().map(|types| types.read());
        let wanted = shown_canvas(state, types.as_deref(), &assets);
        if let Some(mut canvas) = canvases.get_entity(panels.canvas)
            && **canvas != wanted
        {
            **canvas = wanted;
        }
        write_text(panels.breadcrumb, &editor.title);
        write_line(panels.status, &editor.status);
        let shown = editor.current.as_ref().map(|asset| asset.id);
        if let Some(copy) = &state.graph
            && shown.is_some()
            && panels.framed != shown
        {
            panels.framed = shown;
            cmd.entity(panels.canvas).trigger(FrameCanvasNode {
                node: canvas_id(copy.result()),
            });
        }
    }
}

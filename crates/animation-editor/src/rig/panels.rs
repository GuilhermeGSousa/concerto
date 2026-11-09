use std::sync::Arc;

use concerto_animation::retarget::RetargetRig;
use concerto_color::Color;
use concerto_ecs::{
    Added, Component, Entity, Query, Ref, Res, With,
    command::{CommandQueue, EntityCommandQueue},
    query::change_detection::DetectChanges,
    signal::{
        On,
        listener::{IntoListener, Listener},
    },
};
use concerto_editor::{
    asset_editor::{EditorDocument, EditorHosts},
    inspected_asset::InspectedDocument,
    inspector::{InspectedValues, PropertyTarget, WriteValue},
    name_menu::{NameMenu, close_menu, open_name_menu},
    timeline::Timeline,
};
use concerto_mesh::skeleton::{JointName, Skeleton};
use concerto_name::Name;
use concerto_ui::elements::{Label, prelude::*};
use concerto_ui::{
    anchor::UIAnchoredPanel,
    interaction::UIClick,
    items::UIItemContextMenu,
    node::{UINode, UIRect},
    text::UIText,
    theme::UITheme,
    transform::UIValue,
    tree::{UITree, UITreeElement},
};
use concerto_window::input::MouseButton;
use serde_json::{Value, json};

use super::{
    ChainColor, MOTION_ROOT_COLOR, ROOT_COLOR, RigDocument, RigFacts, chain_colors, same_skeleton,
};
use crate::{
    panels::{NO_VALUE, error_line, fact, title},
    transport::{TransportTimeline, scrub_transport, spawn_transport},
};

const NO_SKELETON: &str = "No skeleton";

/// The widgets of a rig's tab that follow its document, kept on the document.
#[derive(Component)]
pub(crate) struct RigPanels {
    bones: BonesPanel,
    inspector: RigInspector,
}

struct BonesPanel {
    tree: Entity,
    empty: Entity,
    shown: Option<Arc<Skeleton>>,
    rows: Vec<Entity>,
}

struct RigInspector {
    scale: Entity,
    chains: Entity,
    driven: Entity,
    status: Entity,
}

struct ChainEnd {
    label: &'static str,
    joint: JointName,
}

#[derive(Clone, Debug, PartialEq)]
struct BoneTag {
    label: String,
    color: Color,
}

/// A row of a rig tab's bone tree: its bone, the tags it shows, and the row they are shown in.
#[derive(Component)]
pub(crate) struct BoneRow {
    bone: usize,
    tags: Vec<BoneTag>,
    tag_row: Entity,
}

#[derive(Clone)]
struct Assignment {
    label: String,
    path: String,
    json: Value,
}

fn open_rig(inspected: &InspectedDocument) -> Option<&RetargetRig> {
    inspected
        .value
        .as_deref()
        .and_then(|value| value.downcast_ref::<RetargetRig>())
}

fn bone_tags(rig: Option<&RetargetRig>, bone: Name, chains: &[ChainColor]) -> Vec<BoneTag> {
    let Some(rig) = rig else {
        return Vec::new();
    };
    let mut tags = Vec::new();
    if rig.root.0 == bone {
        tags.push(BoneTag {
            label: "root".to_owned(),
            color: ROOT_COLOR,
        });
    }
    if rig
        .motion_root
        .is_some_and(|motion_root| motion_root.0 == bone)
    {
        tags.push(BoneTag {
            label: "motion root".to_owned(),
            color: MOTION_ROOT_COLOR,
        });
    }
    for chain in &rig.chains {
        let Some(color) = chains
            .iter()
            .find(|known| known.name == chain.name)
            .map(|known| known.color)
        else {
            continue;
        };
        let ends = [
            ChainEnd {
                label: "start",
                joint: chain.start,
            },
            ChainEnd {
                label: "end",
                joint: chain.end,
            },
        ];
        for end in ends {
            if end.joint.0 == bone {
                tags.push(BoneTag {
                    label: format!("{} {}", chain.name.as_str(), end.label),
                    color,
                });
            }
        }
    }
    tags
}

fn assignments(rig: &RetargetRig, bone: Name) -> Vec<Assignment> {
    let name = bone.as_str();
    let mut listed = vec![
        Assignment {
            label: "Root".to_owned(),
            path: "root".to_owned(),
            json: json!(name),
        },
        Assignment {
            label: "Motion root".to_owned(),
            path: "motion_root".to_owned(),
            json: json!({"Some": {"0": name}}),
        },
    ];
    for (index, chain) in rig.chains.iter().enumerate() {
        for end in ["start", "end"] {
            listed.push(Assignment {
                label: format!("{} {end}", chain.name.as_str()),
                path: format!("chains.{index}.{end}"),
                json: json!(name),
            });
        }
    }
    listed
}

fn assign_bone(
    target: Arc<dyn PropertyTarget>,
    assignment: Option<Assignment>,
) -> Listener<UIClick> {
    (move |on: On<UIClick>,
           menus: Query<&mut UIAnchoredPanel, With<NameMenu>>,
           mut cmd: CommandQueue| {
        if on.signal().button != MouseButton::Left {
            return;
        }
        close_menu(&menus);
        if let Some(assignment) = &assignment {
            cmd.trigger(WriteValue {
                target: Arc::clone(&target),
                path: assignment.path.clone(),
                json: assignment.json.clone(),
            });
        }
    })
    .into_listener()
}

fn open_bone_menu(document: Entity, bone: Name) -> Listener<UIItemContextMenu> {
    (move |on: On<UIItemContextMenu>,
           documents: Query<(&InspectedDocument, &InspectedValues)>,
           menus: Query<(Entity, &mut UIAnchoredPanel), With<NameMenu>>,
           theme: Res<UITheme>,
           mut cmd: CommandQueue| {
        let Some((inspected, values)) = documents.get_entity(document) else {
            return;
        };
        let (Some(rig), Some(target)) = (open_rig(inspected), values.0.first()) else {
            return;
        };
        let listed = assignments(rig, bone);
        let labels: Vec<&str> = listed
            .iter()
            .map(|assignment| assignment.label.as_str())
            .collect();
        open_name_menu(
            on.entity(),
            &labels,
            None,
            |label| {
                let assignment = listed
                    .iter()
                    .find(|assignment| assignment.label == label)
                    .cloned();
                assign_bone(Arc::clone(target), assignment)
            },
            &menus,
            &theme,
            &mut cmd,
        );
    })
    .into_listener()
}

fn tag_label(theme: &UITheme, tag: &BoneTag) -> Label {
    theme
        .label(tag.label.clone())
        .small()
        .color(tag.color)
        .no_wrap()
        .fixed()
}

fn spawn_rows(
    cmd: &mut CommandQueue,
    theme: &UITheme,
    document: Entity,
    tree: Entity,
    skeleton: &Skeleton,
    rig: Option<&RetargetRig>,
) -> Vec<Entity> {
    let chains = rig.map_or_else(Vec::new, |rig| chain_colors(rig, None));
    let rows: Vec<Entity> = skeleton
        .names
        .iter()
        .enumerate()
        .map(|(bone, name)| {
            let tags = bone_tags(rig, *name, &chains);
            let mut tag_row = cmd.spawn(
                theme
                    .row()
                    .gap(theme.spacing_xs)
                    .fixed()
                    .padding(UIRect::axes(0.0, 8.0)),
            );
            for tag in &tags {
                tag_row.add_child(tag_label(theme, tag));
            }
            let tag_row = tag_row.entity();
            let label = cmd
                .spawn((
                    UINode::default()
                        .with_flex_grow(1.0)
                        .with_flex_shrink(1.0)
                        .with_padding(UIRect::axes(0.0, 8.0)),
                    theme.text(name.as_str()).single_line(),
                ))
                .entity();
            let row = cmd
                .spawn((
                    theme.canvas().fill(Color::TRANSPARENT).radius_sm(),
                    UITreeElement { expanded: true },
                    BoneRow {
                        bone,
                        tags,
                        tag_row,
                    },
                    open_bone_menu(document, *name),
                ))
                .entity();
            cmd.add_child(row, label);
            cmd.add_child(row, tag_row);
            row
        })
        .collect();
    for (bone, row) in rows.iter().enumerate() {
        let parent = skeleton.parents[bone]
            .and_then(|parent| rows.get(usize::from(parent)))
            .copied()
            .unwrap_or(tree);
        cmd.add_child(parent, *row);
    }
    rows
}

fn build_bones(host: &mut EntityCommandQueue, theme: &UITheme) -> BonesPanel {
    let mut panel = host.spawn_child(theme.column().gap(theme.spacing_sm).grow());
    panel.add_child(title(theme, "Bones"));
    let empty = panel
        .spawn_child(theme.label(NO_SKELETON).muted().fixed())
        .entity();
    let tree = panel
        .spawn_child((
            UINode::default()
                .with_height(UIValue::Px(0.0))
                .with_flex_grow(1.0),
            UITree::default(),
        ))
        .entity();
    BonesPanel {
        tree,
        empty,
        shown: None,
        rows: Vec::new(),
    }
}

fn build_inspector(
    host: &mut EntityCommandQueue,
    document: Entity,
    theme: &UITheme,
) -> RigInspector {
    let mut panel = host.spawn_child(theme.column().gap(theme.spacing_sm));
    panel.add_child(title(theme, "Retarget"));
    spawn_transport(&mut panel, document, theme);
    RigInspector {
        scale: fact(&mut panel, "Scale", theme),
        chains: fact(&mut panel, "Chains", theme),
        driven: fact(&mut panel, "Driven", theme),
        status: panel.spawn_child(error_line(theme)).entity(),
    }
}

fn build_bottom(host: &mut EntityCommandQueue, document: Entity, theme: &UITheme) {
    host.add_child((
        UINode::default()
            .with_flex_grow(1.0)
            .with_padding(UIRect::all(theme.spacing_sm)),
        Timeline {
            duration: 0.0,
            rate: None,
            time: 0.0,
            selection: None,
            selectable: false,
            markers: Vec::new(),
            selected_marker: None,
            tracks: Vec::new(),
        },
        TransportTimeline { document },
        scrub_transport(document),
    ));
}

pub(crate) fn build_rig_panels(
    documents: Query<(Entity, &EditorHosts), (With<RigDocument>, Added<EditorHosts>)>,
    theme: Res<UITheme>,
    mut cmd: CommandQueue,
) {
    for (document, hosts) in documents.iter() {
        let (Some(left), Some(inspector), Some(bottom)) =
            (hosts.left, hosts.inspector, hosts.bottom)
        else {
            continue;
        };
        build_bottom(&mut cmd.entity(bottom), document, &theme);
        let panels = RigPanels {
            bones: build_bones(&mut cmd.entity(left), &theme),
            inspector: build_inspector(&mut cmd.entity(inspector), document, &theme),
        };
        cmd.insert(panels, document);
    }
}

struct FactLines {
    scale: String,
    chains: String,
    driven: String,
}

fn fact_lines(facts: &RigFacts) -> FactLines {
    FactLines {
        scale: facts
            .scale
            .map_or_else(|| NO_VALUE.to_owned(), |scale| format!("{scale:.2}×")),
        chains: facts.chains.map_or_else(
            || NO_VALUE.to_owned(),
            |chains| format!("{} of {} shared", chains.count, chains.of),
        ),
        driven: facts.driven.map_or_else(
            || NO_VALUE.to_owned(),
            |driven| format!("{} of {}", driven.count, driven.of),
        ),
    }
}

pub(crate) fn sync_rig_panels(
    documents: Query<(
        Entity,
        &EditorDocument,
        Ref<InspectedDocument>,
        &RigDocument,
        &mut RigPanels,
    )>,
    rows: Query<&mut BoneRow>,
    texts: Query<&mut UIText>,
    nodes: Query<&mut UINode>,
    theme: Res<UITheme>,
    mut cmd: CommandQueue,
) {
    let write_text = |entity: Entity, value: &str| {
        if let Some(mut text) = texts.get_entity(entity)
            && text.text != value
        {
            text.text = value.to_owned();
        }
    };
    let write_color = |entity: Entity, color: Color| {
        if let Some(mut text) = texts.get_entity(entity)
            && text.color != color
        {
            text.color = color;
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
    for (document, editor, inspected, state, mut panels) in documents.iter() {
        let rig = open_rig(&inspected);
        if !same_skeleton(panels.bones.shown.as_ref(), state.skeleton.as_ref()) {
            let bones = &mut panels.bones;
            if let Some(shown) = &bones.shown {
                for (bone, row) in bones.rows.iter().enumerate() {
                    if shown.parents.get(bone).is_some_and(Option::is_none) {
                        cmd.despawn(*row);
                    }
                }
            }
            bones.rows = state.skeleton.as_ref().map_or_else(Vec::new, |skeleton| {
                spawn_rows(&mut cmd, &theme, document, bones.tree, skeleton, rig)
            });
            bones.shown = state.skeleton.clone();
        } else if inspected.is_changed() {
            let chains = rig.map_or_else(Vec::new, |rig| chain_colors(rig, None));
            let names = state.skeleton.as_ref().map(|skeleton| &skeleton.names);
            for row in &panels.bones.rows {
                let Some(mut held) = rows.get_entity(*row) else {
                    continue;
                };
                let Some(name) = names.and_then(|names| names.get(held.bone)) else {
                    continue;
                };
                let tags = bone_tags(rig, *name, &chains);
                if held.tags != tags {
                    let mut shown = cmd.entity(held.tag_row).despawn_children();
                    for tag in &tags {
                        shown.add_child(tag_label(&theme, tag));
                    }
                    held.tags = tags;
                }
            }
        }

        let bones = &panels.bones;
        let (empty, color) = match (&state.skeleton, &state.skeleton_error) {
            (Some(_), _) => ("", theme.text_muted),
            (None, Some(error)) => (error.as_str(), theme.error),
            (None, None) => (NO_SKELETON, theme.text_muted),
        };
        write_line(bones.empty, empty);
        write_color(bones.empty, color);

        let inspector = &panels.inspector;
        let facts = fact_lines(&state.facts);
        write_text(inspector.scale, &facts.scale);
        write_text(inspector.chains, &facts.chains);
        write_text(inspector.driven, &facts.driven);
        write_line(inspector.status, &editor.status);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        rig::tests::{PELVIS, RigTab, left_click, rig_tab, write},
        transport::{TransportRow, sync_transport_rows, sync_transport_timelines},
    };
    use concerto_animation::retarget::RetargetChain;
    use concerto_ecs::{
        IntoSystem, World,
        entity::hierarchy::{ChildOf, Children},
    };
    use concerto_editor::hosted::Hosted;
    use concerto_editor::tools::edits::PropertyCommitted;
    use concerto_foundation::assets::handle::AssetHandle;
    use concerto_mesh::skeleton::JointPose;
    use concerto_ui::interaction::{UIDisabled, UIInteractionStyle};
    use glam::{Mat4, Vec2};
    use uuid::Uuid;

    fn paneled(mut tab: RigTab) -> RigTab {
        tab.f.add_delivery(build_rig_panels.into_boxed_system());
        tab.f.add_delivery(sync_rig_panels.into_boxed_system());
        tab.f.add_delivery(sync_transport_rows.into_boxed_system());
        tab.f
            .add_delivery(sync_transport_timelines.into_boxed_system());
        let [left, inspector, bottom] = [(); 3].map(|_| tab.f.world.spawn(UINode::default()));
        tab.f.world.insert(
            EditorHosts {
                canvas: None,
                left: Some(left),
                inspector: Some(inspector),
                bottom: Some(bottom),
            },
            tab.document,
        );
        tab.f.frame();
        tab.f.frame();
        tab
    }

    fn panels(tab: &RigTab) -> &RigPanels {
        tab.f
            .world
            .get_component_for_entity::<RigPanels>(tab.document)
            .expect("the panels were built")
    }

    fn rows(tab: &RigTab) -> Vec<Entity> {
        panels(tab).bones.rows.clone()
    }

    fn parent(world: &World, entity: Entity) -> Option<Entity> {
        world
            .get_component_for_entity::<ChildOf>(entity)
            .map(ChildOf::parent)
    }

    fn children(world: &World, entity: Entity) -> Vec<Entity> {
        world
            .get_component_for_entity::<Children>(entity)
            .map_or_else(Vec::new, |children| children.iter().copied().collect())
    }

    fn text(world: &World, entity: Entity) -> String {
        world
            .get_component_for_entity::<UIText>(entity)
            .map_or_else(String::new, |text| text.text.clone())
    }

    fn shown(tab: &RigTab, entity: Entity) -> bool {
        tab.f
            .world
            .get_component_for_entity::<UINode>(entity)
            .unwrap()
            .visible
    }

    fn row_name(tab: &RigTab, row: Entity) -> String {
        text(&tab.f.world, children(&tab.f.world, row)[0])
    }

    fn shown_tags(tab: &RigTab, bone: usize) -> Vec<String> {
        let world = &tab.f.world;
        let held = world
            .get_component_for_entity::<BoneRow>(rows(tab)[bone])
            .unwrap();
        children(world, held.tag_row)
            .into_iter()
            .map(|tag| text(world, tag))
            .collect()
    }

    fn tag_colors(tab: &RigTab, bone: usize) -> Vec<Color> {
        let world = &tab.f.world;
        let held = world
            .get_component_for_entity::<BoneRow>(rows(tab)[bone])
            .unwrap();
        children(world, held.tag_row)
            .into_iter()
            .map(|tag| world.get_component_for_entity::<UIText>(tag).unwrap().color)
            .collect()
    }

    fn menu(tab: &mut RigTab) -> Option<Entity> {
        let world = &mut tab.f.world;
        world
            .query::<(Entity, &UIAnchoredPanel), With<NameMenu>>()
            .iter(world)
            .find(|(_, panel)| panel.open)
            .map(|(entity, _)| entity)
    }

    fn texts_under(world: &World, entity: Entity, found: &mut Vec<Entity>) {
        for child in children(world, entity) {
            if world.get_component_for_entity::<UIText>(child).is_some() {
                found.push(child);
            }
            texts_under(world, child, found);
        }
    }

    fn menu_texts(tab: &mut RigTab) -> Vec<Entity> {
        let menu = menu(tab).expect("the bone menu is open");
        let mut found = Vec::new();
        texts_under(&tab.f.world, menu, &mut found);
        found
    }

    fn open_menu(tab: &mut RigTab, bone: usize) {
        let row = rows(tab)[bone];
        tab.f.world.trigger_on(
            row,
            UIItemContextMenu {
                position: Vec2::ZERO,
            },
        );
    }

    fn menu_labels(tab: &mut RigTab) -> Vec<String> {
        menu_texts(tab)
            .into_iter()
            .map(|label| text(&tab.f.world, label))
            .collect()
    }

    fn pick(tab: &mut RigTab, label: &str) {
        let item = menu_texts(tab)
            .into_iter()
            .find(|entity| text(&tab.f.world, *entity) == label)
            .and_then(|entity| parent(&tab.f.world, entity))
            .expect("the menu lists the label");
        tab.f.world.trigger_on(item, left_click());
        tab.f.frame();
    }

    fn committed_rig(tab: &RigTab) -> Vec<PropertyCommitted> {
        tab.f
            .written::<Hosted<PropertyCommitted>>()
            .into_iter()
            .map(|hosted| hosted.event)
            .filter(|event| event.address.target.name() == "RetargetRig")
            .collect()
    }

    fn reopen(tab: &mut RigTab, address: &str, rig: &RetargetRig) {
        let entry = write(&mut tab.f, address, rig);
        tab.f.request(&entry);
        tab.until("the rig never opened", |tab| {
            tab.f
                .document(tab.document)
                .current
                .as_ref()
                .is_some_and(|current| current.id == entry.id)
        });
        tab.f.frame();
    }

    fn two_bones() -> Skeleton {
        Skeleton::new(
            vec![Mat4::IDENTITY; 2],
            vec![None, Some(0)],
            vec!["hips".into(), "chest".into()],
            vec![JointPose::IDENTITY; 2],
            vec![Uuid::from_u128(40), Uuid::from_u128(41)],
        )
        .unwrap()
    }

    #[test]
    fn the_bone_rows_nest_the_skeleton_expanded_and_their_tags_follow_the_rig() {
        let mut tab = paneled(rig_tab());
        let tree = panels(&tab).bones.tree;
        let built = rows(&tab);

        let names: Vec<String> = built.iter().map(|row| row_name(&tab, *row)).collect();
        assert_eq!(names, ["root", "pelvis", "spine", "head"]);
        assert_eq!(parent(&tab.f.world, built[0]), Some(tree));
        for bone in 1..built.len() {
            assert_eq!(parent(&tab.f.world, built[bone]), Some(built[bone - 1]));
        }
        assert!(built.iter().all(|row| {
            tab.f
                .world
                .get_component_for_entity::<UITreeElement>(*row)
                .unwrap()
                .expanded
        }));
        assert!(!shown(&tab, panels(&tab).bones.empty));
        assert!(shown_tags(&tab, 0).is_empty());
        assert_eq!(shown_tags(&tab, PELVIS), ["root"]);
        assert_eq!(shown_tags(&tab, 2), ["spine start"]);
        assert_eq!(shown_tags(&tab, 3), ["spine end"]);
        assert_eq!(tag_colors(&tab, PELVIS), [ROOT_COLOR]);
        let rig = RetargetRig::new().with_chain(RetargetChain::new("spine", "spine", "head"));
        assert_eq!(tag_colors(&tab, 2), [chain_colors(&rig, None)[0].color]);

        tab.write_rig("root", json!("spine"));
        tab.write_rig("motion_root", json!({"Some": {"0": "root"}}));
        tab.f.frame();

        assert_eq!(rows(&tab), built);
        assert_eq!(shown_tags(&tab, 0), ["motion root"]);
        assert!(shown_tags(&tab, PELVIS).is_empty());
        assert_eq!(shown_tags(&tab, 2), ["root", "spine start"]);
    }

    #[test]
    fn the_bone_rows_follow_another_rigs_skeleton_and_say_when_there_is_none() {
        let mut tab = paneled(rig_tab());
        let hips = write(&mut tab.f, "content/third/skeleton.gasset", &two_bones());

        reopen(
            &mut tab,
            "content/third/rig.gasset",
            &RetargetRig::new().with_skeleton(AssetHandle::weak(hips.id)),
        );

        let names: Vec<String> = rows(&tab).iter().map(|row| row_name(&tab, *row)).collect();
        assert_eq!(names, ["hips", "chest"]);

        reopen(&mut tab, "content/third/bare.gasset", &RetargetRig::new());

        assert!(rows(&tab).is_empty());
        let empty = panels(&tab).bones.empty;
        assert!(shown(&tab, empty));
        assert_eq!(text(&tab.f.world, empty), "No skeleton");
    }

    #[test]
    fn a_skeleton_that_cannot_be_read_says_why_over_the_tree() {
        let mut tab = paneled(rig_tab());
        let broken = write(&mut tab.f, "content/third/skeleton.gasset", &two_bones());
        std::fs::write(tab.f.project_root.join(&broken.address), b"not a skeleton").unwrap();

        reopen(
            &mut tab,
            "content/third/rig.gasset",
            &RetargetRig::new().with_skeleton(AssetHandle::weak(broken.id)),
        );

        assert!(rows(&tab).is_empty());
        let empty = panels(&tab).bones.empty;
        let reason = tab.state().skeleton_error.clone().expect("the read failed");
        assert!(shown(&tab, empty));
        assert_eq!(text(&tab.f.world, empty), reason);
    }

    #[test]
    fn the_bone_menu_lists_the_assignments_and_a_chain_pick_writes_its_start() {
        let mut tab = paneled(rig_tab());

        open_menu(&mut tab, PELVIS);
        assert_eq!(
            menu_labels(&mut tab),
            ["Root", "Motion root", "spine start", "spine end"]
        );
        pick(&mut tab, "spine start");

        assert_eq!(tab.rig().chains[0].start, "pelvis".into());
        assert!(committed_rig(&tab).iter().all(|event| event.result.is_ok()));
        assert!(tab.f.unsaved(tab.document));
        assert!(menu(&mut tab).is_none());
        assert_eq!(shown_tags(&tab, PELVIS), ["root", "spine start"]);
    }

    #[test]
    fn motion_root_writes_the_motion_root_when_it_was_none() {
        let mut tab = paneled(rig_tab());
        assert_eq!(tab.rig().motion_root, None);

        open_menu(&mut tab, 0);
        pick(&mut tab, "Motion root");

        assert_eq!(tab.rig().motion_root, Some("root".into()));
        assert!(tab.f.unsaved(tab.document));
    }

    fn disabled(tab: &RigTab, entity: Entity) -> bool {
        tab.f
            .world
            .get_component_for_entity::<UIDisabled>(entity)
            .is_some()
    }

    fn transport_row(tab: &mut RigTab) -> TransportRow {
        let document = tab.document;
        let world = &mut tab.f.world;
        *world
            .query::<&TransportRow, ()>()
            .iter(world)
            .find(|row| row.document == document)
            .expect("the transport was spawned")
    }

    fn buttons(row: TransportRow) -> Vec<Entity> {
        let mut buttons = vec![row.step_back, row.play, row.step_forward, row.looping];
        buttons.extend(row.speeds);
        buttons
    }

    #[test]
    fn the_transport_is_disabled_in_reference_mode() {
        let mut tab = paneled(rig_tab().with_other().playing_the_walk());
        let row = transport_row(&mut tab);
        tab.f.frame();
        assert!(buttons(row).iter().all(|button| !disabled(&tab, *button)));

        tab.write_preview("mode", json!("Reference"));
        tab.f.frame();

        assert!(buttons(row).iter().all(|button| disabled(&tab, *button)));
    }

    fn selected(tab: &RigTab, entity: Entity) -> bool {
        tab.f
            .world
            .get_component_for_entity::<UIInteractionStyle>(entity)
            .unwrap()
            .normal
            == UITheme::default().selection()
    }

    #[test]
    fn a_speed_of_one_and_a_half_is_accepted_and_selects_no_rate_button() {
        let mut tab = paneled(rig_tab());
        let speeds = transport_row(&mut tab).speeds;
        assert!(selected(&tab, speeds[2]));

        tab.write_preview("speed", json!(1.5));
        tab.until("the speed never reached the transport", |tab| {
            tab.transport().speed == 1.5
        });
        tab.f.frame();

        assert_eq!(tab.state().preview.speed, 1.5);
        assert!(speeds.iter().all(|speed| !selected(&tab, *speed)));
    }

    #[test]
    fn the_inspector_shows_the_facts_and_the_status() {
        let tab = paneled(rig_tab());
        let inspector = &panels(&tab).inspector;
        assert_eq!(text(&tab.f.world, inspector.scale), "None");
        assert!(shown(&tab, inspector.status));
        assert_eq!(text(&tab.f.world, inspector.status), "Pick the other rig");

        let mut tab = tab.with_other();
        tab.f.frame();

        let inspector = &panels(&tab).inspector;
        assert_eq!(text(&tab.f.world, inspector.scale), "0.90×");
        assert_eq!(text(&tab.f.world, inspector.chains), "1 of 1 shared");
        assert_eq!(text(&tab.f.world, inspector.driven), "None");
        assert!(!shown(&tab, inspector.status));
    }
}

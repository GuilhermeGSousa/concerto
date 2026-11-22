//! What a clip's tab shows in its `left`, `inspector` and `bottom` hosts, kept in step with its document.
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
    asset_picker::{
        AssetPickerMenu, asset_name, open_asset_menu, picker_button, picker_caret, picker_label,
    },
    fonts::MEDIUM,
    preview::PreviewSignals,
    project::ProjectState,
    timeline::{Timeline, TimelineMarker, TimelineMarkerMoved, TimelineMarkerSelected},
};
use concerto_foundation::assets::{Asset, AssetId, registry::AssetRegistry};
use concerto_scene::scene::Scene;
use concerto_ui::elements::{Label, prelude::*};
use concerto_ui::{
    anchor::UIAnchoredPanel,
    checkbox::{UICheckbox, UICheckboxChanged},
    interaction::UIClick,
    node::{UINode, UIRect},
    text::UIText,
    theme::UITheme,
    transform::UIValue,
};
use concerto_window::input::MouseButton;

use super::{
    ClipDocument,
    events::TabEvent,
    preview::{SetRootMotion, SetSkeletonGizmo},
    scene_signal, settle_edit,
    transport::{TransportTimeline, scrub_transport, spawn_transport},
};

mod event_section;

pub(crate) use event_section::{
    apply_clip_event_fields, bind_clip_keys, remove_clip_event, sync_clip_event_context,
    sync_clip_events,
};

const TITLE_HEIGHT: f32 = 16.0;
const BOX_SIZE: f32 = 16.0;
pub(crate) const NO_VALUE: &str = "None";

/// The widgets of a clip's tab that follow its document, kept on the document.
#[derive(Component)]
pub(crate) struct ClipPanels {
    left: StageControls,
    inspector: InspectorPanel,
    timeline: Entity,
}

/// The scene picker's label, the Skeleton checkbox and the scene's error line, which every tab with a preview stage has.
pub(crate) struct StageControls {
    pub(crate) scene: Entity,
    pub(crate) skeleton: Entity,
    pub(crate) scene_error: Entity,
}

struct InspectorPanel {
    root_motion: Entity,
    duration: Entity,
    frames: Entity,
    rate: Entity,
    clip_bones: Entity,
    driven: Entity,
    events: event_section::EventsPanel,
    status: Entity,
}

fn toggle_root_motion(document: Entity) -> Listener<UICheckboxChanged> {
    (move |on: On<UICheckboxChanged>,
           mut documents: Query<&mut ClipDocument>,
           mut previews: PreviewSignals| {
        if let Some(mut state) = documents.get_entity_mut(document) {
            state.root_motion = on.signal().checked;
            previews.trigger(document, SetRootMotion(state.root_motion));
        }
    })
    .into_listener()
}

fn toggle_skeleton(document: Entity) -> Listener<UICheckboxChanged> {
    (move |on: On<UICheckboxChanged>,
           mut documents: Query<&mut ClipDocument>,
           mut previews: PreviewSignals| {
        if let Some(mut state) = documents.get_entity_mut(document) {
            state.skeleton_gizmo = on.signal().checked;
            previews.trigger(document, SetSkeletonGizmo(state.skeleton_gizmo));
        }
    })
    .into_listener()
}

fn select_event(document: Entity) -> Listener<TimelineMarkerSelected> {
    (move |on: On<TimelineMarkerSelected>, mut documents: Query<&mut ClipDocument>| {
        let Some(mut state) = documents.get_entity_mut(document) else {
            return;
        };
        let index = on
            .signal()
            .index
            .filter(|index| *index < state.events.len());
        if state.selected_event != index {
            state.selected_event = index;
        }
    })
    .into_listener()
}

fn move_event(document: Entity) -> Listener<TimelineMarkerMoved> {
    (move |on: On<TimelineMarkerMoved>,
           mut documents: Query<(&mut EditorDocument, &mut ClipDocument)>,
           mut cmd: CommandQueue| {
        let index = on.signal().index;
        let time = on.signal().time;
        let Some((mut editor, mut state)) = documents.get_entity_mut(document) else {
            return;
        };
        if state
            .events
            .get(index)
            .is_some_and(|event| event.time == time)
        {
            return;
        }
        let result = state.move_event(index, time);
        settle_edit(result, document, &mut editor, &mut cmd);
    })
    .into_listener()
}

fn toggle_scene_picker(document: Entity) -> Listener<UIClick> {
    (move |on: On<UIClick>,
           documents: Query<&ClipDocument>,
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

fn pick_scene(document: Entity) -> impl Fn(AssetId) -> Listener<UIClick> {
    move |id| {
        (move |on: On<UIClick>,
               mut menus: Query<&mut UIAnchoredPanel, With<AssetPickerMenu>>,
               mut documents: Query<(&EditorDocument, &mut ClipDocument)>,
               project: Res<ProjectState>,
               assets: Res<AssetRegistry>,
               mut previews: PreviewSignals| {
            if on.signal().button != MouseButton::Left {
                return;
            }
            for mut panel in menus.iter_mut() {
                panel.open = false;
            }
            let Some((editor, mut state)) = documents.get_entity_mut(document) else {
                return;
            };
            state.picked_scene = Some(id);
            if let Some(project) = &project.project {
                let clip = editor.current.as_ref();
                let scene = scene_signal(&mut state, &project.root, clip, &assets);
                previews.trigger(document, scene);
            }
        })
        .into_listener()
    }
}

pub(crate) fn title(theme: &UITheme, text: &str) -> Label {
    theme
        .label(text.to_uppercase())
        .small()
        .muted()
        .weight(MEDIUM)
        .height(UIValue::Px(TITLE_HEIGHT))
        .fixed()
}

pub(crate) fn error_line(theme: &UITheme) -> Label {
    theme.label("").color(theme.error).hidden().fixed()
}

pub(crate) fn check_row(
    parent: &mut EntityCommandQueue,
    label: &str,
    change: Listener<UICheckboxChanged>,
    theme: &UITheme,
) -> Entity {
    let mut row = parent.spawn_child(
        theme
            .row()
            .gap(theme.spacing_sm)
            .height(UIValue::Px(theme.control_height))
            .fixed(),
    );
    let checkbox = row
        .spawn_child(
            theme
                .checkbox("", false)
                .size(UIValue::Px(BOX_SIZE), UIValue::Px(BOX_SIZE))
                .padding(0.0)
                .radius_sm()
                .fixed()
                .on_change(change),
        )
        .entity();
    row.add_child(theme.label(label));
    checkbox
}

pub(crate) fn fact(parent: &mut EntityCommandQueue, label: &str, theme: &UITheme) -> Entity {
    let mut row = parent.spawn_child(theme.row().gap(theme.spacing_sm).fixed());
    row.add_child(theme.label(label).muted().grow());
    row.spawn_child(theme.label("").no_wrap()).entity()
}

pub(crate) fn stage_controls(
    panel: &mut EntityCommandQueue,
    pick_scene: Listener<UIClick>,
    toggle_skeleton: Listener<UICheckboxChanged>,
    theme: &UITheme,
) -> StageControls {
    let mut row = panel.spawn_child(theme.row().fixed());
    let mut button = row.spawn_child(picker_button(theme).on_click(pick_scene));
    let scene = button.spawn_child(picker_label(theme)).entity();
    button.add_child(picker_caret(theme));
    StageControls {
        scene,
        skeleton: check_row(panel, "Skeleton", toggle_skeleton, theme),
        scene_error: panel.spawn_child(error_line(theme)).entity(),
    }
}

fn build_left(host: &mut EntityCommandQueue, document: Entity, theme: &UITheme) -> StageControls {
    let mut panel = host.spawn_child(theme.column().gap(theme.spacing_sm));
    panel.add_child(title(theme, "Preview"));
    stage_controls(
        &mut panel,
        toggle_scene_picker(document),
        toggle_skeleton(document),
        theme,
    )
}

fn build_inspector(
    host: &mut EntityCommandQueue,
    document: Entity,
    theme: &UITheme,
) -> InspectorPanel {
    let mut panel = host.spawn_child(theme.column().gap(theme.spacing_sm));
    panel.add_child(title(theme, "Clip"));
    spawn_transport(&mut panel, document, theme);
    let root_motion = check_row(
        &mut panel,
        "Root motion",
        toggle_root_motion(document),
        theme,
    );
    InspectorPanel {
        root_motion,
        duration: fact(&mut panel, "Duration", theme),
        frames: fact(&mut panel, "Frames", theme),
        rate: fact(&mut panel, "Rate", theme),
        clip_bones: fact(&mut panel, "Clip bones", theme),
        driven: fact(&mut panel, "Driven", theme),
        events: event_section::build_events(&mut panel, document, theme),
        status: panel.spawn_child(error_line(theme)).entity(),
    }
}

fn build_bottom(host: &mut EntityCommandQueue, document: Entity, theme: &UITheme) -> Entity {
    host.spawn_child((
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
        select_event(document),
        move_event(document),
    ))
    .entity()
}

pub(crate) fn build_clip_panels(
    documents: Query<(Entity, &EditorHosts), (With<ClipDocument>, Added<EditorHosts>)>,
    theme: Res<UITheme>,
    mut cmd: CommandQueue,
) {
    for (document, hosts) in documents.iter() {
        let (Some(left), Some(inspector), Some(bottom)) =
            (hosts.left, hosts.inspector, hosts.bottom)
        else {
            continue;
        };
        let panels = ClipPanels {
            left: build_left(&mut cmd.entity(left), document, &theme),
            inspector: build_inspector(&mut cmd.entity(inspector), document, &theme),
            timeline: build_bottom(&mut cmd.entity(bottom), document, &theme),
        };
        cmd.insert(panels, document);
    }
}

fn marker_of(event: &TabEvent) -> TimelineMarker {
    TimelineMarker {
        time: event.time,
        label: event.label().to_owned(),
        locked: event.is_typed(),
    }
}

fn shows_events(markers: &[TimelineMarker], events: &[TabEvent]) -> bool {
    markers.len() == events.len()
        && markers.iter().zip(events).all(|(marker, event)| {
            marker.time == event.time
                && marker.label == event.label()
                && marker.locked == event.is_typed()
        })
}

fn rate_fact(rate: f32, stored: bool) -> String {
    let number = if rate.fract() == 0.0 {
        format!("{rate:.0}")
    } else {
        format!("{rate:.2}")
    };
    if stored {
        format!("{number} fps")
    } else {
        format!("{number} fps, detected")
    }
}

pub(crate) fn write_text(texts: &mut Query<&mut UIText>, entity: Entity, value: &str) {
    if let Some(mut text) = texts.get_entity_mut(entity)
        && text.text != value
    {
        text.text = value.to_owned();
    }
}

pub(crate) fn write_line(
    texts: &mut Query<&mut UIText>,
    nodes: &mut Query<&mut UINode>,
    entity: Entity,
    value: &str,
) {
    write_text(texts, entity, value);
    if let Some(mut node) = nodes.get_entity_mut(entity)
        && node.visible == value.is_empty()
    {
        node.visible = !value.is_empty();
    }
}

pub(crate) fn sync_clip_panels(
    documents: Query<(&EditorDocument, &ClipDocument, &ClipPanels)>,
    mut timelines: Query<&mut Timeline>,
    mut texts: Query<&mut UIText>,
    mut nodes: Query<&mut UINode>,
    mut checkboxes: Query<&mut UICheckbox>,
    assets: Res<AssetRegistry>,
    theme: Res<UITheme>,
) {
    let mut write_checked = |entity: Entity, checked: bool| {
        if let Some(mut checkbox) = checkboxes.get_entity_mut(entity)
            && checkbox.checked != checked
        {
            checkbox.checked = checked;
        }
    };
    for (document, state, panels) in documents.iter() {
        let facts = &state.facts;
        let left = &panels.left;
        let inspector = &panels.inspector;

        if let Some(mut timeline) = timelines.get_entity_mut(panels.timeline) {
            if timeline.selection.is_some() {
                timeline.selection = None;
            }
            if timeline.selectable {
                timeline.selectable = false;
            }
            if timeline.selected_marker != state.selected_event {
                timeline.selected_marker = state.selected_event;
            }
            if !shows_events(&timeline.markers, &state.events) {
                timeline.markers = state.events.iter().map(marker_of).collect();
            }
            if timeline.tracks != state.rows {
                timeline.tracks = state.rows.clone();
            }
        }

        let scene = state
            .shown_scene
            .map_or_else(|| NO_VALUE.to_owned(), |id| asset_name(&assets, id));
        write_text(&mut texts, left.scene, &scene);
        write_checked(left.skeleton, state.skeleton_gizmo);
        write_line(
            &mut texts,
            &mut nodes,
            left.scene_error,
            state.scene_error.as_deref().unwrap_or(""),
        );

        write_checked(inspector.root_motion, state.root_motion);

        write_text(
            &mut texts,
            inspector.duration,
            &format!("{:.2} s", facts.duration),
        );
        let frames = facts.frames.map(|frames| frames.to_string());
        write_text(
            &mut texts,
            inspector.frames,
            frames.as_deref().unwrap_or(NO_VALUE),
        );
        let rate = facts.rate.map(|rate| rate_fact(rate, facts.rate_is_stored));
        write_text(
            &mut texts,
            inspector.rate,
            rate.as_deref().unwrap_or(NO_VALUE),
        );
        write_text(
            &mut texts,
            inspector.clip_bones,
            &facts.clip_bones.to_string(),
        );
        let driven = format!("{} of {}", facts.driven, facts.skeleton_bones);
        write_text(&mut texts, inspector.driven, &driven);
        if let Some(mut text) = texts.get_entity_mut(inspector.driven) {
            let color = if facts.driven == facts.clip_bones {
                theme.text
            } else {
                theme.warning
            };
            if text.color != color {
                text.color = color;
            }
        }
        write_line(&mut texts, &mut nodes, inspector.status, &document.status);
    }
}

#[cfg(test)]
mod tests {
    use super::super::{
        preview::ClipPosing,
        tests::{
            clip_fixture, clip_of, named_key, open_clip, state, transport, typed_key, write_clip,
            write_scene,
        },
        transport::{ClipTransport, sync_transport_rows, sync_transport_timelines},
    };
    use super::*;
    use concerto_animation::clip::AnimationClip;
    use concerto_ecs::{
        IntoSystem, System, World, entity::hierarchy::ChildOf, events::event_channel::EventChannel,
    };
    use concerto_editor::{
        hosted::testing::SceneFixture,
        timeline::{TimelineScrubbed, time_of_frame},
    };
    use concerto_foundation::assets::registry::AssetEntry;
    use concerto_ui::text_input::{UITextInputCancelled, UITextInputSubmitted};
    use concerto_window::input::actions::{ActionFired, ActionMap};

    pub(super) struct Tab {
        pub(super) f: SceneFixture,
        pub(super) document: Entity,
    }

    fn panels_fixture() -> SceneFixture {
        let mut f = clip_fixture();
        f.world
            .insert_resource(EventChannel::<UITextInputSubmitted>::new());
        f.world
            .insert_resource(EventChannel::<UITextInputCancelled>::new());
        f.world.insert_resource(EventChannel::<ActionFired>::new());
        f.world.insert_resource(ActionMap::default());
        let mut bind = bind_clip_keys.into_system();
        bind.initialize(&mut f.world);
        bind.run_and_apply((), &mut f.world);
        f.add_delivery(build_clip_panels.into_boxed_system());
        f.add_delivery(apply_clip_event_fields.into_boxed_system());
        f.add_delivery(remove_clip_event.into_boxed_system());
        f.add_delivery(sync_clip_panels.into_boxed_system());
        f.add_delivery(sync_transport_rows.into_boxed_system());
        f.add_delivery(sync_transport_timelines.into_boxed_system());
        f.add_delivery(sync_clip_events.into_boxed_system());
        f.add_delivery(sync_clip_event_context.into_boxed_system());
        f
    }

    fn tab_of(mut f: SceneFixture, clip: &AssetEntry) -> Tab {
        let document = open_clip(&mut f, clip);
        let [left, inspector, bottom] = [(); 3].map(|_| f.world.spawn(UINode::default()));
        f.world.insert(
            EditorHosts {
                canvas: None,
                left: Some(left),
                inspector: Some(inspector),
                bottom: Some(bottom),
            },
            document,
        );
        f.frame();
        Tab { f, document }
    }

    pub(super) fn tab_of_clip(clip: &AnimationClip) -> Tab {
        let mut f = panels_fixture();
        let asset = write_clip(&mut f, "content/hero/odd.gasset", clip);
        tab_of(f, &asset)
    }

    impl Tab {
        pub(super) fn panels(&self) -> &ClipPanels {
            self.f
                .world
                .get_component_for_entity::<ClipPanels>(self.document)
                .expect("the panels were built")
        }

        pub(super) fn state(&self) -> &ClipDocument {
            state(&self.f, self.document)
        }

        pub(super) fn world(&self) -> &World {
            &self.f.world
        }

        fn timeline(&self) -> &Timeline {
            self.world()
                .get_component_for_entity::<Timeline>(self.panels().timeline)
                .unwrap()
        }

        pub(super) fn click(&mut self, entity: Entity) {
            self.f.world.trigger_on(
                entity,
                UIClick {
                    position: Default::default(),
                    button: MouseButton::Left,
                },
            );
        }

        pub(super) fn scrub(&mut self, time: f32) {
            let timeline = self.panels().timeline;
            self.f.world.trigger_on(timeline, TimelineScrubbed { time });
        }

        pub(super) fn until(&mut self, what: &str, done: impl Fn(&ClipTransport) -> bool) {
            let document = self.document;
            self.f.frames_until(what, |f| done(transport(f, document)));
        }

        fn parent(&self, entity: Entity) -> Entity {
            self.world()
                .get_component_for_entity::<ChildOf>(entity)
                .unwrap()
                .parent()
        }

        fn menu_item(&mut self, name: &str) -> Entity {
            let label = self
                .f
                .world
                .query::<(Entity, &UIText), ()>()
                .iter(&mut self.f.world)
                .find(|(_, text)| text.text == name)
                .map(|(entity, _)| entity)
                .expect("the menu lists the asset");
            self.parent(label)
        }

        fn clip_bytes(&self) -> Vec<u8> {
            let clip = self.f.document(self.document).current.as_ref().unwrap();
            std::fs::read(self.f.project_root.join(&clip.address)).unwrap()
        }

        pub(super) fn clean(&mut self) -> bool {
            !self.f.unsaved(self.document)
        }

        fn move_marker(&mut self, index: usize, time: f32) {
            let timeline = self.panels().timeline;
            self.f
                .world
                .trigger_on(timeline, TimelineMarkerMoved { index, time });
            self.f.frame();
        }

        pub(super) fn status(&self) -> String {
            self.f.status(self.document)
        }

        pub(super) fn event_times(&self) -> Vec<f32> {
            self.state().events.iter().map(|event| event.time).collect()
        }
    }

    pub(super) fn eventful_clip() -> AnimationClip {
        let mut clip = clip_of(10);
        clip.replace_events(vec![
            typed_key(0.25),
            named_key(0.5, "footstep"),
            named_key(1.0, "footstep"),
        ])
        .unwrap();
        clip
    }

    #[test]
    fn root_motion_and_a_picked_scene_reach_the_preview_and_no_control_touches_the_clip() {
        let mut f = panels_fixture();
        write_scene(&mut f, "content/hero/scene.gasset", "sibling");
        write_scene(&mut f, "content/world/level.gasset", "level");
        let walk = write_clip(&mut f, "content/hero/walk.gasset", &clip_of(10));
        let mut tab = tab_of(f, &walk);
        let document = tab.document;
        let on_disk = tab.clip_bytes();

        let root_motion = tab.panels().inspector.root_motion;
        tab.click(root_motion);
        tab.f
            .frames_until("root motion never reached the preview", |f| {
                f.scene_world(document)
                    .get_resource::<ClipPosing>()
                    .unwrap()
                    .root_motion
            });
        let picker = tab.parent(tab.panels().left.scene);
        tab.click(picker);
        let item = tab.menu_item("level");
        tab.click(item);
        tab.f
            .frames_until("the pick never reached the preview", |f| {
                f.scene_names(document).contains(&"level".to_owned())
            });
        assert!(!tab.f.scene_names(document).contains(&"sibling".to_owned()));
        tab.scrub(3.0);
        tab.f.frame();

        assert!(tab.clean());
        assert_eq!(tab.clip_bytes(), on_disk);
    }

    #[test]
    fn a_marker_drag_moves_its_event_and_marks_the_tab_unsaved_unless_it_is_refused() {
        let mut tab = tab_of_clip(&eventful_clip());
        tab.f.frame();
        assert_eq!(tab.timeline().markers.len(), 3);
        assert!(tab.timeline().markers[0].locked);

        tab.move_marker(2, 0.5);
        tab.move_marker(0, 2.0);

        assert_eq!(tab.event_times(), vec![0.25, 0.5, 1.0]);
        assert_ne!(tab.status(), "");
        assert!(tab.clean());

        tab.move_marker(1, time_of_frame(60, 30.0));

        assert_eq!(tab.event_times(), vec![0.25, 2.0, 1.0]);
        assert_eq!(tab.timeline().markers[1].time, 2.0);
        assert_eq!(tab.status(), "");
        assert!(!tab.clean());
    }
}

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
           documents: Query<&mut ClipDocument>,
           mut previews: PreviewSignals| {
        if let Some(mut state) = documents.get_entity(document) {
            state.root_motion = on.signal().checked;
            previews.trigger(document, SetRootMotion(state.root_motion));
        }
    })
    .into_listener()
}

fn toggle_skeleton(document: Entity) -> Listener<UICheckboxChanged> {
    (move |on: On<UICheckboxChanged>,
           documents: Query<&mut ClipDocument>,
           mut previews: PreviewSignals| {
        if let Some(mut state) = documents.get_entity(document) {
            state.skeleton_gizmo = on.signal().checked;
            previews.trigger(document, SetSkeletonGizmo(state.skeleton_gizmo));
        }
    })
    .into_listener()
}

fn select_event(document: Entity) -> Listener<TimelineMarkerSelected> {
    (move |on: On<TimelineMarkerSelected>, documents: Query<&mut ClipDocument>| {
        let Some(mut state) = documents.get_entity(document) else {
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
           documents: Query<(&mut EditorDocument, &mut ClipDocument)>,
           mut cmd: CommandQueue| {
        let index = on.signal().index;
        let time = on.signal().time;
        let Some((mut editor, mut state)) = documents.get_entity(document) else {
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
           menus: Query<(Entity, &mut AssetPickerMenu, &mut UIAnchoredPanel)>,
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
            &menus,
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
               menus: Query<&mut UIAnchoredPanel, With<AssetPickerMenu>>,
               documents: Query<(&EditorDocument, &mut ClipDocument)>,
               project: Res<ProjectState>,
               assets: Res<AssetRegistry>,
               mut previews: PreviewSignals| {
            if on.signal().button != MouseButton::Left {
                return;
            }
            for mut panel in menus.iter() {
                panel.open = false;
            }
            let Some((editor, mut state)) = documents.get_entity(document) else {
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

pub(crate) fn sync_clip_panels(
    documents: Query<(&EditorDocument, &ClipDocument, &ClipPanels)>,
    timelines: Query<&mut Timeline>,
    texts: Query<&mut UIText>,
    nodes: Query<&mut UINode>,
    checkboxes: Query<&mut UICheckbox>,
    assets: Res<AssetRegistry>,
    theme: Res<UITheme>,
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
    let write_checked = |entity: Entity, checked: bool| {
        if let Some(mut checkbox) = checkboxes.get_entity(entity)
            && checkbox.checked != checked
        {
            checkbox.checked = checked;
        }
    };
    for (document, state, panels) in documents.iter() {
        let facts = &state.facts;
        let left = &panels.left;
        let inspector = &panels.inspector;

        if let Some(mut timeline) = timelines.get_entity(panels.timeline) {
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
        write_text(left.scene, &scene);
        write_checked(left.skeleton, state.skeleton_gizmo);
        write_line(left.scene_error, state.scene_error.as_deref().unwrap_or(""));

        write_checked(inspector.root_motion, state.root_motion);

        write_text(inspector.duration, &format!("{:.2} s", facts.duration));
        let frames = facts.frames.map(|frames| frames.to_string());
        write_text(inspector.frames, frames.as_deref().unwrap_or(NO_VALUE));
        let rate = facts.rate.map(|rate| rate_fact(rate, facts.rate_is_stored));
        write_text(inspector.rate, rate.as_deref().unwrap_or(NO_VALUE));
        write_text(inspector.clip_bones, &facts.clip_bones.to_string());
        let driven = format!("{} of {}", facts.driven, facts.skeleton_bones);
        write_text(inspector.driven, &driven);
        if let Some(mut text) = texts.get_entity(inspector.driven) {
            let color = if facts.driven == facts.clip_bones {
                theme.text
            } else {
                theme.warning
            };
            if text.color != color {
                text.color = color;
            }
        }
        write_line(inspector.status, &document.status);
    }
}

#[cfg(test)]
mod tests {
    use super::super::{
        preview::ClipPosing,
        tests::{
            bone, clip_fixture, clip_of, named_key, open_clip, report_bones, report_skeleton,
            state, transport, typed_key, write_broken_scene, write_clip, write_scene,
        },
        transport::{ClipTransport, TransportRow, sync_transport_rows, sync_transport_timelines},
    };
    use super::*;
    use concerto_animation::clip::{AnimationChanelOutput, AnimationChannel, AnimationClip};
    use concerto_ecs::{
        Changed, IntoSystem, ResMut, Resource, System, World, entity::hierarchy::ChildOf,
        events::event_channel::EventChannel,
    };
    use concerto_editor::{
        hosted::testing::SceneFixture,
        inspector::PropertyCommits,
        timeline::{TimelineScrubbed, time_of_frame},
    };
    use concerto_foundation::assets::registry::AssetEntry;
    use concerto_ui::interaction::UIDisabled;
    use concerto_ui::text_input::{UITextInputCancelled, UITextInputSubmitted};
    use concerto_window::input::actions::{ActionFired, ActionMap};
    use uuid::Uuid;

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

    pub(super) fn walking_tab() -> Tab {
        let mut f = panels_fixture();
        let walk = write_clip(&mut f, "content/hero/walk.gasset", &clip_of(10));
        tab_of(f, &walk)
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

        pub(super) fn transport(&self) -> &ClipTransport {
            transport(&self.f, self.document)
        }

        fn transport_row(&mut self) -> TransportRow {
            let document = self.document;
            *self
                .f
                .world
                .query::<&TransportRow, ()>()
                .iter(&mut self.f.world)
                .find(|row| row.document == document)
                .expect("the transport was spawned")
        }

        pub(super) fn timeline(&self) -> &Timeline {
            self.world()
                .get_component_for_entity::<Timeline>(self.panels().timeline)
                .unwrap()
        }

        pub(super) fn text(&self, entity: Entity) -> &UIText {
            self.world()
                .get_component_for_entity::<UIText>(entity)
                .unwrap()
        }

        pub(super) fn shown(&self, entity: Entity) -> bool {
            self.world()
                .get_component_for_entity::<UINode>(entity)
                .unwrap()
                .visible
        }

        pub(super) fn disabled(&self, entity: Entity) -> bool {
            self.world()
                .get_component_for_entity::<UIDisabled>(entity)
                .is_some()
        }

        fn checked(&self, entity: Entity) -> bool {
            self.world()
                .get_component_for_entity::<UICheckbox>(entity)
                .unwrap()
                .checked
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

        fn until_posed(&mut self, what: &str, done: impl Fn(&ClipPosing) -> bool) {
            let document = self.document;
            self.f.frames_until(what, |f| {
                done(
                    f.scene_world(document)
                        .get_resource::<ClipPosing>()
                        .unwrap(),
                )
            });
        }

        fn until_moving(&mut self) {
            self.until("the head never moved", |transport| transport.time > 0.0);
        }

        pub(super) fn paused_at(&mut self, time: f32) {
            self.scrub(time);
            self.until("the seek never came back", |transport| {
                !transport.playing && transport.time == time
            });
            self.f.frame();
        }

        fn parent(&self, entity: Entity) -> Entity {
            self.world()
                .get_component_for_entity::<ChildOf>(entity)
                .unwrap()
                .parent()
        }

        fn open_picker(&mut self) {
            let button = self.parent(self.panels().left.scene);
            self.click(button);
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

        fn menu_open(&mut self) -> bool {
            self.f
                .world
                .query::<&UIAnchoredPanel, With<AssetPickerMenu>>()
                .iter(&mut self.f.world)
                .any(|panel| panel.open)
        }

        fn clip_bytes(&self) -> Vec<u8> {
            let clip = self.f.document(self.document).current.as_ref().unwrap();
            std::fs::read(self.f.project_root.join(&clip.address)).unwrap()
        }

        pub(super) fn clean(&mut self) -> bool {
            !self.f.unsaved(self.document)
        }

        pub(super) fn select_marker(&mut self, index: Option<usize>) {
            let timeline = self.panels().timeline;
            self.f
                .world
                .trigger_on(timeline, TimelineMarkerSelected { index });
        }

        fn move_marker(&mut self, index: usize, time: f32) {
            let timeline = self.panels().timeline;
            self.f
                .world
                .trigger_on(timeline, TimelineMarkerMoved { index, time });
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

    pub(super) fn single_key_clip() -> AnimationClip {
        let mut clip = AnimationClip::default();
        clip.add_channel(
            Uuid::from_u128(1),
            AnimationChannel::new(
                vec![2.0],
                AnimationChanelOutput::from_translation([[0.0; 3]].into_iter()),
            ),
        );
        clip
    }

    #[test]
    fn the_facts_show_what_the_clip_holds() {
        let tab = walking_tab();
        let inspector = &tab.panels().inspector;

        assert_eq!(tab.text(inspector.duration).text, "10.00 s");
        assert_eq!(tab.text(inspector.frames).text, "301");
        assert_eq!(tab.text(inspector.rate).text, "30 fps, detected");
        assert_eq!(tab.text(inspector.clip_bones).text, "1");
    }

    #[test]
    fn a_fractional_rate_keeps_two_decimals() {
        assert_eq!(rate_fact(29.97, false), "29.97 fps, detected");
        assert_eq!(rate_fact(30.0, false), "30 fps, detected");
        assert_eq!(rate_fact(29.97, true), "29.97 fps");
        assert_eq!(rate_fact(30.0, true), "30 fps");
    }

    #[test]
    fn a_clip_with_no_rate_says_so_and_disables_the_step_buttons() {
        let mut tab = tab_of_clip(&single_key_clip());
        tab.f.frame();
        let row = tab.transport_row();
        let inspector = &tab.panels().inspector;

        assert_eq!(tab.text(inspector.frames).text, NO_VALUE);
        assert_eq!(tab.text(inspector.rate).text, NO_VALUE);
        assert!(tab.disabled(row.step_back));
        assert!(tab.disabled(row.step_forward));
        assert!(!tab.disabled(row.play));
        assert!(!tab.disabled(row.looping));
    }

    #[test]
    fn the_status_line_shows_a_failed_clip_on_a_line_of_its_own() {
        let mut f = panels_fixture();
        let walk = write_clip(&mut f, "content/walk.gasset", &clip_of(1));
        std::fs::write(f.project_root.join(&walk.address), b"not a clip").unwrap();

        let tab = tab_of(f, &walk);

        let status = tab.panels().inspector.status;
        assert!(tab.shown(status));
        assert!(tab.text(status).text.contains("content asset"));
    }

    #[test]
    fn the_status_line_is_hidden_while_the_status_is_empty_and_follows_a_preview_failure() {
        let mut tab = walking_tab();
        let status = tab.panels().inspector.status;
        assert!(!tab.shown(status));

        let document = tab.document;
        tab.f
            .world
            .get_component_for_entity_mut::<EditorDocument>(document)
            .unwrap()
            .status = "Preview failed: broken".into();
        tab.f.frame();

        assert!(tab.shown(status));
        assert_eq!(tab.text(status).text, "Preview failed: broken");
    }

    #[test]
    fn the_timeline_follows_the_clip_and_the_preview() {
        let mut tab = walking_tab();

        assert_eq!(tab.timeline().duration, 10.0);
        assert_eq!(tab.timeline().rate, Some(30.0));

        tab.until_moving();
        tab.f.frame();
        assert_eq!(tab.timeline().time, tab.transport().time);
    }

    #[test]
    fn the_root_motion_checkbox_reaches_the_document_and_the_preview() {
        let mut tab = walking_tab();
        let root_motion = tab.panels().inspector.root_motion;
        assert!(!tab.checked(root_motion));

        tab.click(root_motion);

        assert!(tab.state().root_motion);
        tab.until_posed("root motion never reached the preview", |posing| {
            posing.root_motion
        });
        assert!(tab.checked(root_motion));
    }

    #[test]
    fn the_skeleton_checkbox_reaches_the_document_and_the_preview() {
        let mut tab = walking_tab();
        let skeleton = tab.panels().left.skeleton;
        assert!(tab.checked(skeleton));

        tab.click(skeleton);

        assert!(!tab.state().skeleton_gizmo);
        tab.until_posed("the gizmo flag never reached the preview", |posing| {
            !posing.skeleton_gizmo
        });
        assert!(!tab.checked(skeleton));
    }

    #[test]
    fn driven_reads_as_a_share_of_the_skeleton_and_warns_when_it_differs_from_the_clip() {
        let mut tab = walking_tab();
        let driven = tab.panels().inspector.driven;
        let document = tab.document;
        let theme = UITheme::default();

        report_bones(&mut tab.f, document, 65, 60);
        tab.f.frame();

        assert_eq!(tab.text(driven).text, "60 of 65");
        assert_eq!(tab.text(driven).color, theme.warning);

        report_bones(&mut tab.f, document, 66, 1);
        tab.f.frame();

        assert_eq!(tab.text(driven).text, "1 of 66");
        assert_eq!(tab.text(driven).color, theme.text);
    }

    #[test]
    fn the_scene_label_names_the_shown_scene_or_none() {
        let mut f = panels_fixture();
        write_scene(&mut f, "content/hero/scene.gasset", "sibling");
        let walk = write_clip(&mut f, "content/hero/walk.gasset", &clip_of(10));
        let tab = tab_of(f, &walk);
        assert_eq!(tab.text(tab.panels().left.scene).text, "scene");

        let tab = walking_tab();
        assert_eq!(tab.text(tab.panels().left.scene).text, NO_VALUE);
    }

    #[test]
    fn the_scene_picker_opens_with_no_row_and_a_pick_shows_the_scene() {
        let mut f = panels_fixture();
        write_scene(&mut f, "content/hero/scene.gasset", "sibling");
        let level = write_scene(&mut f, "content/world/level.gasset", "level");
        let walk = write_clip(&mut f, "content/hero/walk.gasset", &clip_of(10));
        let mut tab = tab_of(f, &walk);
        let document = tab.document;
        let on_disk = tab.clip_bytes();

        tab.open_picker();
        assert!(tab.menu_open());
        let item = tab.menu_item("level");
        tab.click(item);
        tab.f.frame();

        assert!(!tab.menu_open());
        assert_eq!(tab.state().picked_scene, Some(level.id));
        assert_eq!(tab.state().shown_scene, Some(level.id));
        assert_eq!(tab.state().scene_error, None);
        assert_eq!(tab.text(tab.panels().left.scene).text, "level");
        assert!(
            tab.world()
                .get_resource::<PropertyCommits>()
                .unwrap()
                .0
                .is_empty()
        );
        tab.f
            .frames_until("the pick never reached the preview", |f| {
                f.scene_names(document).contains(&"level".to_owned())
            });
        assert!(!tab.f.scene_names(document).contains(&"sibling".to_owned()));
        assert!(tab.clean());
        assert_eq!(tab.clip_bytes(), on_disk);
    }

    #[test]
    fn a_pick_on_a_tab_whose_clip_failed_shows_the_scene() {
        let mut f = panels_fixture();
        let level = write_scene(&mut f, "content/world/level.gasset", "level");
        let walk = write_clip(&mut f, "content/hero/walk.gasset", &clip_of(1));
        std::fs::write(f.project_root.join(&walk.address), b"not a clip").unwrap();
        let mut tab = tab_of(f, &walk);
        let document = tab.document;

        tab.open_picker();
        let item = tab.menu_item("level");
        tab.click(item);
        tab.f.frame();

        assert_eq!(tab.state().shown_scene, Some(level.id));
        assert_eq!(tab.text(tab.panels().left.scene).text, "level");
        tab.f
            .frames_until("the pick never reached the preview", |f| {
                f.scene_names(document).contains(&"level".to_owned())
            });
    }

    #[test]
    fn the_picker_button_closes_its_menu() {
        let mut tab = walking_tab();

        tab.open_picker();
        tab.open_picker();

        assert!(!tab.menu_open());
    }

    #[test]
    fn a_scene_that_fails_to_load_shows_under_the_picker() {
        let mut f = panels_fixture();
        write_broken_scene(&mut f, "content/world/broken.gasset");
        let walk = write_clip(&mut f, "content/hero/walk.gasset", &clip_of(10));
        let mut tab = tab_of(f, &walk);
        let error = tab.panels().left.scene_error;
        assert!(!tab.shown(error));

        tab.open_picker();
        let item = tab.menu_item("broken");
        tab.click(item);
        tab.f.frame();

        assert!(tab.shown(error));
        assert_eq!(
            tab.text(error).text,
            tab.state().scene_error.clone().unwrap()
        );
        assert_eq!(tab.text(tab.panels().left.scene).text, NO_VALUE);
    }

    #[test]
    fn the_tab_stays_clean_through_every_control() {
        let mut f = panels_fixture();
        write_scene(&mut f, "content/world/level.gasset", "level");
        let walk = write_clip(&mut f, "content/hero/walk.gasset", &clip_of(10));
        let mut tab = tab_of(f, &walk);
        let on_disk = tab.clip_bytes();
        let row = tab.transport_row();
        let left = &tab.panels().left;
        let (skeleton, button) = (left.skeleton, tab.parent(left.scene));
        let inspector = &tab.panels().inspector;
        let buttons = [
            row.play,
            row.step_forward,
            row.step_back,
            row.looping,
            row.speeds[0],
            inspector.root_motion,
            skeleton,
            button,
        ];

        for button in buttons {
            tab.click(button);
            tab.f.frame();
            assert!(tab.clean());
        }
        let item = tab.menu_item("level");
        tab.click(item);
        tab.scrub(3.0);
        tab.f.frame();

        assert!(tab.clean());
        assert_eq!(tab.clip_bytes(), on_disk);
    }

    #[derive(Resource, Default)]
    struct Writes(usize);

    fn count_writes(
        texts: Query<&UIText, Changed<UIText>>,
        nodes: Query<&UINode, Changed<UINode>>,
        timelines: Query<&Timeline, Changed<Timeline>>,
        mut writes: ResMut<Writes>,
    ) {
        writes.0 = texts.iter().count() + nodes.iter().count() + timelines.iter().count();
    }

    #[test]
    fn a_settled_panel_writes_nothing() {
        let mut f = panels_fixture();
        f.world.insert_resource(Writes::default());
        f.add_delivery(count_writes.into_boxed_system());
        let walk = write_clip(&mut f, "content/hero/walk.gasset", &clip_of(10));
        let mut tab = tab_of(f, &walk);
        tab.paused_at(4.0);
        tab.f.frame();
        tab.f.frame();
        tab.f.frame();

        assert_eq!(tab.world().get_resource::<Writes>().unwrap().0, 0);
    }

    #[test]
    fn a_settled_panel_with_markers_writes_nothing() {
        let mut f = panels_fixture();
        f.world.insert_resource(Writes::default());
        f.add_delivery(count_writes.into_boxed_system());
        let walk = write_clip(&mut f, "content/hero/walk.gasset", &eventful_clip());
        let mut tab = tab_of(f, &walk);
        tab.select_marker(Some(1));
        tab.paused_at(4.0);
        tab.f.frame();
        tab.f.frame();
        tab.f.frame();

        assert_eq!(tab.timeline().markers.len(), 3);
        assert_eq!(tab.world().get_resource::<Writes>().unwrap().0, 0);
    }

    #[test]
    fn a_settled_panel_with_bone_rows_writes_nothing() {
        let mut f = panels_fixture();
        f.world.insert_resource(Writes::default());
        f.add_delivery(count_writes.into_boxed_system());
        let walk = write_clip(&mut f, "content/hero/walk.gasset", &clip_of(10));
        let mut tab = tab_of(f, &walk);
        let document = tab.document;
        report_skeleton(&mut tab.f, document, &[bone(Uuid::from_u128(1), "hips")]);
        tab.paused_at(4.0);
        tab.f.frame();
        tab.f.frame();
        tab.f.frame();

        assert_eq!(tab.timeline().tracks[0].label, "hips");
        assert_eq!(tab.world().get_resource::<Writes>().unwrap().0, 0);
    }

    #[test]
    fn the_timeline_shows_the_bone_rows_and_follows_the_skeleton() {
        let mut tab = walking_tab();
        let document = tab.document;

        assert_eq!(tab.timeline().tracks, tab.state().rows);
        assert_eq!(tab.timeline().tracks[0].label, "? 00000000");
        assert!(tab.timeline().tracks[0].warning);

        report_skeleton(&mut tab.f, document, &[bone(Uuid::from_u128(1), "hips")]);
        tab.f.frame();

        let tracks = &tab.timeline().tracks;
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].label, "hips");
        assert!(!tracks[0].warning);
        assert_eq!(tracks[0].keys.len(), 301);
        assert!(tab.clean());
    }

    #[test]
    fn markers_follow_the_events_with_typed_ones_locked() {
        let mut tab = tab_of_clip(&eventful_clip());
        tab.f.frame();

        assert_eq!(
            tab.timeline().markers,
            vec![
                TimelineMarker {
                    time: 0.25,
                    label: "Footstep".into(),
                    locked: true,
                },
                TimelineMarker {
                    time: 0.5,
                    label: "footstep".into(),
                    locked: false,
                },
                TimelineMarker {
                    time: 1.0,
                    label: "footstep".into(),
                    locked: false,
                },
            ]
        );
        assert_eq!(tab.timeline().selected_marker, None);
    }

    #[test]
    fn the_selected_marker_follows_the_selection() {
        let mut tab = tab_of_clip(&eventful_clip());

        tab.select_marker(Some(1));
        tab.f.frame();
        assert_eq!(tab.state().selected_event, Some(1));
        assert_eq!(tab.timeline().selected_marker, Some(1));

        tab.select_marker(Some(9));
        tab.f.frame();
        assert_eq!(tab.state().selected_event, None);

        tab.select_marker(Some(0));
        tab.select_marker(None);
        tab.f.frame();
        assert_eq!(tab.state().selected_event, None);
        assert_eq!(tab.timeline().selected_marker, None);
        assert!(tab.clean());
    }

    #[test]
    fn a_marker_drag_moves_its_event_marks_the_tab_unsaved_and_clears_the_status() {
        let mut tab = tab_of_clip(&eventful_clip());
        let document = tab.document;
        tab.f
            .world
            .get_component_for_entity_mut::<EditorDocument>(document)
            .unwrap()
            .status = "an earlier refusal".into();

        tab.move_marker(1, time_of_frame(60, 30.0));
        tab.f.frame();

        assert_eq!(tab.event_times(), vec![0.25, 2.0, 1.0]);
        assert_eq!(tab.timeline().markers[1].time, 2.0);
        assert_eq!(tab.status(), "");
        assert!(!tab.clean());
    }

    #[test]
    fn a_refused_marker_drag_sets_the_status_and_leaves_the_events_and_the_tab_alone() {
        let mut tab = tab_of_clip(&eventful_clip());

        tab.move_marker(2, 0.5);
        tab.f.frame();

        assert_eq!(tab.status(), "`footstep` is already at frame 15");
        assert_eq!(tab.event_times(), vec![0.25, 0.5, 1.0]);
        assert_eq!(tab.timeline().markers[2].time, 1.0);
        assert!(tab.clean());

        tab.move_marker(0, 2.0);
        tab.f.frame();

        assert_eq!(tab.status(), "Typed events are read-only in the editor");
        assert_eq!(tab.event_times(), vec![0.25, 0.5, 1.0]);
        assert!(tab.clean());
    }

    #[test]
    fn the_tab_stays_clean_while_a_marker_is_dragged_onto_its_own_time() {
        let mut tab = tab_of_clip(&eventful_clip());

        tab.move_marker(1, 0.5);
        tab.f.frame();

        assert!(tab.clean());
        assert_eq!(tab.status(), "");
    }
}

//! What a clip's tab shows in its `left`, `inspector` and `bottom` hosts, kept in step with its document.
use concerto_ecs::{
    Added, Component, Entity, Query, Res, With,
    command::{CommandQueue, EntityCommandQueue},
    signal::{
        On,
        listener::{IntoListener, Listener},
    },
};
use concerto_foundation::assets::{Asset, AssetId, registry::AssetRegistry};
use concerto_scene::scene::Scene;
use concerto_ui::elements::{Label, Listening, Pressable, Text, prelude::*};
use concerto_ui::{
    anchor::UIAnchoredPanel,
    checkbox::{UICheckbox, UICheckboxChanged},
    interaction::{UIClick, UIDisabled, UIInteractionStyle},
    node::{UINode, UIRect},
    text::UIText,
    theme::{ButtonVariant, UITheme},
    transform::UIValue,
};
use concerto_window::input::MouseButton;

use super::{
    ClipDocument,
    preview::{SeekClip, SetClipTransport, SetRootMotion, SetSkeletonGizmo},
    scene_signal,
};
use crate::{
    asset_editor::{EditorDocument, EditorHosts},
    asset_picker::{
        AssetPickerMenu, asset_name, open_asset_menu, picker_button, picker_caret, picker_label,
    },
    fonts::{MEDIUM, glyph, icon},
    preview::PreviewSignals,
    project::ProjectState,
    timeline::{Timeline, TimelineScrubbed, TimelineSelected, frame_at, time_of_frame},
};

const TITLE_HEIGHT: f32 = 16.0;
const BOX_SIZE: f32 = 16.0;
const NO_VALUE: &str = "None";
const RATES: [(f32, &str); 4] = [(0.25, "0.25×"), (0.5, "0.5×"), (1.0, "1×"), (2.0, "2×")];

/// The widgets of a clip's tab that follow its document, kept on the document.
#[derive(Component)]
pub(crate) struct ClipPanels {
    left: LeftPanel,
    inspector: InspectorPanel,
    timeline: Entity,
}

struct LeftPanel {
    scene: Entity,
    skeleton: Entity,
    scene_error: Entity,
}

struct InspectorPanel {
    step_back: Entity,
    play: Entity,
    step_forward: Entity,
    looping: Entity,
    rates: [Entity; RATES.len()],
    root_motion: Entity,
    duration: Entity,
    frames: Entity,
    rate: Entity,
    clip_bones: Entity,
    driven: Entity,
    status: Entity,
}

fn transport(state: &ClipDocument) -> SetClipTransport {
    SetClipTransport {
        playing: state.playing,
        looping: state.looping,
        rate: state.rate,
    }
}

fn pause_at(state: &ClipDocument, document: Entity, time: f32, previews: &mut PreviewSignals) {
    previews.trigger(
        document,
        SetClipTransport {
            playing: false,
            ..transport(state)
        },
    );
    previews.trigger(document, SeekClip(time));
}

fn at_the_end(state: &ClipDocument) -> bool {
    let duration = state.facts.duration;
    state.time >= duration
        || state
            .facts
            .rate
            .is_some_and(|rate| frame_at(state.time, rate) >= frame_at(duration, rate))
}

fn toggle_playing(document: Entity) -> Listener<UIClick> {
    (move |on: On<UIClick>, documents: Query<&ClipDocument>, mut previews: PreviewSignals| {
        if on.signal().button != MouseButton::Left {
            return;
        }
        let Some(state) = documents.get_entity(document) else {
            return;
        };
        let playing = !state.playing;
        if playing && !state.looping && at_the_end(state) {
            previews.trigger(document, SeekClip(0.0));
        }
        previews.trigger(
            document,
            SetClipTransport {
                playing,
                ..transport(state)
            },
        );
    })
    .into_listener()
}

fn step_frame(document: Entity, frames: i32) -> Listener<UIClick> {
    (move |on: On<UIClick>, documents: Query<&ClipDocument>, mut previews: PreviewSignals| {
        if on.signal().button != MouseButton::Left {
            return;
        }
        let Some(state) = documents.get_entity(document) else {
            return;
        };
        let Some(rate) = state.facts.rate else {
            return;
        };
        let frame = frame_at(state.time, rate).saturating_add_signed(frames);
        let time = time_of_frame(frame, rate).min(state.facts.duration);
        pause_at(state, document, time, &mut previews);
    })
    .into_listener()
}

fn toggle_looping(document: Entity) -> Listener<UIClick> {
    (move |on: On<UIClick>, documents: Query<&mut ClipDocument>, mut previews: PreviewSignals| {
        if on.signal().button != MouseButton::Left {
            return;
        }
        if let Some(mut state) = documents.get_entity(document) {
            state.looping = !state.looping;
            previews.trigger(document, transport(&state));
        }
    })
    .into_listener()
}

fn set_rate(document: Entity, rate: f32) -> Listener<UIClick> {
    (move |on: On<UIClick>, documents: Query<&mut ClipDocument>, mut previews: PreviewSignals| {
        if on.signal().button != MouseButton::Left {
            return;
        }
        if let Some(mut state) = documents.get_entity(document) {
            state.rate = rate;
            previews.trigger(document, transport(&state));
        }
    })
    .into_listener()
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

fn scrub_clip(document: Entity) -> Listener<TimelineScrubbed> {
    (move |on: On<TimelineScrubbed>,
           documents: Query<&ClipDocument>,
           mut previews: PreviewSignals| {
        if let Some(state) = documents.get_entity(document) {
            pause_at(state, document, on.signal().time, &mut previews);
        }
    })
    .into_listener()
}

fn select_frames(document: Entity) -> Listener<TimelineSelected> {
    (move |on: On<TimelineSelected>, documents: Query<&mut ClipDocument>| {
        if let Some(mut state) = documents.get_entity(document) {
            state.selection = on.signal().range;
        }
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

fn title(theme: &UITheme, text: &str) -> Label {
    theme
        .label(text.to_uppercase())
        .small()
        .muted()
        .weight(MEDIUM)
        .height(UIValue::Px(TITLE_HEIGHT))
        .fixed()
}

fn error_line(theme: &UITheme) -> Label {
    theme.label("").color(theme.error).hidden().fixed()
}

fn check_row(
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

fn icon_button(
    theme: &UITheme,
    mark: char,
    click: Listener<UIClick>,
) -> (Listening<Pressable, UIClick>, Text) {
    let size = theme.font_size_lg;
    let vertical = (theme.control_height - theme.line_height(size)) / 2.0;
    let horizontal = (theme.control_height - size) / 2.0;
    (
        theme
            .pressable()
            .solid()
            .bordered()
            .size(
                UIValue::Px(theme.control_height),
                UIValue::Px(theme.control_height),
            )
            .padding(UIRect::axes(vertical, horizontal))
            .on_click(click),
        icon(theme, mark, size),
    )
}

fn fact(parent: &mut EntityCommandQueue, label: &str, theme: &UITheme) -> Entity {
    let mut row = parent.spawn_child(theme.row().gap(theme.spacing_sm).fixed());
    row.add_child(theme.label(label).muted().grow());
    row.spawn_child(theme.label("").no_wrap()).entity()
}

fn build_left(host: &mut EntityCommandQueue, document: Entity, theme: &UITheme) -> LeftPanel {
    let mut panel = host.spawn_child(theme.column().gap(theme.spacing_sm));
    panel.add_child(title(theme, "Preview"));
    let mut row = panel.spawn_child(theme.row().fixed());
    let mut button = row.spawn_child(picker_button(theme).on_click(toggle_scene_picker(document)));
    let scene = button.spawn_child(picker_label(theme)).entity();
    button.add_child(picker_caret(theme));
    LeftPanel {
        scene,
        skeleton: check_row(&mut panel, "Skeleton", toggle_skeleton(document), theme),
        scene_error: panel.spawn_child(error_line(theme)).entity(),
    }
}

fn build_inspector(
    host: &mut EntityCommandQueue,
    document: Entity,
    theme: &UITheme,
) -> InspectorPanel {
    let mut panel = host.spawn_child(theme.column().gap(theme.spacing_sm));
    panel.add_child(title(theme, "Clip"));

    let mut controls = panel.spawn_child(theme.row().gap(theme.spacing_xs).fixed());
    let step_back = controls
        .spawn_child(icon_button(
            theme,
            glyph::SKIP_BACK,
            step_frame(document, -1),
        ))
        .entity();
    let play = controls
        .spawn_child(icon_button(theme, glyph::PLAY, toggle_playing(document)))
        .entity();
    let step_forward = controls
        .spawn_child(icon_button(
            theme,
            glyph::SKIP_FORWARD,
            step_frame(document, 1),
        ))
        .entity();
    let looping = controls
        .spawn_child(
            theme
                .button("Loop")
                .ghost()
                .on_click(toggle_looping(document)),
        )
        .entity();

    let mut rate_row = panel.spawn_child(theme.row().gap(theme.spacing_xs).fixed());
    let rates = RATES.map(|(rate, label)| {
        rate_row
            .spawn_child(
                theme
                    .button(label)
                    .ghost()
                    .on_click(set_rate(document, rate)),
            )
            .entity()
    });

    let root_motion = check_row(
        &mut panel,
        "Root motion",
        toggle_root_motion(document),
        theme,
    );
    InspectorPanel {
        step_back,
        play,
        step_forward,
        looping,
        rates,
        root_motion,
        duration: fact(&mut panel, "Duration", theme),
        frames: fact(&mut panel, "Frames", theme),
        rate: fact(&mut panel, "Rate", theme),
        clip_bones: fact(&mut panel, "Clip bones", theme),
        driven: fact(&mut panel, "Driven", theme),
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
        },
        scrub_clip(document),
        select_frames(document),
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

fn rate_fact(rate: f32) -> String {
    if rate.fract() == 0.0 {
        format!("{rate:.0} fps, detected")
    } else {
        format!("{rate:.2} fps, detected")
    }
}

pub(crate) fn sync_clip_panels(
    documents: Query<(&EditorDocument, &ClipDocument, &ClipPanels)>,
    timelines: Query<&mut Timeline>,
    texts: Query<&mut UIText>,
    nodes: Query<&mut UINode>,
    checkboxes: Query<&mut UICheckbox>,
    styles: Query<&mut UIInteractionStyle>,
    disabled: Query<&UIDisabled>,
    assets: Res<AssetRegistry>,
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
    let write_selected = |entity: Entity, selected: bool| {
        let wanted = theme.interaction(ButtonVariant::Ghost, selected);
        if let Some(mut style) = styles.get_entity(entity)
            && style.normal != wanted.normal
        {
            **style = wanted;
        }
    };
    for (document, state, panels) in documents.iter() {
        let facts = &state.facts;
        let left = &panels.left;
        let inspector = &panels.inspector;

        if let Some(mut timeline) = timelines.get_entity(panels.timeline) {
            let shown = Timeline {
                duration: facts.duration,
                rate: facts.rate,
                time: state.time,
                selection: state.selection,
            };
            if **timeline != shown {
                **timeline = shown;
            }
        }

        let scene = state
            .shown_scene
            .map_or_else(|| NO_VALUE.to_owned(), |id| asset_name(&assets, id));
        write_text(left.scene, &scene);
        write_checked(left.skeleton, state.skeleton_gizmo);
        write_line(left.scene_error, state.scene_error.as_deref().unwrap_or(""));

        let mark = if state.playing {
            glyph::PAUSE
        } else {
            glyph::PLAY
        };
        write_text(inspector.play, &mark.to_string());
        write_selected(inspector.looping, state.looping);
        for (button, (rate, _)) in inspector.rates.into_iter().zip(RATES) {
            write_selected(button, rate == state.rate);
        }
        write_checked(inspector.root_motion, state.root_motion);

        write_text(inspector.duration, &format!("{:.2} s", facts.duration));
        let frames = facts.frames.map(|frames| frames.to_string());
        write_text(inspector.frames, frames.as_deref().unwrap_or(NO_VALUE));
        let rate = facts.rate.map(rate_fact);
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

        let no_clip = facts.duration <= 0.0;
        let no_frames = no_clip || facts.rate.is_none();
        for (button, off) in [
            (inspector.step_back, no_frames),
            (inspector.step_forward, no_frames),
            (inspector.play, no_clip),
            (inspector.looping, no_clip),
        ] {
            match (off, disabled.get_entity(button).is_some()) {
                (true, false) => cmd.insert(UIDisabled, button),
                (false, true) => cmd.remove::<UIDisabled>(button),
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::{
        preview::ClipPreview,
        tests::{
            clip_fixture, clip_of, open_clip, report_bones, state, write_broken_scene, write_clip,
            write_scene,
        },
    };
    use super::*;
    use crate::{hosted::testing::SceneFixture, inspector::PropertyCommits};
    use concerto_animation::clip::{AnimationChanelOutput, AnimationChannel, AnimationClip};
    use concerto_ecs::{Changed, IntoSystem, ResMut, Resource, World, entity::hierarchy::ChildOf};
    use concerto_foundation::assets::registry::AssetEntry;
    use uuid::Uuid;

    struct Tab {
        f: SceneFixture,
        document: Entity,
    }

    fn panels_fixture() -> SceneFixture {
        let mut f = clip_fixture();
        f.add_delivery(build_clip_panels.into_boxed_system());
        f.add_delivery(sync_clip_panels.into_boxed_system());
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

    fn walking_tab() -> Tab {
        let mut f = panels_fixture();
        let walk = write_clip(&mut f, "content/hero/walk.gasset", &clip_of(10));
        tab_of(f, &walk)
    }

    fn tab_of_clip(clip: &AnimationClip) -> Tab {
        let mut f = panels_fixture();
        let asset = write_clip(&mut f, "content/hero/odd.gasset", clip);
        tab_of(f, &asset)
    }

    impl Tab {
        fn panels(&self) -> &ClipPanels {
            self.f
                .world
                .get_component_for_entity::<ClipPanels>(self.document)
                .expect("the panels were built")
        }

        fn state(&self) -> &ClipDocument {
            state(&self.f, self.document)
        }

        fn world(&self) -> &World {
            &self.f.world
        }

        fn preview(&self) -> &ClipPreview {
            self.f
                .scene_world(self.document)
                .get_resource::<ClipPreview>()
                .unwrap()
        }

        fn timeline(&self) -> &Timeline {
            self.world()
                .get_component_for_entity::<Timeline>(self.panels().timeline)
                .unwrap()
        }

        fn text(&self, entity: Entity) -> &UIText {
            self.world()
                .get_component_for_entity::<UIText>(entity)
                .unwrap()
        }

        fn shown(&self, entity: Entity) -> bool {
            self.world()
                .get_component_for_entity::<UINode>(entity)
                .unwrap()
                .visible
        }

        fn disabled(&self, entity: Entity) -> bool {
            self.world()
                .get_component_for_entity::<UIDisabled>(entity)
                .is_some()
        }

        fn selected(&self, entity: Entity) -> bool {
            self.world()
                .get_component_for_entity::<UIInteractionStyle>(entity)
                .unwrap()
                .normal
                == UITheme::default().selection()
        }

        fn checked(&self, entity: Entity) -> bool {
            self.world()
                .get_component_for_entity::<UICheckbox>(entity)
                .unwrap()
                .checked
        }

        fn click(&mut self, entity: Entity) {
            self.f.world.trigger_on(
                entity,
                UIClick {
                    position: Default::default(),
                    button: MouseButton::Left,
                },
            );
        }

        fn scrub(&mut self, time: f32) {
            let timeline = self.panels().timeline;
            self.f.world.trigger_on(timeline, TimelineScrubbed { time });
        }

        fn select(&mut self, range: Option<(u32, u32)>) {
            let timeline = self.panels().timeline;
            self.f
                .world
                .trigger_on(timeline, TimelineSelected { range });
            self.f.frame();
        }

        fn until(&mut self, what: &str, done: impl Fn(&ClipDocument) -> bool) {
            let document = self.document;
            self.f.frames_until(what, |f| done(state(f, document)));
        }

        fn until_previewed(&mut self, what: &str, done: impl Fn(&ClipPreview) -> bool) {
            let document = self.document;
            self.f.frames_until(what, |f| {
                done(
                    f.scene_world(document)
                        .get_resource::<ClipPreview>()
                        .unwrap(),
                )
            });
        }

        fn until_moving(&mut self) {
            self.until("the head never moved", |state| state.time > 0.0);
        }

        fn paused_at(&mut self, time: f32) {
            self.scrub(time);
            self.until("the seek never came back", |state| {
                !state.playing && state.time == time
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

        fn clean(&mut self) -> bool {
            !self.f.unsaved(self.document)
        }
    }

    fn single_key_clip() -> AnimationClip {
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
        assert_eq!(rate_fact(29.97), "29.97 fps, detected");
        assert_eq!(rate_fact(30.0), "30 fps, detected");
    }

    #[test]
    fn a_clip_with_no_rate_says_so_and_disables_the_step_buttons() {
        let mut tab = tab_of_clip(&single_key_clip());
        tab.f.frame();
        let inspector = &tab.panels().inspector;

        assert_eq!(tab.text(inspector.frames).text, NO_VALUE);
        assert_eq!(tab.text(inspector.rate).text, NO_VALUE);
        assert!(tab.disabled(inspector.step_back));
        assert!(tab.disabled(inspector.step_forward));
        assert!(!tab.disabled(inspector.play));
        assert!(!tab.disabled(inspector.looping));
    }

    #[test]
    fn a_clip_with_no_duration_disables_the_transport() {
        let mut tab = tab_of_clip(&AnimationClip::default());
        tab.f.frame();
        let inspector = &tab.panels().inspector;

        for button in [
            inspector.step_back,
            inspector.play,
            inspector.step_forward,
            inspector.looping,
        ] {
            assert!(tab.disabled(button));
        }
    }

    #[test]
    fn a_clip_with_frames_leaves_the_transport_enabled() {
        let mut tab = walking_tab();
        tab.f.frame();
        let inspector = &tab.panels().inspector;

        for button in [
            inspector.step_back,
            inspector.play,
            inspector.step_forward,
            inspector.looping,
        ] {
            assert!(!tab.disabled(button));
        }
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
    fn play_pauses_and_plays_again() {
        let mut tab = walking_tab();
        tab.until_moving();
        let play = tab.panels().inspector.play;
        assert_eq!(tab.text(play).text, glyph::PAUSE.to_string());

        tab.click(play);
        tab.until("the clip never paused", |state| !state.playing);
        tab.f.frame();
        assert_eq!(tab.text(play).text, glyph::PLAY.to_string());

        tab.click(play);
        tab.until("the clip never played again", |state| state.playing);
    }

    #[test]
    fn a_scrub_pauses_and_the_clip_stays_paused_after_it() {
        let mut tab = walking_tab();
        tab.until_moving();

        tab.paused_at(4.0);
        for _ in 0..5 {
            tab.f.frame();
        }

        assert!(!tab.state().playing);
        assert_eq!(tab.state().time, 4.0);
        assert_eq!(tab.timeline().time, 4.0);
        assert_eq!(tab.preview().time, 4.0);
    }

    #[test]
    fn play_at_the_end_of_a_clip_that_does_not_loop_starts_again_from_zero() {
        let mut tab = walking_tab();
        let inspector = &tab.panels().inspector;
        let (looping, play) = (inspector.looping, inspector.play);
        tab.click(looping);
        tab.paused_at(10.0);

        tab.click(play);

        tab.until("the clip never started again", |state| {
            state.playing && state.time < 5.0
        });
    }

    #[test]
    fn play_on_the_last_frame_of_a_clip_ending_between_frames_starts_again_from_zero() {
        let mut times: Vec<f32> = (0..=30).map(|frame| frame as f32 / 30.0).collect();
        times.push(1.01);
        let outputs = vec![glam::Vec3::ZERO; times.len()];
        let mut clip = AnimationClip::default();
        clip.add_channel(
            Uuid::from_u128(1),
            AnimationChannel::new(times, AnimationChanelOutput::Translation(outputs)),
        );
        let mut tab = tab_of_clip(&clip);
        let inspector = &tab.panels().inspector;
        let (looping, play) = (inspector.looping, inspector.play);
        tab.click(looping);
        tab.paused_at(1.0);

        tab.click(play);

        tab.until("the clip never started again", |state| {
            state.playing && state.time < 0.5
        });
    }

    #[test]
    fn play_away_from_the_end_carries_on_from_where_it_is() {
        let mut tab = walking_tab();
        let inspector = &tab.panels().inspector;
        let (looping, play) = (inspector.looping, inspector.play);
        tab.click(looping);
        tab.paused_at(4.0);

        tab.click(play);

        tab.until("the clip never played", |state| state.playing);
        assert!(tab.state().time >= 4.0);
    }

    #[test]
    fn a_step_pauses_and_moves_one_frame_from_the_frame_at_the_time() {
        let mut tab = walking_tab();
        tab.until_moving();
        let inspector = &tab.panels().inspector;
        let (back, forward) = (inspector.step_back, inspector.step_forward);
        tab.scrub(2.01);
        tab.until("the scrub never came back", |state| !state.playing);

        tab.click(forward);
        let next = time_of_frame(61, 30.0);
        tab.until("the step forward never landed", |state| state.time == next);
        assert!(!tab.state().playing);

        tab.click(back);
        let back_again = time_of_frame(60, 30.0);
        tab.until("the step back never landed", |state| {
            state.time == back_again
        });
    }

    #[test]
    fn a_step_while_playing_pauses() {
        let mut tab = walking_tab();
        tab.until_moving();
        let forward = tab.panels().inspector.step_forward;

        tab.click(forward);

        tab.until("the step never paused", |state| !state.playing);
    }

    #[test]
    fn a_step_stays_inside_the_clip() {
        let mut tab = walking_tab();
        let inspector = &tab.panels().inspector;
        let (back, forward) = (inspector.step_back, inspector.step_forward);

        tab.paused_at(0.0);
        tab.click(back);
        tab.f.frame();
        tab.f.frame();
        assert_eq!(tab.state().time, 0.0);

        tab.paused_at(10.0);
        tab.click(forward);
        tab.f.frame();
        tab.f.frame();
        assert_eq!(tab.state().time, 10.0);
    }

    #[test]
    fn a_selection_is_written_back_to_the_timeline_and_stays_in_memory() {
        let mut tab = walking_tab();
        let on_disk = tab.clip_bytes();

        tab.select(Some((3, 9)));

        assert_eq!(tab.state().selection, Some((3, 9)));
        assert_eq!(tab.timeline().selection, Some((3, 9)));
        assert!(tab.clean());
        assert_eq!(tab.clip_bytes(), on_disk);

        tab.select(None);

        assert_eq!(tab.state().selection, None);
        assert_eq!(tab.timeline().selection, None);
        assert!(tab.clean());
    }

    #[test]
    fn the_timeline_follows_the_clip_and_the_preview() {
        let mut tab = walking_tab();

        assert_eq!(tab.timeline().duration, 10.0);
        assert_eq!(tab.timeline().rate, Some(30.0));

        tab.until_moving();
        tab.f.frame();
        assert_eq!(tab.timeline().time, tab.state().time);
    }

    #[test]
    fn a_rate_button_sets_the_rate_and_is_the_one_selected() {
        let mut tab = walking_tab();
        let rates = tab.panels().inspector.rates;
        tab.f.frame();
        assert!(tab.selected(rates[2]));

        tab.click(rates[3]);
        tab.f.frame();

        assert_eq!(tab.state().rate, 2.0);
        assert!(tab.selected(rates[3]));
        assert!(!tab.selected(rates[2]));
        tab.until_previewed("the rate never reached the preview", |preview| {
            preview.rate == 2.0
        });
        assert!(tab.clean());
    }

    #[test]
    fn loop_turns_off_and_on_and_shows_it() {
        let mut tab = walking_tab();
        let looping = tab.panels().inspector.looping;
        tab.f.frame();
        assert!(tab.selected(looping));

        tab.click(looping);
        tab.f.frame();

        assert!(!tab.state().looping);
        assert!(!tab.selected(looping));
        tab.until_previewed("the loop flag never reached the preview", |preview| {
            !preview.looping
        });
    }

    #[test]
    fn the_root_motion_checkbox_reaches_the_document_and_the_preview() {
        let mut tab = walking_tab();
        let root_motion = tab.panels().inspector.root_motion;
        assert!(!tab.checked(root_motion));

        tab.click(root_motion);

        assert!(tab.state().root_motion);
        tab.until_previewed("root motion never reached the preview", |preview| {
            preview.root_motion
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
        tab.until_previewed("the gizmo flag never reached the preview", |preview| {
            !preview.skeleton_gizmo
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
    fn a_second_clip_clears_the_timelines_selection() {
        let mut f = panels_fixture();
        let walk = write_clip(&mut f, "content/hero/walk.gasset", &clip_of(10));
        let run = write_clip(&mut f, "content/hero/run.gasset", &clip_of(20));
        let mut tab = tab_of(f, &walk);
        tab.select(Some((3, 9)));

        open_clip(&mut tab.f, &run);
        tab.f.frame();

        assert_eq!(tab.timeline().selection, None);
        assert_eq!(tab.timeline().duration, 20.0);
    }

    #[test]
    fn the_tab_stays_clean_through_every_control() {
        let mut f = panels_fixture();
        write_scene(&mut f, "content/world/level.gasset", "level");
        let walk = write_clip(&mut f, "content/hero/walk.gasset", &clip_of(10));
        let mut tab = tab_of(f, &walk);
        let on_disk = tab.clip_bytes();
        let left = &tab.panels().left;
        let (skeleton, button) = (left.skeleton, tab.parent(left.scene));
        let inspector = &tab.panels().inspector;
        let buttons = [
            inspector.play,
            inspector.step_forward,
            inspector.step_back,
            inspector.looping,
            inspector.rates[0],
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
        tab.select(Some((1, 2)));
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
}

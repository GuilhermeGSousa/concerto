//! The transport row and timeline that play a document's clip in its preview: play, step, loop and speed.
use concerto_app::{App, plugins::Plugin, schedule_groups::Update};
use concerto_ecs::{
    Component, Entity, IntoSystemConfig, Query, Res,
    command::{CommandQueue, EntityCommandQueue},
    events::event_reader::EventReader,
    signal::{
        On,
        listener::{IntoListener, Listener},
    },
};
use concerto_editor::{
    fonts::{glyph, icon},
    hosted::{Hosted, HostedEventsAppExt},
    preview::PreviewSignals,
    timeline::{Timeline, TimelineScrubbed, frame_at, time_of_frame},
};
use concerto_ui::elements::{Listening, Pressable, Text, prelude::*};
use concerto_ui::{
    interaction::{UIClick, UIDisabled, UIInteractionStyle},
    node::UIRect,
    text::UIText,
    theme::{ButtonVariant, UITheme},
    transform::UIValue,
};
use concerto_window::input::MouseButton;

use crate::preview::{ClipPreviewProgress, SeekClip, SetClipTransport};

pub(crate) const SPEEDS: [(f32, &str); 4] =
    [(0.25, "0.25×"), (0.5, "0.5×"), (1.0, "1×"), (2.0, "2×")];

/// Where a document's clip is and how it plays; the owner sets `duration` and `rate`, and the transport keeps the rest.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct ClipTransport {
    pub duration: f32,
    pub rate: Option<f32>,
    pub time: f32,
    pub playing: bool,
    pub looping: bool,
    pub speed: f32,
}

impl Default for ClipTransport {
    fn default() -> Self {
        Self {
            duration: 0.0,
            rate: None,
            time: 0.0,
            playing: true,
            looping: true,
            speed: 1.0,
        }
    }
}

/// Disables every button of a document's transport while it is on the document.
#[derive(Component)]
pub struct TransportDisabled;

/// On a [`Timeline`], has the transport write its duration, rate and time from `document`'s [`ClipTransport`].
#[derive(Component)]
pub struct TransportTimeline {
    pub document: Entity,
}

/// The buttons of a transport row, on the row, and the document they drive.
#[derive(Component, Clone, Copy, Debug)]
pub struct TransportRow {
    pub document: Entity,
    pub step_back: Entity,
    pub play: Entity,
    pub step_forward: Entity,
    pub looping: Entity,
    pub speeds: [Entity; SPEEDS.len()],
}

fn transport_signal(transport: &ClipTransport) -> SetClipTransport {
    SetClipTransport {
        playing: transport.playing,
        looping: transport.looping,
        rate: transport.speed,
    }
}

fn pause_at(transport: &ClipTransport, document: Entity, time: f32, previews: &mut PreviewSignals) {
    previews.trigger(
        document,
        SetClipTransport {
            playing: false,
            ..transport_signal(transport)
        },
    );
    previews.trigger(document, SeekClip(time));
}

fn at_the_end(transport: &ClipTransport) -> bool {
    let duration = transport.duration;
    let last_frame = transport
        .rate
        .map(|rate| time_of_frame(frame_at(duration, rate), rate));
    transport.time >= last_frame.unwrap_or(duration).min(duration)
}

fn toggle_playing(document: Entity) -> Listener<UIClick> {
    (move |on: On<UIClick>, transports: Query<&ClipTransport>, mut previews: PreviewSignals| {
        if on.signal().button != MouseButton::Left {
            return;
        }
        let Some(transport) = transports.get_entity(document) else {
            return;
        };
        let playing = !transport.playing;
        if playing && !transport.looping && at_the_end(transport) {
            previews.trigger(document, SeekClip(0.0));
        }
        previews.trigger(
            document,
            SetClipTransport {
                playing,
                ..transport_signal(transport)
            },
        );
    })
    .into_listener()
}

fn step_frame(document: Entity, frames: i32) -> Listener<UIClick> {
    (move |on: On<UIClick>, transports: Query<&ClipTransport>, mut previews: PreviewSignals| {
        if on.signal().button != MouseButton::Left {
            return;
        }
        let Some(transport) = transports.get_entity(document) else {
            return;
        };
        let Some(rate) = transport.rate else {
            return;
        };
        let frame = frame_at(transport.time, rate).saturating_add_signed(frames);
        let time = time_of_frame(frame, rate).min(transport.duration);
        pause_at(transport, document, time, &mut previews);
    })
    .into_listener()
}

fn toggle_looping(document: Entity) -> Listener<UIClick> {
    (move |on: On<UIClick>, transports: Query<&mut ClipTransport>, mut previews: PreviewSignals| {
        if on.signal().button != MouseButton::Left {
            return;
        }
        if let Some(mut transport) = transports.get_entity(document) {
            transport.looping = !transport.looping;
            previews.trigger(document, transport_signal(&transport));
        }
    })
    .into_listener()
}

fn set_speed(document: Entity, speed: f32) -> Listener<UIClick> {
    (move |on: On<UIClick>, transports: Query<&mut ClipTransport>, mut previews: PreviewSignals| {
        if on.signal().button != MouseButton::Left {
            return;
        }
        if let Some(mut transport) = transports.get_entity(document) {
            transport.speed = speed;
            previews.trigger(document, transport_signal(&transport));
        }
    })
    .into_listener()
}

/// Pauses `document`'s preview at the time a [`Timeline`] is scrubbed to.
pub fn scrub_transport(document: Entity) -> Listener<TimelineScrubbed> {
    (move |on: On<TimelineScrubbed>,
           transports: Query<&ClipTransport>,
           mut previews: PreviewSignals| {
        if let Some(transport) = transports.get_entity(document) {
            pause_at(transport, document, on.signal().time, &mut previews);
        }
    })
    .into_listener()
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

/// Spawns under `parent` the transport of `document`'s clip: step back, play/pause, step forward, Loop, then the speeds.
pub fn spawn_transport(
    parent: &mut EntityCommandQueue,
    document: Entity,
    theme: &UITheme,
) -> TransportRow {
    let mut column = parent.spawn_child(theme.column().fixed());
    let mut controls = column.spawn_child(theme.row().gap(theme.spacing_xs).fixed());
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
    let mut speed_row = column.spawn_child(theme.row().gap(theme.spacing_xs).fixed());
    let speeds = SPEEDS.map(|(speed, label)| {
        speed_row
            .spawn_child(
                theme
                    .button(label)
                    .ghost()
                    .on_click(set_speed(document, speed)),
            )
            .entity()
    });
    let row = TransportRow {
        document,
        step_back,
        play,
        step_forward,
        looping,
        speeds,
    };
    column.insert(row);
    row
}

pub(crate) fn follow_transport_progress(
    mut progress: EventReader<Hosted<ClipPreviewProgress>>,
    transports: Query<&mut ClipTransport>,
) {
    for Hosted { document, event } in progress.read() {
        if let Some(mut transport) = transports.get_entity(*document)
            && (transport.time != event.time || transport.playing != event.playing)
        {
            transport.time = event.time;
            transport.playing = event.playing;
        }
    }
}

pub(crate) fn sync_transport_rows(
    rows: Query<&TransportRow>,
    transports: Query<(&ClipTransport, Option<&TransportDisabled>)>,
    texts: Query<&mut UIText>,
    styles: Query<&mut UIInteractionStyle>,
    disabled: Query<&UIDisabled>,
    theme: Res<UITheme>,
    mut cmd: CommandQueue,
) {
    let write_selected = |entity: Entity, selected: bool| {
        let wanted = theme.interaction(ButtonVariant::Ghost, selected);
        if let Some(mut style) = styles.get_entity(entity)
            && style.normal != wanted.normal
        {
            **style = wanted;
        }
    };
    for row in rows.iter() {
        let Some((transport, off)) = transports.get_entity(row.document) else {
            continue;
        };
        let mark = if transport.playing {
            glyph::PAUSE
        } else {
            glyph::PLAY
        }
        .to_string();
        if let Some(mut text) = texts.get_entity(row.play)
            && text.text != mark
        {
            text.text = mark;
        }
        write_selected(row.looping, transport.looping);
        for (button, (speed, _)) in row.speeds.into_iter().zip(SPEEDS) {
            write_selected(button, speed == transport.speed);
        }

        let no_clip = off.is_some() || transport.duration <= 0.0;
        let no_frames = no_clip || transport.rate.is_none();
        let buttons = [
            (row.step_back, no_frames),
            (row.step_forward, no_frames),
            (row.play, no_clip),
            (row.looping, no_clip),
        ]
        .into_iter()
        .chain(row.speeds.map(|speed| (speed, no_clip)));
        for (button, off) in buttons {
            match (off, disabled.get_entity(button).is_some()) {
                (true, false) => cmd.insert(UIDisabled, button),
                (false, true) => cmd.remove::<UIDisabled>(button),
                _ => {}
            }
        }
    }
}

pub(crate) fn sync_transport_timelines(
    timelines: Query<(&TransportTimeline, &mut Timeline)>,
    transports: Query<&ClipTransport>,
) {
    for (follows, mut timeline) in timelines.iter() {
        let Some(transport) = transports.get_entity(follows.document) else {
            continue;
        };
        if timeline.duration != transport.duration {
            timeline.duration = transport.duration;
        }
        if timeline.rate != transport.rate {
            timeline.rate = transport.rate;
        }
        if timeline.time != transport.time {
            timeline.time = transport.time;
        }
    }
}

/// Copies each preview's [`ClipPreviewProgress`] into its document's [`ClipTransport`], and keeps transport rows and timelines in step with it.
pub struct TransportPlugin;

impl Plugin for TransportPlugin {
    fn build(&self, app: &mut App) {
        app.forward_hosted_event::<ClipPreviewProgress>()
            .add_system(
                Update,
                (
                    follow_transport_progress,
                    (sync_transport_rows, sync_transport_timelines),
                )
                    .chain(),
            );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ClipFacts,
        preview::{ClipPlayback, ClipPreviewProgress, ShowClip, clip_playback},
        tests::clip_of,
    };
    use concerto_animation::clip::{AnimationChanelOutput, AnimationChannel, AnimationClip};
    use concerto_ecs::{
        IntoSystem, System, With, World, events::event_channel::EventChannel, system::BoxedSystem,
    };
    use concerto_editor::{
        asset_editor::{AssetEditor, AssetEditorRegistry, PreviewSpec},
        hosted::{
            Hosted, HostedApp,
            testing::{SceneFixture, forwarding},
        },
        timeline::{TimelineScrubbed, time_of_frame},
    };
    use concerto_foundation::assets::Asset;
    use concerto_ui::{interaction::UIInteractionStyle, node::UINode, text::UIText};
    use std::sync::Arc;
    use uuid::Uuid;

    #[derive(Default)]
    struct PlaybackEditor;

    impl AssetEditor for PlaybackEditor {
        fn build(&self, editor: &mut EntityCommandQueue) {
            editor.insert(ClipTransport::default());
        }

        fn preview(&self) -> Option<PreviewSpec> {
            Some(PreviewSpec {
                build: clip_playback,
            })
        }
    }

    struct Transported {
        f: SceneFixture,
        document: Entity,
        row: TransportRow,
        timeline: Entity,
    }

    fn run_once(f: &mut SceneFixture, mut system: BoxedSystem) {
        system.initialize(&mut f.world);
        system.run_and_apply((), &mut f.world);
    }

    fn transported(clip: AnimationClip) -> Transported {
        let mut f = SceneFixture::new();
        f.world
            .get_resource_mut::<AssetEditorRegistry>()
            .unwrap()
            .register::<AnimationClip, PlaybackEditor>()
            .unwrap();
        f.world
            .insert_resource(EventChannel::<Hosted<ClipPreviewProgress>>::new());
        f.add_delivery(forwarding::<ClipPreviewProgress>());
        f.add_delivery(follow_transport_progress.into_boxed_system());
        f.add_delivery(sync_transport_rows.into_boxed_system());
        f.add_delivery(sync_transport_timelines.into_boxed_system());
        let document = f.request_kind(AnimationClip::name());
        f.frames_until("the preview was never built", |f| {
            f.world
                .get_component_for_entity::<HostedApp>(document)
                .is_some()
        });
        let facts = ClipFacts::of(&clip);
        *f.world
            .get_component_for_entity_mut::<ClipTransport>(document)
            .unwrap() = ClipTransport {
            duration: facts.duration,
            rate: facts.rate,
            ..ClipTransport::default()
        };
        let host = f.world.spawn(UINode::default());
        let clip = Arc::new(clip);
        run_once(
            &mut f,
            (move |theme: Res<UITheme>, mut previews: PreviewSignals, mut cmd: CommandQueue| {
                spawn_transport(&mut cmd.entity(host), document, &theme);
                cmd.spawn((
                    UINode::default(),
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
                previews.trigger(document, ShowClip(Arc::clone(&clip)));
            })
            .into_boxed_system(),
        );
        f.frame();
        let row = *f
            .world
            .query::<&TransportRow, ()>()
            .iter(&mut f.world)
            .next()
            .expect("the row was spawned");
        let timeline = f
            .world
            .query::<Entity, With<TransportTimeline>>()
            .iter(&mut f.world)
            .next()
            .expect("the timeline was spawned");
        Transported {
            f,
            document,
            row,
            timeline,
        }
    }

    fn walking() -> Transported {
        transported(clip_of(10))
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

    impl Transported {
        fn world(&self) -> &World {
            &self.f.world
        }

        fn transport(&self) -> &ClipTransport {
            self.world()
                .get_component_for_entity::<ClipTransport>(self.document)
                .unwrap()
        }

        fn timeline(&self) -> &Timeline {
            self.world()
                .get_component_for_entity::<Timeline>(self.timeline)
                .unwrap()
        }

        fn preview(&self) -> &ClipPlayback {
            self.f
                .scene_world(self.document)
                .get_resource::<ClipPlayback>()
                .unwrap()
        }

        fn text(&self, entity: Entity) -> &UIText {
            self.world()
                .get_component_for_entity::<UIText>(entity)
                .unwrap()
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

        fn buttons(&self) -> Vec<Entity> {
            let row = self.row;
            let mut buttons = vec![row.step_back, row.play, row.step_forward, row.looping];
            buttons.extend(row.speeds);
            buttons
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
            let timeline = self.timeline;
            self.f.world.trigger_on(timeline, TimelineScrubbed { time });
        }

        fn until(&mut self, what: &str, done: impl Fn(&ClipTransport) -> bool) {
            let document = self.document;
            self.f.frames_until(what, |f| {
                done(
                    f.world
                        .get_component_for_entity::<ClipTransport>(document)
                        .unwrap(),
                )
            });
        }

        fn until_previewed(&mut self, what: &str, done: impl Fn(&ClipPlayback) -> bool) {
            let document = self.document;
            self.f.frames_until(what, |f| {
                done(
                    f.scene_world(document)
                        .get_resource::<ClipPlayback>()
                        .unwrap(),
                )
            });
        }

        fn until_moving(&mut self) {
            self.until("the head never moved", |transport| transport.time > 0.0);
        }

        fn paused_at(&mut self, time: f32) {
            self.scrub(time);
            self.until("the seek never came back", |transport| {
                !transport.playing && transport.time == time
            });
            self.f.frame();
        }

        fn clean(&mut self) -> bool {
            !self.f.unsaved(self.document)
        }
    }

    #[test]
    fn progress_updates_the_clip_transport() {
        let mut world = World::new();
        world.insert_resource(EventChannel::<Hosted<ClipPreviewProgress>>::new());
        let document = world.spawn(ClipTransport::default());
        let other = world.spawn(ClipTransport::default());
        world
            .get_resource_mut::<EventChannel<Hosted<ClipPreviewProgress>>>()
            .unwrap()
            .push_event(Hosted {
                document,
                event: ClipPreviewProgress {
                    time: 2.5,
                    playing: false,
                },
            });
        let mut follow = follow_transport_progress.into_system();
        follow.initialize(&mut world);

        follow.run_and_apply((), &mut world);

        let followed = world
            .get_component_for_entity::<ClipTransport>(document)
            .unwrap();
        assert_eq!(followed.time, 2.5);
        assert!(!followed.playing);
        let untouched = world
            .get_component_for_entity::<ClipTransport>(other)
            .unwrap();
        assert_eq!(untouched.time, 0.0);
        assert!(untouched.playing);
    }

    #[test]
    fn a_transport_timeline_follows_the_transport_and_the_preview() {
        let mut playing = walking();

        assert_eq!(playing.timeline().duration, 10.0);
        assert_eq!(playing.timeline().rate, Some(30.0));

        playing.until_moving();
        playing.f.frame();
        assert_eq!(playing.timeline().time, playing.transport().time);
    }

    #[test]
    fn play_pauses_and_plays_again() {
        let mut playing = walking();
        playing.until_moving();
        let play = playing.row.play;
        assert_eq!(playing.text(play).text, glyph::PAUSE.to_string());

        playing.click(play);
        playing.until("the clip never paused", |transport| !transport.playing);
        playing.f.frame();
        assert_eq!(playing.text(play).text, glyph::PLAY.to_string());

        playing.click(play);
        playing.until("the clip never played again", |transport| transport.playing);
    }

    #[test]
    fn a_scrub_pauses_seeks_and_the_clip_stays_paused_after_it() {
        let mut playing = walking();
        playing.until_moving();

        playing.paused_at(4.0);
        for _ in 0..5 {
            playing.f.frame();
        }

        assert!(!playing.transport().playing);
        assert_eq!(playing.transport().time, 4.0);
        assert_eq!(playing.timeline().time, 4.0);
        assert_eq!(playing.preview().time, 4.0);
    }

    #[test]
    fn play_at_the_end_of_a_clip_that_does_not_loop_starts_again_from_zero() {
        let mut playing = walking();
        let (looping, play) = (playing.row.looping, playing.row.play);
        playing.click(looping);
        playing.paused_at(10.0);

        playing.click(play);

        playing.until("the clip never started again", |transport| {
            transport.playing && transport.time < 5.0
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
        let mut playing = transported(clip);
        let (looping, play) = (playing.row.looping, playing.row.play);
        playing.click(looping);
        playing.paused_at(1.0);

        playing.click(play);

        playing.until("the clip never started again", |transport| {
            transport.playing && transport.time < 0.5
        });
    }

    #[test]
    fn play_in_the_last_frame_of_a_sparse_clip_carries_on_to_the_end() {
        let times: Vec<f32> = (0..=3).map(|key| key as f32).collect();
        let outputs = vec![glam::Vec3::ZERO; times.len()];
        let mut clip = AnimationClip::default();
        clip.add_channel(
            Uuid::from_u128(1),
            AnimationChannel::new(times, AnimationChanelOutput::Translation(outputs)),
        );
        let mut playing = transported(clip);
        let (looping, play) = (playing.row.looping, playing.row.play);
        playing.click(looping);
        playing.paused_at(2.6);

        playing.click(play);

        playing.until("the clip never played", |transport| transport.playing);
        assert!(playing.transport().time >= 2.6);
    }

    #[test]
    fn play_away_from_the_end_carries_on_from_where_it_is() {
        let mut playing = walking();
        let (looping, play) = (playing.row.looping, playing.row.play);
        playing.click(looping);
        playing.paused_at(4.0);

        playing.click(play);

        playing.until("the clip never played", |transport| transport.playing);
        assert!(playing.transport().time >= 4.0);
    }

    #[test]
    fn a_step_pauses_and_moves_one_frame_from_the_frame_at_the_time() {
        let mut playing = walking();
        playing.until_moving();
        let (back, forward) = (playing.row.step_back, playing.row.step_forward);
        playing.scrub(2.01);
        playing.until("the scrub never came back", |transport| !transport.playing);

        playing.click(forward);
        let next = time_of_frame(61, 30.0);
        playing.until("the step forward never landed", |transport| {
            transport.time == next
        });
        assert!(!playing.transport().playing);

        playing.click(back);
        let back_again = time_of_frame(60, 30.0);
        playing.until("the step back never landed", |transport| {
            transport.time == back_again
        });
    }

    #[test]
    fn a_step_while_playing_pauses() {
        let mut playing = walking();
        playing.until_moving();
        let forward = playing.row.step_forward;

        playing.click(forward);

        playing.until("the step never paused", |transport| !transport.playing);
    }

    #[test]
    fn a_step_stays_inside_the_clip() {
        let mut playing = walking();
        let (back, forward) = (playing.row.step_back, playing.row.step_forward);

        playing.paused_at(0.0);
        playing.click(back);
        playing.f.frame();
        playing.f.frame();
        assert_eq!(playing.transport().time, 0.0);

        playing.paused_at(10.0);
        playing.click(forward);
        playing.f.frame();
        playing.f.frame();
        assert_eq!(playing.transport().time, 10.0);
    }

    #[test]
    fn a_speed_button_sets_the_speed_and_is_the_one_selected() {
        let mut playing = walking();
        let speeds = playing.row.speeds;
        playing.f.frame();
        assert!(playing.selected(speeds[2]));

        playing.click(speeds[3]);
        playing.f.frame();

        assert_eq!(playing.transport().speed, 2.0);
        assert!(playing.selected(speeds[3]));
        assert!(!playing.selected(speeds[2]));
        playing.until_previewed("the rate never reached the preview", |preview| {
            preview.rate == 2.0
        });
        assert!(playing.clean());
    }

    #[test]
    fn loop_turns_off_and_on_and_shows_it() {
        let mut playing = walking();
        let looping = playing.row.looping;
        playing.f.frame();
        assert!(playing.selected(looping));

        playing.click(looping);
        playing.f.frame();

        assert!(!playing.transport().looping);
        assert!(!playing.selected(looping));
        playing.until_previewed("the loop flag never reached the preview", |preview| {
            !preview.looping
        });
    }

    #[test]
    fn a_clip_with_no_rate_disables_only_the_step_buttons() {
        let mut playing = transported(single_key_clip());
        playing.f.frame();
        let row = playing.row;

        assert!(playing.disabled(row.step_back));
        assert!(playing.disabled(row.step_forward));
        assert!(!playing.disabled(row.play));
        assert!(!playing.disabled(row.looping));
        assert!(row.speeds.iter().all(|speed| !playing.disabled(*speed)));
    }

    #[test]
    fn a_clip_with_no_duration_disables_the_whole_row() {
        let mut playing = transported(AnimationClip::default());
        playing.f.frame();

        for button in playing.buttons() {
            assert!(playing.disabled(button));
        }
    }

    #[test]
    fn a_clip_with_frames_leaves_the_row_enabled() {
        let mut playing = walking();
        playing.f.frame();

        for button in playing.buttons() {
            assert!(!playing.disabled(button));
        }
    }

    #[test]
    fn transport_disabled_disables_the_row_until_it_is_removed() {
        let mut playing = walking();
        let document = playing.document;

        playing.f.world.insert(TransportDisabled, document);
        playing.f.frame();
        for button in playing.buttons() {
            assert!(playing.disabled(button));
        }

        playing
            .f
            .world
            .remove_component::<TransportDisabled>(document);
        playing.f.frame();
        for button in playing.buttons() {
            assert!(!playing.disabled(button));
        }
    }
}

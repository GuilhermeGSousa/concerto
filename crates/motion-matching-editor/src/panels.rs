use concerto_animation::clip::AnimationClip;
use concerto_color::Color;
use concerto_ecs::{
    Added, Component, Entity, Query, Res, With,
    command::{CommandQueue, EntityCommandQueue},
    reflect::AppTypeRegistry,
    signal::{
        On,
        listener::{IntoListener, Listener},
    },
};
use concerto_editor::{
    asset_editor::{EditorDocument, EditorHosts, Unsaved},
    asset_picker::{
        AssetPickerMenu, asset_name, open_asset_menu, picker_button, picker_caret, picker_label,
    },
    clip_editor::ClipFacts,
    fonts::{MEDIUM, glyph, icon},
    hosted::{EditTarget, EditWorld, EditWorldSignals, HostedApp},
    preview::PreviewFailed,
    timeline::{Timeline, TimelineSelected, frame_at, time_of_frame},
    tools::selection::{Select, Selection},
};
use concerto_foundation::assets::{Asset, AssetId, handle::AssetHandle, registry::AssetRegistry};
use concerto_mesh::Skeleton;
use concerto_motion_matching::{MotionEntry, TimeRange};
use concerto_ui::elements::{Label, Listening, Pressable, Text, prelude::*};
use concerto_ui::{
    anchor::UIAnchoredPanel,
    interaction::{Interactable, UIClick, UIDisabled, UIInteractionStyle},
    node::{FlexDirection, UILayout, UINode, UIRect},
    scroll::{SCROLLBAR_WIDTH, UIScrollArea},
    text::UIText,
    theme::{ButtonVariant, UITheme},
    transform::UIValue,
};
use concerto_window::input::MouseButton;

use crate::{
    build::{BakedStats, BuildRequested, build_refusal, progress_line},
    document::DefinitionDocument,
    facts::EntryFacts,
    menu::{NameMenu, close_menu, open_name_menu},
    playback::scrub_entry,
    preview::{
        AddChannel, ChannelKind, ChannelOrder, DatabaseSettings, RemoveChannel, channel_types,
    },
};

const TITLE_HEIGHT: f32 = 16.0;
const ROW_PADDING: f32 = 4.0;
const STAT_HEIGHT: f32 = 16.0;
const STATS_MAX_HEIGHT: f32 = 160.0;

#[derive(Component)]
pub(crate) struct DefinitionPanels {
    view: Entity,
    content: Entity,
    skeleton: Entity,
    skeleton_button: Entity,
    skeleton_error: Entity,
    entries: Entity,
    add_entry: Entity,
    entry_rows: Vec<EntryWidgets>,
    settings: Entity,
    channels: Entity,
    add_channel: Entity,
    channel_rows: Vec<ChannelWidgets>,
    pub(crate) timeline: Entity,
    pub(crate) build: Entity,
    pub(crate) note: Entity,
    pub(crate) baked: Entity,
    pub(crate) stats_view: Entity,
    pub(crate) stats: Entity,
    shown_stats: Option<BakedStats>,
    status: Entity,
}

struct EntryWidgets {
    row: Entity,
    name: Entity,
    range: Entity,
    driven: Entity,
    remove: Entity,
}

struct ChannelWidgets {
    entity: Option<Entity>,
    name: String,
    row: Entity,
}

struct InspectorPanel {
    build: Entity,
    note: Entity,
    baked: Entity,
    stats_view: Entity,
    stats: Entity,
    status: Entity,
}

struct LeftPanel {
    view: Entity,
    content: Entity,
    skeleton: Entity,
    skeleton_button: Entity,
    skeleton_error: Entity,
    entries: Entity,
    add_entry: Entity,
    settings: Entity,
    channels: Entity,
    add_channel: Entity,
}

fn selected_facts(state: &DefinitionDocument) -> Option<&ClipFacts> {
    let entry = state.entries.get(state.selected_entry?)?;
    state.facts_of(entry.clip.id())?.read.as_ref().ok()
}

fn range_of(first: u32, last: u32, rate: f32, duration: f32) -> TimeRange {
    TimeRange {
        start: time_of_frame(first, rate).min(duration),
        end: time_of_frame(last, rate).min(duration),
    }
}

fn select_range(document: Entity) -> Listener<TimelineSelected> {
    (move |on: On<TimelineSelected>,
           documents: Query<(&mut DefinitionDocument, Option<&PreviewFailed>)>,
           mut cmd: CommandQueue| {
        let Some((mut state, None)) = documents.get_entity(document) else {
            return;
        };
        let Some(facts) = selected_facts(&state) else {
            return;
        };
        let Some(rate) = facts.rate else {
            return;
        };
        let duration = facts.duration;
        let range = on
            .signal()
            .range
            .map(|(first, last)| range_of(first, last, rate, duration));
        let Some(index) = state.selected_entry else {
            return;
        };
        if state.entries[index].range != range {
            state.entries[index].range = range;
            cmd.insert(Unsaved, document);
        }
    })
    .into_listener()
}

fn select_entry(document: Entity, index: usize) -> Listener<UIClick> {
    (move |on: On<UIClick>, documents: Query<&mut DefinitionDocument>| {
        if on.signal().button != MouseButton::Left {
            return;
        }
        if let Some(mut state) = documents.get_entity(document)
            && state.selected_entry != Some(index)
        {
            state.selected_entry = Some(index);
        }
    })
    .into_listener()
}

fn selection_after_removal(selected: usize, removed: usize, left: usize) -> Option<usize> {
    if left == 0 {
        None
    } else if selected > removed {
        Some(selected - 1)
    } else {
        Some(selected.min(left - 1))
    }
}

fn remove_entry(document: Entity, index: usize) -> Listener<UIClick> {
    (move |on: On<UIClick>, documents: Query<&mut DefinitionDocument>, mut cmd: CommandQueue| {
        if on.signal().button != MouseButton::Left {
            return;
        }
        let Some(mut state) = documents.get_entity(document) else {
            return;
        };
        if index < state.entries.len() {
            state.entries.remove(index);
            let left = state.entries.len();
            state.selected_entry = state
                .selected_entry
                .and_then(|selected| selection_after_removal(selected, index, left));
            cmd.insert(Unsaved, document);
        }
    })
    .into_listener()
}

fn toggle_skeleton_picker(document: Entity) -> Listener<UIClick> {
    (move |on: On<UIClick>,
           documents: Query<&DefinitionDocument>,
           menus: Query<(Entity, &mut AssetPickerMenu, &mut UIAnchoredPanel)>,
           assets: Res<AssetRegistry>,
           theme: Res<UITheme>,
           mut cmd: CommandQueue| {
        if on.signal().button != MouseButton::Left {
            return;
        }
        let current = documents
            .get_entity(document)
            .map(|state| state.skeleton.id());
        open_asset_menu(
            on.entity(),
            Skeleton::name(),
            None,
            current,
            pick_skeleton(document),
            &menus,
            &assets,
            &theme,
            &mut cmd,
        );
    })
    .into_listener()
}

fn pick_skeleton(document: Entity) -> impl Fn(AssetId) -> Listener<UIClick> {
    move |id| {
        (move |on: On<UIClick>,
               menus: Query<&mut UIAnchoredPanel, With<AssetPickerMenu>>,
               documents: Query<&mut DefinitionDocument>,
               mut cmd: CommandQueue| {
            if on.signal().button != MouseButton::Left {
                return;
            }
            for mut panel in menus.iter() {
                panel.open = false;
            }
            if let Some(mut state) = documents.get_entity(document)
                && state.skeleton.id() != id
            {
                state.skeleton = AssetHandle::weak(id);
                cmd.insert(Unsaved, document);
            }
        })
        .into_listener()
    }
}

fn toggle_clip_picker(document: Entity) -> Listener<UIClick> {
    (move |on: On<UIClick>,
           menus: Query<(Entity, &mut AssetPickerMenu, &mut UIAnchoredPanel)>,
           assets: Res<AssetRegistry>,
           theme: Res<UITheme>,
           mut cmd: CommandQueue| {
        if on.signal().button != MouseButton::Left {
            return;
        }
        open_asset_menu(
            on.entity(),
            AnimationClip::name(),
            None,
            None,
            pick_clip(document),
            &menus,
            &assets,
            &theme,
            &mut cmd,
        );
    })
    .into_listener()
}

fn pick_clip(document: Entity) -> impl Fn(AssetId) -> Listener<UIClick> {
    move |id| {
        (move |on: On<UIClick>,
               menus: Query<&mut UIAnchoredPanel, With<AssetPickerMenu>>,
               documents: Query<&mut DefinitionDocument>,
               mut cmd: CommandQueue| {
            if on.signal().button != MouseButton::Left {
                return;
            }
            for mut panel in menus.iter() {
                panel.open = false;
            }
            if let Some(mut state) = documents.get_entity(document) {
                state.entries.push(MotionEntry {
                    clip: AssetHandle::weak(id),
                    range: None,
                });
                state.selected_entry = Some(state.entries.len() - 1);
                cmd.insert(Unsaved, document);
            }
        })
        .into_listener()
    }
}

fn select_settings(
    on: On<UIClick>,
    settings: EditWorld<Query<Entity, With<DatabaseSettings>>>,
    mut scene: EditWorldSignals,
) {
    if on.signal().button == MouseButton::Left {
        scene.trigger(Select(settings.iter().next()));
    }
}

fn select_channel(channel: Entity) -> Listener<UIClick> {
    (move |on: On<UIClick>, mut scene: EditWorldSignals| {
        if on.signal().button == MouseButton::Left {
            scene.trigger(Select(Some(channel)));
        }
    })
    .into_listener()
}

fn remove_channel(channel: Entity) -> Listener<UIClick> {
    (move |on: On<UIClick>, mut scene: EditWorldSignals| {
        if on.signal().button == MouseButton::Left {
            scene.trigger(RemoveChannel(channel));
        }
    })
    .into_listener()
}

fn toggle_channel_menu(
    on: On<UIClick>,
    types: EditWorld<Option<Res<AppTypeRegistry>>>,
    menus: Query<(Entity, &mut UIAnchoredPanel), With<NameMenu>>,
    theme: Res<UITheme>,
    mut cmd: CommandQueue,
) {
    if on.signal().button != MouseButton::Left {
        return;
    }
    let names = types
        .as_deref()
        .map_or_else(Vec::new, |types| channel_types(&types.read()));
    open_name_menu(
        on.entity(),
        &names,
        None,
        |name| pick_channel(name.to_owned()),
        &menus,
        &theme,
        &mut cmd,
    );
}

fn pick_channel(name: String) -> Listener<UIClick> {
    (move |on: On<UIClick>,
           menus: Query<&mut UIAnchoredPanel, With<NameMenu>>,
           mut scene: EditWorldSignals| {
        if on.signal().button != MouseButton::Left {
            return;
        }
        close_menu(&menus);
        scene.trigger(AddChannel(name.clone()));
    })
    .into_listener()
}

fn request_build(document: Entity) -> Listener<UIClick> {
    (move |on: On<UIClick>, mut cmd: CommandQueue| {
        if on.signal().button == MouseButton::Left {
            cmd.insert(BuildRequested, document);
        }
    })
    .into_listener()
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

fn icon_button(
    theme: &UITheme,
    mark: char,
    click: Listener<UIClick>,
) -> (Listening<Pressable, UIClick>, Text) {
    let size = theme.font_size_md;
    let vertical = (theme.control_height - theme.line_height(size)) / 2.0;
    let horizontal = (theme.control_height - size) / 2.0;
    (
        theme
            .pressable()
            .size(
                UIValue::Px(theme.control_height),
                UIValue::Px(theme.control_height),
            )
            .padding(UIRect::axes(vertical, horizontal))
            .on_click(click),
        icon(theme, mark, size).muted(),
    )
}

fn section(
    content: &mut EntityCommandQueue,
    text: &str,
    add: Listener<UIClick>,
    theme: &UITheme,
) -> (Entity, Entity) {
    let mut header = content.spawn_child(theme.row().fixed());
    header.add_child(title(theme, text).grow());
    let add = header
        .spawn_child(icon_button(theme, glyph::PLUS, add))
        .entity();
    let list = content
        .spawn_child(theme.column().fixed().gap(theme.spacing_xs))
        .entity();
    (add, list)
}

fn build_left(host: &mut EntityCommandQueue, document: Entity, theme: &UITheme) -> LeftPanel {
    let mut view = host.spawn_child((
        UINode::default()
            .with_flex_grow(1.0)
            .with_flex_direction(FlexDirection::Column)
            .clipped(),
        Interactable,
    ));
    let view_entity = view.entity();
    let mut content = view.spawn_child(theme.column().fixed().gap(theme.spacing_sm).padding(
        UIRect {
            right: SCROLLBAR_WIDTH + theme.spacing_xs,
            ..UIRect::all(0.0)
        },
    ));
    let content_entity = content.entity();

    content.add_child(title(theme, "Skeleton"));
    let mut row = content.spawn_child(theme.row().fixed());
    let mut button =
        row.spawn_child(picker_button(theme).on_click(toggle_skeleton_picker(document)));
    let skeleton_button = button.entity();
    let skeleton = button.spawn_child(picker_label(theme)).entity();
    button.add_child(picker_caret(theme));
    let skeleton_error = content
        .spawn_child(theme.label("").color(theme.error).hidden().fixed())
        .entity();

    let (add_entry, entries) =
        section(&mut content, "Entries", toggle_clip_picker(document), theme);

    let mut settings = content.spawn_child(
        theme
            .pressable()
            .row()
            .height(UIValue::Px(theme.control_height))
            .padding(UIRect::axes(0.0, ROW_PADDING))
            .on_click(select_settings),
    );
    let settings_entity = settings.entity();
    settings.add_child(theme.label("Settings").font_size(theme.font_size_sm).grow());

    let (add_channel, channels) = section(
        &mut content,
        "Channels",
        toggle_channel_menu.into_listener(),
        theme,
    );

    view.insert(UIScrollArea {
        content: Some(content_entity),
        ..Default::default()
    });
    LeftPanel {
        view: view_entity,
        content: content_entity,
        skeleton,
        skeleton_button,
        skeleton_error,
        entries,
        add_entry,
        settings: settings_entity,
        channels,
        add_channel,
    }
}

fn spawn_entry_row(
    list: &mut EntityCommandQueue,
    document: Entity,
    index: usize,
    theme: &UITheme,
) -> EntryWidgets {
    let mut row = list.spawn_child(theme.row().fixed().gap(theme.spacing_xs));
    let mut select = row.spawn_child(
        theme
            .pressable()
            .column()
            .grow()
            .shrink(1.0)
            .min_width(UIValue::Px(0.0))
            .padding(UIRect::axes(2.0, ROW_PADDING))
            .on_click(select_entry(document, index)),
    );
    let select_entity = select.entity();
    let name = select
        .spawn_child(theme.label("").font_size(theme.font_size_sm).single_line())
        .entity();
    let mut facts = select.spawn_child(theme.row().fixed());
    let range = facts
        .spawn_child(
            theme
                .label("")
                .small()
                .muted()
                .single_line()
                .grow()
                .shrink(1.0)
                .min_width(UIValue::Px(0.0)),
        )
        .entity();
    let driven = facts
        .spawn_child(theme.label("").small().no_wrap())
        .entity();
    let remove = row
        .spawn_child(icon_button(
            theme,
            glyph::MINUS,
            remove_entry(document, index),
        ))
        .entity();
    EntryWidgets {
        row: select_entity,
        name,
        range,
        driven,
        remove,
    }
}

fn spawn_channel_row(
    list: &mut EntityCommandQueue,
    entity: Option<Entity>,
    name: String,
    theme: &UITheme,
) -> ChannelWidgets {
    let label = theme
        .label(name.as_str())
        .font_size(theme.font_size_sm)
        .single_line()
        .grow()
        .shrink(1.0)
        .min_width(UIValue::Px(0.0));
    let Some(channel) = entity else {
        let row = list.spawn_child(label).entity();
        return ChannelWidgets { entity, name, row };
    };
    let mut row = list.spawn_child(theme.row().fixed().gap(theme.spacing_xs));
    let mut select = row.spawn_child(
        theme
            .pressable()
            .row()
            .grow()
            .shrink(1.0)
            .min_width(UIValue::Px(0.0))
            .height(UIValue::Px(theme.control_height))
            .padding(UIRect::axes(0.0, ROW_PADDING))
            .on_click(select_channel(channel)),
    );
    let select_entity = select.entity();
    select.add_child(label);
    row.add_child(icon_button(theme, glyph::MINUS, remove_channel(channel)));
    ChannelWidgets {
        entity,
        name,
        row: select_entity,
    }
}

fn build_inspector(
    host: &mut EntityCommandQueue,
    document: Entity,
    theme: &UITheme,
) -> InspectorPanel {
    let mut panel = host.spawn_child(theme.column().gap(theme.spacing_sm));
    let build = panel
        .spawn_child(
            theme
                .button("Build")
                .fixed()
                .on_click(request_build(document)),
        )
        .entity();
    let note = panel
        .spawn_child(theme.label("").small().muted().hidden().fixed())
        .entity();
    let baked = panel
        .spawn_child(
            theme
                .label("")
                .font_size(theme.font_size_sm)
                .hidden()
                .fixed(),
        )
        .entity();
    let mut view = panel.spawn_child((theme.column().fixed().clipped().hidden(), Interactable));
    let stats_view = view.entity();
    let stats = view
        .spawn_child(theme.column().fixed().gap(0.0).padding(UIRect {
            right: SCROLLBAR_WIDTH + theme.spacing_xs,
            ..UIRect::all(0.0)
        }))
        .entity();
    view.insert(UIScrollArea {
        content: Some(stats),
        ..Default::default()
    });
    let status = panel.spawn_child(theme.label("").hidden().fixed()).entity();
    InspectorPanel {
        build,
        note,
        baked,
        stats_view,
        stats,
        status,
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
        select_range(document),
        scrub_entry(document),
    ))
    .entity()
}

pub(crate) fn build_definition_panels(
    documents: Query<(Entity, &EditorHosts), (With<DefinitionDocument>, Added<EditorHosts>)>,
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
        let inspector = build_inspector(&mut cmd.entity(inspector), document, &theme);
        let timeline = build_bottom(&mut cmd.entity(bottom), document, &theme);
        cmd.insert(
            DefinitionPanels {
                view: left.view,
                content: left.content,
                skeleton: left.skeleton,
                skeleton_button: left.skeleton_button,
                skeleton_error: left.skeleton_error,
                entries: left.entries,
                add_entry: left.add_entry,
                entry_rows: Vec::new(),
                settings: left.settings,
                channels: left.channels,
                add_channel: left.add_channel,
                channel_rows: Vec::new(),
                timeline,
                build: inspector.build,
                note: inspector.note,
                baked: inspector.baked,
                stats_view: inspector.stats_view,
                stats: inspector.stats,
                shown_stats: None,
                status: inspector.status,
            },
            document,
        );
    }
}

fn range_line(range: Option<TimeRange>) -> String {
    range.map_or_else(
        || "Whole clip".to_owned(),
        |range| format!("{:.2} to {:.2} s", range.start, range.end),
    )
}

fn driven_line(facts: Option<&EntryFacts>, skeleton: bool, theme: &UITheme) -> (String, Color) {
    let Some(facts) = facts else {
        return ("Reading…".into(), theme.text_muted);
    };
    let Ok(read) = &facts.read else {
        return ("Can't read".into(), theme.warning);
    };
    if !skeleton {
        return ("No skeleton".into(), theme.text_muted);
    }
    let color = if read.driven == read.clip_bones {
        theme.text
    } else {
        theme.warning
    };
    (format!("{} of {}", read.driven, read.skeleton_bones), color)
}

struct StatLine {
    name: String,
    value: String,
    warning: bool,
}

impl StatLine {
    fn new(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            value: value.into(),
            warning: false,
        }
    }
}

fn memory_line(bytes: usize) -> String {
    const KIB: f32 = 1024.0;
    let bytes = bytes as f32;
    if bytes < KIB * KIB {
        format!("{:.1} KiB", bytes / KIB)
    } else {
        format!("{:.1} MiB", bytes / (KIB * KIB))
    }
}

fn stat_lines(stats: &BakedStats, assets: &AssetRegistry) -> Vec<StatLine> {
    let mut lines = vec![
        StatLine::new("Frames", stats.frames.to_string()),
        StatLine::new("Memory", memory_line(stats.bytes)),
    ];
    for entry in &stats.entries {
        lines.push(StatLine {
            warning: entry.short,
            ..StatLine::new(asset_name(assets, entry.clip), entry.frames.to_string())
        });
        if entry.short {
            lines.push(StatLine {
                warning: true,
                ..StatLine::new(
                    format!("Shorter than the {:.2} s trajectory", stats.horizon),
                    "",
                )
            });
        }
    }
    for channel in &stats.channels {
        lines.push(StatLine::new(
            channel.name.as_str(),
            format!("{:.3} ± {:.3}", channel.mean, channel.deviation),
        ));
    }
    lines
}

fn spawn_stat_line(list: &mut EntityCommandQueue, line: &StatLine, theme: &UITheme) {
    let color = if line.warning {
        theme.warning
    } else {
        theme.text
    };
    let mut row = list.spawn_child(
        theme
            .row()
            .fixed()
            .height(UIValue::Px(STAT_HEIGHT))
            .gap(theme.spacing_xs),
    );
    row.add_child(
        theme
            .label(line.name.as_str())
            .small()
            .color(color)
            .single_line()
            .grow()
            .shrink(1.0)
            .min_width(UIValue::Px(0.0)),
    );
    row.add_child(
        theme
            .label(line.value.as_str())
            .small()
            .color(color)
            .no_wrap(),
    );
}

fn write_text(texts: &Query<&mut UIText>, entity: Entity, value: &str) {
    if let Some(mut text) = texts.get_entity(entity)
        && text.text != value
    {
        text.text = value.to_owned();
    }
}

fn write_color(texts: &Query<&mut UIText>, entity: Entity, color: Color) {
    if let Some(mut text) = texts.get_entity(entity)
        && text.color != color
    {
        text.color = color;
    }
}

fn write_line(texts: &Query<&mut UIText>, nodes: &Query<&mut UINode>, entity: Entity, value: &str) {
    write_text(texts, entity, value);
    if let Some(mut node) = nodes.get_entity(entity)
        && node.visible == value.is_empty()
    {
        node.visible = !value.is_empty();
    }
}

fn write_selected(
    styles: &Query<&mut UIInteractionStyle>,
    theme: &UITheme,
    entity: Entity,
    selected: bool,
) {
    let wanted = theme.interaction(ButtonVariant::Ghost, selected);
    if let Some(mut style) = styles.get_entity(entity)
        && style.normal != wanted.normal
    {
        **style = wanted;
    }
}

fn write_disabled(
    disabled: &Query<&UIDisabled>,
    cmd: &mut CommandQueue,
    entity: Entity,
    off: bool,
) {
    match (off, disabled.get_entity(entity).is_some()) {
        (true, false) => cmd.insert(UIDisabled, entity),
        (false, true) => cmd.remove::<UIDisabled>(entity),
        _ => {}
    }
}

fn channel_rows_match(rows: &[ChannelWidgets], wanted: &[(Option<Entity>, String)]) -> bool {
    rows.len() == wanted.len()
        && rows
            .iter()
            .zip(wanted)
            .all(|(row, (entity, name))| row.entity == *entity && row.name == *name)
}

fn build_channel_rows(
    list: Entity,
    wanted: Vec<(Option<Entity>, String)>,
    theme: &UITheme,
    cmd: &mut CommandQueue,
) -> Vec<ChannelWidgets> {
    let mut list = cmd.entity(list).despawn_children();
    wanted
        .into_iter()
        .map(|(entity, name)| spawn_channel_row(&mut list, entity, name, theme))
        .collect()
}

fn entry_selection(entry: Option<&MotionEntry>, rate: Option<f32>) -> Option<(u32, u32)> {
    let range = entry?.range?;
    let rate = rate?;
    Some((frame_at(range.start, rate), frame_at(range.end, rate)))
}

pub(crate) fn sync_definition_panels(
    documents: Query<(
        Entity,
        &EditorDocument,
        &DefinitionDocument,
        &mut DefinitionPanels,
        Option<&HostedApp>,
    )>,
    timelines: Query<&mut Timeline>,
    texts: Query<&mut UIText>,
    nodes: Query<&mut UINode>,
    styles: Query<&mut UIInteractionStyle>,
    disabled: Query<&UIDisabled>,
    assets: Res<AssetRegistry>,
    theme: Res<UITheme>,
    mut cmd: CommandQueue,
) {
    for (entity, document, state, mut panels, hosted) in documents.iter() {
        let read_only = hosted.is_none() || state.loaded.is_none();

        write_text(
            &texts,
            panels.skeleton,
            &asset_name(&assets, state.skeleton.id()),
        );
        let error = state
            .shown_skeleton
            .as_ref()
            .and_then(|shown| shown.error.as_deref())
            .unwrap_or("");
        write_line(&texts, &nodes, panels.skeleton_error, error);
        write_line(&texts, &nodes, panels.status, &document.status);

        let refusal = build_refusal(state, hosted.is_some());
        write_disabled(
            &disabled,
            &mut cmd,
            panels.build,
            refusal.is_some() || state.build.is_running(),
        );
        let note = state.build.progress().map_or_else(
            || refusal.unwrap_or_default().to_owned(),
            |(finished, total)| progress_line(finished, total),
        );
        write_line(&texts, &nodes, panels.note, &note);
        write_line(&texts, &nodes, panels.baked, &state.baked.line());
        if panels.shown_stats != state.stats {
            let lines = state
                .stats
                .as_ref()
                .map_or_else(Vec::new, |stats| stat_lines(stats, &assets));
            let mut list = cmd.entity(panels.stats).despawn_children();
            for line in &lines {
                spawn_stat_line(&mut list, line, &theme);
            }
            if let Some(mut node) = nodes.get_entity(panels.stats_view) {
                node.visible = state.stats.is_some();
                node.height = UIValue::Px((lines.len() as f32 * STAT_HEIGHT).min(STATS_MAX_HEIGHT));
            }
            panels.shown_stats = state.stats.clone();
        }

        if panels.entry_rows.len() != state.entries.len() {
            let mut list = cmd.entity(panels.entries).despawn_children();
            let rows = (0..state.entries.len())
                .map(|index| spawn_entry_row(&mut list, entity, index, &theme))
                .collect();
            panels.entry_rows = rows;
        }
        for (index, (row, entry)) in panels.entry_rows.iter().zip(&state.entries).enumerate() {
            let clip = entry.clip.id();
            write_text(&texts, row.name, &asset_name(&assets, clip));
            write_text(&texts, row.range, &range_line(entry.range));
            let (driven, color) =
                driven_line(state.facts_of(clip), !state.skeleton.id().is_nil(), &theme);
            write_text(&texts, row.driven, &driven);
            write_color(&texts, row.driven, color);
            write_selected(
                &styles,
                &theme,
                row.row,
                state.selected_entry == Some(index),
            );
            write_disabled(&disabled, &mut cmd, row.remove, read_only);
        }

        if let Some(mut timeline) = timelines.get_entity(panels.timeline) {
            let facts = selected_facts(state);
            let rate = facts.and_then(|facts| facts.rate);
            let entry = state
                .selected_entry
                .and_then(|index| state.entries.get(index));
            let shown = Timeline {
                duration: facts.map_or(0.0, |facts| facts.duration),
                rate,
                time: state.time,
                selection: entry_selection(entry, rate),
            };
            if **timeline != shown {
                **timeline = shown;
            }
        }

        for button in [
            panels.skeleton_button,
            panels.add_entry,
            panels.add_channel,
            panels.settings,
        ] {
            write_disabled(&disabled, &mut cmd, button, read_only);
        }

        if let (None, Some(definition)) = (hosted, &state.loaded) {
            let wanted: Vec<_> = definition
                .channels
                .iter()
                .map(|channel| (None, channel.type_name.clone()))
                .collect();
            if !channel_rows_match(&panels.channel_rows, &wanted) {
                panels.channel_rows = build_channel_rows(panels.channels, wanted, &theme, &mut cmd);
            }
        }
    }
}

pub(crate) fn sync_channel_rows(
    channels: EditWorld<Query<(Entity, &ChannelOrder, &ChannelKind)>>,
    settings: EditWorld<Query<Entity, With<DatabaseSettings>>>,
    selection: EditWorld<Option<Res<Selection>>>,
    documents: Query<&mut DefinitionPanels, With<EditTarget>>,
    styles: Query<&mut UIInteractionStyle>,
    theme: Res<UITheme>,
    mut cmd: CommandQueue,
) {
    let Some(mut panels) = documents.iter().next() else {
        return;
    };
    let mut wanted: Vec<_> = channels
        .iter()
        .map(|(entity, order, kind)| (*order, Some(entity), kind.0.to_owned()))
        .collect();
    wanted.sort_by_key(|(order, _, _)| *order);
    let wanted: Vec<_> = wanted
        .into_iter()
        .map(|(_, entity, name)| (entity, name))
        .collect();
    if !channel_rows_match(&panels.channel_rows, &wanted) {
        panels.channel_rows = build_channel_rows(panels.channels, wanted, &theme, &mut cmd);
    }

    let selected = selection.as_deref().and_then(Selection::entity);
    let on_settings = selected.is_some() && settings.iter().next() == selected;
    write_selected(&styles, &theme, panels.settings, on_settings);
    for row in &panels.channel_rows {
        write_selected(&styles, &theme, row.row, row.entity == selected);
    }
}

fn fit_scroll_area(
    view: Entity,
    content: Entity,
    layouts: &Query<&UILayout>,
    areas: &Query<&mut UIScrollArea>,
) {
    let (Some(shown), Some(content), Some(mut area)) = (
        layouts.get_entity(view),
        layouts.get_entity(content),
        areas.get_entity(view),
    ) else {
        return;
    };
    let extent = content.rect.size.y;
    if area.content_extent != extent {
        area.content_extent = extent;
    }
    let offset = area
        .offset
        .clamp(0.0, (extent - shown.content_rect.size.y).max(0.0));
    if area.offset != offset {
        area.offset = offset;
    }
}

pub(crate) fn sync_panel_scroll(
    panels: Query<&DefinitionPanels>,
    layouts: Query<&UILayout>,
    areas: Query<&mut UIScrollArea>,
) {
    for panels in panels.iter() {
        fit_scroll_area(panels.view, panels.content, &layouts, &areas);
        fit_scroll_area(panels.stats_view, panels.stats, &layouts, &areas);
    }
}

#[cfg(test)]
pub(crate) mod tests;

//! Dragging a payload from one UI node to another: a [`DragSource`] carries it, a [`DropTarget`]
//! takes it, and [`Dropped`] tells the target.
use std::marker::PhantomData;

use concerto_app::{
    App, Plugin,
    schedule_groups::{LateUpdate, Startup},
};
use concerto_ecs::{
    Component, Entity, IntoSystemConfig, Query, Res, ResMut, Resource, With,
    command::CommandQueue,
    entity::hierarchy::ChildOf,
    signal::{
        EntitySignal, On, Signal,
        lifecycle::{Add, Despawn, Remove},
        listener::{IntoListener, Listener},
    },
};
use concerto_ui::elements::prelude::*;
use concerto_ui::{
    interaction::{DRAG_THRESHOLD, HoveredNode, UIDrag, UIInputState, UIPointerUp},
    material::UIMaterial,
    node::{Position, UIInset, UINode, UIRect},
    sets::UiSet,
    text::UIText,
    theme::UITheme,
    transform::UIValue,
};
use concerto_window::input::{Input, KeyCode, MouseButton, PhysicalKey};
use glam::Vec2;

const DRAG_LAYER: i32 = 950;
const LABEL_OFFSET: Vec2 = Vec2::new(12.0, 16.0);

/// Makes a UI node draggable with the left button: `label` follows the cursor and `payload` goes to the [`DropTarget<P>`] it is released over.
#[derive(Component, Clone)]
pub struct DragSource<P: Clone + Send + Sync + 'static> {
    /// What a drop delivers.
    pub payload: P,
    /// The text that follows the cursor during the drag.
    pub label: String,
}

impl<P: Clone + Send + Sync + 'static> DragSource<P> {
    /// A source that drags `payload`, showing `label`.
    pub fn new(payload: P, label: impl Into<String>) -> Self {
        Self {
            payload,
            label: label.into(),
        }
    }
}

/// Marks a UI node that takes a dragged `P` released over it or over a node inside it.
#[derive(Component)]
pub struct DropTarget<P: Clone + Send + Sync + 'static> {
    marker: PhantomData<fn() -> P>,
}

impl<P: Clone + Send + Sync + 'static> DropTarget<P> {
    /// A target for payloads of type `P`.
    pub fn new() -> Self {
        Self {
            marker: PhantomData,
        }
    }
}

impl<P: Clone + Send + Sync + 'static> Default for DropTarget<P> {
    fn default() -> Self {
        Self::new()
    }
}

/// Sent to a [`DropTarget<P>`] when a dragged [`DragSource<P>`] is released over it, with the pointer position in logical pixels.
pub struct Dropped<P> {
    /// The dragged source's payload.
    pub payload: P,
    /// The pointer's position in the window, in logical pixels.
    pub position: Vec2,
}

impl<P: Clone + Send + Sync + 'static> Signal for Dropped<P> {}
impl<P: Clone + Send + Sync + 'static> EntitySignal for Dropped<P> {}

#[derive(Resource, Default)]
enum DragState {
    #[default]
    Idle,
    Pressed {
        source: Entity,
        travel: Vec2,
    },
    Dragging {
        source: Entity,
    },
    Cancelled,
}

#[derive(Resource)]
struct DragPayloadRegistered<P: 'static> {
    marker: PhantomData<fn() -> P>,
}

#[derive(Component)]
struct DragLabel;

/// Draws the label of a dragged [`DragSource`] and cancels a drag on Esc.
pub struct DragDropPlugin;

impl Plugin for DragDropPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(DragState::default())
            .add_system(Startup, build_drag_label)
            .add_system(LateUpdate, cancel_drag.before(UiSet::Input));
    }
}

/// Registers the payload types nodes can be dragged with.
pub trait DragDropAppExt {
    /// Lets nodes carry [`DragSource<P>`] and [`DropTarget<P>`]; a second call for the same `P` does nothing.
    fn add_drag_payload<P: Clone + Send + Sync + 'static>(&mut self) -> &mut Self;
}

impl DragDropAppExt for App {
    fn add_drag_payload<P: Clone + Send + Sync + 'static>(&mut self) -> &mut Self {
        if self.get_resource::<DragPayloadRegistered<P>>().is_some() {
            return self;
        }
        self.insert_resource(DragPayloadRegistered::<P> {
            marker: PhantomData,
        })
        .add_listener(attach_source::<P>)
        .add_listener(detach_source::<P>)
        .add_listener(forget_source::<P>)
    }
}

fn build_drag_label(mut cmd: CommandQueue, theme: Res<UITheme>) {
    cmd.spawn(
        UINode::default()
            .with_size(UIValue::Percent(100.0), UIValue::Percent(100.0))
            .with_z_index(DRAG_LAYER),
    )
    .add_child((
        theme
            .popup()
            .padding(UIRect::axes(2.0, 8.0))
            .fixed()
            .hidden()
            .node(|node| node.with_position(Position::Absolute)),
        theme.text("").no_wrap(),
        DragLabel,
    ));
}

fn attach_source<P: Clone + Send + Sync + 'static>(
    on: On<Add<DragSource<P>>>,
    sources: Query<&DragSource<P>>,
    mut cmd: CommandQueue,
) {
    let entity = on.signal().entity;
    let Some(source) = sources.get_entity(entity) else {
        return;
    };
    cmd.insert(
        (
            follow_drag::<P>(source.label.clone()),
            finish_drag(source.payload.clone()),
        ),
        entity,
    );
}

fn detach_source<P: Clone + Send + Sync + 'static>(
    on: On<Remove<DragSource<P>>>,
    mut state: ResMut<DragState>,
    mut labels: Query<&mut UINode, With<DragLabel>>,
    mut cmd: CommandQueue,
) {
    let entity = on.signal().entity;
    cmd.remove::<Listener<UIDrag>>(entity);
    cmd.remove::<Listener<UIPointerUp>>(entity);
    drop_source(entity, &mut state, &mut labels);
}

fn forget_source<P: Clone + Send + Sync + 'static>(
    on: On<Despawn<DragSource<P>>>,
    mut state: ResMut<DragState>,
    mut labels: Query<&mut UINode, With<DragLabel>>,
) {
    drop_source(on.signal().entity, &mut state, &mut labels);
}

fn drop_source(
    entity: Entity,
    state: &mut DragState,
    labels: &mut Query<&mut UINode, With<DragLabel>>,
) {
    if matches!(*state, DragState::Dragging { source } if source == entity) {
        hide_label(labels);
        *state = DragState::Cancelled;
    }
}

fn follow_drag<P: Clone + Send + Sync + 'static>(label: String) -> Listener<UIDrag> {
    (move |on: On<UIDrag>,
           mut state: ResMut<DragState>,
           hovered: Res<HoveredNode>,
           parents: Query<&ChildOf>,
           targets: Query<&DropTarget<P>>,
           theme: Res<UITheme>,
           mut labels: Query<(&mut UINode, &mut UIText, &mut UIMaterial), With<DragLabel>>| {
        let drag = on.signal();
        if drag.button != MouseButton::Left {
            return;
        }
        let source = on.entity();
        let travel = match *state {
            DragState::Cancelled => return,
            DragState::Dragging { source: dragged } if dragged == source => Vec2::ZERO,
            DragState::Pressed {
                source: pressed,
                travel,
            } if pressed == source => travel + drag.delta,
            _ => drag.delta,
        };
        let dragging =
            matches!(*state, DragState::Dragging { source: dragged } if dragged == source);
        if !dragging && travel.length() < DRAG_THRESHOLD {
            *state = DragState::Pressed { source, travel };
            return;
        }
        if !dragging {
            *state = DragState::Dragging { source };
        }
        let accepted = target_under(**hovered, &parents, &targets).is_some();
        show_label(&mut labels, &theme, &label, drag.position, accepted);
    })
    .into_listener()
}

fn finish_drag<P: Clone + Send + Sync + 'static>(payload: P) -> Listener<UIPointerUp> {
    (move |on: On<UIPointerUp>,
           mut state: ResMut<DragState>,
           hovered: Res<HoveredNode>,
           parents: Query<&ChildOf>,
           targets: Query<&DropTarget<P>>,
           mut labels: Query<&mut UINode, With<DragLabel>>,
           mut cmd: CommandQueue| {
        let up = on.signal();
        if up.button != MouseButton::Left {
            return;
        }
        let dragged = matches!(*state, DragState::Dragging { source } if source == on.entity());
        *state = DragState::Idle;
        if !dragged {
            return;
        }
        hide_label(&mut labels);
        if let Some(target) = target_under(**hovered, &parents, &targets) {
            cmd.entity(target).trigger(Dropped {
                payload: payload.clone(),
                position: up.position,
            });
        }
    })
    .into_listener()
}

fn target_under<P: Clone + Send + Sync + 'static>(
    hovered: Option<Entity>,
    parents: &Query<&ChildOf>,
    targets: &Query<&DropTarget<P>>,
) -> Option<Entity> {
    let mut node = hovered;
    while let Some(entity) = node {
        if targets.get_entity(entity).is_some() {
            return Some(entity);
        }
        node = parents.get_entity(entity).map(ChildOf::parent);
    }
    None
}

fn show_label(
    labels: &mut Query<(&mut UINode, &mut UIText, &mut UIMaterial), With<DragLabel>>,
    theme: &UITheme,
    label: &str,
    cursor: Vec2,
    accepted: bool,
) {
    let at = cursor + LABEL_OFFSET;
    let inset = UIInset {
        left: UIValue::Px(at.x),
        top: UIValue::Px(at.y),
        ..Default::default()
    };
    let color = if accepted {
        theme.text
    } else {
        theme.text_muted
    };
    let border = if accepted { theme.accent } else { theme.border }.to_linear();
    for (mut node, mut text, mut material) in labels.iter_mut() {
        if !node.visible || node.inset != inset {
            node.visible = true;
            node.inset = inset;
        }
        if text.text != label || text.color != color {
            text.text = label.to_owned();
            text.color = color;
        }
        if material.border_color != border {
            material.border_color = border;
        }
    }
}

fn hide_label(labels: &mut Query<&mut UINode, With<DragLabel>>) {
    for mut node in labels.iter_mut() {
        if node.visible {
            node.visible = false;
        }
    }
}

fn cancel_drag(
    input: Res<Input>,
    pointer: Res<UIInputState>,
    mut state: ResMut<DragState>,
    mut labels: Query<&mut UINode, With<DragLabel>>,
) {
    let captured = pointer.captured(MouseButton::Left);
    let next = match *state {
        DragState::Dragging { source } if captured != Some(source) => DragState::Idle,
        DragState::Dragging { .. } if input.is_just_pressed(PhysicalKey::Code(KeyCode::Escape)) => {
            DragState::Cancelled
        }
        DragState::Pressed { .. } | DragState::Cancelled if captured.is_none() => DragState::Idle,
        _ => return,
    };
    if matches!(*state, DragState::Dragging { .. }) {
        hide_label(&mut labels);
    }
    *state = next;
}

#[cfg(test)]
mod tests {
    use super::*;
    use concerto_app::{main_schedule::MainSchedulePlugin, plugins::TimePlugin};
    use concerto_ui::interaction::UIClick;

    #[derive(Clone, Debug, PartialEq)]
    struct Crate(u32);

    #[derive(Debug, PartialEq)]
    struct HeardDrop {
        target: Entity,
        payload: Crate,
        position: Vec2,
    }

    #[derive(Resource, Default)]
    struct Heard {
        drops: Vec<HeardDrop>,
        clicks: u32,
    }

    struct Fixture {
        app: App,
        source: Entity,
        cursor: Vec2,
    }

    impl Fixture {
        fn new() -> Self {
            let mut app = App::new();
            app.register_plugin(MainSchedulePlugin)
                .register_plugin(TimePlugin)
                .insert_resource(UITheme::default())
                .insert_resource(HoveredNode::default())
                .insert_resource(UIInputState::default())
                .insert_resource(Input::new())
                .insert_resource(Heard::default())
                .register_plugin(DragDropPlugin)
                .add_drag_payload::<Crate>();
            let source = app.main_mut().world_mut().spawn((
                UINode::default(),
                (|_: On<UIClick>, mut heard: ResMut<Heard>| heard.clicks += 1).into_listener(),
            ));
            app.finish_plugin_build();
            app.update();
            Self {
                app,
                source,
                cursor: Vec2::ZERO,
            }
        }

        fn world(&mut self) -> &mut concerto_ecs::World {
            self.app.main_mut().world_mut()
        }

        fn target(&mut self) -> Entity {
            self.world().spawn((
                UINode::default(),
                DropTarget::<Crate>::new(),
                (|on: On<Dropped<Crate>>, mut heard: ResMut<Heard>| {
                    let target = on.entity();
                    let dropped = on.signal();
                    heard.drops.push(HeardDrop {
                        target,
                        payload: dropped.payload.clone(),
                        position: dropped.position,
                    });
                })
                .into_listener(),
            ))
        }

        fn press(&mut self, at: Vec2) {
            self.cursor = at;
            let source = self.source;
            self.world()
                .get_resource_mut::<UIInputState>()
                .unwrap()
                .set_captured(MouseButton::Left, Some(source));
            self.app.update();
        }

        fn drag(&mut self, to: Vec2, over: Option<Entity>) {
            let delta = to - self.cursor;
            self.cursor = to;
            let source = self.source;
            **self.world().get_resource_mut::<HoveredNode>().unwrap() = over;
            self.world().trigger_on(
                source,
                UIDrag {
                    position: to,
                    delta,
                    button: MouseButton::Left,
                },
            );
            self.app.update();
        }

        fn release(&mut self) {
            let source = self.source;
            let position = self.cursor;
            self.world()
                .get_resource_mut::<UIInputState>()
                .unwrap()
                .set_captured(MouseButton::Left, None);
            self.world().trigger_on(
                source,
                UIPointerUp {
                    position,
                    button: MouseButton::Left,
                },
            );
            self.app.update();
        }

        fn click(&mut self) {
            let source = self.source;
            let position = self.cursor;
            self.world().trigger_on(
                source,
                UIClick {
                    position,
                    button: MouseButton::Left,
                },
            );
        }

        fn press_escape(&mut self) {
            let escape = PhysicalKey::Code(KeyCode::Escape);
            self.world()
                .get_resource_mut::<Input>()
                .unwrap()
                .update_key_input(escape, true);
            self.app.update();
            let input = self.world().get_resource_mut::<Input>().unwrap();
            input.update_key_input(escape, false);
            input.update();
        }

        fn drops(&mut self) -> Vec<HeardDrop> {
            std::mem::take(&mut self.world().get_resource_mut::<Heard>().unwrap().drops)
        }
    }

    #[test]
    fn a_drag_past_the_slop_drops_its_payload_on_the_target_under_the_pointer() {
        let mut f = Fixture::new();
        let target = f.target();
        let inside = f.world().spawn((UINode::default(), ChildOf::new(target)));
        let elsewhere = f.world().spawn(UINode::default());
        let source = f.source;
        f.world().insert(DragSource::new(Crate(7), "crate"), source);
        let start = Vec2::new(20.0, 20.0);
        let over = Vec2::new(300.0, 200.0);

        f.press(start);
        f.drag(start + Vec2::new(2.0, 0.0), Some(source));
        f.drag(over, Some(inside));
        f.release();
        assert_eq!(
            f.drops(),
            [HeardDrop {
                target,
                payload: Crate(7),
                position: over,
            }]
        );

        f.press(start);
        f.drag(over, Some(elsewhere));
        f.release();
        assert_eq!(f.drops(), [], "a release off every target cancels");

        f.press(start);
        f.drag(over, Some(target));
        f.press_escape();
        f.drag(over + Vec2::X, Some(target));
        f.release();
        assert_eq!(
            f.drops(),
            [],
            "Esc cancels the drag while the button is held"
        );

        f.press(start);
        f.drag(start, Some(source));
        f.release();
        f.click();
        assert_eq!(f.drops(), []);
        assert_eq!(
            f.world().get_resource::<Heard>().unwrap().clicks,
            1,
            "a press that does not move is still a click"
        );

        f.press(start);
        f.drag(over, Some(target));
        f.world().remove_component::<DragSource<Crate>>(source);
        f.release();
        f.press(start);
        f.drag(over, Some(target));
        f.release();
        assert_eq!(f.drops(), [], "losing the source ends its drag");
    }
}

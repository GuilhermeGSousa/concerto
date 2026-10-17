use std::ops::{Deref, DerefMut};

use concerto_ecs::{
    Entity, Query, Res, ResMut, Resource, World,
    command::CommandQueue,
    signal::Signal,
    system::{
        access::SystemAccess,
        input::{ReadOnlySystemInput, SystemInput, SystemInputData},
        meta::SystemMetadata,
    },
    world::UnsafeWorldCell,
    world_id::WorldId,
};

use super::{HostedApp, PlayState, outbound};
use crate::asset_editor::ActiveEditor;

fn edit_target<'w>(world: UnsafeWorldCell<'w>) -> Option<(Entity, &'w World)> {
    let world = world.world();
    let document = world.get_resource::<ActiveEditor>()?.0?;
    if world.get_component_for_entity::<PlayState>(document) == Some(&PlayState::Starting) {
        return None;
    }
    let scene = world
        .get_component_for_entity::<HostedApp>(document)?
        .world();
    let announced = world.get_resource::<LastEditTarget>()?.0;
    (announced == Some((document, scene.id()))).then_some((document, scene))
}

fn read_edit_target(access: &mut SystemAccess) {
    access.read_resource::<ActiveEditor>();
    access.read_component::<HostedApp>();
    access.read_component::<PlayState>();
    access.read_resource::<LastEditTarget>();
}

/// Reads the edit target's main world. The system is skipped while there is no announced edit target, or its game is starting.
pub struct Editor<'world, 'state, T: ReadOnlySystemInput + 'static>(
    SystemInputData<'world, 'state, T>,
);

impl<'world, 'state, T: ReadOnlySystemInput + 'static> Deref for Editor<'world, 'state, T> {
    type Target = SystemInputData<'world, 'state, T>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T: ReadOnlySystemInput + 'static> DerefMut for Editor<'_, '_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

/// An [`Editor`] parameter's state, rebuilt from its first form whenever the edit target is another world.
pub struct EditorState<S> {
    built_for: Option<WorldId>,
    first: S,
    current: S,
}

impl<S> EditorState<S> {
    fn for_world(&self, scene: &World) -> Option<&S> {
        (self.built_for == Some(scene.id())).then_some(&self.current)
    }
}

impl<T: ReadOnlySystemInput + 'static> SystemInput for Editor<'_, '_, T>
where
    T::State: Clone,
{
    type State = EditorState<T::State>;
    type Data<'world, 'state> = Editor<'world, 'state, T>;

    fn init_state(world: &mut World) -> Self::State {
        let first = T::init_state(world);
        EditorState {
            built_for: None,
            current: first.clone(),
            first,
        }
    }

    fn validate(state: &Self::State, world: UnsafeWorldCell) -> bool {
        edit_target(world).is_some_and(|(_, scene)| {
            let current = state.for_world(scene).unwrap_or(&state.first);
            T::validate(current, scene.as_unsafe_world_cell())
        })
    }

    fn get_data<'world, 'state>(
        state: &'state mut Self::State,
        world: UnsafeWorldCell<'world>,
    ) -> Self::Data<'world, 'state> {
        let (_, scene) = edit_target(world).expect("validated before the system ran");
        if state.for_world(scene).is_none() {
            state.current = state.first.clone();
            state.built_for = Some(scene.id());
        }
        Editor(T::get_data(
            &mut state.current,
            scene.as_unsafe_world_cell(),
        ))
    }

    fn fill_access(_meta: &mut SystemMetadata, access: &mut SystemAccess) {
        read_edit_target(access);
    }
}

impl<T: ReadOnlySystemInput + 'static> ReadOnlySystemInput for Editor<'_, '_, T> where
    T::State: Clone
{
}

type QueuedSignal = Box<dyn FnOnce(&mut HostedApp) + Send + Sync>;

/// The signals a system queued for one hosted `App`.
#[derive(Default)]
pub struct SignalQueue {
    made_for: Option<(Entity, WorldId)>,
    signals: Vec<QueuedSignal>,
}

/// Sends signals to the edit target. The system is skipped while there is no announced edit target, or its game is starting.
pub struct EditorSignals<'state> {
    queue: &'state mut SignalQueue,
}

impl EditorSignals<'_> {
    /// Queues `signal`; its listeners run in the edit target when this system's commands are applied.
    pub fn trigger<S: Signal>(&mut self, signal: S) {
        self.queue
            .signals
            .push(Box::new(move |hosted| hosted.trigger(signal)));
    }
}

impl SystemInput for EditorSignals<'_> {
    type State = SignalQueue;
    type Data<'world, 'state> = EditorSignals<'state>;

    fn init_state(_world: &mut World) -> Self::State {
        SignalQueue::default()
    }

    fn validate(_state: &Self::State, world: UnsafeWorldCell) -> bool {
        edit_target(world).is_some()
    }

    fn get_data<'world, 'state>(
        state: &'state mut Self::State,
        world: UnsafeWorldCell<'world>,
    ) -> Self::Data<'world, 'state> {
        let target = edit_target(world).map(|(document, scene)| (document, scene.id()));
        if state.made_for != target {
            state.signals.clear();
        }
        state.made_for = target;
        EditorSignals { queue: state }
    }

    fn apply(state: &mut Self::State, world: &mut World) {
        let signals = std::mem::take(&mut state.signals);
        let Some((document, made_for)) = state.made_for.take() else {
            return;
        };
        if signals.is_empty() {
            return;
        }
        let Some(hosted) = world.get_component_for_entity_mut::<HostedApp>(document) else {
            return;
        };
        if hosted.world().id() != made_for {
            return;
        }
        for signal in signals {
            signal(hosted);
        }
        outbound::forward(world, document);
    }

    fn fill_access(_meta: &mut SystemMetadata, access: &mut SystemAccess) {
        read_edit_target(access);
        access.set_needs_apply();
    }
}

/// The edit target became a different `App`, or none.
pub struct EditTargetChanged {
    pub document: Option<Entity>,
}
impl Signal for EditTargetChanged {}

/// The edit target as it was when last announced.
#[derive(Resource, Default)]
pub struct LastEditTarget(Option<(Entity, WorldId)>);

#[cfg(test)]
impl LastEditTarget {
    pub(crate) fn naming(document: Entity, world: WorldId) -> Self {
        Self(Some((document, world)))
    }
}

pub(crate) fn announce_edit_target(
    active: Res<ActiveEditor>,
    hosted: Query<&HostedApp>,
    mut last: ResMut<LastEditTarget>,
    mut cmd: CommandQueue,
) {
    let now = active.0.and_then(|document| {
        hosted
            .get_entity(document)
            .map(|hosted| (document, hosted.world().id()))
    });
    if last.0 != now {
        last.0 = now;
        cmd.trigger(EditTargetChanged {
            document: now.map(|(document, _)| document),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        asset_editor::ActiveEditor,
        hosted::{HostedApp, testing::*},
    };
    use concerto_app::App;
    use concerto_ecs::{
        Component, IntoSystem, Query, Res, ResMut, Resource, System, World, signal::On,
    };

    #[derive(Component)]
    struct Marker(u32);

    #[derive(Resource, Default)]
    struct Seen(Vec<u32>);

    #[derive(Resource, Default)]
    struct Runs(usize);

    fn recipe_with(value: u32) -> fn(&mut App) {
        fn one(app: &mut App) {
            plain_recipe(app);
            app.main_mut().world_mut().spawn(Marker(1));
        }
        fn two(app: &mut App) {
            plain_recipe(app);
            app.main_mut().world_mut().spawn(());
            app.main_mut().world_mut().spawn(Marker(2));
        }
        if value == 1 { one } else { two }
    }

    fn read_markers(
        markers: Editor<Query<&Marker>>,
        mut seen: ResMut<Seen>,
        mut runs: ResMut<Runs>,
    ) {
        runs.0 += 1;
        seen.0 = markers.iter().map(|marker| marker.0).collect();
    }

    fn run<M>(world: &mut World, system: impl IntoSystem<(), M>) {
        let mut system = system.into_system();
        system.initialize(world);
        system.run_and_apply((), world);
    }

    fn announce(world: &mut World) {
        run(world, announce_edit_target);
    }

    fn seam_world(value: u32) -> (World, concerto_ecs::Entity) {
        let (mut world, document) = host_world(recipe_with(value));
        world.insert_resource(Seen::default());
        world.insert_resource(Runs::default());
        (world, document)
    }

    fn runs(world: &World) -> usize {
        world.get_resource::<Runs>().unwrap().0
    }

    fn seen(world: &World) -> &[u32] {
        &world.get_resource::<Seen>().unwrap().0
    }

    #[test]
    fn a_query_reads_the_edit_targets_world() {
        let (mut world, _) = seam_world(1);

        run(&mut world, read_markers);

        assert_eq!(world.get_resource::<Seen>().unwrap().0, vec![1]);
    }

    #[test]
    fn a_system_is_skipped_while_there_is_no_edit_target() {
        let (mut world, _) = seam_world(1);
        world.get_resource_mut::<ActiveEditor>().unwrap().0 = None;

        run(&mut world, read_markers);

        assert_eq!(world.get_resource::<Runs>().unwrap().0, 0);
    }

    #[test]
    fn a_query_follows_a_replacement_app() {
        let (mut world, document) = seam_world(1);
        let mut system = read_markers.into_system();
        system.initialize(&mut world);
        system.run_and_apply((), &mut world);

        world.insert(hosted_now(recipe_with(2), |_| {}), document);
        announce(&mut world);
        system.run_and_apply((), &mut world);

        assert_eq!(world.get_resource::<Seen>().unwrap().0, vec![2]);
    }

    #[test]
    fn a_query_follows_a_change_of_active_document() {
        let (mut world, _) = seam_world(1);
        let other = world.spawn(hosted_now(recipe_with(2), |_| {}));
        let mut system = read_markers.into_system();
        system.initialize(&mut world);
        system.run_and_apply((), &mut world);

        world.get_resource_mut::<ActiveEditor>().unwrap().0 = Some(other);
        announce(&mut world);
        system.run_and_apply((), &mut world);

        assert_eq!(world.get_resource::<Seen>().unwrap().0, vec![2]);
    }

    #[derive(Component)]
    struct Other;

    fn hosting(spawn: impl FnOnce(&mut World)) -> HostedApp {
        hosted_now(plain_recipe, |app| spawn(app.main_mut().world_mut()))
    }

    fn markers_after_replacing(
        first: impl FnOnce(&mut World),
        second: impl FnOnce(&mut World),
    ) -> Vec<u32> {
        let (mut world, document) = seam_world(1);
        world.insert(hosting(first), document);
        announce(&mut world);
        let mut system = read_markers.into_system();
        system.initialize(&mut world);
        system.run_and_apply((), &mut world);

        world.insert(hosting(second), document);
        announce(&mut world);
        system.run_and_apply((), &mut world);

        let mut seen = seen(&world).to_vec();
        seen.sort_unstable();
        seen
    }

    #[test]
    fn a_query_follows_a_replacement_with_the_same_number_of_archetypes() {
        let seen = markers_after_replacing(
            |world| {
                world.spawn(Other);
                world.spawn(Marker(1));
            },
            |world| {
                world.spawn(Marker(2));
                world.spawn(Other);
            },
        );

        assert_eq!(seen, vec![2]);
    }

    #[test]
    fn a_query_follows_a_replacement_with_fewer_archetypes() {
        let seen = markers_after_replacing(
            |world| {
                world.spawn(Marker(1));
                world.spawn((Marker(3), Other));
                world.spawn(Other);
            },
            |world| {
                world.spawn(Marker(2));
            },
        );

        assert_eq!(seen, vec![2]);
    }

    #[test]
    fn a_query_follows_a_replacement_with_more_archetypes() {
        let seen = markers_after_replacing(
            |world| {
                world.spawn(Marker(1));
            },
            |world| {
                world.spawn(Other);
                world.spawn((Marker(2), Other));
                world.spawn(Marker(4));
            },
        );

        assert_eq!(seen, vec![2, 4]);
    }

    #[test]
    fn a_query_reads_the_first_app_again_after_another_was_edited() {
        let (mut world, first) = seam_world(1);
        world.insert(
            hosting(|world| {
                world.spawn(Marker(1));
            }),
            first,
        );
        let second = world.spawn(hosting(|world| {
            world.spawn(Other);
            world.spawn(Marker(2));
        }));
        announce(&mut world);
        let mut system = read_markers.into_system();
        system.initialize(&mut world);
        system.run_and_apply((), &mut world);

        world.get_resource_mut::<ActiveEditor>().unwrap().0 = Some(second);
        announce(&mut world);
        system.run_and_apply((), &mut world);
        world.get_resource_mut::<ActiveEditor>().unwrap().0 = Some(first);
        announce(&mut world);
        system.run_and_apply((), &mut world);

        assert_eq!(seen(&world), [1]);
    }

    #[derive(Resource, Default)]
    struct FramesSeen(Vec<usize>);

    fn read_frames(frames: Editor<Res<Frames>>, mut seen: ResMut<FramesSeen>) {
        let frames: &Frames = &frames;
        seen.0.push(frames.0);
    }

    fn step(world: &mut World, document: concerto_ecs::Entity) {
        world
            .get_component_for_entity_mut::<HostedApp>(document)
            .unwrap()
            .step();
    }

    #[test]
    fn a_resource_is_read_from_the_edit_target_and_follows_a_change_of_active_document() {
        let (mut world, document) = seam_world(1);
        world.insert_resource(FramesSeen::default());
        let other = world.spawn(hosted_now(recipe_with(2), |_| {}));
        step(&mut world, document);
        step(&mut world, other);
        step(&mut world, other);
        step(&mut world, other);
        let mut system = read_frames.into_system();
        system.initialize(&mut world);
        system.run_and_apply((), &mut world);

        world.get_resource_mut::<ActiveEditor>().unwrap().0 = Some(other);
        announce(&mut world);
        system.run_and_apply((), &mut world);

        assert_eq!(world.get_resource::<FramesSeen>().unwrap().0, vec![1, 3]);
    }

    #[derive(Resource, Default)]
    struct Heard(Vec<u32>);

    #[derive(Resource, Default)]
    struct Answered(Vec<concerto_ecs::Entity>);

    fn hear(on: On<crate::hosted::Hosted<Echo>>, mut heard: ResMut<Heard>) {
        heard.0.push(on.signal().event.0);
    }

    fn note_document(on: On<crate::hosted::Hosted<Echo>>, mut answered: ResMut<Answered>) {
        answered.0.push(on.signal().document);
    }

    fn say_five(mut scene: EditorSignals) {
        scene.trigger(Say(5));
    }

    fn count_and_say(mut scene: EditorSignals, mut runs: ResMut<Runs>) {
        runs.0 += 1;
        scene.trigger(Say(5));
    }

    fn say_one_two_three(mut scene: EditorSignals) {
        scene.trigger(Say(1));
        scene.trigger(Say(2));
        scene.trigger(Say(3));
    }

    fn hearing_world() -> (World, concerto_ecs::Entity) {
        let (mut world, document) = seam_world(1);
        world
            .get_resource_mut::<crate::hosted::OutboundEvents>()
            .unwrap()
            .register::<Echo>();
        world.insert_resource(Heard::default());
        world.insert_resource(Answered::default());
        world.add_listener(hear);
        world.add_listener(note_document);
        (world, document)
    }

    fn heard(world: &World) -> &[u32] {
        &world.get_resource::<Heard>().unwrap().0
    }

    fn answered(world: &World) -> &[concerto_ecs::Entity] {
        &world.get_resource::<Answered>().unwrap().0
    }

    #[test]
    fn a_triggered_signal_runs_in_the_edit_target_and_its_answer_comes_back_at_once() {
        let (mut world, _) = hearing_world();

        run(&mut world, say_five);

        assert_eq!(world.get_resource::<Heard>().unwrap().0, vec![5]);
    }

    #[test]
    fn signals_triggered_in_one_run_arrive_in_the_order_they_were_triggered() {
        let (mut world, _) = hearing_world();

        run(&mut world, say_one_two_three);

        assert_eq!(heard(&world), [1, 2, 3]);
    }

    #[test]
    fn a_queue_made_for_an_app_that_is_gone_is_discarded() {
        let (mut world, document) = hearing_world();
        let mut system = say_five.into_system();
        system.initialize(&mut world);
        system.run((), &mut world);

        world.insert(hosted_now(recipe_with(2), |_| {}), document);
        system.apply(&mut world);

        assert!(world.get_resource::<Heard>().unwrap().0.is_empty());
    }

    #[test]
    fn a_queue_made_for_an_app_that_is_gone_is_discarded_after_its_replacement_is_announced() {
        let (mut world, document) = hearing_world();
        let mut system = say_five.into_system();
        system.initialize(&mut world);
        system.run((), &mut world);

        world.insert(hosted_now(recipe_with(2), |_| {}), document);
        announce(&mut world);
        system.apply(&mut world);

        assert!(heard(&world).is_empty());
    }

    #[test]
    fn a_queue_is_discarded_when_the_hosted_app_was_removed_before_it_is_applied() {
        let (mut world, document) = hearing_world();
        let mut system = say_five.into_system();
        system.initialize(&mut world);
        system.run((), &mut world);

        world.remove_component::<HostedApp>(document);
        system.apply(&mut world);

        assert!(heard(&world).is_empty());
    }

    #[test]
    fn signals_left_by_a_run_that_was_never_applied_are_not_sent_to_another_app() {
        let (mut world, _) = hearing_world();
        let other = world.spawn(hosted_now(recipe_with(2), |_| {}));
        let mut system = say_five.into_system();
        system.initialize(&mut world);
        system.run((), &mut world);

        world.get_resource_mut::<ActiveEditor>().unwrap().0 = Some(other);
        announce(&mut world);
        system.run_and_apply((), &mut world);

        assert_eq!(heard(&world), [5]);
        assert_eq!(answered(&world), [other]);
    }

    struct Poke(u32);
    impl Signal for Poke {}

    fn say_when_poked(on: On<Poke>, mut scene: EditorSignals) {
        scene.trigger(Say(on.signal().0));
    }

    #[test]
    fn a_listener_sends_a_signal_into_the_edit_target_and_its_answer_is_heard() {
        let (mut world, _) = hearing_world();
        world.add_listener(say_when_poked);

        world.trigger(Poke(8));

        assert_eq!(world.get_resource::<Heard>().unwrap().0, vec![8]);
    }

    #[test]
    fn a_signalling_system_is_skipped_while_there_is_no_edit_target() {
        let (mut world, _) = hearing_world();
        world.get_resource_mut::<ActiveEditor>().unwrap().0 = None;
        announce(&mut world);

        run(&mut world, count_and_say);

        assert_eq!(runs(&world), 0);
        assert!(heard(&world).is_empty());
    }

    #[test]
    fn both_parameters_decline_while_the_active_document_has_no_hosted_app() {
        let (mut world, _) = hearing_world();
        let bare = world.spawn(());
        world.get_resource_mut::<ActiveEditor>().unwrap().0 = Some(bare);
        announce(&mut world);

        run(&mut world, read_markers);
        run(&mut world, count_and_say);

        assert_eq!(runs(&world), 0);
        assert!(heard(&world).is_empty());
    }

    #[test]
    fn both_parameters_decline_from_a_change_of_active_document_until_it_is_announced() {
        let (mut world, _) = hearing_world();
        let other = world.spawn(hosted_now(recipe_with(2), |_| {}));
        world.get_resource_mut::<ActiveEditor>().unwrap().0 = Some(other);

        run(&mut world, read_markers);
        run(&mut world, count_and_say);
        assert_eq!(runs(&world), 0);
        assert!(heard(&world).is_empty());

        announce(&mut world);
        run(&mut world, read_markers);
        run(&mut world, count_and_say);

        assert_eq!(runs(&world), 2);
        assert_eq!(seen(&world), [2]);
        assert_eq!(answered(&world), [other]);
    }

    #[test]
    fn both_parameters_decline_from_a_replacement_of_the_hosted_app_until_it_is_announced() {
        let (mut world, document) = hearing_world();
        world.insert(hosted_now(recipe_with(2), |_| {}), document);

        run(&mut world, read_markers);
        run(&mut world, count_and_say);
        assert_eq!(runs(&world), 0);
        assert!(heard(&world).is_empty());

        announce(&mut world);
        run(&mut world, read_markers);
        run(&mut world, count_and_say);

        assert_eq!(runs(&world), 2);
        assert_eq!(seen(&world), [2]);
        assert_eq!(answered(&world), [document]);
    }

    #[test]
    fn both_parameters_decline_while_no_edit_target_was_ever_announced() {
        let (mut world, _) = hearing_world();
        world.remove_resource::<LastEditTarget>();

        run(&mut world, read_markers);
        run(&mut world, count_and_say);

        assert_eq!(runs(&world), 0);
        assert!(heard(&world).is_empty());
    }

    fn access_of<P: SystemInput>() -> SystemAccess {
        let mut access = SystemAccess::default();
        P::fill_access(&mut SystemMetadata::default(), &mut access);
        access
    }

    fn conflict(a: &SystemAccess, b: &SystemAccess) -> bool {
        !SystemAccess::are_disjoint(a, b) && !SystemAccess::are_disjoint(b, a)
    }

    fn parallel(a: &SystemAccess, b: &SystemAccess) -> bool {
        SystemAccess::are_disjoint(a, b) && SystemAccess::are_disjoint(b, a)
    }

    #[test]
    fn both_parameters_conflict_with_a_system_that_writes_the_hosted_app() {
        let mut writer = SystemAccess::default();
        writer.write_component::<HostedApp>();

        assert!(conflict(&access_of::<Editor<Query<&Marker>>>(), &writer));
        assert!(conflict(&access_of::<EditorSignals>(), &writer));
    }

    #[test]
    fn both_parameters_conflict_with_a_system_that_changes_the_active_document() {
        let mut writer = SystemAccess::default();
        writer.write_resource::<ActiveEditor>();

        assert!(conflict(&access_of::<Editor<Query<&Marker>>>(), &writer));
        assert!(conflict(&access_of::<EditorSignals>(), &writer));
    }

    #[test]
    fn both_parameters_conflict_with_the_system_that_announces_the_edit_target() {
        let mut writer = SystemAccess::default();
        writer.write_resource::<LastEditTarget>();

        assert!(conflict(&access_of::<Editor<Query<&Marker>>>(), &writer));
        assert!(conflict(&access_of::<EditorSignals>(), &writer));
    }

    #[test]
    fn systems_that_read_the_edit_target_can_run_in_parallel() {
        let markers = access_of::<Editor<Query<&Marker>>>();
        let frames = access_of::<Editor<Res<Frames>>>();

        assert!(parallel(&markers, &markers));
        assert!(parallel(&markers, &frames));
    }

    #[test]
    fn a_reading_parameter_claims_nothing_of_its_inner_parameter_in_the_editors_world() {
        let mut writer = SystemAccess::default();
        writer.write_component::<Marker>();
        writer.write_resource::<Frames>();

        assert!(parallel(&access_of::<Editor<Query<&Marker>>>(), &writer));
        assert!(parallel(&access_of::<Editor<Res<Frames>>>(), &writer));
    }

    #[test]
    fn only_the_signalling_parameter_needs_a_sync_point() {
        assert!(!access_of::<Editor<Query<&Marker>>>().needs_apply());
        assert!(access_of::<EditorSignals>().needs_apply());
    }

    #[derive(Resource, Default)]
    struct Targets(Vec<Option<concerto_ecs::Entity>>);

    fn note_target(on: On<EditTargetChanged>, mut targets: ResMut<Targets>) {
        targets.0.push(on.signal().document);
    }

    #[test]
    fn a_change_of_edit_target_is_announced_once_for_each_of_the_four_ways() {
        let (mut world, document) = seam_world(1);
        world.insert_resource(LastEditTarget::default());
        world.insert_resource(Targets::default());
        world.add_listener(note_target);
        let mut system = announce_edit_target.into_system();
        system.initialize(&mut world);

        system.run_and_apply((), &mut world);
        system.run_and_apply((), &mut world);
        assert_eq!(
            world.get_resource::<Targets>().unwrap().0,
            vec![Some(document)]
        );

        world.insert(hosted_now(recipe_with(2), |_| {}), document);
        system.run_and_apply((), &mut world);
        world.get_resource_mut::<ActiveEditor>().unwrap().0 = None;
        system.run_and_apply((), &mut world);
        world.get_resource_mut::<ActiveEditor>().unwrap().0 = Some(document);
        system.run_and_apply((), &mut world);
        world.despawn(document);
        system.run_and_apply((), &mut world);

        assert_eq!(
            world.get_resource::<Targets>().unwrap().0,
            vec![Some(document), Some(document), None, Some(document), None]
        );
    }

    #[test]
    fn both_parameters_decline_while_the_game_starts() {
        let (mut world, document) = hearing_world();
        start_play(&mut world, document, recipe_with(2), |_| {});

        run(&mut world, read_markers);
        run(&mut world, count_and_say);

        assert_eq!(runs(&world), 0);
        assert!(heard(&world).is_empty());
    }

    #[test]
    fn parking_and_returning_the_edit_app_are_announced_and_the_play_world_is_read_while_playing() {
        let (mut world, document) = seam_world(1);
        world.insert_resource(Targets::default());
        world.add_listener(note_target);
        let mut system = read_markers.into_system();
        system.initialize(&mut world);

        play_now(&mut world, document, recipe_with(2), |_| {});
        announce(&mut world);
        system.run_and_apply((), &mut world);
        assert_eq!(seen(&world), [2]);

        world.trigger(crate::hosted::StopPlay(document));
        announce(&mut world);
        system.run_and_apply((), &mut world);

        assert_eq!(seen(&world), [1]);
        assert_eq!(
            world.get_resource::<Targets>().unwrap().0,
            vec![Some(document), Some(document)]
        );
    }
}

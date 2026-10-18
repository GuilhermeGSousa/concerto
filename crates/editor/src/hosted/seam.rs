use std::ops::{Deref, DerefMut};

use concerto_ecs::{
    Entity, Query, With, World,
    query::state::QueryState,
    signal::Signal,
    system::{
        access::SystemAccess,
        input::{ReadOnlySystemInput, SystemInput, SystemInputData, ValidationError},
        meta::SystemMetadata,
    },
    world::UnsafeWorldCell,
    world_id::WorldId,
};

use super::{HostedApp, target::EditTarget};

type Targets = QueryState<(Entity, &'static HostedApp), With<EditTarget>>;

/// # Safety
/// The caller must hold the access [`read_edit_target`] declares.
unsafe fn edit_target<'w>(
    world: UnsafeWorldCell<'w>,
    targets: &mut Targets,
) -> Option<(Entity, &'w World)> {
    // SAFETY: the query reads only what `read_edit_target` declares, which the caller holds.
    let targets = unsafe { Query::new(world, targets) };
    targets
        .iter()
        .next()
        .map(|(document, hosted)| (document, hosted.world()))
}

fn no_edit_target<T: ?Sized>() -> ValidationError {
    ValidationError::skip::<T>("no document is the edit target")
}

fn read_edit_target(access: &mut SystemAccess) {
    access.read_component::<EditTarget>();
    access.read_component::<HostedApp>();
}

/// Reads the edit target's main world. The system is skipped while no document is the edit target.
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
    targets: Targets,
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
            targets: world.query(),
            built_for: None,
            current: first.clone(),
            first,
        }
    }

    unsafe fn get_data<'world, 'state>(
        state: &'state mut Self::State,
        world: UnsafeWorldCell<'world>,
    ) -> Result<Self::Data<'world, 'state>, ValidationError> {
        // SAFETY: `fill_access` declares what `edit_target` reads.
        let (_, scene) =
            unsafe { edit_target(world, &mut state.targets) }.ok_or_else(no_edit_target::<Self>)?;
        if state.for_world(scene).is_none() {
            state.current = state.first.clone();
            state.built_for = Some(scene.id());
        }
        // SAFETY: the scene's world belongs to its hosted `App`, which no system runs while the
        // editor's systems do, and `T` only reads it.
        unsafe { T::get_data(&mut state.current, scene.as_unsafe_world_cell()) }.map(Editor)
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
pub struct SignalQueue {
    targets: Targets,
    made_for: Option<(Entity, WorldId)>,
    signals: Vec<QueuedSignal>,
}

/// Sends signals to the edit target. The system is skipped while no document is the edit target.
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

    fn init_state(world: &mut World) -> Self::State {
        SignalQueue {
            targets: world.query(),
            made_for: None,
            signals: Vec::new(),
        }
    }

    unsafe fn get_data<'world, 'state>(
        state: &'state mut Self::State,
        world: UnsafeWorldCell<'world>,
    ) -> Result<Self::Data<'world, 'state>, ValidationError> {
        // SAFETY: `fill_access` declares what `edit_target` reads.
        let target = unsafe { edit_target(world, &mut state.targets) }
            .map(|(document, scene)| (document, scene.id()));
        if state.made_for != target {
            state.signals.clear();
        }
        state.made_for = target;
        if target.is_none() {
            return Err(no_edit_target::<Self>());
        }
        Ok(EditorSignals { queue: state })
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
    }

    fn fill_access(_meta: &mut SystemMetadata, access: &mut SystemAccess) {
        read_edit_target(access);
        access.set_needs_apply();
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

    fn retarget(world: &mut World) {
        let active = world.get_resource::<ActiveEditor>().unwrap().0;
        world.trigger(crate::hosted::SetEditTarget(active));
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

    #[derive(Resource)]
    struct NotInTheScene;

    fn read_missing(_: Editor<Res<NotInTheScene>>, mut runs: ResMut<Runs>) {
        runs.0 += 1;
    }

    #[test]
    fn a_resource_missing_from_the_scene_skips_the_system_instead_of_panicking() {
        let (mut world, _) = seam_world(1);

        run(&mut world, read_missing);

        assert_eq!(runs(&world), 0);
    }

    #[test]
    fn a_system_is_skipped_while_there_is_no_edit_target() {
        let (mut world, _) = seam_world(1);
        world.get_resource_mut::<ActiveEditor>().unwrap().0 = None;
        retarget(&mut world);

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
        retarget(&mut world);
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
        retarget(&mut world);
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
        retarget(&mut world);
        let mut system = read_markers.into_system();
        system.initialize(&mut world);
        system.run_and_apply((), &mut world);

        world.insert(hosting(second), document);
        retarget(&mut world);
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
        retarget(&mut world);
        let mut system = read_markers.into_system();
        system.initialize(&mut world);
        system.run_and_apply((), &mut world);

        world.get_resource_mut::<ActiveEditor>().unwrap().0 = Some(second);
        retarget(&mut world);
        system.run_and_apply((), &mut world);
        world.get_resource_mut::<ActiveEditor>().unwrap().0 = Some(first);
        retarget(&mut world);
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
        retarget(&mut world);
        system.run_and_apply((), &mut world);

        assert_eq!(world.get_resource::<FramesSeen>().unwrap().0, vec![1, 3]);
    }

    #[derive(Resource, Default)]
    struct Heard(Vec<u32>);

    #[derive(Resource, Default)]
    struct Answered(Vec<concerto_ecs::Entity>);

    fn record(
        mut events: crate::hosted::HostedAppEvents<Echo>,
        mut heard: ResMut<Heard>,
        mut answered: ResMut<Answered>,
    ) {
        for (document, echo) in events.read() {
            heard.0.push(echo.0);
            answered.0.push(document);
        }
    }

    #[derive(Resource)]
    struct Hearing(Option<concerto_ecs::system::BoxedSystem>);

    fn listen(world: &mut World) {
        let mut system: concerto_ecs::system::BoxedSystem = Box::new(record.into_system());
        system.initialize(world);
        world.insert_resource(Hearing(Some(system)));
    }

    fn read_hosted(world: &mut World) {
        let mut system = world
            .get_resource_mut::<Hearing>()
            .unwrap()
            .0
            .take()
            .unwrap();
        system.run_and_apply((), world);
        world.get_resource_mut::<Hearing>().unwrap().0 = Some(system);
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
        world.insert_resource(Heard::default());
        world.insert_resource(Answered::default());
        listen(&mut world);
        (world, document)
    }

    fn heard(world: &mut World) -> Vec<u32> {
        read_hosted(world);
        world.get_resource::<Heard>().unwrap().0.clone()
    }

    fn answered(world: &mut World) -> Vec<concerto_ecs::Entity> {
        read_hosted(world);
        world.get_resource::<Answered>().unwrap().0.clone()
    }

    #[test]
    fn a_triggered_signal_runs_in_the_edit_target_and_its_answer_comes_back() {
        let (mut world, _) = hearing_world();

        run(&mut world, say_five);

        assert_eq!(heard(&mut world), vec![5]);
    }

    #[test]
    fn signals_triggered_in_one_run_arrive_in_the_order_they_were_triggered() {
        let (mut world, _) = hearing_world();

        run(&mut world, say_one_two_three);

        assert_eq!(heard(&mut world), [1, 2, 3]);
    }

    #[test]
    fn a_queue_made_for_an_app_that_is_gone_is_discarded() {
        let (mut world, document) = hearing_world();
        let mut system = say_five.into_system();
        system.initialize(&mut world);
        system.run((), &mut world);

        world.insert(hosted_now(recipe_with(2), |_| {}), document);
        system.apply(&mut world);

        assert!(heard(&mut world).is_empty());
    }

    #[test]
    fn a_queue_made_for_an_app_that_is_gone_is_discarded_after_its_replacement_is_announced() {
        let (mut world, document) = hearing_world();
        let mut system = say_five.into_system();
        system.initialize(&mut world);
        system.run((), &mut world);

        world.insert(hosted_now(recipe_with(2), |_| {}), document);
        retarget(&mut world);
        system.apply(&mut world);

        assert!(heard(&mut world).is_empty());
    }

    #[test]
    fn a_queue_is_discarded_when_the_hosted_app_was_removed_before_it_is_applied() {
        let (mut world, document) = hearing_world();
        let mut system = say_five.into_system();
        system.initialize(&mut world);
        system.run((), &mut world);

        world.remove_component::<HostedApp>(document);
        system.apply(&mut world);

        assert!(heard(&mut world).is_empty());
    }

    #[test]
    fn signals_left_by_a_run_that_was_never_applied_are_not_sent_to_another_app() {
        let (mut world, _) = hearing_world();
        let other = world.spawn(hosted_now(recipe_with(2), |_| {}));
        let mut system = say_five.into_system();
        system.initialize(&mut world);
        system.run((), &mut world);

        world.get_resource_mut::<ActiveEditor>().unwrap().0 = Some(other);
        retarget(&mut world);
        system.run_and_apply((), &mut world);

        assert_eq!(heard(&mut world), [5]);
        assert_eq!(answered(&mut world), [other]);
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

        assert_eq!(heard(&mut world), vec![8]);
    }

    #[test]
    fn a_signalling_system_is_skipped_while_there_is_no_edit_target() {
        let (mut world, _) = hearing_world();
        world.get_resource_mut::<ActiveEditor>().unwrap().0 = None;
        retarget(&mut world);

        run(&mut world, count_and_say);

        assert_eq!(runs(&world), 0);
        assert!(heard(&mut world).is_empty());
    }

    #[test]
    fn both_parameters_decline_while_the_active_document_has_no_hosted_app() {
        let (mut world, _) = hearing_world();
        let bare = world.spawn(());
        world.get_resource_mut::<ActiveEditor>().unwrap().0 = Some(bare);
        retarget(&mut world);

        run(&mut world, read_markers);
        run(&mut world, count_and_say);

        assert_eq!(runs(&world), 0);
        assert!(heard(&mut world).is_empty());
    }

    #[test]
    fn both_parameters_keep_reaching_the_target_until_another_document_is_made_it() {
        let (mut world, document) = hearing_world();
        let other = world.spawn(hosted_now(recipe_with(2), |_| {}));
        world.get_resource_mut::<ActiveEditor>().unwrap().0 = Some(other);

        run(&mut world, read_markers);
        run(&mut world, count_and_say);
        assert_eq!(seen(&world), [1]);
        assert_eq!(answered(&mut world), [document]);
        world.get_resource_mut::<Runs>().unwrap().0 = 0;
        world.get_resource_mut::<Answered>().unwrap().0.clear();

        retarget(&mut world);
        run(&mut world, read_markers);
        run(&mut world, count_and_say);

        assert_eq!(runs(&world), 2);
        assert_eq!(seen(&world), [2]);
        assert_eq!(answered(&mut world), [other]);
    }

    #[test]
    fn both_parameters_reach_an_app_replaced_on_the_target_at_once() {
        let (mut world, document) = hearing_world();
        world.insert(hosted_now(recipe_with(2), |_| {}), document);

        run(&mut world, read_markers);
        run(&mut world, count_and_say);

        assert_eq!(runs(&world), 2);
        assert_eq!(seen(&world), [2]);
        assert_eq!(answered(&mut world), [document]);
    }

    #[test]
    fn both_parameters_decline_while_no_document_is_the_edit_target() {
        let (mut world, _) = hearing_world();
        world.trigger(crate::hosted::SetEditTarget(None));

        run(&mut world, read_markers);
        run(&mut world, count_and_say);

        assert_eq!(runs(&world), 0);
        assert!(heard(&mut world).is_empty());
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
    fn both_parameters_conflict_with_a_system_that_moves_the_edit_target() {
        let mut writer = SystemAccess::default();
        writer.write_component::<crate::hosted::EditTarget>();

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
    struct Moves(Vec<String>);

    fn note_add(
        on: On<concerto_ecs::signal::lifecycle::Add<crate::hosted::EditTarget>>,
        mut moves: ResMut<Moves>,
    ) {
        moves.0.push(format!("add {}", on.signal().entity.index()));
    }

    fn note_remove(
        on: On<concerto_ecs::signal::lifecycle::Remove<crate::hosted::EditTarget>>,
        mut moves: ResMut<Moves>,
    ) {
        moves
            .0
            .push(format!("remove {}", on.signal().entity.index()));
    }

    fn watch_moves(world: &mut World) {
        world.insert_resource(Moves::default());
        world.add_listener(note_add);
        world.add_listener(note_remove);
    }

    fn moves(world: &mut World) -> Vec<String> {
        std::mem::take(&mut world.get_resource_mut::<Moves>().unwrap().0)
    }

    #[test]
    fn each_way_the_target_changes_is_a_remove_or_an_add() {
        let (mut world, document) = seam_world(1);
        watch_moves(&mut world);
        let d = document.index();

        world.insert(hosted_now(recipe_with(2), |_| {}), document);
        retarget(&mut world);
        assert_eq!(
            moves(&mut world),
            [format!("remove {d}"), format!("add {d}")]
        );

        world.get_resource_mut::<ActiveEditor>().unwrap().0 = None;
        retarget(&mut world);
        assert_eq!(moves(&mut world), [format!("remove {d}")]);

        world.get_resource_mut::<ActiveEditor>().unwrap().0 = Some(document);
        retarget(&mut world);
        assert_eq!(moves(&mut world), [format!("add {d}")]);

        world.despawn(document);
        assert_eq!(moves(&mut world), [format!("remove {d}")]);
    }

    #[test]
    fn both_parameters_decline_while_the_game_starts() {
        let (mut world, document) = hearing_world();
        start_play(&mut world, document, recipe_with(2), |_| {});

        run(&mut world, read_markers);
        run(&mut world, count_and_say);

        assert_eq!(runs(&world), 0);
        assert!(heard(&mut world).is_empty());
    }

    #[test]
    fn parking_and_returning_the_edit_app_move_the_target_and_the_play_world_is_read_while_playing()
    {
        let (mut world, document) = seam_world(1);
        watch_moves(&mut world);
        let d = document.index();
        let mut system = read_markers.into_system();
        system.initialize(&mut world);

        play_now(&mut world, document, recipe_with(2), |_| {});
        system.run_and_apply((), &mut world);
        assert_eq!(seen(&world), [2]);

        world.trigger(crate::hosted::StopPlay(document));
        system.run_and_apply((), &mut world);

        assert_eq!(seen(&world), [1]);
        assert_eq!(
            moves(&mut world),
            [
                format!("remove {d}"),
                format!("add {d}"),
                format!("remove {d}"),
                format!("add {d}")
            ]
        );
    }
}

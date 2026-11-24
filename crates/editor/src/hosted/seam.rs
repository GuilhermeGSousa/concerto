use std::ops::{Deref, DerefMut};

use concerto_ecs::{
    Entity, Query, With, World,
    component::SystemTicks,
    query::state::QueryState,
    signal::Signal,
    system::{
        access::SystemAccess,
        input::{ForeignTicks, ReadOnlySystemInput, SystemInput, SystemInputData, ValidationError},
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
    ticks: SystemTicks,
) -> Option<(Entity, &'w World)> {
    // SAFETY: the query reads only what `read_edit_target` declares, which the caller holds.
    let targets = unsafe { Query::new(world, targets, ticks) };
    targets
        .iter_inner()
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
pub struct EditWorld<'world, 'state, T: ReadOnlySystemInput + 'static>(
    SystemInputData<'world, 'state, T>,
);

impl<'world, 'state, T: ReadOnlySystemInput + 'static> Deref for EditWorld<'world, 'state, T> {
    type Target = SystemInputData<'world, 'state, T>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T: ReadOnlySystemInput + 'static> DerefMut for EditWorld<'_, '_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

/// An [`Editor`] parameter's state, rebuilt from its first form whenever the edit target is another world.
pub struct EditWorldState<S> {
    targets: Targets,
    built_for: Option<WorldId>,
    ticks: ForeignTicks,
    first: S,
    current: S,
}

impl<S> EditWorldState<S> {
    fn for_world(&self, scene: &World) -> Option<&S> {
        (self.built_for == Some(scene.id())).then_some(&self.current)
    }
}

impl<T: ReadOnlySystemInput + 'static> SystemInput for EditWorld<'_, '_, T>
where
    T::State: Clone,
{
    type State = EditWorldState<T::State>;
    type Data<'world, 'state> = EditWorld<'world, 'state, T>;

    fn init_state(world: &mut World) -> Self::State {
        let first = T::init_state(world);
        EditWorldState {
            targets: world.query(),
            built_for: None,
            ticks: ForeignTicks::default(),
            current: first.clone(),
            first,
        }
    }

    unsafe fn get_data<'world, 'state>(
        state: &'state mut Self::State,
        world: UnsafeWorldCell<'world>,
        ticks: SystemTicks,
    ) -> Result<Self::Data<'world, 'state>, ValidationError> {
        // SAFETY: `fill_access` declares what `edit_target` reads.
        let (_, scene) = unsafe { edit_target(world, &mut state.targets, ticks) }
            .ok_or_else(no_edit_target::<Self>)?;
        if state.for_world(scene).is_none() {
            state.current = state.first.clone();
            state.built_for = Some(scene.id());
            state.ticks = ForeignTicks::default();
        }
        let ticks = state.ticks.begin(scene);
        // SAFETY: the scene's world belongs to its hosted `App`, which no system runs while the
        // editor's systems do, and `T` only reads it.
        unsafe { T::get_data(&mut state.current, scene.as_unsafe_world_cell(), ticks) }
            .map(EditWorld)
    }

    fn ran(state: &mut Self::State) {
        T::ran(&mut state.current);
        state.ticks.commit();
    }

    fn fill_access(_meta: &mut SystemMetadata, access: &mut SystemAccess) {
        read_edit_target(access);
    }
}

impl<T: ReadOnlySystemInput + 'static> ReadOnlySystemInput for EditWorld<'_, '_, T> where
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
pub struct EditWorldSignals<'state> {
    queue: &'state mut SignalQueue,
}

impl EditWorldSignals<'_> {
    /// Queues `signal`; its listeners run in the edit target when this system's commands are applied.
    pub fn trigger<S: Signal>(&mut self, signal: S) {
        self.queue
            .signals
            .push(Box::new(move |hosted| hosted.trigger(signal)));
    }
}

impl SystemInput for EditWorldSignals<'_> {
    type State = SignalQueue;
    type Data<'world, 'state> = EditWorldSignals<'state>;

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
        ticks: SystemTicks,
    ) -> Result<Self::Data<'world, 'state>, ValidationError> {
        // SAFETY: `fill_access` declares what `edit_target` reads.
        let target = unsafe { edit_target(world, &mut state.targets, ticks) }
            .map(|(document, scene)| (document, scene.id()));
        if state.made_for != target {
            state.signals.clear();
        }
        state.made_for = target;
        if target.is_none() {
            return Err(no_edit_target::<Self>());
        }
        Ok(EditWorldSignals { queue: state })
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
    use concerto_ecs::{
        Component, IntoSystem, Query, Res, ResMut, Resource, System, World,
        query::change_detection::DetectChanges,
    };

    #[derive(Component)]
    struct Marker(u32);

    #[derive(Component)]
    struct Other;

    #[derive(Resource, Default)]
    struct Seen(Vec<u32>);

    #[derive(Resource, Default)]
    struct Runs(usize);

    fn read_markers(
        markers: EditWorld<Query<&Marker>>,
        mut seen: ResMut<Seen>,
        mut runs: ResMut<Runs>,
    ) {
        runs.0 += 1;
        let mut markers: Vec<u32> = markers.iter().map(|marker| marker.0).collect();
        markers.sort_unstable();
        seen.0 = markers;
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

    fn activate(world: &mut World, document: Option<concerto_ecs::Entity>) {
        world.get_resource_mut::<ActiveEditor>().unwrap().0 = document;
        retarget(world);
    }

    fn hosting(spawn: impl FnOnce(&mut World)) -> HostedApp {
        hosted_now(plain_recipe, |app| spawn(app.main_mut().world_mut()))
    }

    fn markers(values: &'static [u32], others: usize) -> impl FnOnce(&mut World) {
        move |world: &mut World| {
            for _ in 0..others {
                world.spawn(Other);
            }
            for value in values {
                world.spawn(Marker(*value));
            }
            world.spawn((Marker(0), Other));
        }
    }

    fn seam_world() -> HostWorld {
        let HostWorld {
            mut world,
            document,
        } = host_world(plain_recipe);
        world.insert(hosting(markers(&[1], 0)), document);
        retarget(&mut world);
        world.insert_resource(Seen::default());
        world.insert_resource(Runs::default());
        HostWorld { world, document }
    }

    fn runs(world: &World) -> usize {
        world.get_resource::<Runs>().unwrap().0
    }

    fn seen(world: &World) -> &[u32] {
        &world.get_resource::<Seen>().unwrap().0
    }

    fn scene_of(world: &mut World, document: concerto_ecs::Entity) -> &mut World {
        world
            .get_component_for_entity_mut::<HostedApp>(document)
            .unwrap()
            .app
            .main_mut()
            .world_mut()
    }

    #[test]
    fn edit_world_reads_its_target_through_replacements_changes_of_document_and_play() {
        let HostWorld {
            mut world,
            document,
        } = seam_world();
        let other = world.spawn(hosting(markers(&[7], 2)));
        let mut system = read_markers.into_system();
        system.initialize(&mut world);
        system.run_and_apply((), &mut world);
        assert_eq!(seen(&world), [0, 1]);

        world.insert(hosting(markers(&[2, 3], 3)), document);
        system.run_and_apply((), &mut world);
        assert_eq!(seen(&world), [0, 2, 3], "a replacement is read at once");
        world.insert(hosting(markers(&[4], 0)), document);
        retarget(&mut world);
        system.run_and_apply((), &mut world);
        assert_eq!(seen(&world), [0, 4]);

        activate(&mut world, Some(other));
        system.run_and_apply((), &mut world);
        assert_eq!(seen(&world), [0, 7]);
        activate(&mut world, Some(document));
        system.run_and_apply((), &mut world);
        assert_eq!(seen(&world), [0, 4]);

        play_now(&mut world, document, plain_recipe, |app| {
            app.main_mut().world_mut().spawn(Marker(9));
        });
        system.run_and_apply((), &mut world);
        assert_eq!(seen(&world), [9]);
        world.trigger(crate::hosted::StopPlay(document));
        system.run_and_apply((), &mut world);
        assert_eq!(seen(&world), [0, 4]);
    }

    fn say_one_two_three(mut scene: EditWorldSignals) {
        scene.trigger(Say(1));
        scene.trigger(Say(2));
        scene.trigger(Say(3));
    }

    fn heard(world: &mut World, document: concerto_ecs::Entity) -> Vec<u32> {
        scene_of(world, document)
            .get_resource::<concerto_ecs::events::event_channel::EventChannel<Echo>>()
            .unwrap()
            .read_from(&mut concerto_ecs::events::event_cursor::EventCursor::default())
            .map(|echo| echo.0)
            .collect()
    }

    #[test]
    fn signals_reach_the_target_in_order_and_a_queue_for_an_app_that_is_gone_is_dropped() {
        let HostWorld {
            mut world,
            document,
        } = seam_world();

        run(&mut world, say_one_two_three);
        assert_eq!(heard(&mut world, document), [1, 2, 3]);

        let mut system = say_one_two_three.into_system();
        system.initialize(&mut world);
        system.run((), &mut world);
        world.insert(hosting(markers(&[2], 0)), document);
        system.apply(&mut world);
        assert!(heard(&mut world, document).is_empty());
    }

    #[derive(Resource)]
    struct NotInTheScene;

    fn count_and_say(mut scene: EditWorldSignals, mut runs: ResMut<Runs>) {
        runs.0 += 1;
        scene.trigger(Say(5));
    }

    fn read_missing(_: EditWorld<Res<NotInTheScene>>, mut runs: ResMut<Runs>) {
        runs.0 += 1;
    }

    #[test]
    fn edit_world_and_its_signals_skip_the_system_without_a_target_to_reach() {
        let HostWorld {
            mut world,
            document,
        } = seam_world();
        run(&mut world, read_missing);
        assert_eq!(
            runs(&world),
            0,
            "a resource the scene lacks skips the system"
        );

        let bare = world.spawn(());
        activate(&mut world, Some(bare));
        run(&mut world, read_markers);
        run(&mut world, count_and_say);
        assert_eq!(runs(&world), 0);

        activate(&mut world, None);
        run(&mut world, read_markers);
        run(&mut world, count_and_say);
        assert_eq!(runs(&world), 0);

        activate(&mut world, Some(document));
        start_play(&mut world, document, plain_recipe, |_| {});
        run(&mut world, read_markers);
        run(&mut world, count_and_say);
        assert_eq!(runs(&world), 0);
        assert!(heard(&mut world, document).is_empty());
    }

    #[derive(Resource, Default)]
    struct Changes(Vec<Vec<u32>>);

    fn changed_markers(
        markers: EditWorld<Query<&Marker, concerto_ecs::Changed<Marker>>>,
        mut changes: ResMut<Changes>,
    ) {
        let mut seen: Vec<u32> = markers.iter().map(|marker| marker.0).collect();
        seen.sort_unstable();
        changes.0.push(seen);
    }

    fn changes(world: &mut World) -> Vec<Vec<u32>> {
        std::mem::take(&mut world.get_resource_mut::<Changes>().unwrap().0)
    }

    fn touch_marker(world: &mut World, document: concerto_ecs::Entity, value: u32) {
        run(
            scene_of(world, document),
            move |mut markers: Query<&mut Marker>| {
                for mut marker in markers.iter_mut() {
                    if marker.0 == value {
                        marker.0 = value;
                    }
                }
            },
        );
    }

    #[test]
    fn edit_world_sees_changes_since_its_last_run_through_skipped_runs_and_a_new_target() {
        let HostWorld {
            mut world,
            document,
        } = seam_world();
        world.insert_resource(Changes::default());
        let other = world.spawn(hosting(markers(&[3], 0)));
        for _ in 0..100_000 {
            scene_of(&mut world, other).increment_change_tick();
        }
        let mut system = changed_markers.into_system();
        system.initialize(&mut world);

        system.run_and_apply((), &mut world);
        system.run_and_apply((), &mut world);
        touch_marker(&mut world, document, 1);
        system.run_and_apply((), &mut world);
        system.run_and_apply((), &mut world);
        assert_eq!(changes(&mut world), [vec![0, 1], vec![], vec![1], vec![]]);

        touch_marker(&mut world, document, 1);
        activate(&mut world, None);
        system.run_and_apply((), &mut world);
        activate(&mut world, Some(document));
        system.run_and_apply((), &mut world);
        assert_eq!(
            changes(&mut world),
            [vec![1]],
            "a skipped run keeps the change"
        );

        activate(&mut world, Some(other));
        system.run_and_apply((), &mut world);
        system.run_and_apply((), &mut world);
        assert_eq!(changes(&mut world), [vec![0, 3], vec![]]);
    }

    #[derive(Resource, Default)]
    struct Flags(Vec<bool>);

    fn selection_changed(
        selection: EditWorld<Res<crate::tools::selection::Selection>>,
        mut flags: ResMut<Flags>,
    ) {
        flags.0.push(selection.is_changed());
    }

    #[test]
    fn edit_world_sees_a_signal_sent_through_edit_world_signals() {
        let HostWorld {
            mut world,
            document,
        } = seam_world();
        world.insert_resource(Flags::default());
        world.insert(
            hosted_now(plain_recipe, |app| {
                app.register_plugin(crate::tools::EditToolsCorePlugin);
            }),
            document,
        );
        retarget(&mut world);
        let entity = scene_of(&mut world, document).spawn(());
        let mut system = selection_changed.into_system();
        system.initialize(&mut world);

        system.run_and_apply((), &mut world);
        system.run_and_apply((), &mut world);
        run(&mut world, move |mut scene: EditWorldSignals| {
            scene.trigger(crate::tools::selection::Select::only(entity));
        });
        system.run_and_apply((), &mut world);
        system.run_and_apply((), &mut world);

        assert_eq!(
            world.get_resource::<Flags>().unwrap().0,
            [true, false, true, false]
        );
    }

    #[derive(Resource, Default)]
    struct Removed(Vec<concerto_ecs::Entity>);

    fn read_removed(
        mut removed: EditWorld<concerto_ecs::component::removed::RemovedComponents<Marker>>,
        mut seen: ResMut<Removed>,
    ) {
        seen.0.extend(removed.read());
    }

    fn tracking_markers(blanks: usize) -> (HostedApp, concerto_ecs::Entity) {
        let mut marked = None;
        let hosted = hosted_now(plain_recipe, |app| {
            app.track_removals::<Marker>();
            let world = app.main_mut().world_mut();
            for _ in 0..blanks {
                world.spawn(());
            }
            marked = Some(world.spawn(Marker(1)));
        });
        (hosted, marked.unwrap())
    }

    #[test]
    fn edit_world_reads_removals_in_the_target_and_in_the_next_target() {
        let HostWorld {
            mut world,
            document: first,
        } = seam_world();
        world.insert_resource(Removed::default());
        let (hosted, a) = tracking_markers(0);
        world.insert(hosted, first);
        let (hosted, b) = tracking_markers(1);
        let second = world.spawn(hosted);
        retarget(&mut world);
        let mut system = read_removed.into_system();
        system.initialize(&mut world);
        system.run_and_apply((), &mut world);

        scene_of(&mut world, first).remove_component::<Marker>(a);
        system.run_and_apply((), &mut world);
        scene_of(&mut world, second).remove_component::<Marker>(b);
        activate(&mut world, Some(second));
        system.run_and_apply((), &mut world);
        system.run_and_apply((), &mut world);

        assert_ne!(a, b);
        assert_eq!(world.get_resource::<Removed>().unwrap().0, [a, b]);
    }
}

use concerto_ecs::{
    component::scene::SceneComponent,
    system::schedule::{InternedScheduleLabel, ScheduleLabel, Schedules},
    IntoSetConfigs, IntoSystemConfig, IntoSystemConfigs, Resource, World,
};

use crate::{extractor::ExtractFn, schedule_groups::Startup};

#[doc(hidden)]
#[derive(Default)]
pub struct SubApps {
    main: SubApp,
    render: SubApp,
    extract_fn: Option<ExtractFn>,
}

impl SubApps {
    pub fn main(&self) -> &SubApp {
        &self.main
    }

    pub fn main_mut(&mut self) -> &mut SubApp {
        &mut self.main
    }

    pub fn render(&self) -> &SubApp {
        &self.render
    }

    pub fn render_mut(&mut self) -> &mut SubApp {
        &mut self.render
    }

    pub fn startup(&mut self) {
        self.main.world.run_schedule(Startup);
        self.render.world.run_schedule(Startup);
    }

    pub fn update(&mut self) {
        self.main.update();

        if let Some(mut extract_fn) = self.extract_fn.take() {
            extract_fn(&mut self.main.world, &mut self.render.world);
            self.extract_fn = Some(extract_fn);
        }

        self.render.update();
    }

    pub fn set_extract_fn(&mut self, extract_fn: impl FnMut(&mut World, &mut World) + 'static) {
        self.extract_fn = Some(Box::new(extract_fn));
    }
}

/// One world with its own schedules, such as the main or the render world of an
/// [`App`](crate::App).
///
/// A new `SubApp` has an empty [`Schedules`] resource. Its update schedule, set with
/// [`set_update_schedule`](SubApp::set_update_schedule), runs once per
/// [`update`](SubApp::update).
///
/// # Examples
///
/// ```
/// use concerto_app::subapp::SubApp;
/// use concerto_ecs::system::schedule::ScheduleLabel;
///
/// #[derive(ScheduleLabel, Clone, PartialEq, Eq, Hash, Debug)]
/// struct Tick;
///
/// fn count() {}
///
/// let mut sub_app = SubApp::default();
/// sub_app.add_system(Tick, count).set_update_schedule(Tick);
/// ```
pub struct SubApp {
    world: World,
    update_schedule: Option<InternedScheduleLabel>,
}

impl Default for SubApp {
    fn default() -> Self {
        let mut world = World::new();
        world.init_resource::<Schedules>();
        Self {
            world,
            update_schedule: None,
        }
    }
}

impl SubApp {
    /// Inserts a resource into this world, replacing any of the same type.
    ///
    /// # Examples
    ///
    /// ```
    /// use concerto_app::subapp::SubApp;
    /// use concerto_ecs::Resource;
    ///
    /// #[derive(Resource)]
    /// struct Score(u64);
    ///
    /// let mut sub_app = SubApp::default();
    /// sub_app.insert_resource(Score(0));
    /// ```
    pub fn insert_resource<R: Resource>(&mut self, value: R) -> &mut Self {
        self.world.insert_resource(value);
        self
    }

    /// Removes a resource from this world and returns it, if present.
    ///
    /// # Examples
    ///
    /// ```
    /// use concerto_app::subapp::SubApp;
    /// use concerto_ecs::Resource;
    ///
    /// #[derive(Resource)]
    /// struct Score(u64);
    ///
    /// let mut sub_app = SubApp::default();
    /// sub_app.insert_resource(Score(0));
    ///
    /// assert_eq!(sub_app.remove_resource::<Score>().map(|score| score.0), Some(0));
    /// ```
    pub fn remove_resource<R: Resource>(&mut self) -> Option<R> {
        self.world.remove_resource::<R>()
    }

    /// Returns a resource of this world, if present.
    ///
    /// # Examples
    ///
    /// ```
    /// use concerto_app::subapp::SubApp;
    /// use concerto_ecs::Resource;
    ///
    /// #[derive(Resource)]
    /// struct Score(u64);
    ///
    /// let mut sub_app = SubApp::default();
    /// sub_app.insert_resource(Score(0));
    ///
    /// assert_eq!(sub_app.get_resource::<Score>().map(|score| score.0), Some(0));
    /// ```
    pub fn get_resource<R: Resource>(&self) -> Option<&R> {
        self.world.get_resource::<R>()
    }

    /// Returns a resource of this world mutably, if present.
    ///
    /// # Examples
    ///
    /// ```
    /// use concerto_app::subapp::SubApp;
    /// use concerto_ecs::Resource;
    ///
    /// #[derive(Resource)]
    /// struct Score(u64);
    ///
    /// let mut sub_app = SubApp::default();
    /// sub_app.insert_resource(Score(0));
    ///
    /// if let Some(score) = sub_app.get_resource_mut::<Score>() {
    ///     score.0 += 10;
    /// }
    /// ```
    pub fn get_resource_mut<R: Resource>(&mut self) -> Option<&mut R> {
        self.world.get_resource_mut::<R>()
    }

    /// Registers a [`SceneComponent`] so serialized scenes can spawn it by type name.
    ///
    /// # Examples
    ///
    /// ```
    /// use concerto_app::subapp::SubApp;
    /// use concerto_foundation::transform::Transform;
    ///
    /// let mut sub_app = SubApp::default();
    /// sub_app.register_scene_component::<Transform>();
    /// ```
    pub fn register_scene_component<T: SceneComponent>(&mut self) -> &mut Self {
        self.world.register_component_type::<T>();
        self
    }

    /// Adds a system to this world's schedule labelled `update_group`.
    ///
    /// See [`Schedule::add_system`](concerto_ecs::Schedule::add_system).
    ///
    /// # Panics
    ///
    /// Panics if this world's [`Schedules`] have already been compiled.
    ///
    /// # Examples
    ///
    /// ```
    /// use concerto_app::subapp::SubApp;
    /// use concerto_ecs::system::schedule::ScheduleLabel;
    ///
    /// #[derive(ScheduleLabel, Clone, PartialEq, Eq, Hash, Debug)]
    /// struct Tick;
    ///
    /// fn count() {}
    ///
    /// let mut sub_app = SubApp::default();
    /// sub_app.add_system(Tick, count);
    /// ```
    pub fn add_system<M>(
        &mut self,
        update_group: impl ScheduleLabel,
        system: impl IntoSystemConfig<M> + 'static,
    ) -> &mut Self {
        self.get_resource_mut::<Schedules>()
            .expect("Schedules resource not found!")
            .add_system(update_group, system);
        self
    }

    /// Adds several systems to this world's schedule labelled `update_group`.
    ///
    /// See [`Schedule::add_systems`](concerto_ecs::Schedule::add_systems).
    ///
    /// # Panics
    ///
    /// Panics if this world's [`Schedules`] have already been compiled.
    ///
    /// # Examples
    ///
    /// ```
    /// use concerto_app::subapp::SubApp;
    /// use concerto_ecs::{IntoSystemConfigs, system::schedule::ScheduleLabel};
    ///
    /// #[derive(ScheduleLabel, Clone, PartialEq, Eq, Hash, Debug)]
    /// struct Tick;
    ///
    /// fn read() {}
    /// fn write() {}
    ///
    /// let mut sub_app = SubApp::default();
    /// sub_app.add_systems(Tick, (read, write).chain());
    /// ```
    pub fn add_systems<M>(
        &mut self,
        update_group: impl ScheduleLabel,
        systems: impl IntoSystemConfigs<M>,
    ) -> &mut Self {
        self.get_resource_mut::<Schedules>()
            .expect("Schedules resource not found!")
            .add_systems(update_group, systems);
        self
    }

    /// Adds ordering constraints between sets in this world's schedule labelled
    /// `update_group`.
    ///
    /// See [`Schedule::configure_sets`](concerto_ecs::Schedule::configure_sets).
    ///
    /// # Panics
    ///
    /// Panics if this world's [`Schedules`] have already been compiled.
    ///
    /// # Examples
    ///
    /// ```
    /// use concerto_app::subapp::SubApp;
    /// use concerto_ecs::{IntoSetConfigs, SystemSet, system::schedule::ScheduleLabel};
    ///
    /// #[derive(ScheduleLabel, Clone, PartialEq, Eq, Hash, Debug)]
    /// struct Tick;
    ///
    /// #[derive(SystemSet, Clone, PartialEq, Eq, Hash, Debug)]
    /// enum Phase {
    ///     Read,
    ///     Write,
    /// }
    ///
    /// let mut sub_app = SubApp::default();
    /// sub_app.configure_sets(Tick, (Phase::Read, Phase::Write).chain());
    /// ```
    pub fn configure_sets(
        &mut self,
        update_group: impl ScheduleLabel,
        configs: impl IntoSetConfigs,
    ) -> &mut Self {
        self.get_resource_mut::<Schedules>()
            .expect("Schedules resource not found!")
            .configure_sets(update_group, configs);
        self
    }

    /// Runs this world's update schedule.
    ///
    /// Does nothing if no update schedule is set or it has not been compiled into
    /// this world's [`CompiledSchedules`](concerto_ecs::system::schedule::CompiledSchedules).
    ///
    /// # Examples
    ///
    /// ```
    /// use concerto_app::subapp::SubApp;
    ///
    /// let mut sub_app = SubApp::default();
    /// sub_app.update();
    /// ```
    pub fn update(&mut self) {
        if let Some(label) = self.update_schedule {
            self.world.run_schedule(label);
        }
    }

    /// Sets the schedule [`update`](SubApp::update) runs.
    ///
    /// # Examples
    ///
    /// ```
    /// use concerto_app::subapp::SubApp;
    /// use concerto_ecs::system::schedule::ScheduleLabel;
    ///
    /// #[derive(ScheduleLabel, Clone, PartialEq, Eq, Hash, Debug)]
    /// struct Tick;
    ///
    /// let mut sub_app = SubApp::default();
    /// sub_app.set_update_schedule(Tick);
    /// ```
    pub fn set_update_schedule(&mut self, label: impl ScheduleLabel) -> &mut Self {
        self.update_schedule = Some(label.intern());
        self
    }

    /// Returns this sub-application's world.
    ///
    /// # Examples
    ///
    /// ```
    /// use concerto_app::subapp::SubApp;
    ///
    /// let sub_app = SubApp::default();
    /// let world = sub_app.world();
    /// ```
    pub fn world(&self) -> &World {
        &self.world
    }

    /// Returns this sub-application's world mutably.
    ///
    /// # Examples
    ///
    /// ```
    /// use concerto_app::subapp::SubApp;
    ///
    /// let mut sub_app = SubApp::default();
    /// let world = sub_app.world_mut();
    /// ```
    pub fn world_mut(&mut self) -> &mut World {
        &mut self.world
    }
}

use ecs::{
    component::scene::SceneComponent,
    system::schedule::{InternedScheduleLabel, ScheduleLabel, Schedules},
    IntoSystemConfig, Resource, World,
};

use crate::{extractor::ExtractFn, schedule_groups::Startup};

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
    pub fn insert_resource<R: Resource>(&mut self, value: R) -> &mut Self {
        self.world.insert_resource(value);
        self
    }
    pub fn remove_resource<R: Resource>(&mut self) -> Option<R> {
        self.world.remove_resource::<R>()
    }

    pub fn get_resource<R: Resource>(&self) -> Option<&R> {
        self.world.get_resource::<R>()
    }

    pub fn get_resource_mut<R: Resource>(&mut self) -> Option<&mut R> {
        self.world.get_resource_mut::<R>()
    }

    pub fn register_scene_component<T: SceneComponent>(&mut self) -> &mut Self {
        self.world.register_component_type::<T>();
        self
    }

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

    pub fn update(&mut self) {
        if let Some(label) = self.update_schedule {
            self.world.run_schedule(label);
        }
    }

    pub fn set_update_schedule(&mut self, label: impl ScheduleLabel) -> &mut Self {
        self.update_schedule = Some(label.intern());
        self
    }

    pub fn world(&self) -> &World {
        &self.world
    }

    pub fn world_mut(&mut self) -> &mut World {
        &mut self.world
    }
}

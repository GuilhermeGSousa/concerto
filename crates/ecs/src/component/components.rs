use super::{
    Component, ComponentId, ComponentLifecycleCallbacks, RequiredComponent, RequiredComponents,
};
use crate::utilities::TypeIdMap;

pub(crate) struct Requirements {
    direct: RequiredComponents,
    all: RequiredComponents,
    declared: usize,
}

#[derive(Default)]
pub(crate) struct ComponentInfo {
    lifecycle: Option<ComponentLifecycleCallbacks>,
    requirements: Option<Requirements>,
}

/// What a world knows about each component type: its lifecycle callbacks and its requirements.
#[derive(Default)]
pub(crate) struct Components {
    infos: TypeIdMap<ComponentInfo>,
}

impl Components {
    pub(crate) fn register_lifecycle<T: Component>(&mut self) {
        self.infos
            .entry(ComponentId::of::<T>())
            .or_default()
            .lifecycle
            .get_or_insert_with(ComponentLifecycleCallbacks::from_component::<T>);
    }

    pub(crate) fn lifecycle(&self, id: &ComponentId) -> Option<&ComponentLifecycleCallbacks> {
        self.infos.get(id)?.lifecycle.as_ref()
    }

    pub(crate) fn required(&self, id: ComponentId) -> Option<&RequiredComponents> {
        self.infos.get(&id)?.requirements.as_ref().map(|r| &r.all)
    }

    pub(crate) fn register_requirements<T: Component>(&mut self) {
        self.register_required_in::<T>(&mut Vec::new());
    }

    pub(crate) fn register_required_in<T: Component>(
        &mut self,
        stack: &mut Vec<(ComponentId, &'static str)>,
    ) {
        let id = ComponentId::of::<T>();
        if self.required(id).is_some() {
            return;
        }
        if let Some(start) = stack.iter().position(|(seen, _)| *seen == id) {
            let path = stack[start..]
                .iter()
                .map(|(_, name)| *name)
                .chain([T::name()])
                .collect::<Vec<_>>()
                .join(" -> ");
            panic!("required components form a cycle: {path}");
        }
        let mut direct = RequiredComponents::default();
        T::register_required(&mut direct);
        let declared = direct.len();
        stack.push((id, T::name()));
        direct.register_all(self, stack);
        stack.pop();
        let all = direct.flatten(&|id| self.required(id));
        self.infos.entry(id).or_default().requirements = Some(Requirements {
            direct,
            all,
            declared,
        });
    }

    /// Registers `U`, panics if `T` requiring it would form a cycle, and returns the components whose requirements change.
    pub(crate) fn prepare_external<T: Component, U: Component>(&mut self) -> Vec<ComponentId> {
        let target = ComponentId::of::<T>();
        self.register_required_in::<U>(&mut Vec::new());
        let cycles = target == ComponentId::of::<U>()
            || self
                .required(ComponentId::of::<U>())
                .unwrap()
                .contains(target);
        if cycles {
            panic!(
                "required components form a cycle: {} -> {} -> {}",
                T::name(),
                U::name(),
                T::name()
            );
        }
        self.affected_by(target)
    }

    fn affected_by(&self, target: ComponentId) -> Vec<ComponentId> {
        self.infos
            .iter()
            .filter_map(|(id, info)| Some((id, info.requirements.as_ref()?)))
            .filter(|(id, requirements)| **id == target || requirements.all.contains(target))
            .map(|(id, _)| *id)
            .collect()
    }

    pub(crate) fn set_external<T: Component, U: Component>(&mut self, constructor: fn() -> U) {
        let target = ComponentId::of::<T>();
        self.register_required_in::<T>(&mut Vec::new());
        let requirements = self
            .infos
            .get_mut(&target)
            .and_then(|info| info.requirements.as_mut())
            .unwrap();
        let declared = requirements
            .direct
            .iter()
            .take(requirements.declared)
            .any(|entry| entry.id() == ComponentId::of::<U>());
        if !declared {
            requirements.direct.set(constructor);
        }
        let mut affected = self.affected_by(target);
        affected.sort_by_key(|id| self.required(*id).map_or(0, RequiredComponents::len));
        for id in affected {
            let all = self.infos[&id]
                .requirements
                .as_ref()
                .unwrap()
                .direct
                .flatten(&|id| self.required(id));
            self.infos
                .get_mut(&id)
                .and_then(|info| info.requirements.as_mut())
                .unwrap()
                .all = all;
        }
    }

    pub(crate) fn missing_required(
        &self,
        present: &[ComponentId],
        added: &[ComponentId],
    ) -> Vec<RequiredComponent> {
        let mut missing: Vec<RequiredComponent> = Vec::new();
        for id in added {
            let Some(required) = self.required(*id) else {
                continue;
            };
            for entry in required.iter() {
                if !present.contains(&entry.id()) && !missing.iter().any(|m| m.id() == entry.id()) {
                    missing.push(entry.clone());
                }
            }
        }
        missing
    }

    pub(crate) fn add_order(
        &self,
        bundle_ids: &[ComponentId],
        required: &[RequiredComponent],
    ) -> Vec<ComponentId> {
        let is_new = |id: &ComponentId| {
            bundle_ids.contains(id) || required.iter().any(|entry| entry.id() == *id)
        };
        let has_requirements = |id: &ComponentId| {
            self.required(*id)
                .is_some_and(|required| !required.is_empty())
        };
        let mut members = bundle_ids.to_vec();
        members.sort_by_key(|id| (!has_requirements(id), *id));
        members.dedup();

        let mut order: Vec<ComponentId> = Vec::new();
        for member in members {
            if let Some(requirements) = self.required(member) {
                for entry in requirements.iter() {
                    if is_new(&entry.id()) && !order.contains(&entry.id()) {
                        order.push(entry.id());
                    }
                }
            }
            if !order.contains(&member) {
                order.push(member);
            }
        }
        order
    }
}

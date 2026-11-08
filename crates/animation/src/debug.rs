use std::{
    any::{Any, TypeId},
    collections::HashMap,
};

trait Slots: Any + Send + Sync {
    fn clear(&mut self);
}

struct SlotList<T> {
    values: Vec<T>,
    used: usize,
}

impl<T: Send + Sync + 'static> Slots for SlotList<T> {
    fn clear(&mut self) {
        self.used = 0;
    }
}

/// What the nodes of one player's graph published on its last evaluation, a list per type.
#[derive(Default)]
pub struct AnimationDebug {
    slots: HashMap<TypeId, Box<dyn Slots>>,
}

impl AnimationDebug {
    /// The values of type `T` published on the last evaluation, in publishing order.
    pub fn get<T: Send + Sync + 'static>(&self) -> &[T] {
        self.slots
            .get(&TypeId::of::<T>())
            .and_then(|slots| (&**slots as &dyn Any).downcast_ref::<SlotList<T>>())
            .map_or(&[], |list| &list.values[..list.used])
    }

    pub(crate) fn clear(&mut self) {
        for slots in self.slots.values_mut() {
            slots.clear();
        }
    }

    pub(crate) fn publish<T: Default + Send + Sync + 'static>(
        &mut self,
        fill: impl FnOnce(&mut T),
    ) {
        let slots = self.slots.entry(TypeId::of::<T>()).or_insert_with(|| {
            Box::new(SlotList::<T> {
                values: Vec::new(),
                used: 0,
            })
        });
        let Some(list) = (&mut **slots as &mut dyn Any).downcast_mut::<SlotList<T>>() else {
            return;
        };
        if list.used == list.values.len() {
            list.values.push(T::default());
        }
        fill(&mut list.values[list.used]);
        list.used += 1;
    }
}

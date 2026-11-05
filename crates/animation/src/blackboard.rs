use std::collections::HashMap;

use concerto_name::Name;
use concerto_reflect::Reflect;
use glam::Vec2;

#[derive(Default)]
pub struct AnimationBlackboard {
    values: HashMap<Name, Box<dyn Reflect>>,
}

impl AnimationBlackboard {
    /// Stores `value` under `key`, replacing whatever was there, of any type.
    pub fn set<T: Reflect>(&mut self, key: Name, value: T) {
        self.values.insert(key, Box::new(value));
    }

    /// The value under `key` as `T`, or `None` if it is missing or of another type.
    pub fn get<T: Reflect>(&self, key: Name) -> Option<&T> {
        self.values.get(&key)?.downcast_ref::<T>()
    }

    pub fn get_bool(&self, key: Name) -> Option<bool> {
        self.get::<bool>(key).copied()
    }

    pub fn get_int(&self, key: Name) -> Option<u32> {
        self.get::<u32>(key).copied()
    }

    pub fn get_float(&self, key: Name) -> Option<f32> {
        self.get::<f32>(key).copied()
    }

    pub fn get_vec2(&self, key: Name) -> Option<Vec2> {
        self.get::<Vec2>(key).copied()
    }
}

#[cfg(test)]
mod tests {
    use concerto_foundation::assets::asset_store::AssetStore;
    use concerto_name::name;
    use concerto_reflect::Reflect;

    use super::AnimationBlackboard;
    use crate::{evaluation::AnimationGraphContext, test_walks::Walks};

    #[derive(Debug, PartialEq, Reflect)]
    struct Stance {
        crouched: bool,
        lean: f32,
    }

    fn read_stance(context: &AnimationGraphContext<'_>) -> Option<(bool, f32)> {
        let stance = context.blackboard().get::<Stance>(name!("stance"))?;
        Some((stance.crouched, stance.lean))
    }

    #[test]
    fn a_custom_reflect_type_round_trips_through_the_graph_context() {
        let clips = AssetStore::new();
        let graphs = AssetStore::new();
        let mut blackboard = AnimationBlackboard::default();
        blackboard.set(
            name!("stance"),
            Stance {
                crouched: true,
                lean: 0.5,
            },
        );
        let walks = Walks::new(clips, graphs);

        let stance = walks.update(&blackboard, read_stance);

        assert_eq!(stance, Some((true, 0.5)));
    }

    #[test]
    fn a_value_of_another_type_reads_as_none() {
        let mut blackboard = AnimationBlackboard::default();
        blackboard.set(name!("speed"), 2.0_f32);

        assert_eq!(blackboard.get::<f32>(name!("speed")), Some(&2.0));
        assert!(blackboard.get::<bool>(name!("speed")).is_none());
        assert!(blackboard.get_int(name!("speed")).is_none());
        assert!(blackboard.get::<f32>(name!("missing")).is_none());
    }
}

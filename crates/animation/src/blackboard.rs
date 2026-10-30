use std::collections::HashMap;

use concerto_reflect::Reflect;
use glam::Vec2;

#[derive(Default)]
pub struct AnimationBlackboard {
    values: HashMap<String, Box<dyn Reflect>>,
}

impl AnimationBlackboard {
    /// Stores `value` under `key`, replacing whatever was there, of any type.
    pub fn set<T: Reflect>(&mut self, key: impl Into<String>, value: T) {
        self.values.insert(key.into(), Box::new(value));
    }

    /// The value under `key` as `T`, or `None` if it is missing or of another type.
    pub fn get<T: Reflect>(&self, key: &str) -> Option<&T> {
        self.values.get(key)?.downcast_ref::<T>()
    }

    pub fn get_bool(&self, key: &str) -> Option<bool> {
        self.get::<bool>(key).copied()
    }

    pub fn get_int(&self, key: &str) -> Option<u32> {
        self.get::<u32>(key).copied()
    }

    pub fn get_float(&self, key: &str) -> Option<f32> {
        self.get::<f32>(key).copied()
    }

    pub fn get_vec2(&self, key: &str) -> Option<Vec2> {
        self.get::<Vec2>(key).copied()
    }
}

#[cfg(test)]
mod tests {
    use concerto_foundation::assets::asset_store::AssetStore;
    use concerto_reflect::Reflect;
    use glam::Vec2;

    use super::AnimationBlackboard;
    use crate::evaluation::AnimationGraphContext;

    #[derive(Debug, PartialEq, Reflect)]
    struct Stance {
        crouched: bool,
        lean: f32,
    }

    fn read_stance(context: &AnimationGraphContext<'_>) -> Option<(bool, f32)> {
        let stance = context.blackboard().get::<Stance>("stance")?;
        Some((stance.crouched, stance.lean))
    }

    #[test]
    fn a_custom_reflect_type_round_trips_through_the_graph_context() {
        let clips = AssetStore::new();
        let graphs = AssetStore::new();
        let mut blackboard = AnimationBlackboard::default();
        blackboard.set(
            "stance",
            Stance {
                crouched: true,
                lean: 0.5,
            },
        );
        let context = AnimationGraphContext {
            animation_clips: &clips,
            animation_graphs: &graphs,
            blackboard: &blackboard,
        };

        assert_eq!(read_stance(&context), Some((true, 0.5)));
    }

    #[test]
    fn a_value_of_another_type_reads_as_none() {
        let mut blackboard = AnimationBlackboard::default();
        blackboard.set("speed", 2.0_f32);

        assert_eq!(blackboard.get::<f32>("speed"), Some(&2.0));
        assert!(blackboard.get::<bool>("speed").is_none());
        assert!(blackboard.get_int("speed").is_none());
        assert!(blackboard.get::<f32>("missing").is_none());
    }

    #[test]
    fn setting_a_key_again_replaces_the_value_and_its_type() {
        let mut blackboard = AnimationBlackboard::default();
        blackboard.set("flag", true);
        blackboard.set("flag", 3_u32);

        assert_eq!(blackboard.get_bool("flag"), None);
        assert_eq!(blackboard.get_int("flag"), Some(3));
    }

    #[test]
    fn typed_helpers_read_their_own_types() {
        let mut blackboard = AnimationBlackboard::default();
        blackboard.set("a", Vec2::new(1.0, 2.0));
        blackboard.set("b", 1.5_f32);

        assert_eq!(blackboard.get_vec2("a"), Some(Vec2::new(1.0, 2.0)));
        assert_eq!(blackboard.get_float("b"), Some(1.5));
        assert_eq!(blackboard.get_float("a"), None);
    }
}

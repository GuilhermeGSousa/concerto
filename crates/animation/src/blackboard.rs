use std::collections::HashMap;

use concerto_name::Name;
use concerto_reflect::Reflect;
use glam::Vec2;

use crate::param::Param;

/// The named parameters an [`AnimationPlayer`](crate::player::AnimationPlayer) gives its graph's
/// nodes, each a reflected value of any type.
#[derive(Default)]
pub struct AnimationBlackboard {
    values: HashMap<Name, Box<dyn Reflect>>,
}

impl AnimationBlackboard {
    /// Stores `value` under `key`, replacing whatever was there, of any type.
    pub fn set<T: Reflect>(&mut self, key: Name, value: T) {
        self.values.insert(key, Box::new(value));
    }

    /// Stores a value built elsewhere under `key`, as [`set`](Self::set) does.
    pub fn set_boxed(&mut self, key: Name, value: Box<dyn Reflect>) {
        self.values.insert(key, value);
    }

    /// The value under `key` as `T`, or `None` if it is missing or of another type.
    pub fn get<T: Reflect>(&self, key: Name) -> Option<&T> {
        self.values.get(&key)?.downcast_ref::<T>()
    }

    /// The value `param` names, or `None` if it is missing or of another type.
    pub fn read<T: Reflect>(&self, param: &Param<T>) -> Option<&T> {
        self.get::<T>(param.0)
    }

    /// The `bool` under `key`, or `None` if it is missing or of another type.
    pub fn get_bool(&self, key: Name) -> Option<bool> {
        self.get::<bool>(key).copied()
    }

    /// The `u32` under `key`, or `None` if it is missing or of another type.
    pub fn get_int(&self, key: Name) -> Option<u32> {
        self.get::<u32>(key).copied()
    }

    /// The `f32` under `key`, or `None` if it is missing or of another type.
    pub fn get_float(&self, key: Name) -> Option<f32> {
        self.get::<f32>(key).copied()
    }

    /// The [`Vec2`] under `key`, or `None` if it is missing or of another type.
    pub fn get_vec2(&self, key: Name) -> Option<Vec2> {
        self.get::<Vec2>(key).copied()
    }
}

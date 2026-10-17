use concerto_color::Color;
use glam::{Quat, Vec3};

use crate::Reflect;

impl Reflect for f32 {}
impl Reflect for f64 {}
impl Reflect for Vec3 {}
impl Reflect for Quat {}
impl Reflect for String {}
impl Reflect for bool {}
impl Reflect for Color {}

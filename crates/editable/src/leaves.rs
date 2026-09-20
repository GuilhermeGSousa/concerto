use glam::{Quat, Vec3};

use crate::Editable;

impl Editable for f32 {}
impl Editable for f64 {}
impl Editable for Vec3 {}
impl Editable for Quat {}
impl Editable for String {}
impl Editable for bool {}

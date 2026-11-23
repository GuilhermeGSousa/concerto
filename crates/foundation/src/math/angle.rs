use concerto_reflect::{
    FromType, GetTypeRegistration, JsonLeaf, ReflectDefault, ReflectJson, TypeRegistration,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// An angle, held in radians and encoded as its radians; editors and JSON read and write it in degrees.
#[derive(Clone, Copy, Debug, Default, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Angle(f32);

impl Angle {
    pub const ZERO: Self = Self(0.0);

    pub const fn from_radians(radians: f32) -> Self {
        Self(radians)
    }

    pub fn from_degrees(degrees: f32) -> Self {
        Self((f64::from(degrees)).to_radians() as f32)
    }

    pub const fn radians(self) -> f32 {
        self.0
    }

    pub fn degrees(self) -> f32 {
        (f64::from(self.0)).to_degrees() as f32
    }
}

impl std::ops::Neg for Angle {
    type Output = Self;

    fn neg(self) -> Self {
        Self(-self.0)
    }
}

impl std::fmt::Display for Angle {
    /// The angle in degrees with a degree sign, at the precision asked for.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match f.precision() {
            Some(precision) => write!(f, "{:.*}°", precision, self.degrees()),
            None => write!(f, "{}°", self.degrees()),
        }
    }
}

concerto_reflect::impl_reflect_leaf!(@reflect Angle);

impl GetTypeRegistration for Angle {
    fn get_type_registration() -> TypeRegistration {
        let mut registration = TypeRegistration::of::<Self>();
        registration.insert(<ReflectJson as FromType<Self>>::from_type());
        registration.insert(<ReflectDefault as FromType<Self>>::from_type());
        registration
    }
}

impl JsonLeaf for Angle {
    /// The angle in degrees.
    fn to_json(&self) -> Value {
        self.degrees().to_json()
    }

    fn from_json(value: &Value) -> Result<Self, String> {
        f32::from_json(value)
            .map(Self::from_degrees)
            .map_err(|_| "expected an angle in degrees".to_owned())
    }
}

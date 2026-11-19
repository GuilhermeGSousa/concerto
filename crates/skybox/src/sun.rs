use concerto_color::Color;
use concerto_ecs::{
    component::Component,
    reflect::{ReflectComponent, ReflectSceneComponent},
};
use concerto_reflect::{Reflect, ReflectDefault};
use concerto_render::components::{camera::Camera, light::Light};
use glam::Vec3;

/// Marks the directional light that is the sun; its rotation sets the time of day.
#[derive(Component, Reflect, Clone, Debug, PartialEq)]
#[reflect(Component, Default, SceneComponent)]
#[require(Light = Light::directional_light())]
pub struct Sun {
    pub atmosphere: Atmosphere,
    /// Angular radius of the drawn sun disk, in degrees.
    pub disk_radius: f32,
}

impl Default for Sun {
    fn default() -> Self {
        Self {
            atmosphere: Atmosphere::default(),
            disk_radius: 0.27,
        }
    }
}

impl Sun {
    pub fn with_atmosphere(mut self, atmosphere: Atmosphere) -> Self {
        self.atmosphere = atmosphere;
        self
    }

    pub fn with_disk_radius(mut self, disk_radius: f32) -> Self {
        self.disk_radius = disk_radius;
        self
    }
}

/// The air the sunlight passes through, in kilometres and per-kilometre coefficients.
#[derive(Reflect, Clone, Debug, PartialEq)]
pub struct Atmosphere {
    pub planet_radius: f32,
    pub atmosphere_height: f32,
    pub ground_albedo: Color,
    pub rayleigh_scattering: Vec3,
    pub rayleigh_scale_height: f32,
    pub mie_scattering: f32,
    pub mie_absorption: f32,
    pub mie_scale_height: f32,
    pub mie_asymmetry: f32,
    pub ozone_absorption: Vec3,
    pub ozone_center_height: f32,
    pub ozone_width: f32,
}

impl Default for Atmosphere {
    fn default() -> Self {
        Self {
            planet_radius: 6360.0,
            atmosphere_height: 100.0,
            ground_albedo: Color::rgba(0.3, 0.3, 0.3, 1.0),
            rayleigh_scattering: Vec3::new(5.802e-3, 13.558e-3, 33.1e-3),
            rayleigh_scale_height: 8.0,
            mie_scattering: 3.996e-3,
            mie_absorption: 4.4e-3,
            mie_scale_height: 1.2,
            mie_asymmetry: 0.8,
            ozone_absorption: Vec3::new(0.650e-3, 1.881e-3, 0.085e-3),
            ozone_center_height: 25.0,
            ozone_width: 30.0,
        }
    }
}

impl Atmosphere {
    pub fn with_planet_radius(mut self, planet_radius: f32) -> Self {
        self.planet_radius = planet_radius;
        self
    }

    pub fn with_atmosphere_height(mut self, atmosphere_height: f32) -> Self {
        self.atmosphere_height = atmosphere_height;
        self
    }

    pub fn with_ground_albedo(mut self, ground_albedo: Color) -> Self {
        self.ground_albedo = ground_albedo;
        self
    }

    pub fn with_rayleigh_scattering(mut self, rayleigh_scattering: Vec3) -> Self {
        self.rayleigh_scattering = rayleigh_scattering;
        self
    }

    pub fn with_rayleigh_scale_height(mut self, rayleigh_scale_height: f32) -> Self {
        self.rayleigh_scale_height = rayleigh_scale_height;
        self
    }

    pub fn with_mie_scattering(mut self, mie_scattering: f32) -> Self {
        self.mie_scattering = mie_scattering;
        self
    }

    pub fn with_mie_absorption(mut self, mie_absorption: f32) -> Self {
        self.mie_absorption = mie_absorption;
        self
    }

    pub fn with_mie_scale_height(mut self, mie_scale_height: f32) -> Self {
        self.mie_scale_height = mie_scale_height;
        self
    }

    pub fn with_mie_asymmetry(mut self, mie_asymmetry: f32) -> Self {
        self.mie_asymmetry = mie_asymmetry;
        self
    }

    pub fn with_ozone_absorption(mut self, ozone_absorption: Vec3) -> Self {
        self.ozone_absorption = ozone_absorption;
        self
    }

    pub fn with_ozone_center_height(mut self, ozone_center_height: f32) -> Self {
        self.ozone_center_height = ozone_center_height;
        self
    }

    pub fn with_ozone_width(mut self, ozone_width: f32) -> Self {
        self.ozone_width = ozone_width;
        self
    }
}

/// Draws a sky behind everything this camera sees and lights the scene with it.
#[derive(Component, Reflect, Clone, Debug, PartialEq)]
#[reflect(Component, Default, SceneComponent)]
#[require(Camera)]
pub struct Sky {
    /// Radiance added to the sky and to sky-colored fog, so night is not black.
    pub night_color: Color,
    /// Light added to the ambient term, in the units of `CameraEnvironment::ambient`.
    pub night_ambient: Color,
}

impl Default for Sky {
    fn default() -> Self {
        Self {
            night_color: Color::rgba(0.008, 0.009, 0.012, 1.0),
            night_ambient: Color::rgba(0.016, 0.018, 0.024, 1.0),
        }
    }
}

impl Sky {
    pub fn with_night_color(mut self, night_color: Color) -> Self {
        self.night_color = night_color;
        self
    }

    pub fn with_night_ambient(mut self, night_ambient: Color) -> Self {
        self.night_ambient = night_ambient;
        self
    }
}

use concerto_color::{Color, LinearRgba};
use concerto_ecs::resource::Resource;
use encase::ShaderType;

// TODO: Actually use this
#[derive(Resource, ShaderType)]
pub struct WorldEnvironment {
    ambient_color: LinearRgba,
}

impl WorldEnvironment {
    pub fn new(ambient_color: Color) -> Self {
        Self {
            ambient_color: ambient_color.to_linear(),
        }
    }

    pub fn ambient_color(&self) -> &LinearRgba {
        &self.ambient_color
    }
}

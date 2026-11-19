use concerto_app::{
    App,
    main_schedule::MainSchedulePlugin,
    plugins::{TimePlugin, TransformPlugin},
};
use concerto_color::{Color, LinearRgba};
use concerto_ecs::entity::Entity;
use concerto_foundation::transform::Transform;
use concerto_render::{
    CameraSkyLight, LightTint,
    components::light::{Light, LightType},
};
use concerto_skybox::{Sky, SkyPlugin, Sun};
use glam::{Quat, Vec3};

fn sun_at(elevation_degrees: f32) -> Transform {
    Transform::from_rotation(Quat::from_rotation_x(-elevation_degrees.to_radians()))
}

fn turn_sun(app: &mut App, sun: Entity, elevation_degrees: f32) {
    *app.main_mut()
        .world_mut()
        .get_component_for_entity_mut::<Transform>(sun)
        .unwrap() = sun_at(elevation_degrees);
    app.update();
}

fn tint(app: &mut App, light: Entity) -> Option<LinearRgba> {
    app.main_mut()
        .world_mut()
        .get_component_for_entity::<LightTint>(light)
        .map(|tint| tint.0)
}

fn sky_light(app: &mut App, camera: Entity) -> Option<CameraSkyLight> {
    app.main_mut()
        .world_mut()
        .get_component_for_entity::<CameraSkyLight>(camera)
        .copied()
}

fn blueness(irradiance: Vec3) -> f32 {
    irradiance.z / irradiance.x
}

fn brightness(tint: LinearRgba) -> f32 {
    tint.r + tint.g + tint.b
}

#[test]
fn a_sun_sets_the_sky_lighting_from_noon_to_night() {
    let mut app = App::new();
    app.register_plugin(MainSchedulePlugin)
        .register_plugin(TimePlugin)
        .register_plugin(TransformPlugin)
        .register_plugin(SkyPlugin);
    app.finish_plugin_build();
    let world = app.main_mut().world_mut();
    let camera = world.spawn((Sky::default(), Transform::default()));
    let sun = world.spawn((Sun::default(), sun_at(90.0)));
    app.update();

    let light = app
        .main_mut()
        .world_mut()
        .get_component_for_entity::<Light>(sun)
        .unwrap();
    assert!(matches!(light.light_type, LightType::Directional));

    let noon = sky_light(&mut app, camera).unwrap().irradiance;
    let up = noon.irradiance(Vec3::Y);
    let down = noon.irradiance(Vec3::NEG_Y);
    assert!(
        blueness(up) > blueness(down),
        "the sky above is bluer than the ground below: up {up}, down {down}"
    );
    let noon_tint = tint(&mut app, sun).unwrap();
    assert!(
        [noon_tint.r, noon_tint.g, noon_tint.b]
            .iter()
            .all(|channel| (0.7..=1.0).contains(channel)),
        "the noon sun is near white: {noon_tint:?}"
    );

    turn_sun(&mut app, sun, 2.0);
    let low_tint = tint(&mut app, sun).unwrap();
    assert!(
        low_tint.r > low_tint.g && low_tint.g > low_tint.b,
        "a low sun is red: {low_tint:?}"
    );
    assert!(brightness(low_tint) < brightness(noon_tint));

    turn_sun(&mut app, sun, -10.0);
    let night_tint = tint(&mut app, sun).unwrap();
    assert_eq!(brightness(night_tint), 0.0);
    let night_up = sky_light(&mut app, camera)
        .unwrap()
        .irradiance
        .irradiance(Vec3::Y);
    assert!(
        night_up.length() < 1e-3 * up.length(),
        "past twilight the sky is dark: {night_up}"
    );

    let night_ambient = Color::rgba(0.1, 0.2, 0.3, 1.0);
    app.main_mut()
        .world_mut()
        .get_component_for_entity_mut::<Sky>(camera)
        .unwrap()
        .night_ambient = night_ambient;
    app.update();
    assert_eq!(
        sky_light(&mut app, camera).unwrap().night_ambient,
        night_ambient.to_linear()
    );

    app.main_mut().world_mut().remove_component::<Sun>(sun);
    app.update();
    assert_eq!(tint(&mut app, sun), None);
    assert_eq!(
        sky_light(&mut app, camera).unwrap().irradiance.coefficients,
        [Vec3::ZERO; 9]
    );

    let spot =
        app.main_mut()
            .world_mut()
            .spawn((Light::spot_light(0.5), Sun::default(), sun_at(90.0)));
    app.update();
    assert_eq!(tint(&mut app, spot), None);
    assert_eq!(
        sky_light(&mut app, camera).unwrap().irradiance.coefficients,
        [Vec3::ZERO; 9]
    );

    app.main_mut().world_mut().remove_component::<Sky>(camera);
    app.update();
    assert_eq!(sky_light(&mut app, camera), None);
}

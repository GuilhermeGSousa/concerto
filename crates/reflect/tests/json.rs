use std::any::{Any, TypeId};
use std::fmt::Debug;

use concerto_color::Color;
use concerto_reflect::{
    LeafError, LeafHook, Reflect, ReflectDefault, ReflectJson, ReflectedValue, TypeRegistry,
    apply_value, json_to_value, json_to_value_with, to_json, to_value,
};
use glam::{Quat, Vec2, Vec3, Vec4};
use serde_json::{Value, json};

fn codec<T: Reflect>(registry: &TypeRegistry) -> &ReflectJson {
    registry
        .get_type_data::<ReflectJson>(TypeId::of::<T>())
        .unwrap_or_else(|| panic!("{} has no JSON codec", std::any::type_name::<T>()))
}

fn decode<T: Reflect + Clone>(registry: &TypeRegistry, value: &Value) -> Result<T, String> {
    let decoded = (codec::<T>(registry).from_json)(value)?;
    Ok((decoded.as_ref() as &dyn Any)
        .downcast_ref::<T>()
        .expect("the codec builds its own type")
        .clone())
}

fn assert_codec<T: Reflect + Clone + Default + PartialEq + Debug>(value: T, expected: Value) {
    let registry = TypeRegistry::default();
    assert_eq!((codec::<T>(&registry).to_json)(&value), expected);
    assert_eq!(to_json(&value, &registry), expected);
    assert_eq!(decode::<T>(&registry, &expected), Ok(value.clone()));
    assert_eq!(write::<T>(&registry, &expected, T::default()), Ok(value));
}

fn write<T: Reflect + Debug>(
    registry: &TypeRegistry,
    json: &Value,
    mut onto: T,
) -> Result<T, String> {
    let stored = json_to_value(json, TypeId::of::<T>(), registry)?;
    let warnings = apply_value(&mut onto, &stored);
    assert!(warnings.is_empty(), "{warnings:?}");
    Ok(onto)
}

fn write_error<T: Reflect>(registry: &TypeRegistry, json: Value) -> String {
    json_to_value(&json, TypeId::of::<T>(), registry).expect_err("the value must not decode")
}

#[test]
fn bool_is_a_json_boolean() {
    assert_codec(true, json!(true));
    assert_codec(false, json!(false));
}

#[test]
fn integers_are_json_numbers_at_their_full_range() {
    assert_codec(u8::MAX, json!(255));
    assert_codec(u16::MAX, json!(65535));
    assert_codec(u32::MAX, json!(4294967295_u64));
    assert_codec(u64::MAX, json!(u64::MAX));
    assert_codec(usize::MAX, json!(usize::MAX));
    assert_codec(i8::MIN, json!(-128));
    assert_codec(i16::MIN, json!(-32768));
    assert_codec(i32::MIN, json!(-2147483648_i64));
    assert_codec(i64::MIN, json!(i64::MIN));
    assert_codec(isize::MIN, json!(isize::MIN));
}

#[test]
fn an_integer_out_of_range_or_not_an_integer_does_not_decode() {
    let registry = TypeRegistry::default();
    assert!(decode::<u8>(&registry, &json!(256)).is_err());
    assert!(decode::<u8>(&registry, &json!(-1)).is_err());
    assert!(decode::<i8>(&registry, &json!(128)).is_err());
    assert!(decode::<u32>(&registry, &json!(1.5)).is_err());
    assert!(decode::<u32>(&registry, &json!("1")).is_err());
}

#[test]
fn floats_are_json_numbers_in_their_shortest_form() {
    assert_codec(0.1_f32, json!(0.1));
    assert_codec(-2.5_f32, json!(-2.5));
    assert_codec(f32::MAX, json!(3.4028235e38));
    assert_codec(f32::MIN_POSITIVE, json!(1.1754944e-38));
    assert_codec(0.1_f64, json!(0.1));
    assert_codec(f64::MAX, json!(f64::MAX));
}

#[test]
fn a_float_decodes_from_an_integer() {
    let registry = TypeRegistry::default();
    assert_eq!(decode::<f32>(&registry, &json!(3)), Ok(3.0));
    assert_eq!(decode::<f64>(&registry, &json!(-3)), Ok(-3.0));
}

#[test]
fn a_leaf_written_from_json_is_the_codecs_value_encoded() {
    let registry = TypeRegistry::default();
    assert_eq!(
        json_to_value(&json!([1.0, 2.0, 3.0]), TypeId::of::<Vec3>(), &registry),
        Ok(to_value(&Vec3::new(1.0, 2.0, 3.0)).unwrap())
    );
}

#[test]
fn non_finite_floats_are_strings() {
    assert_codec(f32::INFINITY, json!("inf"));
    assert_codec(f32::NEG_INFINITY, json!("-inf"));
    assert_codec(f64::INFINITY, json!("inf"));
    assert_codec(f64::NEG_INFINITY, json!("-inf"));

    let registry = TypeRegistry::default();
    let nan = f32::NAN;
    assert_eq!(to_json(&nan, &registry), json!("NaN"));
    assert!(decode::<f32>(&registry, &json!("NaN")).unwrap().is_nan());
    assert_eq!(to_json(&f64::NAN, &registry), json!("NaN"));
    assert!(decode::<f64>(&registry, &json!("NaN")).unwrap().is_nan());
    assert!(
        write::<f32>(&registry, &json!("NaN"), 0.0)
            .unwrap()
            .is_nan()
    );
}

#[test]
fn a_float_does_not_decode_from_a_finite_string_or_out_of_range() {
    let registry = TypeRegistry::default();
    assert!(decode::<f32>(&registry, &json!("1.5")).is_err());
    assert!(decode::<f32>(&registry, &json!("fast")).is_err());
    assert!(decode::<f32>(&registry, &json!(1e40)).is_err());
    assert!(decode::<f64>(&registry, &json!(true)).is_err());
}

#[test]
fn string_is_a_json_string() {
    assert_codec(String::from("Hero"), json!("Hero"));
    assert!(decode::<String>(&TypeRegistry::default(), &json!(1)).is_err());
}

#[test]
fn vectors_and_quaternions_are_arrays_of_their_components() {
    assert_codec(Vec2::new(1.0, -0.5), json!([1.0, -0.5]));
    assert_codec(Vec3::new(0.1, 2.0, 3.0), json!([0.1, 2.0, 3.0]));
    assert_codec(Vec4::new(1.0, 2.0, 3.0, 4.0), json!([1.0, 2.0, 3.0, 4.0]));
    assert_codec(
        Quat::from_xyzw(0.0, 0.6, 0.0, 0.8),
        json!([0.0, 0.6, 0.0, 0.8]),
    );
}

#[test]
fn a_vector_of_the_wrong_length_or_with_a_non_number_does_not_decode() {
    let registry = TypeRegistry::default();
    assert!(decode::<Vec3>(&registry, &json!([1.0, 2.0])).is_err());
    assert!(decode::<Vec3>(&registry, &json!([1.0, 2.0, 3.0, 4.0])).is_err());
    assert!(decode::<Vec3>(&registry, &json!([1.0, "x", 3.0])).is_err());
    assert!(decode::<Quat>(&registry, &json!({ "x": 0.0 })).is_err());
}

#[test]
fn a_color_is_an_array_of_its_linear_components() {
    assert_codec(
        Color::rgba(0.1, 0.5, 1.0, 0.25),
        json!([0.1, 0.5, 1.0, 0.25]),
    );
    assert_codec(Color::WHITE, json!([1.0, 1.0, 1.0, 1.0]));
}

#[test]
fn a_color_in_another_space_is_written_as_its_linear_components() {
    let registry = TypeRegistry::default();
    let srgb = Color::srgba(0.5, 0.5, 0.5, 1.0);
    let [r, g, b, a] = srgb.to_array();

    let json = to_json(&srgb, &registry);
    assert_eq!(
        decode::<Color>(&registry, &json),
        Ok(Color::rgba(r, g, b, a))
    );
}

#[test]
fn a_color_of_the_wrong_length_or_with_a_non_number_does_not_decode() {
    let registry = TypeRegistry::default();
    assert!(decode::<Color>(&registry, &json!([1.0, 1.0, 1.0])).is_err());
    assert!(decode::<Color>(&registry, &json!([1.0, 1.0, 1.0, 1.0, 1.0])).is_err());
    assert!(decode::<Color>(&registry, &json!([1.0, "x", 1.0, 1.0])).is_err());
    assert_eq!(
        write_error::<Color>(&registry, json!("#ffffff")),
        "expected an array of 4 numbers"
    );
}

#[derive(Reflect, Default, Debug, PartialEq, Clone, serde::Serialize, serde::Deserialize)]
#[reflect(opaque, Default)]
struct Secret(u32);

#[derive(Reflect, Default, Debug, PartialEq, Clone)]
enum Shape {
    #[default]
    Point,
    Sphere(f32),
    Cone {
        angle: f32,
        length: f32,
    },
}

#[derive(Reflect, Default, Debug, PartialEq, Clone)]
struct Inner {
    weight: f32,
    label: String,
}

#[derive(Reflect, Default, Debug, PartialEq, Clone)]
struct Pair(u8, bool);

#[derive(Reflect, Default, Debug, PartialEq, Clone)]
#[reflect(Default)]
struct Nested {
    position: Vec3,
    shape: Shape,
    point: Shape,
    cone: Shape,
    list: Vec<Inner>,
    pair: Pair,
    maybe: Option<i32>,
    never: Option<i32>,
    secret: Secret,
    tint: Color,
}

#[test]
fn a_nested_struct_walks_structs_enums_and_lists_and_marks_opaque_leaves() {
    let value = Nested {
        position: Vec3::new(1.0, 2.0, 3.0),
        shape: Shape::Sphere(2.0),
        point: Shape::Point,
        cone: Shape::Cone {
            angle: 0.25,
            length: 8.0,
        },
        list: vec![
            Inner {
                weight: 1.0,
                label: "a".into(),
            },
            Inner {
                weight: 2.0,
                label: "b".into(),
            },
        ],
        pair: Pair(7, true),
        maybe: Some(4),
        never: None,
        secret: Secret(42),
        tint: Color::default(),
    };

    assert_eq!(
        to_json(&value, &TypeRegistry::default()),
        json!({
            "position": [1.0, 2.0, 3.0],
            "shape": { "Sphere": { "0": 2.0 } },
            "point": "Point",
            "cone": { "Cone": { "angle": 0.25, "length": 8.0 } },
            "list": [
                { "weight": 1.0, "label": "a" },
                { "weight": 2.0, "label": "b" },
            ],
            "pair": { "0": 7, "1": true },
            "maybe": { "Some": { "0": 4 } },
            "never": "None",
            "secret": { "opaque": std::any::type_name::<Secret>() },
            "tint": [1.0, 1.0, 1.0, 1.0],
        })
    );
}

#[test]
fn a_leaf_with_no_codec_is_opaque_at_the_root_and_in_a_list() {
    let registry = TypeRegistry::default();
    let opaque = json!({ "opaque": std::any::type_name::<Secret>() });
    assert_eq!(to_json(&Secret(1), &registry), opaque);
    assert_eq!(
        to_json(&vec![Secret(1), Secret(2)], &registry),
        json!([opaque, opaque])
    );
}

fn nested() -> Nested {
    Nested {
        position: Vec3::new(1.0, 2.0, 3.0),
        shape: Shape::Sphere(2.0),
        point: Shape::Point,
        cone: Shape::Cone {
            angle: 0.25,
            length: 8.0,
        },
        list: vec![
            Inner {
                weight: 1.0,
                label: "a".into(),
            },
            Inner {
                weight: 2.0,
                label: "b".into(),
            },
        ],
        pair: Pair(7, true),
        maybe: Some(4),
        never: None,
        secret: Secret(42),
        tint: Color::default(),
    }
}

fn nested_registry() -> TypeRegistry {
    let mut registry = TypeRegistry::default();
    registry.register::<Nested>();
    registry
}

#[test]
fn a_nested_value_written_back_from_its_json_is_the_value_again() {
    let registry = nested_registry();
    let value = nested();
    let json = to_json(&value, &registry);

    let onto = Nested {
        secret: Secret(42),
        never: Some(9),
        ..Nested::default()
    };
    assert_eq!(write(&registry, &json, onto), Ok(value));
}

#[test]
fn an_opaque_field_written_as_its_marker_is_left_as_it_was() {
    let registry = nested_registry();
    let json = json!({ "secret": { "opaque": std::any::type_name::<Secret>() } });

    assert_eq!(
        json_to_value(&json, TypeId::of::<Nested>(), &registry),
        Ok(ReflectedValue::Struct(Vec::new()))
    );
}

#[test]
fn fields_left_out_of_the_json_are_not_written() {
    let registry = nested_registry();
    let json = json!({ "position": [4.0, 5.0, 6.0], "cone": { "Cone": { "angle": 1.0 } } });

    let written = write(&registry, &json, nested()).unwrap();
    assert_eq!(
        written,
        Nested {
            position: Vec3::new(4.0, 5.0, 6.0),
            cone: Shape::Cone {
                angle: 1.0,
                length: 8.0,
            },
            ..nested()
        }
    );
}

#[test]
fn a_variant_switched_by_name_alone_starts_from_its_defaults() {
    let registry = nested_registry();
    let written = write(
        &registry,
        &json!({ "point": "Cone", "maybe": "None" }),
        nested(),
    );

    assert_eq!(
        written.map(|written| (written.point, written.maybe)),
        Ok((
            Shape::Cone {
                angle: 0.0,
                length: 0.0
            },
            None
        ))
    );
}

#[test]
fn a_list_written_from_json_takes_the_json_length() {
    let registry = nested_registry();
    let written = write(
        &registry,
        &json!({ "list": [{ "label": "only" }] }),
        nested(),
    );

    assert_eq!(
        written.map(|written| written.list),
        Ok(vec![Inner {
            weight: 1.0,
            label: "only".into(),
        }])
    );
}

#[test]
fn a_leaf_that_does_not_decode_names_its_path_and_shape() {
    let registry = nested_registry();
    assert_eq!(
        write_error::<Nested>(&registry, json!({ "position": [1.0, "x", 3.0] })),
        "position: expected an array of 3 numbers"
    );
    assert_eq!(
        write_error::<Nested>(
            &registry,
            json!({ "list": [{ "weight": 1.0 }, { "weight": "heavy" }] })
        ),
        "list[1].weight: expected a number, or \"NaN\", \"inf\" or \"-inf\""
    );
    assert_eq!(
        write_error::<Nested>(&registry, json!({ "cone": { "Cone": { "angle": true } } })),
        "cone.angle: expected a number, or \"NaN\", \"inf\" or \"-inf\""
    );
    assert_eq!(
        write_error::<Nested>(&registry, json!({ "pair": { "0": 256 } })),
        "pair.0: expected an integer from 0 to 255"
    );
    assert_eq!(
        write_error::<Vec3>(&registry, json!("up")),
        "expected an array of 3 numbers"
    );
}

#[test]
fn a_struct_that_is_not_an_object_names_its_fields() {
    let registry = nested_registry();
    assert_eq!(
        write_error::<Nested>(&registry, json!({ "pair": 3 })),
        "pair: expected an object with the fields 0, 1"
    );
    assert_eq!(
        write_error::<Nested>(&registry, json!([])),
        "expected an object with the fields position, shape, point, cone, list, pair, maybe, never, secret, tint"
    );
}

#[test]
fn an_unknown_field_is_named_with_the_fields_there_are() {
    let registry = nested_registry();
    assert_eq!(
        write_error::<Nested>(&registry, json!({ "list": [{ "mass": 1.0 }] })),
        "list[0].mass: no such field; the fields are weight, label"
    );
}

#[test]
fn an_enum_names_the_variant_shape_it_expects() {
    let registry = nested_registry();
    assert_eq!(
        write_error::<Nested>(&registry, json!({ "shape": "Cube" })),
        "shape: no variant 'Cube'"
    );
    assert_eq!(
        write_error::<Nested>(&registry, json!({ "shape": { "Cube": {} } })),
        "shape: no variant 'Cube'"
    );
    let shape = "shape: expected a variant name, or an object with one key naming the variant";
    assert_eq!(
        write_error::<Nested>(&registry, json!({ "shape": { "Sphere": {}, "Cone": {} } })),
        shape
    );
    assert_eq!(
        write_error::<Nested>(&registry, json!({ "shape": 1 })),
        shape
    );
    assert_eq!(
        write_error::<Nested>(&registry, json!({ "shape": { "Sphere": 2.0 } })),
        "shape: expected an object with the fields 0"
    );
}

#[test]
fn a_list_that_is_not_an_array_says_so() {
    let registry = nested_registry();
    assert_eq!(
        write_error::<Nested>(&registry, json!({ "list": { "weight": 1.0 } })),
        "list: expected an array"
    );
}

#[test]
fn an_opaque_leaf_cannot_be_written() {
    let registry = nested_registry();
    let opaque = format!(
        "{} has no JSON form, so it cannot be written",
        std::any::type_name::<Secret>()
    );
    assert_eq!(
        write_error::<Nested>(&registry, json!({ "secret": 42 })),
        format!("secret: {opaque}")
    );
    assert_eq!(
        write_error::<Nested>(&registry, json!({ "secret": { "opaque": "other::Type" } })),
        format!("secret: {opaque}")
    );

    let mut registry = TypeRegistry::default();
    registry.register::<Secret>();
    assert_eq!(
        write_error::<Secret>(
            &registry,
            json!({ "opaque": std::any::type_name::<Secret>() })
        ),
        opaque
    );
}

#[test]
fn a_type_with_no_codec_and_no_registered_default_cannot_be_built() {
    let registry = nested_registry();
    assert_eq!(
        write_error::<Inner>(&registry, json!({})),
        "the type is not registered, so JSON cannot build it"
    );

    let mut registry = TypeRegistry::default();
    registry.add(concerto_reflect::TypeRegistration::of::<Inner>());
    assert_eq!(
        write_error::<Inner>(&registry, json!({})),
        format!(
            "{} has no JSON form and no registered Default, so JSON cannot build it",
            std::any::type_name::<Inner>()
        )
    );
}

struct Refusing;

impl LeafHook for Refusing {
    fn encode(&mut self, leaf: &dyn Reflect) -> Option<Result<Vec<u8>, LeafError>> {
        (leaf as &dyn Any)
            .downcast_ref::<String>()
            .map(|_| Err(LeafError("strings are not stored here".into())))
    }

    fn decode(&mut self, _: &mut dyn Reflect, _: &[u8]) -> Option<Result<(), LeafError>> {
        None
    }
}

#[test]
fn a_hook_encodes_leaves_first_and_its_errors_name_the_path() {
    let registry = nested_registry();
    let json = json!({ "position": [0.0, 0.0, 0.0], "list": [{ "label": "a" }] });

    assert_eq!(
        json_to_value_with(&json, TypeId::of::<Nested>(), &registry, &mut Refusing),
        Err("list[0].label: strings are not stored here".to_owned())
    );
}

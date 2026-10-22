use std::any::{Any, TypeId};
use std::fmt::Debug;

use concerto_color::Color;
use concerto_reflect::{Reflect, ReflectJson, TypeRegistry, to_json};
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

fn assert_codec<T: Reflect + Clone + PartialEq + Debug>(value: T, expected: Value) {
    let registry = TypeRegistry::default();
    assert_eq!((codec::<T>(&registry).to_json)(&value), expected);
    assert_eq!(to_json(&value, &registry), expected);
    assert_eq!(decode::<T>(&registry, &expected), Ok(value));
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

#[derive(Reflect, Default, Debug, PartialEq, Clone, serde::Serialize, serde::Deserialize)]
#[reflect(opaque)]
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
            "tint": { "opaque": "concerto_color::Color" },
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

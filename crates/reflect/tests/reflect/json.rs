use std::any::{Any, TypeId};
use std::fmt::Debug;

use concerto_color::Color;
use concerto_reflect::{
    Reflect, ReflectDefault, ReflectJson, ReflectedValue, TypeRegistry, to_json,
};
use glam::{Quat, Vec2, Vec3, Vec4};
use serde_json::{Value, json};

struct Codec {
    value: Box<dyn Reflect>,
    json: Value,
}

fn codec(value: impl Reflect, json: Value) -> Codec {
    Codec {
        value: Box::new(value),
        json,
    }
}

#[test]
fn each_leaf_with_a_codec_is_written_as_its_json_and_built_back_from_it() {
    let registry = TypeRegistry::default();
    let cases = [
        codec(true, json!(true)),
        codec(u8::MAX, json!(255)),
        codec(u64::MAX, json!(u64::MAX)),
        codec(i8::MIN, json!(-128)),
        codec(i64::MIN, json!(i64::MIN)),
        codec(0.1_f32, json!(0.1)),
        codec(f32::MAX, json!(3.4028235e38)),
        codec(0.1_f64, json!(0.1)),
        codec(f32::INFINITY, json!("inf")),
        codec(f32::NEG_INFINITY, json!("-inf")),
        codec(f32::NAN, json!("NaN")),
        codec(f64::NAN, json!("NaN")),
        codec(String::from("Hero"), json!("Hero")),
        codec(Vec2::new(1.0, -0.5), json!([1.0, -0.5])),
        codec(Vec3::new(0.1, 2.0, 3.0), json!([0.1, 2.0, 3.0])),
        codec(Vec4::new(1.0, 2.0, 3.0, 4.0), json!([1.0, 2.0, 3.0, 4.0])),
        codec(
            Quat::from_xyzw(0.0, 0.6, 0.0, 0.8),
            json!([0.0, 0.6, 0.0, 0.8]),
        ),
        codec(
            Color::rgba(0.1, 0.5, 1.0, 0.25),
            json!([0.1, 0.5, 1.0, 0.25]),
        ),
    ];
    for case in cases {
        let value = case.value.as_ref();
        let codec = registry
            .get_type_data::<ReflectJson>((value as &dyn Any).type_id())
            .unwrap_or_else(|| panic!("{} has no JSON codec", case.json));
        assert_eq!(to_json(value, &registry), case.json);
        assert_eq!((codec.to_json)(value), case.json);
        let built = (codec.from_json)(&case.json).unwrap();
        assert_eq!(to_json(built.as_ref(), &registry), case.json);
        assert_eq!(
            ReflectedValue::from_json(&case.json, (value as &dyn Any).type_id(), &registry),
            Ok(ReflectedValue::encode(value).unwrap()),
            "{}",
            case.json
        );
    }

    for case in [codec(3.0_f32, json!(3)), codec(-3.0_f64, json!(-3))] {
        let value = case.value.as_ref();
        assert_eq!(
            ReflectedValue::from_json(&case.json, (value as &dyn Any).type_id(), &registry),
            Ok(ReflectedValue::encode(value).unwrap()),
            "a float decodes from the integer {}",
            case.json
        );
    }

    let srgb = Color::srgba(0.5, 0.5, 0.5, 1.0);
    let [r, g, b, a] = srgb.to_array();
    assert_eq!(
        to_json(&srgb, &registry),
        to_json(&Color::rgba(r, g, b, a), &registry)
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

fn write<T: Reflect + Debug>(
    registry: &TypeRegistry,
    json: &Value,
    mut onto: T,
) -> Result<T, String> {
    let stored = ReflectedValue::from_json(json, TypeId::of::<T>(), registry)?;
    let warnings = stored.apply(&mut onto);
    assert!(warnings.is_empty(), "{warnings:?}");
    Ok(onto)
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

struct Partial {
    name: &'static str,
    json: Value,
    expected: Nested,
}

#[test]
fn a_partial_write_changes_only_what_the_json_names() {
    let registry = nested_registry();
    let cases = [
        Partial {
            name: "fields left out of the json are not written",
            json: json!({ "position": [4.0, 5.0, 6.0], "cone": { "Cone": { "angle": 1.0 } } }),
            expected: Nested {
                position: Vec3::new(4.0, 5.0, 6.0),
                cone: Shape::Cone {
                    angle: 1.0,
                    length: 8.0,
                },
                ..nested()
            },
        },
        Partial {
            name: "a variant switched by name alone starts from its defaults",
            json: json!({ "point": "Cone", "maybe": "None" }),
            expected: Nested {
                point: Shape::Cone {
                    angle: 0.0,
                    length: 0.0,
                },
                maybe: None,
                ..nested()
            },
        },
        Partial {
            name: "a list takes the json length",
            json: json!({ "list": [{ "label": "only" }] }),
            expected: Nested {
                list: vec![Inner {
                    weight: 1.0,
                    label: "only".into(),
                }],
                ..nested()
            },
        },
    ];
    for case in cases {
        assert_eq!(
            write(&registry, &case.json, nested()),
            Ok(case.expected),
            "{}",
            case.name
        );
    }
}

struct Refused {
    target: TypeId,
    json: Value,
}

fn refused<T: Reflect>(json: Value) -> Refused {
    Refused {
        target: TypeId::of::<T>(),
        json,
    }
}

#[test]
fn json_that_does_not_fit_the_type_is_refused() {
    let cases = [
        refused::<u8>(json!(256)),
        refused::<f32>(json!(1e40)),
        refused::<Vec3>(json!([1.0, 2.0])),
        refused::<Nested>(json!({ "list": [{ "mass": 1.0 }] })),
        refused::<Nested>(json!({ "shape": "Cube" })),
        refused::<Nested>(json!({ "secret": 42 })),
        refused::<Inner>(json!({})),
    ];
    for case in cases {
        assert!(
            ReflectedValue::from_json(&case.json, case.target, &nested_registry()).is_err(),
            "{}",
            case.json
        );
    }
}

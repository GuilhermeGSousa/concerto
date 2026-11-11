use std::any::{Any, TypeId};
use std::fmt::Debug;

use concerto_color::Color;
use concerto_reflect::{
    LeafError, LeafHook, Reflect, ReflectDefault, ReflectJson, ReflectedValue, TypeRegistration,
    TypeRegistry, apply_value, json_to_value, json_to_value_with, to_json, to_value,
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
        codec(false, json!(false)),
        codec(u8::MAX, json!(255)),
        codec(u16::MAX, json!(65535)),
        codec(u32::MAX, json!(4294967295_u64)),
        codec(u64::MAX, json!(u64::MAX)),
        codec(usize::MAX, json!(usize::MAX)),
        codec(i8::MIN, json!(-128)),
        codec(i16::MIN, json!(-32768)),
        codec(i32::MIN, json!(-2147483648_i64)),
        codec(i64::MIN, json!(i64::MIN)),
        codec(isize::MIN, json!(isize::MIN)),
        codec(0.1_f32, json!(0.1)),
        codec(-2.5_f32, json!(-2.5)),
        codec(f32::MAX, json!(3.4028235e38)),
        codec(f32::MIN_POSITIVE, json!(1.1754944e-38)),
        codec(0.1_f64, json!(0.1)),
        codec(f64::MAX, json!(f64::MAX)),
        codec(f32::INFINITY, json!("inf")),
        codec(f32::NEG_INFINITY, json!("-inf")),
        codec(f64::INFINITY, json!("inf")),
        codec(f64::NEG_INFINITY, json!("-inf")),
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
        codec(Color::WHITE, json!([1.0, 1.0, 1.0, 1.0])),
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
            json_to_value(&case.json, (value as &dyn Any).type_id(), &registry),
            Ok(to_value(value).unwrap()),
            "{}",
            case.json
        );
    }

    for case in [codec(3.0_f32, json!(3)), codec(-3.0_f64, json!(-3))] {
        let value = case.value.as_ref();
        assert_eq!(
            json_to_value(&case.json, (value as &dyn Any).type_id(), &registry),
            Ok(to_value(value).unwrap()),
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
    registry: fn() -> TypeRegistry,
    json: Value,
    error: String,
}

fn refused<T: Reflect>(json: Value, error: &str) -> Refused {
    Refused {
        target: TypeId::of::<T>(),
        registry: nested_registry,
        json,
        error: error.into(),
    }
}

#[test]
fn json_that_does_not_fit_the_type_is_refused_naming_the_path_and_the_shape_it_expects() {
    let number = "expected a number, or \"NaN\", \"inf\" or \"-inf\"";
    let secret = std::any::type_name::<Secret>();
    let opaque = format!("{secret} has no JSON form, so it cannot be written");
    let no_default = format!(
        "{} has no JSON form and no registered Default, so JSON cannot build it",
        std::any::type_name::<Inner>()
    );
    let variant = "shape: expected a variant name, or an object with one key naming the variant";
    let cases = [
        refused::<u8>(json!(256), "expected an integer from 0 to 255"),
        refused::<u8>(json!(-1), "expected an integer from 0 to 255"),
        refused::<i8>(json!(128), "expected an integer from -128 to 127"),
        refused::<u32>(json!(1.5), "expected an integer from 0 to 4294967295"),
        refused::<u32>(json!("1"), "expected an integer from 0 to 4294967295"),
        refused::<f32>(json!("1.5"), number),
        refused::<f32>(json!("fast"), number),
        refused::<f32>(json!(1e40), number),
        refused::<f64>(json!(true), number),
        refused::<String>(json!(1), "expected a string"),
        refused::<Vec3>(json!([1.0, 2.0]), "expected an array of 3 numbers"),
        refused::<Vec3>(
            json!([1.0, 2.0, 3.0, 4.0]),
            "expected an array of 3 numbers",
        ),
        refused::<Vec3>(json!([1.0, "x", 3.0]), "expected an array of 3 numbers"),
        refused::<Vec3>(json!("up"), "expected an array of 3 numbers"),
        refused::<Quat>(json!({ "x": 0.0 }), "expected an array of 4 numbers"),
        refused::<Color>(json!([1.0, 1.0, 1.0]), "expected an array of 4 numbers"),
        refused::<Color>(
            json!([1.0, 1.0, 1.0, 1.0, 1.0]),
            "expected an array of 4 numbers",
        ),
        refused::<Color>(
            json!([1.0, "x", 1.0, 1.0]),
            "expected an array of 4 numbers",
        ),
        refused::<Color>(json!("#ffffff"), "expected an array of 4 numbers"),
        refused::<Nested>(
            json!({ "position": [1.0, "x", 3.0] }),
            "position: expected an array of 3 numbers",
        ),
        refused::<Nested>(
            json!({ "list": [{ "weight": 1.0 }, { "weight": "heavy" }] }),
            &format!("list[1].weight: {number}"),
        ),
        refused::<Nested>(
            json!({ "cone": { "Cone": { "angle": true } } }),
            &format!("cone.angle: {number}"),
        ),
        refused::<Nested>(
            json!({ "pair": { "0": 256 } }),
            "pair.0: expected an integer from 0 to 255",
        ),
        refused::<Nested>(
            json!({ "pair": 3 }),
            "pair: expected an object with the fields 0, 1",
        ),
        refused::<Nested>(
            json!([]),
            "expected an object with the fields position, shape, point, cone, list, pair, maybe, never, secret, tint",
        ),
        refused::<Nested>(
            json!({ "list": [{ "mass": 1.0 }] }),
            "list[0].mass: no such field; the fields are weight, label",
        ),
        refused::<Nested>(
            json!({ "shape": "Cube" }),
            "shape: no variant 'Cube'; the variants are Point, Sphere, Cone",
        ),
        refused::<Nested>(
            json!({ "shape": { "Cube": {} } }),
            "shape: no variant 'Cube'; the variants are Point, Sphere, Cone",
        ),
        refused::<Nested>(
            json!({ "maybe": "Many" }),
            "maybe: no variant 'Many'; the variants are None, Some",
        ),
        refused::<Nested>(json!({ "shape": { "Sphere": {}, "Cone": {} } }), variant),
        refused::<Nested>(json!({ "shape": 1 }), variant),
        refused::<Nested>(
            json!({ "shape": { "Sphere": 2.0 } }),
            "shape: expected an object with the fields 0",
        ),
        refused::<Nested>(
            json!({ "list": { "weight": 1.0 } }),
            "list: expected an array",
        ),
        refused::<Nested>(json!({ "secret": 42 }), &format!("secret: {opaque}")),
        refused::<Nested>(
            json!({ "secret": { "opaque": "other::Type" } }),
            &format!("secret: {opaque}"),
        ),
        Refused {
            registry: || {
                let mut registry = TypeRegistry::default();
                registry.register::<Secret>();
                registry
            },
            ..refused::<Secret>(json!({ "opaque": secret }), &opaque)
        },
        refused::<Inner>(json!({}), &no_default),
        Refused {
            registry: || {
                let mut registry = TypeRegistry::default();
                registry.add(TypeRegistration::of::<Inner>());
                registry
            },
            ..refused::<Inner>(json!({}), &no_default)
        },
    ];
    for case in cases {
        assert_eq!(
            json_to_value(&case.json, case.target, &(case.registry)()),
            Err(case.error),
            "{}",
            case.json
        );
    }
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

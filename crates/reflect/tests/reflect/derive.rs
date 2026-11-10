use std::any::{Any, TypeId};

use concerto_reflect::{
    PathError, PathSegment, PropertyPath, PropertyVisitor, PropertyVisitorMut, Reflect, ReflectRef,
    apply_value, to_value, with_property, with_property_mut,
};
use glam::{Quat, Vec3};

#[derive(Reflect, Default)]
struct Inner {
    weight: f32,
    label: String,
    enabled: bool,
}

#[derive(Reflect, Default)]
struct Outer {
    position: Vec3,
    rotation: Quat,
    inner: Inner,
}

#[derive(Reflect, Default)]
struct Sample {
    weight: f32,
}

#[derive(Reflect, Default)]
struct Track {
    weights: Vec<f32>,
    samples: Vec<Sample>,
    rows: Vec<Vec<f32>>,
    fog: Option<Sample>,
    outer: Outer,
}

fn track() -> Track {
    Track {
        weights: vec![1.0, 2.0],
        samples: vec![Sample { weight: 3.0 }, Sample { weight: 4.0 }],
        rows: vec![vec![5.0], vec![6.0, 7.0]],
        fog: Some(Sample { weight: 8.0 }),
        outer: Outer {
            position: Vec3::X,
            rotation: Quat::IDENTITY,
            inner: Inner {
                weight: 1.0,
                label: "weight".into(),
                enabled: true,
            },
        },
    }
}

struct Field {
    name: &'static str,
    type_id: TypeId,
}

#[test]
fn derive_visits_fields_in_declaration_order_with_concrete_types() {
    struct Fields(Vec<Field>);
    impl PropertyVisitor for Fields {
        fn field(&mut self, name: &'static str, value: &dyn Reflect) {
            self.0.push(Field {
                name,
                type_id: (value as &dyn Any).type_id(),
            });
        }
    }
    let mut fields = Fields(Vec::new());
    track().outer.visit(&mut fields);
    assert_eq!(
        fields.0.iter().map(|field| field.name).collect::<Vec<_>>(),
        ["position", "rotation", "inner"]
    );
    assert_eq!(
        fields
            .0
            .iter()
            .map(|field| field.type_id)
            .collect::<Vec<_>>(),
        [
            TypeId::of::<Vec3>(),
            TypeId::of::<Quat>(),
            TypeId::of::<Inner>()
        ]
    );
}

enum Reached {
    Float(f32),
    Type(TypeId),
    NotFound,
}

struct Access {
    path: PropertyPath,
    reached: Reached,
}

fn fields(names: &[&'static str]) -> PropertyPath {
    PropertyPath::new(names.iter().copied())
}

#[test]
fn a_path_reaches_exactly_the_property_it_names_for_reading_and_writing() {
    use PathSegment::{Field, Index};
    let cases = [
        Access {
            path: fields(&["outer", "inner", "weight"]),
            reached: Reached::Float(1.0),
        },
        Access {
            path: fields(&["outer", "inner"]),
            reached: Reached::Type(TypeId::of::<Inner>()),
        },
        Access {
            path: PropertyPath::default(),
            reached: Reached::Type(TypeId::of::<Track>()),
        },
        Access {
            path: PropertyPath::new([Field("weights"), Index(1)]),
            reached: Reached::Float(2.0),
        },
        Access {
            path: PropertyPath::new([Field("samples"), Index(1), Field("weight")]),
            reached: Reached::Float(4.0),
        },
        Access {
            path: PropertyPath::new([Field("rows"), Index(1), Index(1)]),
            reached: Reached::Float(7.0),
        },
        Access {
            path: fields(&["fog", "0", "weight"]),
            reached: Reached::Float(8.0),
        },
        Access {
            path: fields(&["missing"]),
            reached: Reached::NotFound,
        },
        Access {
            path: fields(&["outer", "missing"]),
            reached: Reached::NotFound,
        },
        Access {
            path: fields(&["outer", "position", "x"]),
            reached: Reached::NotFound,
        },
        Access {
            path: PropertyPath::new([Field("weights"), Index(2)]),
            reached: Reached::NotFound,
        },
        Access {
            path: PropertyPath::new([Field("rows"), Index(0), Index(1)]),
            reached: Reached::NotFound,
        },
        Access {
            path: PropertyPath::new([Index(0)]),
            reached: Reached::NotFound,
        },
        Access {
            path: PropertyPath::new([Field("samples"), Field("0")]),
            reached: Reached::NotFound,
        },
        Access {
            path: PropertyPath::new([Field("fog"), Index(0), Field("weight")]),
            reached: Reached::NotFound,
        },
    ];
    for case in cases {
        let check = |value: &dyn Reflect| match case.reached {
            Reached::Float(expected) => {
                assert_eq!(
                    value.downcast_ref::<f32>(),
                    Some(&expected),
                    "{}",
                    case.path
                )
            }
            Reached::Type(expected) => {
                assert_eq!((value as &dyn Any).type_id(), expected, "{}", case.path)
            }
            Reached::NotFound => panic!("{} should not be found", case.path),
        };
        let expected = match case.reached {
            Reached::NotFound => Err(PathError::NotFound),
            _ => Ok(()),
        };
        let mut calls = 0;
        let read = with_property(&track(), &case.path, &mut |value| {
            check(value);
            calls += 1;
        });
        let written = with_property_mut(&mut track(), &case.path, &mut |value| {
            check(value);
            calls += 1;
        });
        assert_eq!(read, expected, "{}", case.path);
        assert_eq!(written, expected, "{}", case.path);
        assert_eq!(calls, 2 * usize::from(expected.is_ok()), "{}", case.path);
    }
}

#[test]
fn a_write_through_a_path_lands_in_the_property_it_names() {
    use PathSegment::{Field, Index};
    let mut value = track();
    for path in [
        fields(&["outer", "inner", "weight"]),
        PropertyPath::new([Field("weights"), Index(1)]),
        PropertyPath::new([Field("samples"), Index(1), Field("weight")]),
        PropertyPath::new([Field("rows"), Index(1), Index(1)]),
    ] {
        with_property_mut(&mut value, &path, &mut |v| {
            *v.downcast_mut::<f32>().unwrap() = 9.0;
        })
        .unwrap();
    }
    assert_eq!(value.outer.inner.weight, 9.0);
    assert_eq!(value.weights, [1.0, 9.0]);
    assert_eq!(value.samples[1].weight, 9.0);
    assert_eq!(value.rows[1], [6.0, 9.0]);
}

struct Named {
    path: PropertyPath,
    name: &'static str,
    shown: &'static str,
}

#[test]
fn a_path_is_named_by_its_last_field_and_displays_its_segments_joined_by_dots() {
    use PathSegment::{Field, Index};
    let cases = [
        Named {
            path: fields(&["inner", "weight"]),
            name: "weight",
            shown: "inner.weight",
        },
        Named {
            path: PropertyPath::new([Field("samples"), Index(2)]),
            name: "samples",
            shown: "samples.2",
        },
        Named {
            path: PropertyPath::new([Field("samples"), Index(2), Field("weight")]),
            name: "weight",
            shown: "samples.2.weight",
        },
        Named {
            path: PropertyPath::new([Index(2)]),
            name: "",
            shown: "2",
        },
        Named {
            path: PropertyPath::default(),
            name: "",
            shown: "",
        },
    ];
    for case in cases {
        assert_eq!(case.path.name(), case.name, "{}", case.shown);
        assert_eq!(case.path.to_string(), case.shown);
    }
    assert_eq!(PathSegment::Field("weight"), "weight");
    assert_ne!(PathSegment::Field("weight"), "other");
    assert_ne!(PathSegment::Index(0), "0");
    assert_eq!(
        PropertyPath::new([PathSegment::from("samples"), PathSegment::from(1_usize)]).segments(),
        [Field("samples"), Index(1)]
    );
}

#[test]
fn visiting_a_list_reports_each_element_in_order_and_never_a_field() {
    struct Elements(Vec<usize>);
    impl PropertyVisitor for Elements {
        fn field(&mut self, name: &'static str, _: &dyn Reflect) {
            panic!("a list has no field {name}");
        }
        fn element(&mut self, index: usize, value: &dyn Reflect) {
            assert_eq!(value.downcast_ref::<f32>(), Some(&(index as f32 + 1.0)));
            self.0.push(index);
        }
    }
    struct Double;
    impl PropertyVisitorMut for Double {
        fn field(&mut self, name: &'static str, _: &mut dyn Reflect) {
            panic!("a list has no field {name}");
        }
        fn element(&mut self, index: usize, value: &mut dyn Reflect) {
            *value.downcast_mut::<f32>().unwrap() *= (index + 2) as f32;
        }
    }
    let mut list = vec![1.0_f32, 2.0];
    let mut elements = Elements(Vec::new());
    list.visit(&mut elements);
    assert_eq!(elements.0, [0, 1]);
    list.visit_mut(&mut Double);
    assert_eq!(list, [2.0, 6.0]);
}

struct Unreflected(u32);

impl Default for Unreflected {
    fn default() -> Self {
        Self(7)
    }
}

#[derive(Reflect, Default)]
struct WithSkipped {
    first: f32,
    #[reflect(skip)]
    hidden: Unreflected,
    last: f32,
}

#[test]
fn a_skipped_field_is_not_a_field_and_keeps_its_value_when_a_stored_value_is_applied() {
    let value = WithSkipped {
        first: 1.0,
        hidden: Unreflected(3),
        last: 2.0,
    };
    let ReflectRef::Struct(fields) = value.reflect_ref() else {
        panic!("a struct reflects as a struct");
    };
    assert_eq!(fields.field_len(), 2);
    assert_eq!(fields.name_at(0), Some("first"));
    assert_eq!(fields.name_at(1), Some("last"));
    assert_eq!(fields.field_at(1).unwrap().downcast_ref(), Some(&2.0f32));
    assert_eq!(
        with_property(&value, &PropertyPath::new(["hidden"]), &mut |_| {}),
        Err(PathError::NotFound)
    );

    let mut loaded = WithSkipped::default();
    assert!(apply_value(&mut loaded, &to_value(&value).unwrap()).is_empty());
    assert_eq!(loaded.first, 1.0);
    assert_eq!(loaded.last, 2.0);
    assert_eq!(loaded.hidden.0, 7);
}

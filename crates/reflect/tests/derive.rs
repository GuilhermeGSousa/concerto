use std::any::{Any, TypeId};

use concerto_reflect::{
    PathError, PathSegment, PropertyPath, PropertyVisitor, PropertyVisitorMut, Reflect, ReflectRef,
    apply_value, to_value, with_property, with_property_mut,
};
use glam::{Quat, Vec3};

#[derive(Reflect)]
struct Inner {
    weight: f32,
    label: String,
    enabled: bool,
}

#[derive(Reflect)]
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
}

fn track() -> Track {
    Track {
        weights: vec![1.0, 2.0],
        samples: vec![Sample { weight: 3.0 }, Sample { weight: 4.0 }],
        rows: vec![vec![5.0], vec![6.0, 7.0]],
        fog: Some(Sample { weight: 8.0 }),
    }
}

fn outer() -> Outer {
    Outer {
        position: Vec3::X,
        rotation: Quat::IDENTITY,
        inner: Inner {
            weight: 1.0,
            label: "weight".into(),
            enabled: true,
        },
    }
}

#[test]
fn derive_visits_fields_in_declaration_order_with_concrete_types() {
    struct Fields(Vec<(&'static str, TypeId)>);
    impl PropertyVisitor for Fields {
        fn field(&mut self, name: &'static str, value: &dyn Reflect) {
            self.0.push((name, (value as &dyn Any).type_id()));
        }
    }
    let mut fields = Fields(Vec::new());
    outer().visit(&mut fields);
    assert_eq!(
        fields.0,
        vec![
            ("position", TypeId::of::<Vec3>()),
            ("rotation", TypeId::of::<Quat>()),
            ("inner", TypeId::of::<Inner>())
        ]
    );
}

#[test]
fn access_resolves_nested_leaf_composite_and_root() {
    let value = outer();
    for (path, expected) in [
        (PropertyPath::new(["inner", "weight"]), TypeId::of::<f32>()),
        (PropertyPath::new(["inner"]), TypeId::of::<Inner>()),
        (PropertyPath::default(), TypeId::of::<Outer>()),
    ] {
        let mut calls = 0;
        with_property(&value, &path, &mut |v| {
            assert_eq!((v as &dyn Any).type_id(), expected);
            calls += 1;
        })
        .unwrap();
        assert_eq!(calls, 1);
    }
}

#[test]
fn mutable_access_can_edit_nested_fields_composites_and_root() {
    let mut value = outer();
    with_property_mut(
        &mut value,
        &PropertyPath::new(["inner", "weight"]),
        &mut |v| {
            *(v as &mut dyn Any).downcast_mut::<f32>().unwrap() = 4.0;
        },
    )
    .unwrap();
    assert_eq!(value.inner.weight, 4.0);
    with_property_mut(&mut value, &PropertyPath::new(["inner"]), &mut |v| {
        (v as &mut dyn Any).downcast_mut::<Inner>().unwrap().enabled = false;
    })
    .unwrap();
    assert!(!value.inner.enabled);
    with_property_mut(&mut value, &PropertyPath::default(), &mut |v| {
        (v as &mut dyn Any)
            .downcast_mut::<Outer>()
            .unwrap()
            .position = Vec3::Y;
    })
    .unwrap();
    assert_eq!(value.position, Vec3::Y);
}

#[test]
fn unknown_paths_and_paths_past_opaque_values_do_not_call_back() {
    for path in [
        PropertyPath::new(["missing"]),
        PropertyPath::new(["inner", "missing"]),
        PropertyPath::new(["position", "x"]),
    ] {
        assert_eq!(
            with_property(&outer(), &path, &mut |_| panic!("unexpected callback")),
            Err(PathError::NotFound)
        );
        assert_eq!(
            with_property_mut(&mut outer(), &path, &mut |_| panic!("unexpected callback")),
            Err(PathError::NotFound)
        );
    }
}

#[test]
fn paths_preserve_names_depth_and_segments() {
    let path = PropertyPath::new(["inner", "weight"]);
    assert_eq!(path.name(), "weight");
    assert_eq!(path.depth(), 2);
    assert_eq!(path.get_depth(0), Some(PathSegment::Field("inner")));
    assert_eq!(path.get_depth(2), None);
    assert_eq!(PropertyPath::default().name(), "");
}

#[test]
fn a_path_mixes_field_names_and_indices() {
    let path = PropertyPath::new([PathSegment::from("samples"), PathSegment::from(1_usize)]);
    assert_eq!(
        path.segments(),
        [PathSegment::Field("samples"), PathSegment::Index(1)]
    );
    assert_eq!(path.get_depth(1), Some(PathSegment::Index(1)));
    assert_eq!(PropertyPath::new([0_usize, 2]).depth(), 2);
}

#[test]
fn a_paths_name_is_its_last_field() {
    let path = PropertyPath::new([PathSegment::Field("samples"), PathSegment::Index(2)]);
    assert_eq!(path.name(), "samples");
    assert_eq!(PropertyPath::new([PathSegment::Index(2)]).name(), "");
}

#[test]
fn a_segment_equals_the_text_of_its_field_name_only() {
    assert_eq!(PathSegment::Field("weight"), "weight");
    assert_ne!(PathSegment::Field("weight"), "other");
    assert_ne!(PathSegment::Index(0), "0");
    assert_eq!(
        PropertyPath::new(["inner", "weight"]).segments(),
        ["inner", "weight"]
    );
}

#[test]
fn a_path_displays_its_segments_joined_by_dots() {
    assert_eq!(
        PropertyPath::new(["fog", "0", "density"]).to_string(),
        "fog.0.density"
    );
    assert_eq!(
        PropertyPath::new([
            PathSegment::Field("samples"),
            PathSegment::Index(2),
            PathSegment::Field("weight"),
        ])
        .to_string(),
        "samples.2.weight"
    );
    assert_eq!(PropertyPath::default().to_string(), "");
    assert_eq!(PathSegment::Field("fog").to_string(), "fog");
    assert_eq!(PathSegment::Index(3).to_string(), "3");
}

#[test]
fn visit_reports_each_list_element_in_order() {
    struct Elements(Vec<(usize, f32)>);
    impl PropertyVisitor for Elements {
        fn field(&mut self, name: &'static str, _: &dyn Reflect) {
            panic!("a list has no field {name}");
        }
        fn element(&mut self, index: usize, value: &dyn Reflect) {
            self.0.push((index, *value.downcast_ref::<f32>().unwrap()));
        }
    }
    let mut elements = Elements(Vec::new());
    vec![1.0_f32, 2.0].visit(&mut elements);
    assert_eq!(elements.0, [(0, 1.0), (1, 2.0)]);
}

#[test]
fn visit_mut_reports_each_list_element_in_order() {
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
    list.visit_mut(&mut Double);
    assert_eq!(list, [2.0, 6.0]);
}

#[test]
fn an_options_value_is_a_field_named_zero() {
    struct Parts {
        fields: Vec<&'static str>,
        elements: usize,
    }
    impl PropertyVisitor for Parts {
        fn field(&mut self, name: &'static str, _: &dyn Reflect) {
            self.fields.push(name);
        }
        fn element(&mut self, _: usize, _: &dyn Reflect) {
            self.elements += 1;
        }
    }
    let mut parts = Parts {
        fields: Vec::new(),
        elements: 0,
    };
    Some(1.0_f32).visit(&mut parts);
    assert_eq!(parts.fields, ["0"]);
    assert_eq!(parts.elements, 0);

    let mut weight = None;
    with_property(
        &track(),
        &PropertyPath::new(["fog", "0", "weight"]),
        &mut |value| weight = value.downcast_ref::<f32>().copied(),
    )
    .unwrap();
    assert_eq!(weight, Some(8.0));
    assert_eq!(
        with_property(
            &track(),
            &PropertyPath::new([
                PathSegment::Field("fog"),
                PathSegment::Index(0),
                PathSegment::Field("weight"),
            ]),
            &mut |_| panic!("an Option has no elements"),
        ),
        Err(PathError::NotFound)
    );
}

#[test]
fn access_reaches_list_elements_and_what_they_hold() {
    let value = track();
    for (path, expected) in [
        (
            PropertyPath::new([PathSegment::Field("weights"), PathSegment::Index(1)]),
            2.0,
        ),
        (
            PropertyPath::new([
                PathSegment::Field("samples"),
                PathSegment::Index(1),
                PathSegment::Field("weight"),
            ]),
            4.0,
        ),
        (
            PropertyPath::new([
                PathSegment::Field("rows"),
                PathSegment::Index(1),
                PathSegment::Index(1),
            ]),
            7.0,
        ),
    ] {
        let mut read = None;
        with_property(&value, &path, &mut |v| {
            read = v.downcast_ref::<f32>().copied();
        })
        .unwrap();
        assert_eq!(read, Some(expected));
    }
}

#[test]
fn mutable_access_reaches_list_elements_and_what_they_hold() {
    let mut value = track();
    for path in [
        PropertyPath::new([PathSegment::Field("weights"), PathSegment::Index(1)]),
        PropertyPath::new([
            PathSegment::Field("samples"),
            PathSegment::Index(1),
            PathSegment::Field("weight"),
        ]),
        PropertyPath::new([
            PathSegment::Field("rows"),
            PathSegment::Index(1),
            PathSegment::Index(1),
        ]),
    ] {
        with_property_mut(&mut value, &path, &mut |v| {
            *v.downcast_mut::<f32>().unwrap() = 9.0;
        })
        .unwrap();
    }
    assert_eq!(value.weights, [1.0, 9.0]);
    assert_eq!(value.samples[1].weight, 9.0);
    assert_eq!(value.rows[1], [6.0, 9.0]);
}

#[test]
fn an_index_out_of_range_or_off_a_list_and_a_field_on_a_list_are_not_found() {
    for path in [
        PropertyPath::new([PathSegment::Field("weights"), PathSegment::Index(2)]),
        PropertyPath::new([
            PathSegment::Field("rows"),
            PathSegment::Index(0),
            PathSegment::Index(1),
        ]),
        PropertyPath::new([PathSegment::Index(0)]),
        PropertyPath::new([PathSegment::Field("samples"), PathSegment::Field("0")]),
    ] {
        assert_eq!(
            with_property(&track(), &path, &mut |_| panic!("unexpected callback")),
            Err(PathError::NotFound)
        );
        assert_eq!(
            with_property_mut(&mut track(), &path, &mut |_| panic!("unexpected callback")),
            Err(PathError::NotFound)
        );
    }
}

#[test]
fn downcast_reads_the_concrete_type_and_refuses_another() {
    let mut value: Box<dyn Reflect> = Box::new(Inner {
        weight: 2.0,
        label: "a".into(),
        enabled: false,
    });
    assert_eq!(
        value.downcast_ref::<Inner>().map(|inner| inner.weight),
        Some(2.0)
    );
    assert!(value.downcast_ref::<Outer>().is_none());
    value.downcast_mut::<Inner>().unwrap().weight = 5.0;
    assert_eq!(value.downcast_ref::<Inner>().unwrap().weight, 5.0);
    assert!(value.downcast_mut::<f32>().is_none());
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
fn a_skipped_field_is_not_a_field() {
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
}

#[test]
fn a_skipped_field_keeps_its_value_when_a_stored_value_is_applied() {
    let stored = to_value(&WithSkipped {
        first: 1.0,
        hidden: Unreflected(3),
        last: 2.0,
    })
    .unwrap();

    let mut loaded = WithSkipped::default();
    let warnings = apply_value(&mut loaded, &stored);

    assert!(warnings.is_empty());
    assert_eq!(loaded.first, 1.0);
    assert_eq!(loaded.last, 2.0);
    assert_eq!(loaded.hidden.0, 7);
}

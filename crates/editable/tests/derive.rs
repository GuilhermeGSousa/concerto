use std::any::{Any, TypeId};

use editable::{
    Editable, PathError, PropertyPath, PropertyVisitor, with_property, with_property_mut,
};
use glam::{Quat, Vec3};

#[derive(Editable)]
struct Inner {
    weight: f32,
    label: String,
    enabled: bool,
}

#[derive(Editable)]
struct Outer {
    position: Vec3,
    rotation: Quat,
    inner: Inner,
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
        fn field(&mut self, name: &'static str, value: &dyn Editable) {
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
    assert_eq!(path.get_depth(0), Some("inner"));
    assert_eq!(path.get_depth(2), None);
    assert_eq!(PropertyPath::default().name(), "");
}

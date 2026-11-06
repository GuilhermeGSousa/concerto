use std::{any::TypeId, thread};

use concerto_name::{name, Name};
use concerto_reflect::{
    apply_value, json_to_value, to_json, to_value, ReflectDefault, ReflectJson, TypeRegistry,
};
use serde_json::json;

#[test]
fn the_same_text_interns_to_equal_names() {
    assert_eq!(Name::new("Hips"), Name::new("Hips"));
    assert_eq!(Name::new("Hips"), Name::from(String::from("Hips")));
}

#[test]
fn different_texts_intern_to_different_names() {
    assert_ne!(Name::new("Hips"), Name::new("Spine"));
    assert_ne!(Name::new("hips"), Name::new("Hips"));
}

#[test]
fn a_name_reads_back_its_text() {
    let name = Name::new("LeftFoot");

    assert_eq!(name.as_str(), "LeftFoot");
    assert_eq!(name, "LeftFoot");
    assert_eq!(name, String::from("LeftFoot"));
    assert_eq!(name.to_string(), "LeftFoot");
    assert_eq!(format!("{name:?}"), "\"LeftFoot\"");
}

#[test]
fn the_default_name_is_the_empty_text() {
    assert_eq!(Name::default(), Name::new(""));
    assert_eq!(Name::default().as_str(), "");
}

#[test]
fn names_interned_on_several_threads_are_equal_per_text() {
    let texts: Vec<String> = (0..64)
        .map(|index| format!("thread-{}", index % 8))
        .collect();
    let handles: Vec<_> = (0..8)
        .map(|_| {
            let texts = texts.clone();
            thread::spawn(move || texts.iter().map(|text| Name::new(text)).collect::<Vec<_>>())
        })
        .collect();
    let results: Vec<Vec<Name>> = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .collect();

    for names in &results {
        for (name, text) in names.iter().zip(&texts) {
            assert_eq!(*name, Name::new(text));
            assert_eq!(name.as_str(), text);
        }
    }
}

#[test]
fn the_macro_returns_the_name_of_its_text_on_every_call() {
    let read = || name!("Spine");

    assert_eq!(read(), Name::new("Spine"));
    assert_eq!(read(), read());
}

#[test]
fn a_name_encodes_with_bincode_as_the_string_of_its_text() {
    let bytes = bincode::serialize(&Name::new("Hips")).unwrap();

    assert_eq!(bytes, bincode::serialize(&String::from("Hips")).unwrap());
    assert_eq!(
        bincode::deserialize::<Name>(&bytes).unwrap(),
        Name::new("Hips")
    );
}

#[test]
fn a_name_round_trips_through_its_reflected_value() {
    let stored = to_value(&Name::new("Head")).unwrap();

    assert_eq!(stored, to_value(&String::from("Head")).unwrap());

    let mut read = Name::default();
    assert!(apply_value(&mut read, &stored).is_empty());
    assert_eq!(read, Name::new("Head"));
}

#[test]
fn a_name_writes_and_reads_json_as_a_string() {
    let mut registry = TypeRegistry::default();
    registry.register::<Name>();
    let id = TypeId::of::<Name>();
    let registration = registry.get(id).unwrap();

    assert!(registration.data::<ReflectDefault>().is_some());
    assert!(registration.data::<ReflectJson>().is_some());
    assert_eq!(to_json(&Name::new("Neck"), &registry), json!("Neck"));
    assert_eq!(
        json_to_value(&json!("Neck"), id, &registry).unwrap(),
        to_value(&Name::new("Neck")).unwrap()
    );
    assert_eq!(
        json_to_value(&json!(3), id, &registry).unwrap_err(),
        "expected a string"
    );
}

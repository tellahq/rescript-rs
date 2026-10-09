#![cfg(feature = "spice")]

use rescript_rs::{
    spice::{Declare, DecodeError, Dict, Failure, Null, Plain, Spice},
    Spice,
};
use serde_json::{json, Value};

#[derive(Spice, Debug, Clone, PartialEq)]
#[spice(module = "SerdePoint", name = "t")]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

/// Externally tagged.
#[derive(Spice, Debug, Clone, PartialEq)]
#[spice(module = "SerdeExt", name = "t", serde)]
pub enum Ext {
    Unit,
    #[spice(alias = "renamed")]
    Renamed,
    One(i32),
    Two(i32, String),
    Rec {
        a: i32,
        b: Option<String>,
    },
    #[spice(alias = "full")]
    Full {
        #[spice(key = "punch_in")]
        punch_in: bool,
    },
    Opt(Option<i32>),
    Arr(Vec<Option<i32>>),
    Point(Point),
    Nested(Box<Ext>),
    Inner(Int),
}

/// Internally tagged.
#[derive(Spice, Debug, Clone, PartialEq)]
#[spice(module = "SerdeInt", name = "t", serde, tag = "type")]
pub enum Int {
    Unit,
    Point(Point),
    Rec {
        a: i32,
        #[spice(optional)]
        b: Option<String>,
    },
    #[spice(alias = "full")]
    Full {
        #[spice(name = "screenFit")]
        screen_fit: String,
        #[spice(key = "punch_in")]
        punch_in: bool,
    },
    Tree {
        children: Vec<Int>,
    },
}

/// Field names that are also the generated decoder's locals.
#[derive(Spice, Debug, Clone, PartialEq)]
#[spice(module = "SerdeClash", name = "t", serde)]
pub enum Clash {
    Clash {
        json: i32,
        fields: i32,
        payload: i32,
        items: i32,
        map: i32,
        name: i32,
    },
}

#[derive(Spice, Debug, Clone, PartialEq)]
#[spice(module = "SerdeClashTagged", name = "t", serde, tag = "kind")]
pub enum ClashTagged {
    Clash {
        json: i32,
        fields: i32,
        payload: i32,
        items: i32,
        map: i32,
        name: i32,
    },
}

#[derive(Spice, Debug, Clone, PartialEq)]
#[spice(module = "SerdeFields", name = "t", serde, tag = "type")]
pub enum Fields {
    Fields {
        #[spice(default = "7")]
        count: i32,
        #[spice(optional)]
        optional: Option<String>,
        nullable: Null<i32>,
        tuple: (i32, String),
        result: Result<i32, String>,
        dict: Dict<bool>,
        #[spice(codec = "Upper.codec", with = "upper")]
        upper: String,
    },
}

pub mod upper {
    use rescript_rs::spice::{Failure, Host, Result, Value, View};

    pub fn decode<H: Host>(json: &Value) -> Result<String> {
        match H::view(json) {
            View::String(s) => Ok(s.to_lowercase()),
            _ => Err(Failure::error("Not a string", json)),
        }
    }

    pub fn encode<H: Host>(value: &String) -> Value {
        Value::String(value.to_uppercase())
    }

    pub fn samples() -> Vec<String> {
        vec!["ab".into()]
    }
}

#[derive(Spice, Debug, Clone, PartialEq)]
#[spice(module = "SerdeNames", name = "t", serde)]
pub enum Names {
    #[spice(alias = "하나")]
    Hana,
    #[spice(alias = "with space")]
    Space,
    #[spice(alias = "")]
    Empty,
}

#[derive(Spice, Debug, Clone, PartialEq)]
#[spice(module = "SerdeDecodeOnly", name = "t", serde, decode_only)]
pub enum DecodeOnly {
    D1,
    D2(i32),
}

#[derive(Spice, Debug, Clone, PartialEq)]
#[spice(
    module = "SerdeEncodeOnly",
    name = "t",
    serde,
    tag = "type",
    encode_only
)]
pub enum EncodeOnly {
    E1,
    E2 { x: i32 },
}

fn decode<T: Spice>(json: Value) -> Result<T, Failure> {
    T::decode::<Plain>(&json)
}

fn error(path: &str, message: &str, value: Value) -> Failure {
    Failure::Decode(DecodeError {
        path: path.into(),
        message: message.into(),
        value,
    })
}

fn errors<T: Spice + std::fmt::Debug>(cases: &[(Value, (&str, &str, Value))]) {
    for (input, (path, message, value)) in cases {
        assert_eq!(
            decode::<T>(input.clone()).map(|_| ()),
            Err(error(path, message, value.clone())),
            "{input}"
        );
    }
}

#[test]
fn declarations() {
    assert_eq!(
        Int::declaration(),
        "// Internally tagged.
@spice.serde @tag(\"type\")
type rec t =
  | Unit
  | Point(SerdePoint.t)
  | Rec({a: int, b?: string})
  | @spice.as(\"full\") Full({screenFit: string, punch_in: bool})
  | Tree({children: array<SerdeInt.t>})"
    );
    assert_eq!(
        Names::declaration(),
        "@spice.serde
type t =
  | @spice.as(\"하나\") Hana
  | @spice.as(\"with space\") Space
  | @spice.as(\"\") Empty"
    );
    assert!(DecodeOnly::declaration().starts_with("@spice.serde @spice.decode\ntype t ="));
    assert!(EncodeOnly::declaration()
        .starts_with("@spice.serde @spice.encode @tag(\"type\")\ntype t ="));
}

#[test]
fn externally_tagged() {
    let cases = [
        (Ext::Unit, json!("Unit")),
        (Ext::Renamed, json!("renamed")),
        (Ext::One(1), json!({"One": 1})),
        (Ext::Two(1, "a".into()), json!({"Two": [1, "a"]})),
        (Ext::Rec { a: 1, b: None }, json!({"Rec": {"a": 1}})),
        (
            Ext::Full { punch_in: true },
            json!({"full": {"punch_in": true}}),
        ),
        (Ext::Opt(None), json!({"Opt": null})),
        (Ext::Arr(vec![Some(1), None]), json!({"Arr": [1, null]})),
        (
            Ext::Point(Point { x: 1, y: 2 }),
            json!({"Point": {"x": 1, "y": 2}}),
        ),
        (
            Ext::Nested(Box::new(Ext::One(2))),
            json!({"Nested": {"One": 2}}),
        ),
        (Ext::Inner(Int::Unit), json!({"Inner": {"type": "Unit"}})),
    ];

    for (value, encoded) in cases {
        assert_eq!(value.encode::<Plain>(), encoded);
        assert_eq!(decode::<Ext>(encoded), Ok(value));
    }

    // serde also writes a unit variant as a map with null.
    assert_eq!(decode::<Ext>(json!({"Unit": null})), Ok(Ext::Unit));
}

#[test]
fn internally_tagged() {
    let tree = Int::Tree {
        children: vec![
            Int::Unit,
            Int::Rec {
                a: 1,
                b: Some("b".into()),
            },
        ],
    };
    let cases = [
        (Int::Unit, json!({"type": "Unit"})),
        (
            Int::Point(Point { x: 1, y: 2 }),
            json!({"type": "Point", "x": 1, "y": 2}),
        ),
        (Int::Rec { a: 1, b: None }, json!({"type": "Rec", "a": 1})),
        (
            Int::Full {
                screen_fit: "fill".into(),
                punch_in: false,
            },
            json!({"type": "full", "screenFit": "fill", "punch_in": false}),
        ),
        (
            tree,
            json!({"type": "Tree", "children": [{"type": "Unit"}, {"type": "Rec", "a": 1, "b": "b"}]}),
        ),
    ];

    for (value, encoded) in cases {
        assert_eq!(value.encode::<Plain>().to_string(), encoded.to_string());
        assert_eq!(decode::<Int>(encoded), Ok(value));
    }

    // A bare name reads a unit; extra keys are ignored, as in serde.
    assert_eq!(decode::<Int>(json!("Unit")), Ok(Int::Unit));
    assert_eq!(
        decode::<Int>(json!({"type": "Unit", "x": 1})),
        Ok(Int::Unit)
    );
    assert_eq!(
        decode::<Int>(json!({"x": 1, "type": "Point", "y": 2, "z": 3})),
        Ok(Int::Point(Point { x: 1, y: 2 }))
    );
}

#[test]
#[should_panic(expected = "its payload doesn't encode to an object")]
fn a_tagged_value_that_is_not_an_object_panics() {
    #[derive(Spice, Debug, Clone, PartialEq)]
    #[spice(serde, tag = "type")]
    enum Bad {
        Number(i32),
    }

    Bad::Number(1).encode::<Plain>();
}

#[test]
fn legacy_arrays_still_decode() {
    assert_eq!(decode::<Ext>(json!(["Unit"])), Ok(Ext::Unit));
    assert_eq!(
        decode::<Ext>(json!(["Two", 1, "a"])),
        Ok(Ext::Two(1, "a".into()))
    );
    // The array names the constructor, not its @spice.as name.
    assert_eq!(
        decode::<Ext>(json!(["Full", {"punch_in": true}])),
        Ok(Ext::Full { punch_in: true })
    );
    assert_eq!(
        decode::<Ext>(json!(["Nested", ["Inner", ["Point", {"x": 1, "y": 2}]]])),
        Ok(Ext::Nested(Box::new(Ext::Inner(Int::Point(Point {
            x: 1,
            y: 2
        })))))
    );
    assert_eq!(
        decode::<Int>(json!(["Tree", {"children": [["Unit"], {"type": "Unit"}, "Unit"]}])),
        Ok(Int::Tree {
            children: vec![Int::Unit, Int::Unit, Int::Unit]
        })
    );
}

#[test]
fn field_names_used_by_the_decoder() {
    let fields = json!({"json": 1, "fields": 2, "payload": 3, "items": 4, "map": 5, "name": 6});
    let mut tagged = json!({"kind": "Clash"});

    tagged
        .as_object_mut()
        .unwrap()
        .extend(fields.as_object().unwrap().clone());

    let clash = Clash::Clash {
        json: 1,
        fields: 2,
        payload: 3,
        items: 4,
        map: 5,
        name: 6,
    };
    let clash_tagged = ClashTagged::Clash {
        json: 1,
        fields: 2,
        payload: 3,
        items: 4,
        map: 5,
        name: 6,
    };

    assert_eq!(decode::<Clash>(json!({"Clash": fields})), Ok(clash));
    assert_eq!(
        decode::<ClashTagged>(tagged.clone()),
        Ok(clash_tagged.clone())
    );
    assert_eq!(
        clash_tagged.encode::<Plain>().to_string(),
        tagged.to_string()
    );
    // A missing field reports the whole value, as spice does.
    assert_eq!(
        decode::<Clash>(json!({"Clash": {"json": 1}})),
        Err(error(
            ".Clash.fields",
            "fields missing",
            json!({"Clash": {"json": 1}})
        ))
    );
}

#[test]
fn field_attributes_and_types() {
    let value = Fields::Fields {
        count: 1,
        optional: Some("o".into()),
        nullable: Null::Value(3),
        tuple: (1, "x".into()),
        result: Err("e".into()),
        dict: Dict(vec![("1".into(), true), ("a".into(), false)]),
        upper: "ab".into(),
    };
    let encoded = json!({
        "type": "Fields", "count": 1, "optional": "o", "nullable": 3, "tuple": [1, "x"],
        "result": ["Error", "e"], "dict": {"1": true, "a": false}, "upper": "AB",
    });

    assert_eq!(value.encode::<Plain>().to_string(), encoded.to_string());
    assert_eq!(decode::<Fields>(encoded), Ok(value));
    assert_eq!(
        decode::<Fields>(json!({
            "type": "Fields", "optional": null, "nullable": null, "tuple": [1, "x"],
            "result": ["Ok", 1], "dict": {}, "upper": "AB",
        })),
        Ok(Fields::Fields {
            count: 7,
            optional: None,
            nullable: Null::Null,
            tuple: (1, "x".into()),
            result: Ok(1),
            dict: Dict(vec![]),
            upper: "ab".into(),
        })
    );
}

#[test]
fn names() {
    for (value, encoded) in [
        (Names::Hana, json!("하나")),
        (Names::Space, json!("with space")),
        (Names::Empty, json!("")),
    ] {
        assert_eq!(value.encode::<Plain>(), encoded);
        assert_eq!(decode::<Names>(encoded), Ok(value));
    }

    assert_eq!(decode::<Names>(json!({"": null})), Ok(Names::Empty));
}

#[test]
fn decode_and_encode_only() {
    assert_eq!(
        decode::<DecodeOnly>(json!({"D2": 1})),
        Ok(DecodeOnly::D2(1))
    );
    assert_eq!(
        EncodeOnly::E2 { x: 1 }.encode::<Plain>(),
        json!({"type": "E2", "x": 1})
    );
}

#[test]
fn samples_of_recursive_types_and_tuples() {
    let trees = Int::samples();

    assert!(trees.contains(&Int::Tree { children: vec![] }));
    assert!(Fields::samples().len() > 1);
}

#[test]
fn rejected_externally_tagged() {
    errors::<Ext>(&[
        (json!(1), ("", "Not a variant", json!(1))),
        (json!(true), ("", "Not a variant", json!(true))),
        (json!(null), ("", "Not a variant", json!(null))),
        (
            json!("One"),
            ("", "Invalid variant constructor", json!("One")),
        ),
        (
            json!("Nope"),
            ("", "Invalid variant constructor", json!("Nope")),
        ),
        (
            json!([]),
            ("", "Expected variant, found empty array", json!([])),
        ),
        (
            json!({}),
            ("", "Expected an object with one key", json!({})),
        ),
        (
            json!({"One": 1, "Unit": null}),
            (
                "",
                "Expected an object with one key",
                json!({"One": 1, "Unit": null}),
            ),
        ),
        (
            json!({"Nope": 1}),
            ("", "Invalid variant constructor", json!({"Nope": 1})),
        ),
        (json!({"Unit": 1}), (".Unit", "Expected null", json!(1))),
        (json!({"One": "x"}), (".One", "Not a number", json!("x"))),
        (json!({"One": null}), (".One", "Not a number", json!(null))),
        (json!({"Two": 1}), (".Two", "Not an array", json!(1))),
        (
            json!({"Two": [1]}),
            (
                ".Two",
                "Invalid number of arguments to variant constructor",
                json!([1]),
            ),
        ),
        (
            json!({"Two": ["a", 2]}),
            (".Two[0]", "Not a number", json!("a")),
        ),
        (
            json!({"Two": [1, 2]}),
            (".Two[1]", "Not a string", json!(2)),
        ),
        (json!({"Rec": 1}), (".Rec", "Not an object", json!(1))),
        (json!({"Rec": null}), (".Rec", "Not an object", json!(null))),
        (
            json!({"Rec": {"b": "x"}}),
            (".Rec.a", "a missing", json!({"Rec": {"b": "x"}})),
        ),
        (
            json!({"full": {}}),
            (".full.punch_in", "punch_in missing", json!({"full": {}})),
        ),
        (
            json!({"Full": {}}),
            ("", "Invalid variant constructor", json!({"Full": {}})),
        ),
        (
            json!({"Arr": [1, "x"]}),
            (".Arr[1]", "Not a number", json!("x")),
        ),
        (
            json!({"Point": {"x": 1}}),
            (".Point.y", "y missing", json!({"x": 1})),
        ),
        (
            json!({"Nested": {"Two": [1, 2]}}),
            (".Nested.Two[1]", "Not a string", json!(2)),
        ),
        (
            json!({"Inner": {"type": "Rec"}}),
            (".Inner.a", "a missing", json!({"type": "Rec"})),
        ),
        (
            json!({"Inner": {"a": 1}}),
            (".Inner", "type missing", json!({"a": 1})),
        ),
        // The default spice arrays, as plain spice rejects them.
        (
            json!(["One"]),
            (
                "",
                "Invalid number of arguments to variant constructor",
                json!(["One"]),
            ),
        ),
        (
            json!(["Unit", 1]),
            (
                "",
                "Invalid number of arguments to variant constructor",
                json!(["Unit", 1]),
            ),
        ),
        (
            json!(["Nope", 1]),
            ("", "Invalid variant constructor", json!("Nope")),
        ),
        (
            json!(["renamed"]),
            ("", "Invalid variant constructor", json!("renamed")),
        ),
        (json!([1]), ("", "Invalid variant constructor", json!(1))),
        (json!(["Two", "a", 1]), ("[1]", "Not a number", json!("a"))),
        (json!(["Rec", {}]), ("[1].a", "a missing", json!({}))),
    ]);
}

#[test]
fn rejected_internally_tagged() {
    errors::<Int>(&[
        (json!(1), ("", "Not a variant", json!(1))),
        (json!(null), ("", "Not a variant", json!(null))),
        (
            json!("Rec"),
            ("", "Invalid variant constructor", json!("Rec")),
        ),
        (
            json!([]),
            ("", "Expected variant, found empty array", json!([])),
        ),
        (json!({}), ("", "type missing", json!({}))),
        (json!({"a": 1}), ("", "type missing", json!({"a": 1}))),
        (json!({"type": 1}), ("", "type missing", json!({"type": 1}))),
        (
            json!({"type": null}),
            ("", "type missing", json!({"type": null})),
        ),
        (
            json!({"type": "Nope"}),
            ("", "Invalid variant constructor", json!({"type": "Nope"})),
        ),
        (
            json!({"type": "Full"}),
            ("", "Invalid variant constructor", json!({"type": "Full"})),
        ),
        (
            json!({"type": "Rec"}),
            (".a", "a missing", json!({"type": "Rec"})),
        ),
        (
            json!({"type": "Rec", "a": "x"}),
            (".a", "Not a number", json!("x")),
        ),
        (
            json!({"type": "Point", "x": 1}),
            (".y", "y missing", json!({"x": 1})),
        ),
        (
            json!({"type": "Point", "x": null, "y": 1}),
            (".x", "Not a number", json!(null)),
        ),
        (
            json!({"type": "Tree", "children": [{"type": "Unit"}, {"type": "Rec", "a": []}]}),
            (".children[1].a", "Not a number", json!([])),
        ),
        (
            json!({"type": "Tree", "children": [{}]}),
            (".children[0]", "type missing", json!({})),
        ),
        (json!(["Rec", {}]), ("[1].a", "a missing", json!({}))),
        (json!(["Point", 1]), ("[1]", "Not an object", json!(1))),
    ]);
}

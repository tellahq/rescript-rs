#![cfg(feature = "spice")]

use rescript_rs::{
    spice::{Declare, Plain, Spice},
    ReScript,
};
use serde_json::{json, Value};

#[derive(ReScript, Debug, Clone, PartialEq)]
#[rescript(codec, module = "LegacyPoint", name = "t")]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

#[derive(ReScript, Debug, Clone, PartialEq)]
#[rescript(codec, legacy_write, module = "LegacyExt", name = "t")]
pub enum Ext {
    Unit,
    #[rescript(rename = "renamed")]
    Renamed,
    One(i32),
    Two(i32, String),
    Rec { a: i32, b: Option<String> },
}

#[derive(ReScript, Debug, Clone, PartialEq)]
#[rescript(codec, legacy_write, module = "LegacyInt", name = "t", tag = "type")]
pub enum Int {
    Unit,
    Point(Point),
}

#[derive(ReScript, Debug, Clone, PartialEq)]
#[rescript(codec, legacy_write, module = "LegacyPoly", name = "t", poly)]
pub enum Poly {
    A,
    B(i32),
}

fn decode<T: Spice>(json: Value) -> T {
    T::decode::<Plain>(&json).expect("decodes")
}

#[test]
fn writes_the_default_spice_encoding() {
    assert_eq!(Ext::Unit.encode::<Plain>(), json!(["Unit"]));
    assert_eq!(Ext::Renamed.encode::<Plain>(), json!(["Renamed"]));
    assert_eq!(Ext::One(1).encode::<Plain>(), json!(["One", 1]));
    assert_eq!(Ext::Two(1, "a".into()).encode::<Plain>(), json!(["Two", 1, "a"]));
    assert_eq!(
        Ext::Rec { a: 1, b: None }.encode::<Plain>(),
        json!(["Rec", {"a": 1}])
    );
    assert_eq!(Int::Unit.encode::<Plain>(), json!(["Unit"]));
    assert_eq!(
        Int::Point(Point { x: 1, y: 2 }).encode::<Plain>(),
        json!(["Point", {"x": 1, "y": 2}])
    );
    assert_eq!(Poly::B(3).encode::<Plain>(), json!(["B", 3]));
}

#[test]
fn reads_both_encodings() {
    assert_eq!(decode::<Ext>(json!(["Renamed"])), Ext::Renamed);
    assert_eq!(decode::<Ext>(json!("renamed")), Ext::Renamed);
    assert_eq!(decode::<Ext>(json!({"Two": [1, "a"]})), Ext::Two(1, "a".into()));
    assert_eq!(
        decode::<Ext>(json!(["Rec", {"a": 1, "b": "x"}])),
        Ext::Rec { a: 1, b: Some("x".into()) }
    );
    assert_eq!(
        decode::<Int>(json!({"type": "Point", "x": 1, "y": 2})),
        Int::Point(Point { x: 1, y: 2 })
    );
    assert_eq!(decode::<Poly>(json!({"B": 4})), Poly::B(4));
    assert_eq!(decode::<Poly>(json!(["B", 4])), Poly::B(4));

    for value in Ext::samples() {
        assert_eq!(decode::<Ext>(value.encode::<Plain>()), value);
    }
}

#[test]
fn declares_legacy_write() {
    assert!(Ext::declaration().starts_with("@spice.serde({legacyWrite: true})\ntype t ="));
    assert!(Int::declaration()
        .starts_with("@spice.serde({legacyWrite: true}) @tag(\"type\")\ntype t ="));
}

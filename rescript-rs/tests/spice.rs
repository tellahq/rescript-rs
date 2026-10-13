#![cfg(feature = "spice")]

use rescript_rs::{
    spice::{Declare, DecodeError, Dict, Failure, Module, Null, Plain, Spice},
    ReScript,
};
use serde_json::{json, Value};

#[derive(ReScript, Debug, Clone, PartialEq)]
#[rescript(codec, module = "Kind")]
enum Kind {
    Story,
    Scene,
}

#[derive(ReScript, Debug, Clone, PartialEq)]
#[rescript(codec, module = "Row", name = "t")]
/// A stored row.
struct Row {
    #[rescript(name = "pk", key = "PK")]
    pk: String,
    #[rescript(name = "type_", key = "type", default = "Story")]
    kind: Kind,
    count: i32,
    #[rescript(default = "[]")]
    words: Vec<String>,
    note: Option<String>,
    #[rescript(optional)]
    title: Option<String>,
    #[rescript(default = "1.5")]
    scale: f64,
    tags: Dict<bool>,
    parent: Null<String>,
}

#[derive(ReScript, Debug, Clone, PartialEq)]
#[rescript(codec, module = "Shape", name = "t")]
enum Shape {
    Empty,
    Circle(f64),
    Rect(f64, f64),
    Named { name: String, label: Option<String> },
}

#[derive(ReScript, Debug, Clone, PartialEq)]
#[rescript(codec, module = "Color", name = "t", attrs = "@genType")]
enum Color {
    #[rescript(rename = "red")]
    Red,
    #[rescript(rename = "dark-blue")]
    Blue,
}

#[derive(ReScript, Debug, Clone, PartialEq)]
#[rescript(codec, module = "Words", name = "t")]
struct Words(Vec<String>);

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

#[test]
fn declarations() {
    assert_eq!(
        Module::new("Row").with::<Row>().render(),
        "// A stored row.
@spice
type t = {
  @spice.key(\"PK\")
  pk: string,
  @spice.key(\"type\") @spice.default(Story)
  type_: Kind.kind,
  count: int,
  @spice.default([])
  words: array<string>,
  note: option<string>,
  title?: string,
  @spice.default(1.5)
  scale: float,
  tags: dict<bool>,
  parent: Null.t<string>,
}
"
    );
    assert_eq!(
        Shape::declaration(),
        "@spice.serde
type t =
  | Empty
  | Circle(float)
  | Rect(float, float)
  | Named({name: string, label: option<string>})"
    );
    assert_eq!(
        Color::declaration(),
        "@spice.serde @genType
type t =
  | @as(\"red\") Red
  | @as(\"dark-blue\") Blue"
    );
    assert_eq!(Words::declaration(), "@spice\ntype t = array<string>");
    assert_eq!(
        Module::new("Kind").with::<Kind>().render(),
        "@spice.serde\ntype kind =\n  | Story\n  | Scene\n"
    );
}

#[test]
fn records() {
    let row = json!({
        "PK": "a", "count": 2, "note": "n", "tags": {"b": true, "1": false},
        "parent": null, "extra": 1,
    });

    let decoded: Row = decode(row).unwrap();

    assert_eq!(
        decoded,
        Row {
            pk: "a".into(),
            kind: Kind::Story,
            count: 2,
            words: vec![],
            note: Some("n".into()),
            title: None,
            scale: 1.5,
            tags: Dict(vec![("1".into(), false), ("b".into(), true)]),
            parent: Null::Null,
        }
    );
    assert_eq!(
        decoded.encode::<Plain>(),
        json!({"PK": "a", "type": "Story", "count": 2, "words": [], "note": "n", "scale": 1.5, "tags": {"1": false, "b": true}, "parent": null})
    );
}

#[test]
fn record_errors() {
    assert_eq!(
        decode::<Row>(json!([])),
        Err(error("", "Not an object", json!([])))
    );
    assert_eq!(
        decode::<Row>(json!({"count": 1})),
        Err(error(".PK", "PK missing", json!({"count": 1})))
    );
    assert_eq!(
        decode::<Row>(json!({"PK": "a", "type": "Nope", "count": 1})),
        Err(error(".type", "Invalid variant constructor", json!("Nope")))
    );
    assert_eq!(
        decode::<Row>(json!({"PK": "a", "count": 1.5})),
        Err(error(".count", "Not an integer", json!(1.5)))
    );
    assert_eq!(
        decode::<Row>(json!({"PK": "a", "count": 1, "words": ["x", 1, true]})),
        Err(error(".words[1]", "Not a string", json!(1)))
    );
    // A null option field the inner type rejects is None, not an error.
    let row: Row = decode(json!({
        "PK": "a", "count": 4294967297.0, "note": null, "title": null,
        "tags": {}, "parent": "p",
    }))
    .unwrap();

    assert_eq!((row.count, row.note, row.title), (1, None, None));
    assert_eq!(row.parent, Null::Value("p".into()));
}

#[test]
fn renames_and_newtypes() {
    assert_eq!(decode::<Color>(json!("dark-blue")), Ok(Color::Blue));
    assert_eq!(Color::Red.encode::<Plain>(), json!("red"));
    assert_eq!(
        decode::<Color>(json!("Blue")),
        Err(error("", "Invalid variant constructor", json!("Blue")))
    );
    assert_eq!(decode::<Words>(json!(["a"])), Ok(Words(vec!["a".into()])));
}

#[derive(ReScript, Debug, Clone, PartialEq)]
#[rescript(codec, module = "Kind.Row", name = "t", decode_only)]
struct KindRow {
    #[rescript(name = "type_", key = "type")]
    kind: Kind,
}

#[test]
fn nested_modules() {
    assert_eq!(
        Module::new("Kind")
            .header("// generated\n")
            .with::<Kind>()
            .nested(Module::new("Kind.Row").with::<KindRow>())
            .render(),
        "// generated
@spice.serde
type kind =
  | Story
  | Scene

module Row = {
  @spice.decode
  type t = {
    @spice.key(\"type\")
    type_: kind,
  }
}
"
    );
}

#[test]
fn samples_cover_fields_and_constructors() {
    let shapes = Shape::samples();

    assert_eq!(shapes[0], Shape::Empty);
    assert!(shapes.contains(&Shape::Rect(0.0, 1.5)));
    assert!(shapes.contains(&Shape::Named {
        name: "a".into(),
        label: None
    }));

    let rows = Row::samples();

    assert_eq!(rows[0].count, 0);
    assert!(rows.iter().any(|row| row.kind == Kind::Scene));
    assert!(rows.iter().any(|row| row.title == Some("a".into())));
}

// No Debug or PartialEq: std stops at 12-value tuples.
#[derive(ReScript, Clone)]
#[rescript(codec, module = "Wide", name = "t")]
struct Wide {
    values: Vec<(
        i32,
        i32,
        i32,
        i32,
        i32,
        i32,
        i32,
        i32,
        i32,
        i32,
        i32,
        i32,
        i32,
        i32,
        i32,
        String,
    )>,
}

#[test]
fn sixteen_value_tuples() {
    let json = json!({"values": [[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, "p"]]});
    let wide: Wide = decode(json.clone()).unwrap();

    assert_eq!(wide.values[0].15, "p");
    assert_eq!(wide.encode::<Plain>(), json);
    assert_eq!(
        Wide::declaration(),
        "@spice\ntype t = {\n  values: array<(int, int, int, int, int, int, int, int, int, int, int, int, int, int, int, string)>,\n}"
    );
    assert_eq!(
        decode::<Wide>(json!({"values": [[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]]}))
            .err(),
        Some(error(".values[0][15]", "Not a string", json!(15)))
    );
}

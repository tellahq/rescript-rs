#![cfg(feature = "spice")]

//! `#[rescript(codec)]` reads serde's attributes: `rename`, `rename_all`,
//! `rename_all_fields`, `tag` and `default`, whose Rust value it writes
//! as the ReScript `@spice.default(…)`.

use rescript_rs::{
    spice::{Module, Plain, Spice},
    ReScript,
};
use serde_json::json;

#[derive(ReScript, Debug, Clone, Copy, PartialEq, Default)]
#[rescript(codec, module = "Attrs", name = "grouping")]
#[serde(rename_all = "snake_case")]
pub enum Grouping {
    #[default]
    Chunked,
    SingleWord,
    #[serde(rename = "WORDS")]
    Words,
}

#[derive(ReScript, Debug, Clone, PartialEq)]
#[rescript(codec, module = "Attrs", name = "point")]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

fn centre() -> Point {
    Point { x: 0.5, y: 1. }
}

fn forty() -> f64 {
    40.
}

#[derive(ReScript, Debug, Clone, PartialEq)]
#[rescript(codec, module = "Attrs", name = "t")]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub story_id: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(rename = "PK")]
    pub pk: String,
    #[serde(rename = "og_image")]
    pub og_image: String,
    #[serde(default)]
    pub grouping: Grouping,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default = "forty")]
    pub font_size: f64,
    #[serde(default = "centre")]
    pub focus: Point,
}

#[derive(ReScript, Debug, Clone, PartialEq)]
#[rescript(codec, module = "Attrs", name = "shape")]
#[serde(tag = "type", rename_all_fields = "camelCase")]
pub enum Shape {
    Empty,
    Circle { radius_px: f64 },
}

#[test]
fn declarations() {
    let rendered = Module::new("Attrs")
        .with::<Grouping>()
        .with::<Point>()
        .with::<Settings>()
        .with::<Shape>()
        .render();

    assert_eq!(
        rendered,
        r#"@spice.serde
type grouping =
  | @spice.as("chunked") Chunked
  | @spice.as("single_word") SingleWord
  | @spice.as("WORDS") Words

@spice.serde
type point = {
  x: float,
  y: float,
}

@spice.serde
type t = {
  storyId: string,
  @spice.key("type")
  type_: string,
  @spice.key("PK")
  pk: string,
  og_image: string,
  @spice.default(Chunked)
  grouping: grouping,
  @spice.default([])
  tags: array<string>,
  @spice.default(false)
  enabled: bool,
  @spice.default(None)
  note: option<string>,
  @spice.default(40.)
  fontSize: float,
  @spice.default({x: 0.5, y: 1.})
  focus: point,
}

@spice.serde @tag("type")
type shape =
  | Empty
  | Circle({radiusPx: float})
"#
    );
}

#[test]
fn reads_and_writes_the_serde_names() {
    let json = json!({"storyId": "s", "type": "k", "PK": "p", "og_image": "o"});
    let settings = Settings::decode::<Plain>(&json).unwrap();

    assert_eq!(settings.grouping, Grouping::Chunked);
    assert_eq!(settings.font_size, 40.);
    assert_eq!(settings.focus, centre());
    assert_eq!(
        settings.encode::<Plain>(),
        json!({
            "storyId": "s", "type": "k", "PK": "p", "og_image": "o",
            "grouping": "chunked", "tags": [], "enabled": false,
            "fontSize": 40, "focus": {"x": 0.5, "y": 1},
        })
    );
    assert_eq!(Grouping::SingleWord.encode::<Plain>(), json!("single_word"));
    assert_eq!(Grouping::Words.encode::<Plain>(), json!("WORDS"));
    assert_eq!(
        Shape::Circle { radius_px: 2. }.encode::<Plain>(),
        json!({"type": "Circle", "radiusPx": 2})
    );
}

#[test]
fn values_as_rescript() {
    assert_eq!(Grouping::Words.rescript_value().unwrap(), "Attrs.Words");
    assert_eq!(
        Shape::Circle { radius_px: 2. }.rescript_value().unwrap(),
        "Attrs.Circle({radiusPx: 2.})"
    );
    assert_eq!(
        Some(vec![1.5, -2.]).rescript_value().unwrap(),
        "Some([1.5, -2.])"
    );
    assert_eq!("a\"b".to_string().rescript_value().unwrap(), r#""a\"b""#);
    assert_eq!(f64::NAN.rescript_value(), None);
}

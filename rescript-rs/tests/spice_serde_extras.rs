#![cfg(feature = "spice")]

//! serde's `other`, `alias` and variant `untagged`, written as
//! `@spice.other`, `@spice.alias(…)` and `@spice.untagged`, and
//! `#[rescript(fallback = "path")]`, written as `@spice.fallback(…)`.

use rescript_rs::{
    spice::{DecodeError, Failure, Module, Plain, Spice},
    ReScript,
};
use serde_json::{json, Value};

#[derive(ReScript, Debug, Clone, Copy, PartialEq)]
#[rescript(codec, module = "Extras", name = "shadow")]
#[serde(rename_all = "UPPERCASE")]
pub enum Shadow {
    #[serde(other)]
    None,
    Regular,
    Deep,
}

#[derive(ReScript, Debug, Clone, Copy, PartialEq)]
#[rescript(codec, legacy, module = "Extras", name = "scope")]
#[serde(rename_all = "UPPERCASE")]
pub enum Scope {
    #[serde(alias = "Public")]
    Public,
    #[serde(alias = "OrgListed", alias = "ORG_LISTED")]
    Org,
    #[serde(alias = "Invite")]
    Private,
}

#[derive(ReScript, Debug, Clone, PartialEq)]
#[rescript(codec, module = "Extras", name = "codec")]
pub enum Codec {
    #[serde(rename = "prores")]
    ProRes,
    #[serde(untagged)]
    Other(String),
}

#[derive(ReScript, Debug, Clone, PartialEq)]
#[rescript(codec, module = "Extras", name = "shape")]
#[serde(tag = "type")]
pub enum Shape {
    Circle {
        radius: f64,
    },
    #[serde(other)]
    Unknown,
}

#[derive(ReScript, Debug, Clone, PartialEq)]
#[rescript(codec, module = "Extras", name = "color")]
pub struct Color {
    pub r: i32,
    pub g: i32,
    pub b: i32,
}

fn white() -> Background {
    Background::Solid(Color {
        r: 255,
        g: 255,
        b: 255,
    })
}

#[derive(ReScript, Debug, Clone, PartialEq)]
#[rescript(codec, module = "Extras", name = "background", fallback = "white")]
pub enum Background {
    Solid(Color),
    Image(String),
}

#[derive(ReScript, Debug, Clone, PartialEq)]
#[rescript(codec, module = "Extras", name = "mixed")]
pub enum Mixed {
    Circle(f64),
    Rect {
        w: f64,
    },
    #[serde(other)]
    UnknownMixed,
}

#[derive(ReScript, Debug, Clone, PartialEq)]
#[rescript(codec, legacy, module = "Extras", name = "legacyOther")]
pub enum LegacyOther {
    Known,
    #[serde(other)]
    UnknownLegacy,
}

fn failure(path: &str, message: &str, value: Value) -> Failure {
    Failure::Decode(DecodeError {
        path: path.into(),
        message: message.into(),
        value,
    })
}

/// As serde: `other` is a unit variant, so an unknown name keyed to a
/// value and a bare name of a variant with data don't decode to it.
#[test]
fn other_is_a_unit_variant() {
    assert_eq!(decode(json!({"Square": null})), Some(Mixed::UnknownMixed));
    assert_eq!(decode(json!("Square")), Some(Mixed::UnknownMixed));
    assert_eq!(
        Mixed::decode::<Plain>(&json!({"Square": 5})).unwrap_err(),
        failure(".Square", "Expected null", json!(5))
    );
    assert_eq!(
        Mixed::decode::<Plain>(&json!("Rect")).unwrap_err(),
        failure("", "Invalid variant constructor", json!("Rect"))
    );
    assert_eq!(decode(json!(["Zzz"])), Some(LegacyOther::UnknownLegacy));
    assert_eq!(
        LegacyOther::decode::<Plain>(&json!(["Zzz", 1])).unwrap_err(),
        failure(
            "",
            "Invalid number of arguments to variant constructor",
            json!(["Zzz", 1])
        )
    );
}

#[test]
fn declarations() {
    let rendered = Module::new("Extras")
        .with::<Shadow>()
        .with::<Scope>()
        .with::<Codec>()
        .with::<Shape>()
        .with::<Color>()
        .with::<Background>()
        .render();

    assert_eq!(
        rendered,
        r#"@spice.serde
type shadow =
  | @as("NONE") @spice.other None
  | @as("REGULAR") Regular
  | @as("DEEP") Deep

@spice.serde({legacy: true})
type scope =
  | @as("PUBLIC") @spice.alias("Public") Public
  | @as("ORG") @spice.alias("OrgListed") @spice.alias("ORG_LISTED") Org
  | @as("PRIVATE") @spice.alias("Invite") Private

@spice.serde
type codec =
  | @as("prores") ProRes
  | @spice.untagged Other(string)

@spice.serde @tag("type")
type shape =
  | Circle({radius: float})
  | @spice.other Unknown

@spice
type color = {
  r: int,
  g: int,
  b: int,
}

@spice.serde @spice.fallback(Solid(({r: 255, g: 255, b: 255}: color)))
type background =
  | Solid(color)
  | Image(string)
"#
    );
}

fn decode<T: Spice>(json: Value) -> Option<T> {
    T::decode::<Plain>(&json).ok()
}

#[test]
fn other() {
    assert_eq!(decode(json!("DEEP")), Some(Shadow::Deep));
    assert_eq!(decode(json!("GLOW")), Some(Shadow::None));
    assert_eq!(decode(json!({"GLOW": null})), Some(Shadow::None));
    assert_eq!(decode::<Shadow>(json!(1)), None);
    assert_eq!(Shadow::None.encode::<Plain>(), json!("NONE"));
    assert_eq!(decode(json!({"type": "Square"})), Some(Shape::Unknown));
    assert_eq!(
        decode(json!({"type": "Circle", "radius": 1})),
        Some(Shape::Circle { radius: 1. })
    );
}

#[test]
fn alias() {
    assert_eq!(decode(json!("ORG")), Some(Scope::Org));
    assert_eq!(decode(json!("OrgListed")), Some(Scope::Org));
    assert_eq!(decode(json!("ORG_LISTED")), Some(Scope::Org));
    assert_eq!(decode(json!({"Invite": null})), Some(Scope::Private));
    assert_eq!(decode(json!(["Invite"])), Some(Scope::Private));
    assert_eq!(decode(json!(["Public"])), Some(Scope::Public));
    assert_eq!(decode(json!(["ORG_LISTED"])), Some(Scope::Org));
    assert_eq!(decode::<Scope>(json!("Nope")), None);
    assert_eq!(Scope::Org.encode::<Plain>(), json!("ORG"));
}

#[test]
fn untagged() {
    assert_eq!(decode(json!("prores")), Some(Codec::ProRes));
    assert_eq!(decode(json!("h264")), Some(Codec::Other("h264".into())));
    assert_eq!(decode::<Codec>(json!({"Other": "x"})), None);
    assert_eq!(
        Codec::decode::<Plain>(&json!(1)).unwrap_err(),
        Failure::Decode(DecodeError {
            path: "".into(),
            message: "Not a variant".into(),
            value: json!(1),
        })
    );
    assert_eq!(Codec::Other("h264".into()).encode::<Plain>(), json!("h264"));
    assert_eq!(Codec::ProRes.encode::<Plain>(), json!("prores"));
}

#[test]
fn fallback() {
    assert_eq!(
        decode(json!({"Image": "u"})),
        Some(Background::Image("u".into()))
    );
    assert_eq!(decode(json!({"Ambient": null})), Some(white()));
    assert_eq!(decode(json!(1)), Some(white()));
}

#[test]
fn serde_agrees() {
    #[derive(serde::Deserialize, Debug, PartialEq)]
    enum Codec {
        #[serde(rename = "prores")]
        ProRes,
        #[serde(untagged)]
        Other(String),
    }

    #[derive(serde::Deserialize, Debug, PartialEq)]
    #[serde(rename_all = "UPPERCASE")]
    enum Shadow {
        Regular,
        #[serde(other)]
        None,
    }

    let codec: Codec = serde_json::from_value(json!("h264")).unwrap();
    assert_eq!(codec, Codec::Other("h264".into()));
    let shadow: Shadow = serde_json::from_value(json!("GLOW")).unwrap();
    assert_eq!(shadow, Shadow::None);
}

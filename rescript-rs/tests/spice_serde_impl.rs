#![cfg(feature = "spice")]

//! `#[rescript(codec, serde)]`: serde reads and writes the JSON the codec
//! does, so a codec type can be a field of a serde type.

use rescript_rs::{
    spice::{Plain, Spice},
    ReScript,
};
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(ReScript, Debug, Clone, PartialEq)]
#[rescript(codec)]
#[serde(rename_all = "camelCase")]
pub struct Tint {
    pub alpha: f32,
    #[serde(default)]
    pub keep_hue: bool,
}

#[derive(ReScript, Debug, Clone, PartialEq)]
#[rescript(codec, legacy_write, serde, tag = "type")]
pub enum Accent {
    None,
    Tint(Tint),
    #[serde(other)]
    Unknown,
}

#[derive(ReScript, Debug, Clone, PartialEq)]
#[rescript(codec, serde)]
pub struct Size<T> {
    pub width: T,
    pub height: T,
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
struct Layer {
    accent: Accent,
    size: Size<f32>,
}

fn tint() -> Accent {
    Accent::Tint(Tint {
        alpha: 0.1,
        keep_hue: false,
    })
}

#[test]
fn reads_every_form_the_codec_reads() {
    let read = |json: serde_json::Value| serde_json::from_value::<Accent>(json).unwrap();

    assert_eq!(read(json!("None")), Accent::None);
    assert_eq!(read(json!(["None"])), Accent::None);
    assert_eq!(read(json!({"type": "None"})), Accent::None);
    assert_eq!(read(json!(["Tint", {"alpha": 0.1}])), tint());
    assert_eq!(read(json!({"type": "Tint", "alpha": 0.1})), tint());
    assert_eq!(read(json!({"type": "Glow", "radius": 2})), Accent::Unknown);
}

#[test]
fn writes_what_the_codec_writes() {
    let layer = Layer {
        accent: tint(),
        size: Size {
            width: 0.1,
            height: 1080.,
        },
    };
    let written = serde_json::to_value(&layer).unwrap();

    assert_eq!(
        written,
        json!({"accent": ["Tint", {"alpha": 0.1, "keepHue": false}], "size": {"width": 0.1, "height": 1080}})
    );
    assert_eq!(serde_json::from_value::<Layer>(written).unwrap(), layer);
    assert_eq!(
        serde_json::to_value(&layer.accent).unwrap(),
        layer.accent.encode::<Plain>()
    );
}

#[test]
fn a_rejected_value_is_a_serde_error_at_its_path() {
    let error = serde_json::from_value::<Layer>(json!({
        "accent": ["Tint", {"alpha": "opaque"}],
        "size": {"width": 1, "height": 1},
    }))
    .unwrap_err();

    assert_eq!(error.to_string(), "Not a number at `[1].alpha`");
}

#[test]
fn f32_is_written_as_its_shortest_decimal() {
    assert_eq!(0.1f32.encode::<Plain>(), json!(0.1));
    assert_eq!(1e-7f32.encode::<Plain>(), json!(1e-7));
    assert_eq!(16777216f32.encode::<Plain>(), json!(16777216));
    assert_eq!(f32::decode::<Plain>(&json!(0.1)), Ok(0.1));
    assert_eq!(f32::decode::<Plain>(&json!(3)), Ok(3.));
    assert_eq!(0.1f32.rescript_value().as_deref(), Some("0.1"));
    assert_eq!(<Size<f32>>::rescript_type(), "size<float>");
}

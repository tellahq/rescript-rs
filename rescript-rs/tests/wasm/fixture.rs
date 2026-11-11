use rescript_rs::{
    spice::{Dict, Value},
    ReScript,
};
use serde::{Deserialize, Serialize};

#[derive(ReScript, Serialize, Deserialize, Debug, Clone, PartialEq)]
#[rescript(codec, wasm, module = "Geometry")]
#[serde(rename_all = "camelCase")]
pub struct Size {
    pub width: f64,
    pub height: f64,
    #[rescript(optional)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(ReScript, Serialize, Deserialize, Debug, Clone, PartialEq)]
#[rescript(codec, wasm, module = "Geometry")]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Fit {
    Unbounded,
    Within { size: Size },
}

/// The size scaled to fit.
/// A size that already fits is returned as is.
#[rescript_rs::wasm(js_name = "geometryFitted")]
pub fn fitted(size: &Size, fit: Fit, labels: &[String], name: &str) -> Size {
    let scale = match fit {
        Fit::Unbounded => 1.0,
        Fit::Within { size: bounds } => (bounds.width / size.width)
            .min(bounds.height / size.height)
            .min(1.0),
    };

    Size {
        width: size.width * scale,
        height: size.height * scale,
        label: (!name.is_empty()).then(|| format!("{name}{}", labels.join(""))),
    }
}

#[rescript_rs::wasm]
pub fn square_root(value: f64) -> Result<f64, String> {
    if value < 0.0 {
        Err(format!("{value} has no square root"))
    } else {
        Ok(value.sqrt())
    }
}

/// The scores as JSON, under the given names.
#[rescript_rs::wasm(codec)]
pub fn scored(names: Option<Vec<String>>, scores: &Dict<Option<f64>>) -> Value {
    let names = names.unwrap_or_default();
    let scores = scores
        .0
        .iter()
        .filter(|(name, _)| names.contains(name))
        .map(|(name, score)| (name.clone(), (*score).into()));

    Value::Object(scores.collect())
}

#[rescript_rs::wasm]
pub fn unit_size() -> Size {
    Size {
        width: 1.0,
        height: 1.0,
        label: None,
    }
}

#[derive(ReScript, Serialize, Deserialize, Debug, Clone, PartialEq)]
#[rescript(codec, wasm, module = "Geometry")]
pub struct Metadata {
    #[serde(rename = "content-type")]
    pub content_type: String,
    #[rescript(optional)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visible: Option<bool>,
}

#[derive(ReScript, Serialize, Deserialize, Debug, Clone, PartialEq)]
#[rescript(codec, wasm, module = "Geometry")]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Shape {
    Empty,
    Placed {
        metadata: Metadata,
        sizes: Vec<Size>,
    },
}

#[rescript_rs::wasm]
pub fn echo_shape(shape: Shape) -> Shape {
    shape
}

#[rescript_rs::wasm]
pub fn echo_optional(value: Option<String>) -> Option<String> {
    value
}

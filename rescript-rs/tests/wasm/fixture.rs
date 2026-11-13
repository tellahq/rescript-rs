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

/// A standalone option crosses into Rust as it is (serde reads `undefined`
/// as `None`), and the string that comes back needs no decoding.
#[rescript_rs::wasm]
pub fn label_or(label: Option<String>, fallback: &str) -> String {
    label.unwrap_or_else(|| fallback.into())
}

/// A payload-free enum is a string on both sides: serde's default name, or
/// the renamed one. The names serde only reads (an alias, any other name)
/// leave both as they are.
#[derive(ReScript, Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
#[rescript(codec, wasm, module = "Geometry")]
pub enum Format {
    #[serde(alias = "hardback")]
    Hardcover,
    Paperback,
    #[serde(rename = "e-book")]
    Ebook,
    #[serde(other)]
    Unlisted,
}

#[rescript_rs::wasm]
pub fn echo_format(format: Format) -> Format {
    format
}

/// serde writes an absent `controls` as `null`, which ReScript's `None`
/// (`undefined`) is not: only a curve going into Rust crosses as it is.
#[derive(ReScript, Serialize, Deserialize, Debug, Clone, PartialEq)]
#[rescript(codec, wasm, module = "Geometry")]
#[serde(rename_all = "camelCase")]
pub struct Curve {
    pub name: String,
    pub controls: Option<Vec<f64>>,
}

#[rescript_rs::wasm]
pub fn echo_curve(curve: Curve) -> Curve {
    curve
}

/// An option of a scalar crosses as wasm-bindgen passes it: `None` is
/// `undefined` in both directions, as ReScript's is.
#[rescript_rs::wasm]
pub fn echo_number(number: Option<f64>) -> Option<f64> {
    number
}

#[rescript_rs::wasm]
pub fn echo_count(count: Option<i32>) -> Option<i32> {
    count
}

#[rescript_rs::wasm]
pub fn echo_flag(flag: Option<bool>) -> Option<bool> {
    flag
}

/// Sizes added up as they come.
pub struct Tally {
    label: String,
    sizes: Vec<Size>,
}

/// Sizes added up as they come.
#[rescript_rs::wasm(js_name = "GeometryTally")]
impl Tally {
    /// A tally of `sizes` under `label`, which is not empty.
    pub fn new(sizes: &[Size], label: &str) -> Result<Self, String> {
        if label.is_empty() {
            return Err("a tally needs a label".into());
        }

        Ok(Self {
            label: label.into(),
            sizes: sizes.to_vec(),
        })
    }

    /// `self::Size` is a path from this module: the export has to read
    /// it from here too.
    pub fn add(&mut self, size: self::Size) {
        self.sizes.push(size);
    }

    /// The area of every size, times `scale` squared.
    pub fn area(&self, scale: f64) -> f64 {
        self.sizes
            .iter()
            .map(|size| size.width * size.height)
            .sum::<f64>()
            * scale
            * scale
    }

    pub fn count(&self) -> i32 {
        self.sizes.len() as i32
    }

    pub fn is_empty(&self) -> bool {
        self.sizes.is_empty()
    }

    pub fn label(&self) -> String {
        self.label.clone()
    }

    pub fn largest(&self) -> Option<Size> {
        self.sizes
            .iter()
            .max_by(|a, b| (a.width * a.height).total_cmp(&(b.width * b.height)))
            .cloned()
    }

    pub fn scaled(&self, scale: f64) -> Vec<Size> {
        self.sizes
            .iter()
            .map(|size| Size {
                width: size.width * scale,
                height: size.height * scale,
                label: Some(self.label.clone()),
            })
            .collect()
    }

    /// Not bound: only pub functions are.
    fn total_width(&self) -> f64 {
        self.sizes.iter().map(|size| size.width).sum()
    }

    pub fn widest(&self) -> Result<f64, String> {
        match self.total_width() {
            0.0 => Err("no sizes".into()),
            _ => Ok(self.sizes.iter().map(|size| size.width).fold(0.0, f64::max)),
        }
    }
}

/// `f32`, `u32` and a tuple have the same layout on both sides.
#[derive(ReScript, Serialize, Deserialize, Debug, Clone, PartialEq)]
#[rescript(codec, wasm, module = "Geometry")]
#[serde(rename_all = "camelCase")]
pub struct Reading {
    pub level: f32,
    pub order: u32,
    pub at: (i32, f64),
}

/// The readings of `sizes`, one per size, levelled by `scale`; answered
/// once the call has waited for nothing.
#[rescript_rs::wasm]
pub async fn readings(sizes: Vec<Size>, scale: f32) -> Result<Vec<Reading>, String> {
    std::future::ready(()).await;

    if scale <= 0.0 {
        return Err(format!("{scale} is no scale"));
    }

    Ok(sizes
        .iter()
        .enumerate()
        .map(|(index, size)| Reading {
            level: size.width as f32 * scale,
            order: index as u32,
            at: (index as i32, size.height),
        })
        .collect())
}

/// Each format with its position, and `None` past the formats' end: serde
/// writes that `None` as `null` in the tuple, so a `Positions` is decoded.
/// (A tuple that needs a codec must be inside a named type: ppx-spice has
/// no codec for a bare one.)
#[derive(ReScript, Serialize, Deserialize, Debug, Clone, PartialEq)]
#[rescript(codec, wasm, module = "Geometry")]
pub struct Positions {
    pub positions: Vec<(i32, Option<Format>)>,
}

#[rescript_rs::wasm]
pub async fn positions(formats: Vec<Format>, count: i32) -> Positions {
    Positions {
        positions: (0..count)
            .map(|index| (index, formats.get(index as usize).copied()))
            .collect(),
    }
}

#![cfg(feature = "spice")]

use rescript_rs::{
    spice::{Dict, Module, Value},
    ReScript,
};

#[derive(ReScript, Debug, Clone, PartialEq)]
#[rescript(codec, serde, module = "Geometry")]
#[serde(rename_all = "camelCase")]
pub struct Size {
    pub width: f64,
    pub height: f64,
    #[rescript(optional)]
    pub label: Option<String>,
}

#[derive(ReScript, Debug, Clone, PartialEq)]
#[rescript(codec, serde, runtime_names, module = "Geometry", tag = "type")]
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
#[rescript_rs::wasm]
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

/// The binding is laid out as `rescript format` leaves it, and encodes
/// each argument and decodes the result with the codecs ppx-spice picks for
/// its type.
#[test]
fn a_function_is_bound_with_its_own_types() {
    let rendered = Module::new("Geometry")
        .with::<Size>()
        .function("./wasm/geometry.js", fitted_rescript())
        .function("./wasm/geometry.js", square_root_rescript())
        .function("./wasm/geometry.js", scored_rescript())
        .function("./wasm/geometry.js", unit_size_rescript())
        .render();
    let functions = rendered.split_once("\n\n// The size scaled").unwrap().1;

    assert_eq!(
        format!("// The size scaled{functions}"),
        r#"// The size scaled to fit.
// A size that already fits is returned as is.
module Fitted = {
  @module("./wasm/geometry.js") @variadic
  external wasm: array<JSON.t> => JSON.t = "geometryFitted"

  let call = (arg0, arg1, arg2, arg3) => {
    let arg0 = size_encode(arg0)
    let arg1 = fit_encode(arg1)
    let arg2 = Spice.arrayToJson(Spice.stringToJson, arg2)
    let arg3 = Spice.stringToJson(arg3)

    wasm([arg0, arg1, arg2, arg3])->size_decode
  }
}
let fitted = Fitted.call

module SquareRoot = {
  @module("./wasm/geometry.js") @variadic
  external wasm: array<JSON.t> => JSON.t = "squareRoot"

  let call = arg0 => {
    let arg0 = Spice.floatToJson(arg0)

    wasm([arg0])->Spice.floatFromJson
  }
}
let squareRoot = SquareRoot.call

// The scores as JSON, under the given names.
module Scored = {
  @module("./wasm/geometry.js") @variadic
  external wasm: array<JSON.t> => JSON.t = "scored"

  let call = (arg0, arg1) => {
    let arg0 = Spice.optionToNullableJson(Spice.arrayToJson(Spice.stringToJson, ...), arg0)
    let arg1 = Spice.dictOptionalToJson(Spice.optionToJson(Spice.floatToJson, ...), arg1)

    Ok(wasm([arg0, arg1]))
  }
}
let scored = Scored.call

module UnitSize = {
  @module("./wasm/geometry.js") @variadic
  external wasm: array<JSON.t> => JSON.t = "unitSize"

  let call = () => wasm([])->size_decode
}
let unitSize = UnitSize.call
"#
    );
}

/// The exports, called as the generated JavaScript glue calls them.
#[cfg(all(target_arch = "wasm32", feature = "wasm-bindgen"))]
mod exports {
    use js_sys::{Array, Object, Reflect, JSON};
    use rescript_rs::wasm::JsValue;
    use wasm_bindgen_test::wasm_bindgen_test;

    use super::*;

    fn js(json: &str) -> JsValue {
        JSON::parse(json).unwrap()
    }

    fn json(value: &JsValue) -> String {
        JSON::stringify(value).unwrap().into()
    }

    #[wasm_bindgen_test]
    fn arguments_and_the_result_cross_as_their_spice_json() {
        let size = js(r#"{"width": 400, "height": 100}"#);
        // What JavaScript holds and JSON does not: an undefined property
        // is absent.
        Reflect::set(&size, &"label".into(), &JsValue::UNDEFINED).unwrap();

        let fitted = fitted_wasm(
            size,
            js(r#"{"type": "within", "size": {"width": 100, "height": 100}}"#),
            js(r#"["a", "b"]"#),
            "n".into(),
        )
        .unwrap();

        assert_eq!(json(&fitted), r#"{"height":25,"label":"nab","width":100}"#);
        assert!(Object::get_prototype_of(&fitted) == Object::get_prototype_of(&Object::new()));
        assert_eq!(
            json(&unit_size_wasm().unwrap()),
            r#"{"height":1,"width":1}"#
        );
        assert_eq!(square_root_wasm(9.0.into()).unwrap().as_f64(), Some(3.0));
    }

    #[wasm_bindgen_test]
    fn a_refused_argument_and_an_err_are_thrown() {
        let thrown = |result: Result<JsValue, JsValue>| result.unwrap_err().as_string().unwrap();
        let unbounded = || js(r#"{"type": "unbounded"}"#);
        let none = || JsValue::from(Array::new());

        assert_eq!(
            thrown(fitted_wasm(
                js(r#"{"width": "wide", "height": 1}"#),
                unbounded(),
                none(),
                "".into()
            )),
            "argument 0.width: Not a number"
        );
        assert_eq!(
            thrown(fitted_wasm(
                js(r#"{"width": 1, "height": 1}"#),
                unbounded(),
                none(),
                js_sys::BigInt::from(1).into()
            )),
            "argument 3: not a JSON value"
        );

        let cyclic = Array::new();
        cyclic.push(&cyclic);

        assert_eq!(
            thrown(fitted_wasm(
                js(r#"{"width": 1, "height": 1}"#),
                unbounded(),
                cyclic.into(),
                "".into()
            )),
            "argument 2: nested too deep"
        );
        assert_eq!(
            thrown(square_root_wasm((-1.0).into())),
            "-1 has no square root"
        );
    }
}

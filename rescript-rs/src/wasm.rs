//! Calling Rust from ReScript through wasm-bindgen with the same types on
//! both sides.
//!
//! `#[rescript_rs::wasm]` on a function whose parameters and return type
//! are [`Spice`] types exports it from the wasm module and describes it to
//! [`Module::function`](crate::spice::Module::function), which writes the
//! ReScript binding: a function of the same parameters that returns
//! `result<output, Spice.decodeError>`. Neither side writes a `JsValue` or
//! a `JSON.t`.
//!
//! ```ignore
//! #[rescript_rs::wasm]
//! pub fn area(size: &Size, scale: f64) -> f64 { … }
//!
//! Module::new("Geometry")
//!     .with::<Size>()
//!     .function("./geometry_wasm.js", area_rescript())
//! ```
//!
//! writes, in `Geometry.res`, `let area: (size, float) => result<float, _>`.
//!
//! A value crosses as the JavaScript value of its spice JSON: the objects,
//! arrays, strings and numbers `JSON.parse` would give, built directly. The
//! ReScript side encodes and decodes it with the codecs ppx-spice
//! generates, so a value is read the same way on both sides. An argument
//! the Rust codec refuses, an `Err` the function returns and a panic are
//! thrown as JavaScript exceptions.
//!
//! The export only exists with the `wasm-bindgen` feature, which only the
//! crate that builds the wasm module needs to turn on. A function in a
//! library crate is exported once the wasm crate re-exports it (`pub use`),
//! so the linker keeps it.

use js_sys::{Array, Object, Reflect};
pub use wasm_bindgen::{self, JsValue};

use crate::spice::{js_number, Failure, Map, Plain, Spice, Value};

/// Deeper values than any spice type nests: a cyclic object ends here.
const MAX_DEPTH: usize = 128;

/// `value` as the JavaScript value of its spice JSON.
pub fn to_js<T: Spice>(value: &T) -> JsValue {
    js_of(&value.encode::<Plain>())
}

/// The argument at `position` (from 0), read as its ReScript encoder wrote
/// it. The error is the message the export throws.
pub fn from_js<T: Spice>(value: &JsValue, position: usize) -> Result<T, JsValue> {
    let refused = |path: &str, message: &str| {
        JsValue::from_str(&format!("argument {position}{path}: {message}"))
    };
    let json = value_of(value, 0).map_err(|message| refused("", message))?;

    T::decode::<Plain>(&json).map_err(|failure| match failure {
        Failure::Decode(error) => refused(&error.path, &error.message),
        Failure::Throw(message) => refused("", &message),
    })
}

/// An `Err` a function returned, as the exception its export throws.
pub fn error(error: &dyn std::fmt::Display) -> JsValue {
    JsValue::from_str(&error.to_string())
}

fn js_of(value: &Value) -> JsValue {
    match value {
        Value::Null => JsValue::NULL,
        Value::Bool(value) => JsValue::from_bool(*value),
        Value::Number(number) => JsValue::from_f64(number.as_f64().unwrap_or(f64::NAN)),
        Value::String(text) => JsValue::from_str(text),
        Value::Array(items) => items.iter().map(js_of).collect::<Array>().into(),
        Value::Object(fields) => {
            let object = Object::new();

            for (key, value) in fields {
                // Setting a property of a plain object cannot fail.
                let _ = Reflect::set(&object, &JsValue::from_str(key), &js_of(value));
            }

            object.into()
        }
    }
}

/// What `JSON.parse(JSON.stringify(value))` holds: an `undefined` property
/// is left out, `undefined` anywhere else and a non-finite number are
/// `null`. Unlike `JSON.stringify`, a `bigint`, a function and a symbol
/// are refused instead of dropped, and `toJSON` is not called.
fn value_of(value: &JsValue, depth: usize) -> Result<Value, &'static str> {
    if value.is_null() || value.is_undefined() {
        return Ok(Value::Null);
    }
    if let Some(value) = value.as_bool() {
        return Ok(Value::Bool(value));
    }
    if let Some(number) = value.as_f64() {
        return Ok(js_number(number));
    }
    if let Some(text) = value.as_string() {
        return Ok(Value::String(text));
    }
    if !value.is_object() {
        return Err("not a JSON value");
    }
    if depth == MAX_DEPTH {
        return Err("nested too deep");
    }
    if Array::is_array(value) {
        return Array::from(value)
            .iter()
            .map(|item| value_of(&item, depth + 1))
            .collect::<Result<_, _>>()
            .map(Value::Array);
    }

    let mut fields = Map::new();

    for entry in Object::entries(&Object::from(value.clone())) {
        let entry = Array::from(&entry);
        let field = entry.get(1);

        if !field.is_undefined() {
            let key = entry.get(0).as_string().ok_or("not a JSON value")?;

            fields.insert(key, value_of(&field, depth + 1)?);
        }
    }

    Ok(Value::Object(fields))
}

//! Native serde transport for generated ReScript WASM bindings.
//!
//! `#[rescript_rs::wasm]` uses `serde-wasm-bindgen` directly. Its types
//! derive native serde and `#[rescript(codec, wasm)]`; the generator aligns
//! runtime keys, constructor names and tagged unit variants, then omits JS
//! codecs wherever the layout matches. No intermediate JSON tree is built
//! by this transport. Native serde owns validation and error messages.
//!
//! `#[rescript_rs::wasm(codec)]` explicitly retains the old Spice bridge for
//! legacy/custom codecs. It uses the JSON-tree helpers below, including
//! their JSON normalization rules. Native serde is not a drop-in replacement
//! for that contract (notably f32, non-finite floats, defaults and tuples).

use js_sys::{Array, Object, Reflect};
pub use wasm_bindgen::{self, JsValue};
pub use wasm_bindgen_futures;

mod layout;

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

/// Native serde transport: no intermediate `serde_json::Value` tree, unless
/// the type reads as JSON ([`Wasm::READ_AS_JSON`](crate::spice::Wasm)).
pub fn from_serde<T: crate::spice::Wasm>(value: &JsValue, position: usize) -> Result<T, JsValue> {
    let refused =
        |error: &dyn std::fmt::Display| JsValue::from_str(&format!("argument {position}: {error}"));

    if T::READ_AS_JSON {
        let json = value_of(value, 0).map_err(|message| refused(&message))?;

        return serde_json::from_value(json).map_err(|error| refused(&error));
    }

    serde_wasm_bindgen::from_value(value.clone()).map_err(|error| refused(&error))
}

/// An `f32` is written as the `f64` its shortest decimal reads as. A value
/// that reaches ReScript as it is has its `None`s written as `undefined`,
/// ReScript's own; one a Spice decoder reads keeps JSON's `null`.
pub fn to_serde<T: crate::spice::Wasm>(value: &T) -> Result<JsValue, JsValue> {
    value
        .serialize(layout::Layout {
            inner: &serde_wasm_bindgen::Serializer::json_compatible(),
            undefined: crosses_as_it_is::<T>(),
        })
        .map_err(|error| JsValue::from_str(&error.to_string()))
}

/// `T::output_identity()`, worked out once per type: it walks the type's
/// fields, and an export may be called every frame.
fn crosses_as_it_is<T: crate::spice::Wasm>() -> bool {
    use std::{any::TypeId, cell::RefCell, collections::HashMap};

    thread_local! {
        static KNOWN: RefCell<HashMap<TypeId, bool>> = RefCell::new(HashMap::new());
    }

    KNOWN.with(|known| {
        *known
            .borrow_mut()
            .entry(TypeId::of::<T>())
            .or_insert_with(T::output_identity)
    })
}

pub fn codec_result<T: Spice>(value: &T) -> Result<JsValue, JsValue> {
    Ok(to_js(value))
}

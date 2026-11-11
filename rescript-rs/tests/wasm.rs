#![cfg(feature = "spice")]

#[path = "wasm/fixture.rs"]
pub mod fixture;
use fixture::*;
use rescript_rs::spice::Module;

/// Binding layouts are inferred from the same types as the exports.
#[test]
fn a_function_is_bound_with_its_own_types() {
    use rescript_rs::spice::{BoundaryType, Wasm};

    let function = fitted_rescript();
    assert_eq!(
        function.args,
        vec![
            BoundaryType {
                ty: "Geometry.size".into(),
                identity: true
            },
            BoundaryType {
                ty: "Geometry.fit".into(),
                identity: true
            },
            BoundaryType {
                ty: "array<string>".into(),
                identity: true
            },
            BoundaryType {
                ty: "string".into(),
                identity: true
            },
        ]
    );
    assert_eq!(
        function.output,
        BoundaryType {
            ty: "Geometry.size".into(),
            identity: true
        }
    );
    assert!(!<Option<String>>::output_identity());
    assert!(!scored_rescript().args[1].identity);
}

#[test]
fn export_wasm_binding() {
    if let Ok(path) = std::env::var("WASM_TEST_BINDING") {
        std::fs::write(
            path,
            Module::new("Geometry")
                .with::<Size>()
                .with::<Fit>()
                .with::<Metadata>()
                .with::<Shape>()
                .function("./geometry.js", fitted_rescript())
                .function("./geometry.js", square_root_rescript())
                .function("./geometry.js", scored_rescript())
                .function("./geometry.js", unit_size_rescript())
                .function("./geometry.js", echo_shape_rescript())
                .function("./geometry.js", echo_optional_rescript())
                .render(),
        )
        .unwrap();
    }
}

/// The exports, called as the generated JavaScript glue calls them.
#[cfg(all(target_arch = "wasm32", feature = "wasm-bindgen"))]
mod exports {
    use js_sys::{Array, Object, Reflect, JSON};
    use rescript_rs::{spice::Value, wasm::JsValue};
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

        assert_eq!(
            serde_json::from_str::<Value>(&json(&fitted)).unwrap(),
            serde_json::json!({"width":100,"height":25,"label":"nab"})
        );
        assert!(Object::get_prototype_of(&fitted) == Object::get_prototype_of(&Object::new()));
        assert_eq!(
            serde_json::from_str::<Value>(&json(&unit_size_wasm().unwrap())).unwrap(),
            serde_json::json!({"width":1,"height":1})
        );
        assert_eq!(square_root_wasm(9.0.into()).unwrap().as_f64(), Some(3.0));
    }

    #[wasm_bindgen_test]
    fn a_refused_argument_and_an_err_are_thrown() {
        let thrown = |result: Result<JsValue, JsValue>| result.unwrap_err().as_string().unwrap();
        let unbounded = || js(r#"{"type": "unbounded"}"#);
        let none = || JsValue::from(Array::new());

        assert!(fitted_wasm(
            js(r#"{"width": "wide", "height": 1}"#),
            unbounded(),
            none(),
            "".into()
        )
        .is_err());
        assert!(fitted_wasm(
            js(r#"{"width": 1, "height": 1}"#),
            unbounded(),
            none(),
            js_sys::BigInt::from(1).into()
        )
        .is_err());

        assert_eq!(
            thrown(square_root_wasm((-1.0).into())),
            "-1 has no square root"
        );
    }
}

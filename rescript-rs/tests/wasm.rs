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

/// A payload-free enum needs neither a tag nor `poly`: an ordinary variant
/// is the string serde writes, under the same name.
#[test]
fn a_string_enum_is_an_ordinary_variant() {
    use rescript_rs::spice::{BoundaryType, Declare};

    assert_eq!(
        Format::declaration(),
        "// A payload-free enum is a string on both sides: serde's default name, or\n// the renamed one.\n@spice.serde\ntype format =\n  | Hardcover\n  | Paperback\n  | @as(\"e-book\") Ebook"
    );
    assert_eq!(
        serde_json::to_string(&[Format::Hardcover, Format::Ebook]).unwrap(),
        r#"["Hardcover","e-book"]"#
    );

    let boundary = BoundaryType {
        ty: "Geometry.format".into(),
        identity: true,
    };
    let function = echo_format_rescript();

    assert_eq!(function.args, vec![boundary.clone()]);
    assert_eq!(function.output, boundary);
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
                .with::<Format>()
                .function("./geometry.js", fitted_rescript())
                .function("./geometry.js", square_root_rescript())
                .function("./geometry.js", scored_rescript())
                .function("./geometry.js", unit_size_rescript())
                .function("./geometry.js", echo_shape_rescript())
                .function("./geometry.js", echo_optional_rescript())
                .function("./geometry.js", echo_format_rescript())
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
    fn a_string_enum_crosses_as_its_name() {
        let echoed = |name: &str| echo_format_wasm(name.into()).unwrap().as_string();

        assert_eq!(echoed("Paperback").as_deref(), Some("Paperback"));
        assert_eq!(echoed("e-book").as_deref(), Some("e-book"));
        assert!(echo_format_wasm("Ebook".into()).is_err());
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

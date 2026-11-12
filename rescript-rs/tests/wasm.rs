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

/// A layout crosses without a codec in each direction it is equal in:
/// ReScript's None is `undefined`, which serde reads as None, but serde
/// writes a field it does not omit as `null`.
#[test]
fn an_option_field_serde_writes_as_null_crosses_only_into_rust() {
    use rescript_rs::spice::{BoundaryType, Wasm};

    assert!(Curve::input_identity());
    assert!(!Curve::output_identity());

    let function = echo_curve_rescript();

    assert_eq!(
        (function.args, function.output),
        (
            vec![BoundaryType {
                ty: "Geometry.curve".into(),
                identity: true
            }],
            BoundaryType {
                ty: "Geometry.curve".into(),
                identity: false
            }
        )
    );
}

/// The constructor is the pub function returning Self, the methods the pub
/// ones taking self; scalars and `()` cross as wasm-bindgen passes them.
#[test]
fn a_class_is_bound_with_its_constructor_and_methods() {
    let class = tally_rescript();
    fn layout(
        function: &rescript_rs::spice::Function,
    ) -> (&'static str, &'static str, Vec<(&str, bool)>, &str, bool) {
        let args: Vec<_> = function
            .args
            .iter()
            .map(|arg| (arg.ty.as_str(), arg.identity))
            .collect();

        (
            function.name,
            function.js_name,
            args,
            function.output.ty.as_str(),
            function.output.identity,
        )
    }

    assert_eq!((class.name, class.js_name), ("Tally", "GeometryTally"));
    assert_eq!(
        layout(&class.constructor),
        (
            "make",
            "GeometryTally",
            vec![("array<Geometry.size>", true), ("string", true)],
            "",
            true
        )
    );
    assert_eq!(
        class.methods.iter().map(layout).collect::<Vec<_>>(),
        vec![
            ("add", "add", vec![("Geometry.size", true)], "unit", true),
            ("area", "area", vec![("float", true)], "float", true),
            ("count", "count", vec![], "int", true),
            ("isEmpty", "isEmpty", vec![], "bool", true),
            ("label", "label", vec![], "string", true),
            ("largest", "largest", vec![], "option<Geometry.size>", false),
            (
                "scaled",
                "scaled",
                vec![("float", true)],
                "array<Geometry.size>",
                true
            ),
            ("widest", "widest", vec![], "float", true),
        ]
    );
}

/// Inside `module Tally = { type t … }`, the module's own `t` would be the
/// object's.
#[test]
#[should_panic(expected = "names Geometry.t")]
fn a_class_cannot_take_its_module_s_own_t() {
    let mut class = tally_rescript();
    class.methods[0].args[0].ty = "Geometry.t".into();

    Module::new("Geometry").class("./geometry.js", class);
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
                .with::<Curve>()
                .function("./geometry.js", echo_curve_rescript())
                .class("./geometry.js", tally_rescript())
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
        assert_eq!(square_root_wasm(9.0), Ok(3.0));
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
            square_root_wasm(-1.0).unwrap_err().as_string().as_deref(),
            Some("-1 has no square root")
        );
        assert_eq!(
            thrown(TallyWasm::new(none(), "".into()).map(|_| JsValue::NULL)),
            "a tally needs a label"
        );
    }

    #[wasm_bindgen_test]
    fn an_undefined_option_field_reads_as_none() {
        let curve = js(r#"{"name": "ease"}"#);
        Reflect::set(&curve, &"controls".into(), &JsValue::UNDEFINED).unwrap();

        assert_eq!(
            serde_json::from_str::<Value>(&json(&echo_curve_wasm(curve).unwrap())).unwrap(),
            serde_json::json!({"name": "ease", "controls": null})
        );
    }

    #[wasm_bindgen_test]
    fn a_class_is_constructed_and_called() {
        let mut tally =
            TallyWasm::new(js(r#"[{"width": 2, "height": 3}]"#), "box".into()).unwrap();

        assert_eq!(tally.add(js(r#"{"width": 1, "height": 1}"#)), Ok(()));
        assert_eq!(tally.area(2.0), Ok(28.0));
        assert_eq!(tally.count(), Ok(2));
        assert_eq!(tally.is_empty(), Ok(false));
        assert_eq!(tally.widest(), Ok(2.0));
        assert_eq!(tally.label().unwrap().as_string().as_deref(), Some("box"));
        assert_eq!(
            serde_json::from_str::<Value>(&json(&tally.scaled(2.0).unwrap())).unwrap(),
            serde_json::json!([
                {"width": 4, "height": 6, "label": "box"},
                {"width": 2, "height": 2, "label": "box"}
            ])
        );
        assert_eq!(
            serde_json::from_str::<Value>(&json(&tally.largest().unwrap())).unwrap(),
            serde_json::json!({"width": 2, "height": 3})
        );
        assert!(tally.add(js(r#"{"width": 1}"#)).is_err());
    }
}

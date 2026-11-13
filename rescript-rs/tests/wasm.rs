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

/// A binding is a `result` only where it decodes what comes back, which is
/// all that can fail: with no codec it is the external itself.
#[test]
fn only_a_decoded_result_is_a_result() {
    assert_eq!(
        Module::new("Geometry")
            .function("./geometry.js", unit_size_rescript())
            .function("./geometry.js", label_or_rescript())
            .function("./geometry.js", echo_optional_rescript())
            .render(),
        "@module(\"./geometry.js\")
external unitSize: unit => size = \"unitSize\"

// A standalone option crosses into Rust as it is (serde reads `undefined`
// as `None`), and the string that comes back needs no decoding.
@module(\"./geometry.js\")
external labelOr: (option<string>, string) => string = \"labelOr\"

module EchoOptional = {
  @module(\"./geometry.js\")
  external wasm: option<string> => JSON.t = \"echoOptional\"

  let call = arg0 => wasm(arg0)->Spice.optionFromJson(Spice.stringFromJson, _)
}
let echoOptional = EchoOptional.call
"
    );
}

/// A payload-free enum needs neither a tag nor `poly`: an ordinary variant
/// is the string serde writes, under the same name.
#[test]
fn a_string_enum_is_an_ordinary_variant() {
    use rescript_rs::spice::{BoundaryType, Declare};

    assert_eq!(
        Format::declaration(),
        "// A payload-free enum is a string on both sides: serde's default name, or\n// the renamed one. The names serde only reads (an alias, any other name)\n// leave both as they are.\n@spice.serde\ntype format =\n  | @spice.alias(\"hardback\") Hardcover\n  | Paperback\n  | @as(\"e-book\") Ebook\n  | @spice.other Unlisted"
    );
    assert_eq!(
        serde_json::to_string(&[Format::Hardcover, Format::Ebook, Format::Unlisted]).unwrap(),
        r#"["Hardcover","e-book","Unlisted"]"#
    );
    assert_eq!(
        serde_json::from_str::<Vec<Format>>(r#"["hardback","pamphlet"]"#).unwrap(),
        [Format::Hardcover, Format::Unlisted]
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

/// wasm-bindgen's `None` is `undefined` in both directions, as ReScript's
/// is: an option of a scalar needs no codec either way.
#[test]
fn an_option_of_a_scalar_crosses_as_it_is() {
    use rescript_rs::spice::BoundaryType;

    let layouts = [
        (echo_number_rescript(), "option<float>"),
        (echo_count_rescript(), "option<int>"),
        (echo_flag_rescript(), "option<bool>"),
    ];

    for (function, ty) in layouts {
        let boundary = BoundaryType {
            ty: ty.into(),
            identity: true,
        };

        assert_eq!(function.args, vec![boundary.clone()]);
        assert_eq!(function.output, boundary);
    }
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
            "t",
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

/// An async function is described as one; `f32`, `u32` and tuples cross
/// without a codec, and a tuple holding an `Option` is decoded coming back.
#[test]
fn an_async_function_and_f32_u32_tuple_layouts_are_bound() {
    use rescript_rs::spice::{BoundaryType, Wasm};

    let boundary = |ty: &str, identity| BoundaryType {
        ty: ty.into(),
        identity,
    };
    let readings = readings_rescript();
    let positions = positions_rescript();

    assert_eq!(
        (readings.asynchronous, readings.args, readings.output),
        (
            true,
            vec![
                boundary("array<Geometry.size>", true),
                boundary("float", true)
            ],
            boundary("array<Geometry.reading>", true)
        )
    );
    assert_eq!(
        (positions.asynchronous, positions.output),
        (true, boundary("Geometry.positions", false))
    );
    assert!(Positions::input_identity());
    assert!(!fitted_rescript().asynchronous);
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
                .function("./geometry.js", label_or_rescript())
                .function("./geometry.js", echo_format_rescript())
                .with::<Curve>()
                .function("./geometry.js", echo_curve_rescript())
                .function("./geometry.js", echo_number_rescript())
                .function("./geometry.js", echo_count_rescript())
                .function("./geometry.js", echo_flag_rescript())
                .class("./geometry.js", tally_rescript())
                .with::<Reading>()
                .function("./geometry.js", readings_rescript())
                .with::<Positions>()
                .function("./geometry.js", positions_rescript())
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
        assert_eq!(echo_number_wasm(Some(0.0)), Ok(Some(0.0)));
        assert_eq!(echo_count_wasm(None), Ok(None));
        assert_eq!(echo_flag_wasm(Some(false)), Ok(Some(false)));
    }

    #[wasm_bindgen_test]
    fn a_string_enum_crosses_as_its_name() {
        let echoed = |name: &str| echo_format_wasm(name.into()).unwrap().as_string();

        assert_eq!(echoed("Paperback").as_deref(), Some("Paperback"));
        assert_eq!(echoed("e-book").as_deref(), Some("e-book"));
        assert_eq!(echoed("Ebook").as_deref(), Some("Unlisted"));
        assert!(echo_format_wasm(1.into()).is_err());
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

    /// An `f32` reaches JavaScript as its shortest decimal, as JSON text
    /// of it would: `0.1`, not `0.10000000149011612`.
    #[wasm_bindgen_test]
    async fn an_async_function_answers_its_value_with_f32_as_its_shortest_decimal() {
        let answered = readings_wasm(
            js(r#"[{"width": 0.1, "height": 2}, {"width": 3, "height": 4}]"#),
            1.0.into(),
        )
        .await
        .unwrap();

        assert_eq!(
            serde_json::from_str::<Value>(&json(&answered)).unwrap(),
            serde_json::json!([
                {"level": 0.1, "order": 0, "at": [0, 2]},
                {"level": 3, "order": 1, "at": [1, 4]}
            ])
        );
        assert_eq!(
            readings_wasm(js("[]"), 0.0.into())
                .await
                .unwrap_err()
                .as_string()
                .as_deref(),
            Some("0 is no scale")
        );
        assert_eq!(
            serde_json::from_str::<Value>(&json(
                &positions_wasm(js(r#"["Paperback"]"#), 2).await.unwrap()
            ))
            .unwrap(),
            serde_json::json!({"positions": [[0, "Paperback"], [1, null]]})
        );
    }

    /// An f32 reaches JavaScript as the number JSON text of it parses to,
    /// and a JavaScript number reaches Rust as the f32 serde reads from
    /// `JSON.stringify` of it: over every exponent, subnormals included.
    #[wasm_bindgen_test]
    fn an_f32_crosses_as_its_json_text_does() {
        use rescript_rs::wasm::{from_serde, to_serde};

        let mut state = 0x2545_f491_u32;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state
        };

        for _ in 0..20_000 {
            let value = f32::from_bits(next());
            if !value.is_finite() {
                continue;
            }
            let text = serde_json::to_string(&value).unwrap();
            let returned = to_serde(&value).unwrap().as_f64().unwrap();

            assert_eq!(
                returned.to_bits(),
                JSON::parse(&text).unwrap().as_f64().unwrap().to_bits(),
                "{text}"
            );

            let number = f64::from(value) * 1.000_000_1;
            let read: f32 = from_serde(&JsValue::from(number), 0).unwrap();
            let expected: f32 = serde_json::from_str(&json(&JsValue::from(number))).unwrap();

            assert_eq!(read.to_bits(), expected.to_bits(), "{number}");
        }
    }

    #[wasm_bindgen_test]
    fn a_negative_int_is_no_u32() {
        assert_eq!(echo_order_wasm(7.into()).unwrap().as_f64(), Some(7.0));
        assert!(echo_order_wasm((-1).into()).is_err());
        assert!(echo_order_wasm(1.5.into()).is_err());
    }

    /// A type read as JSON reads what `JSON.stringify` of the value holds.
    #[wasm_bindgen_test]
    fn a_type_read_as_json_reads_its_json() {
        let shelf = js(r#"{"type": "Shelf", "count": 2}"#);
        Reflect::set(&shelf, &"extra".into(), &JsValue::UNDEFINED).unwrap();

        assert_eq!(
            serde_json::from_str::<Value>(&json(&echo_stored_wasm(shelf).unwrap())).unwrap(),
            serde_json::json!({"type": "Shelf", "count": 2})
        );

        let not_a_count = js(r#"{"type": "Shelf"}"#);
        Reflect::set(&not_a_count, &"count".into(), &JsValue::from_f64(f64::NAN)).unwrap();

        assert!(echo_stored_wasm(not_a_count).is_err());
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
        let mut tally = TallyWasm::new(js(r#"[{"width": 2, "height": 3}]"#), "box".into()).unwrap();

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

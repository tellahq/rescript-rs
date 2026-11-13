//! Rust codecs that read and write JSON exactly as
//! [ppx-spice](https://github.com/jfrolich/ppx_spice) does in ReScript.
//!
//! `#[derive(ReScript)]` with `#[rescript(codec)]` on a struct or enum
//! gives it a [`Spice`] impl and a ReScript declaration with spice
//! annotations ([`Declare`]). ppx-spice
//! then generates the ReScript side's `t_decode` and `t_encode` from that
//! declaration, so a type defined once in Rust is read and written the same
//! way on both sides: same accepted inputs, same error paths and messages,
//! same output.
//!
//! JavaScript has values JSON cannot hold (`Infinity` from `JSON.parse`, a
//! `bigint` or `Set` from a DynamoDB client). A [`Host`] says how such values
//! stand in a [`serde_json::Value`]; [`Plain`] is plain JSON.

use std::borrow::Cow;

pub use serde_json::{self, Map, Number, Value};

use crate::export::format::format;

/// `Spice.decodeError`.
#[derive(Debug, Clone, PartialEq)]
pub struct DecodeError {
    pub path: String,
    pub message: String,
    pub value: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Failure {
    /// A decoder's `Error(decodeError)`.
    Decode(DecodeError),
    /// An exception a hand-written ReScript codec throws, which no spice
    /// decoder catches (except an omittable field's decoder given `null`).
    Throw(String),
}

impl Failure {
    pub fn error(message: impl Into<String>, value: &Value) -> Self {
        Failure::Decode(DecodeError {
            path: String::new(),
            message: message.into(),
            value: value.clone(),
        })
    }

    /// The error with `prefix` in front of its path.
    pub fn prefixed(self, prefix: &str) -> Self {
        match self {
            Failure::Decode(DecodeError {
                path,
                message,
                value,
            }) => Failure::Decode(DecodeError {
                path: format!("{prefix}{path}"),
                message,
                value,
            }),
            throw => throw,
        }
    }
}

pub type Result<T> = std::result::Result<T, Failure>;

/// What a JavaScript value is to spice's `switch (json: JSON.t)`.
pub enum View<'a> {
    Null,
    Bool(bool),
    Number(f64),
    String(&'a str),
    /// `Array.isArray`.
    Array(Cow<'a, [Value]>),
    /// Any other `typeof … === "object"`: its own enumerable properties in
    /// JavaScript's order (see [`js_order`]).
    Object(Cow<'a, Map<String, Value>>),
    /// `undefined`, a `bigint`, a function: no JSON case matches.
    Other,
}

/// How JavaScript values stand in a `serde_json::Value`.
pub trait Host {
    fn view(value: &Value) -> View<'_>;

    /// A JavaScript number as a `Value`.
    fn number(value: f64) -> Value;
}

/// Plain JSON: every value is what it says. Numbers print as JavaScript
/// prints them, and a non-finite number (which JSON cannot hold) is `null`.
pub struct Plain;

impl Host for Plain {
    fn view(value: &Value) -> View<'_> {
        match value {
            Value::Null => View::Null,
            Value::Bool(b) => View::Bool(*b),
            Value::Number(n) => View::Number(n.as_f64().unwrap_or(f64::NAN)),
            Value::String(s) => View::String(s),
            Value::Array(items) => View::Array(Cow::Borrowed(items)),
            Value::Object(map) => View::Object(in_js_order(map)),
        }
    }

    fn number(value: f64) -> Value {
        js_number(value)
    }
}

/// A finite number as `JSON.stringify` prints it: integral values without
/// a fraction, `-0` as `0`. Anything else is `null`.
pub fn js_number(value: f64) -> Value {
    if value.fract() == 0.0 && value.abs() < 9_007_199_254_740_992.0 {
        return Value::Number(Number::from(value as i64));
    }

    Number::from_f64(value).map_or(Value::Null, Value::Number)
}

/// The key as an array index (a canonical integer below 2^32 - 1), which
/// JavaScript lists before every other key, in ascending order.
pub fn array_index(key: &str) -> Option<u32> {
    let index: u32 = key.parse().ok()?;

    (index != u32::MAX && index.to_string() == key).then_some(index)
}

/// The entries in the order JavaScript gives an object's own keys: array
/// indices ascending, then the rest in insertion order.
pub fn js_order(map: Map<String, Value>) -> Map<String, Value> {
    if !map.keys().any(|key| array_index(key).is_some()) {
        return map;
    }

    let (mut indices, rest): (Vec<_>, Vec<_>) = map
        .into_iter()
        .partition(|(key, _)| array_index(key).is_some());

    indices.sort_by_key(|(key, _)| array_index(key));
    indices.into_iter().chain(rest).collect()
}

/// [`js_order`] without copying a map that is already in that order.
pub fn in_js_order(map: &Map<String, Value>) -> Cow<'_, Map<String, Value>> {
    if map.keys().any(|key| array_index(key).is_some()) {
        Cow::Owned(js_order(map.clone()))
    } else {
        Cow::Borrowed(map)
    }
}

/// A type read and written as ppx-spice reads and writes its ReScript twin.
pub trait Spice: Sized + Clone {
    /// The ReScript type expression, e.g. `array<string>` or `DB__Types.t`.
    fn rescript_type() -> String;

    /// Example values, for tests that compare this codec with ppx-spice's:
    /// for a record, one with every field's first sample and one more per
    /// other sample of a field; for a variant, every constructor.
    fn samples() -> Vec<Self> {
        vec![]
    }

    /// `t_decode`.
    fn decode<H: Host>(json: &Value) -> Result<Self>;

    /// `t_encode`.
    fn encode<H: Host>(&self) -> Value;

    /// The value as a ReScript expression, e.g. `Some(SubtitleGrouping.Chunked)`,
    /// with constructors and record types qualified by their module: what
    /// `@spice.default(…)` is written with for a `#[serde(default)]` field.
    /// `None` when the type has no literal form.
    fn rescript_value(&self) -> Option<String> {
        None
    }
}

/// A ReScript string literal.
pub fn rescript_string(text: &str) -> String {
    serde_json::to_string(text).expect("a string serializes")
}

/// A ReScript float literal, `None` for a value that has none.
pub fn rescript_float(value: f64) -> Option<String> {
    if !value.is_finite() {
        return None;
    }

    let text = value.to_string();

    Some(if text.contains(['.', 'e']) {
        text
    } else {
        format!("{text}.")
    })
}

/// The module part of a qualified ReScript type (`Module.t` gives
/// `Module.`), which qualifies its constructors and record literals.
pub fn rescript_scope(rescript_type: &str) -> String {
    rescript_type
        .rsplit_once('.')
        .map_or_else(String::new, |(module, _)| format!("{module}."))
}

/// `samples` of a type, or none while that type's samples are already
/// being made: a recursive type (`children: Vec<Self>`) samples its
/// recursive fields with an empty array, `None` and so on.
pub fn samples_once<T: 'static>(samples: impl FnOnce() -> Vec<T>) -> Vec<T> {
    use std::{any::TypeId, cell::RefCell, collections::HashSet};

    thread_local! {
        static SAMPLING: RefCell<HashSet<TypeId>> = RefCell::new(HashSet::new());
    }

    let id = TypeId::of::<T>();

    if !SAMPLING.with(|sampling| sampling.borrow_mut().insert(id)) {
        return vec![];
    }

    struct Done(TypeId);

    impl Drop for Done {
        fn drop(&mut self) {
            SAMPLING.with(|sampling| sampling.borrow_mut().remove(&self.0));
        }
    }

    let _done = Done(id);

    samples()
}

/// A ReScript type declaration generated by `#[rescript(codec)]`.
pub trait Declare {
    /// The ReScript module (file name without `.res`) the type lives in.
    fn module() -> &'static str;

    /// `@spice\ntype name = …`, with types from other modules qualified.
    fn declaration() -> String;
}

/// A ReScript module's generated source: the declarations and nested
/// modules in the order they were added, separated by blank lines.
/// References to the module's own types (and its nested modules' types)
/// lose their `Module.` qualifier.
pub struct Module<H: Host = Plain> {
    name: &'static str,
    header: String,
    items: Vec<Item<H>>,
    host: std::marker::PhantomData<H>,
}

enum Item<H: Host> {
    Declaration(String, Codec),
    Module(Module<H>),
    Function(String),
}

/// A native serde type whose ReScript runtime layout is known to the generator.
/// Implemented by `#[rescript(codec, wasm)]` alongside ordinary serde derives.
///
/// A present JSON null must not silently become an absent optional field:
/// ```compile_fail
/// use rescript_rs::{ReScript, spice::Wasm};
/// #[derive(Clone, serde::Serialize, serde::Deserialize, ReScript)]
/// #[rescript(codec, wasm)]
/// struct Invalid {
///     #[serde(skip_serializing_if = "Option::is_none")]
///     value: Option<serde_json::Value>,
/// }
/// let _ = Invalid::input_identity();
/// ```
pub trait Wasm: Spice + 'static + ::serde::Serialize + ::serde::de::DeserializeOwned {
    /// Whether a present value can itself serialize as null. Such a value
    /// cannot be an optional field: serde would read it back as None.
    const NULLABLE: bool = false;
    fn input_identity() -> bool {
        false
    }
    fn output_identity() -> bool {
        false
    }
}

/// Recursive layouts conservatively retain their JS codecs.
pub fn identity_once<T: 'static>(check: impl FnOnce() -> bool) -> bool {
    use std::{any::TypeId, cell::RefCell, collections::HashSet};
    thread_local! { static VISITING: RefCell<HashSet<TypeId>> = RefCell::new(HashSet::new()); }
    let id = TypeId::of::<T>();
    if !VISITING.with(|set| set.borrow_mut().insert(id)) {
        return false;
    }
    let result = check();
    VISITING.with(|set| set.borrow_mut().remove(&id));
    result
}

macro_rules! wasm_scalar {
    ($($ty:ty),*) => {$(
        impl Wasm for $ty {
            fn input_identity() -> bool { true }
            fn output_identity() -> bool { true }
        }
    )*};
}
wasm_scalar!(String, bool, i32, u32, f64);
/// Written as the `f64` its shortest decimal reads as (`0.1`, not
/// `0.10000000149011612`), as JSON text of it would be.
impl Wasm for f32 {
    fn input_identity() -> bool {
        true
    }
    fn output_identity() -> bool {
        true
    }
}
impl Wasm for Value {
    const NULLABLE: bool = true;
    fn input_identity() -> bool {
        true
    }
    fn output_identity() -> bool {
        true
    }
}
impl Wasm for () {
    const NULLABLE: bool = true;
}
impl<T: Wasm> Wasm for Vec<T> {
    fn input_identity() -> bool {
        T::input_identity()
    }
    fn output_identity() -> bool {
        T::output_identity()
    }
}
/// Going into Rust, ReScript's `None` is `undefined`, which serde reads as
/// `None`, unless the value inside can itself be null. Coming back, serde
/// writes `null`.
impl<T: Wasm> Wasm for Option<T> {
    const NULLABLE: bool = true;
    fn input_identity() -> bool {
        !T::NULLABLE && T::input_identity()
    }
}
impl<T: Wasm> Wasm for Box<T> {
    const NULLABLE: bool = T::NULLABLE;
    fn input_identity() -> bool {
        T::input_identity()
    }
    fn output_identity() -> bool {
        T::output_identity()
    }
}

/// A boundary value's declared type and whether it needs a JS codec.
#[derive(Debug, Clone, PartialEq)]
pub struct BoundaryType {
    pub ty: String,
    pub identity: bool,
}

/// A Rust function ReScript calls through wasm-bindgen.
#[derive(Debug, Clone, PartialEq)]
pub struct Function {
    pub name: &'static str,
    pub js_name: &'static str,
    pub docs: &'static [&'static str],
    pub args: Vec<BoundaryType>,
    pub output: BoundaryType,
    /// An `async fn`: the export answers a `promise` of its output.
    pub asynchronous: bool,
}

impl Function {
    fn render(&self, js_module: &str) -> String {
        let external = format!("@module({})", rescript_string(js_module));

        binding(self, &external, false)
    }
}

/// A Rust type ReScript holds as an opaque wasm-bindgen object: its
/// constructor and its methods.
///
/// The class has its own `free`, which releases the object. A method of
/// that name would replace it in JavaScript:
/// ```compile_fail
/// pub struct Pool;
///
/// #[rescript_rs::wasm]
/// impl Pool {
///     pub fn new() -> Self {
///         Pool
///     }
///
///     pub fn free(&mut self) {}
/// }
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct Class {
    /// The ReScript module the object's type `t` is declared in.
    pub name: &'static str,
    /// The JavaScript class wasm-bindgen exports.
    pub js_name: &'static str,
    pub docs: &'static [&'static str],
    /// `make`, which answers a new object, `t`.
    pub constructor: Function,
    /// Called on an object, which is not among their `args`.
    pub methods: Vec<Function>,
}

impl Class {
    fn render(&self, js_module: &str) -> String {
        let new = format!("@module({}) @new", rescript_string(js_module));
        let mut members = vec![binding(&self.constructor, &new, false)];

        for method in &self.methods {
            members.push(binding(method, "@send", true));
        }
        members.push("@send external free: t => unit = \"free\"\n".into());

        let mut out: String = self
            .docs
            .iter()
            .map(|line| format!("// {line}\n"))
            .collect();

        out.push_str(&format!("module {} = {{\n  type t\n", self.name));

        for member in members {
            out.push('\n');

            for line in member.lines() {
                if !line.is_empty() {
                    out.push_str("  ");
                }
                out.push_str(line);
                out.push('\n');
            }
        }
        out.push_str("}\n");
        out
    }

    /// Every type a member takes or answers.
    fn types(&self) -> impl Iterator<Item = &str> {
        let members = std::iter::once(&self.constructor).chain(&self.methods);

        members
            .flat_map(|member| member.args.iter().chain(Some(&member.output)))
            .map(|boundary| boundary.ty.as_str())
    }
}

/// `function` as ReScript calls it. Where every runtime layout equals
/// serde's, that is its typed external. Otherwise it is a module holding
/// the external, `wasm`, and `call`, which encodes the arguments and
/// decodes what comes back. Only decoding can fail, so only a decoded
/// result is a `result`. A method's external also takes the object, `t`.
/// An async function answers a `promise` of all that.
fn binding(function: &Function, external: &str, method: bool) -> String {
    let args: Vec<String> = (0..function.args.len())
        .map(|i| format!("arg{i}"))
        .collect();
    let ty = |value: &BoundaryType| {
        if value.identity {
            value.ty.clone()
        } else {
            "JSON.t".into()
        }
    };
    let types: Vec<String> = method
        .then(|| "t".to_owned())
        .into_iter()
        .chain(function.args.iter().map(ty))
        .collect();
    let promised = |ty: String| match function.asynchronous {
        true => format!("promise<{ty}>"),
        false => ty,
    };
    let signature = format!(
        "{} => {} = {}",
        match types.as_slice() {
            [] => "unit".into(),
            [one] => one.clone(),
            many => format!("({})", many.join(", ")),
        },
        promised(ty(&function.output)),
        rescript_string(function.js_name),
    );
    let mut out: String = function
        .docs
        .iter()
        .map(|line| format!("// {line}\n"))
        .collect();

    if function.output.identity && function.args.iter().all(|arg| arg.identity) {
        out.push_str(&format!(
            "{external}\nexternal {}: {signature}\n",
            function.name
        ));
        return out;
    }

    let mut module = function.name.to_owned();
    module[..1].make_ascii_uppercase();
    let passed: Vec<String> = method
        .then(|| "self".to_owned())
        .into_iter()
        .chain(args.iter().cloned())
        .collect();
    let parameters = match passed.as_slice() {
        [] => "()".into(),
        [one] => one.clone(),
        _ => format!("({})", passed.join(", ")),
    };
    let converted: Vec<String> = function
        .args
        .iter()
        .zip(&args)
        .filter_map(|(ty, arg)| {
            (!ty.identity)
                .then(|| format!("    let {arg} = {}\n", ValueCodec::of(&ty.ty).encode(arg)))
        })
        .collect();
    let call = format!("wasm({})", passed.join(", "));
    let call = match (function.output.identity, function.asynchronous) {
        (true, _) => call,
        (false, false) => ValueCodec::of(&function.output.ty).decode(&call),
        (false, true) => format!(
            "{call}->Promise.thenResolve(json => {})",
            ValueCodec::of(&function.output.ty).decode("json")
        ),
    };

    out.push_str(&format!(
        "module {module} = {{\n  {external}\n  external wasm: {signature}\n\n"
    ));

    if converted.is_empty() {
        out.push_str(&format!("  let call = {parameters} => {call}\n"));
    } else {
        out.push_str(&format!(
            "  let call = {parameters} => {{\n{}\n    {call}\n  }}\n",
            converted.concat()
        ));
    }
    out.push_str(&format!("}}\nlet {} = {module}.call\n", function.name));
    out
}

/// The codecs ppx-spice reads and writes a value of a ReScript type
/// expression with, as `codecs.ml` picks them for an array item: a
/// built-in type's from `Spice`, which take their type arguments' codecs
/// and the value in one call, and any other type's own `t_encode` and
/// `t_decode`, which take them one at a time.
#[derive(Debug, PartialEq)]
enum ValueCodec {
    Spice {
        encode: &'static str,
        decode: &'static str,
        args: Vec<ValueCodec>,
    },
    Named {
        name: String,
        args: Vec<ValueCodec>,
    },
    /// `JSON.t`, which is its own encoding.
    Json,
}

impl ValueCodec {
    fn of(rescript_type: &str) -> Self {
        let mut tokens = TypeTokens(rescript_type).peekable();
        let codec = Self::parse(&mut tokens, rescript_type);

        if let Some(token) = tokens.next() {
            panic!("unexpected `{token}` in the ReScript type `{rescript_type}`");
        }

        codec
    }

    fn parse(tokens: &mut std::iter::Peekable<TypeTokens<'_>>, source: &str) -> Self {
        let name = match tokens.next() {
            Some(name) if name.starts_with(|c: char| c.is_alphabetic() || c == '_') => name,
            Some(token) => panic!(
                "a function called from ReScript cannot take or return `{source}`: \
                 ppx-spice has no named codec for `{token}`, so give the type a name"
            ),
            None => panic!("the ReScript type `{source}` ends early"),
        };
        let mut args = vec![];

        if tokens.peek() == Some(&"<") {
            tokens.next();

            loop {
                args.push(Self::parse(tokens, source));

                match tokens.next() {
                    Some(",") => {}
                    Some(">") => break,
                    _ => panic!("unclosed `<` in the ReScript type `{source}`"),
                }
            }
        }

        let (encode, decode) = match name {
            "string" => ("stringToJson", "stringFromJson"),
            "int" => ("intToJson", "intFromJson"),
            "bigint" => ("bigintToJson", "bigintFromJson"),
            "float" => ("floatToJson", "floatFromJson"),
            "bool" => ("boolToJson", "boolFromJson"),
            "unit" => ("unitToJson", "unitFromJson"),
            "array" => ("arrayToJson", "arrayFromJson"),
            "list" => ("listToJson", "listFromJson"),
            "option" => ("optionToNullableJson", "optionFromJson"),
            "Js.null" | "Js.Null.t" | "Null.t" => ("nullToJson", "nullFromJson"),
            "result" | "Result.t" | "Belt.Result.t" => ("resultToJson", "resultFromJson"),
            "dict" | "Dict.t" | "Js.Dict.t" => return Self::dict(args),
            "JSON.t" | "Js.Json.t" => return Self::Json,
            _ => {
                return Self::Named {
                    name: name.to_owned(),
                    args,
                }
            }
        };

        Self::Spice {
            encode,
            decode,
            args,
        }
    }

    /// A dict of optional values leaves out the `None`s.
    fn dict(mut args: Vec<Self>) -> Self {
        match args.as_mut_slice() {
            [Self::Spice {
                encode: "optionToNullableJson",
                args: values,
                ..
            }] => Self::Spice {
                encode: "dictOptionalToJson",
                decode: "dictFromJson",
                args: vec![Self::Spice {
                    encode: "optionToJson",
                    decode: "optionFromJson",
                    args: std::mem::take(values),
                }],
            },
            _ => Self::Spice {
                encode: "dictToJson",
                decode: "dictFromJson",
                args,
            },
        }
    }

    /// The expression that encodes `value`.
    fn encode(&self, value: &str) -> String {
        self.apply(true, value)
    }

    /// The expression that decodes `json`.
    fn decode(&self, json: &str) -> String {
        match self {
            Self::Spice { args, .. } if !args.is_empty() => {
                format!("{json}->{}", self.apply(false, "_"))
            }
            Self::Spice { .. } => format!("{json}->{}", self.name(false)),
            Self::Named { args, .. } if args.is_empty() => format!("{json}->{}", self.name(false)),
            _ => self.apply(false, json),
        }
    }

    fn apply(&self, encode: bool, value: &str) -> String {
        match self {
            Self::Spice { args, .. } => {
                let mut args = Self::functions(args, encode);

                args.push(value.to_owned());
                format!("{}({})", self.name(encode), args.join(", "))
            }
            Self::Named { .. } => format!("{}({value})", self.function(encode)),
            Self::Json if encode => value.to_owned(),
            Self::Json => format!("Ok({value})"),
        }
    }

    /// The codec as a function of the value.
    fn function(&self, encode: bool) -> String {
        match self {
            Self::Spice { args, .. } if args.is_empty() => self.name(encode),
            Self::Spice { .. } => self.apply(encode, "..."),
            Self::Named { args, .. } => {
                let args: String = Self::functions(args, encode)
                    .iter()
                    .map(|arg| format!("({arg})"))
                    .collect();

                format!("{}{args}", self.name(encode))
            }
            Self::Json if encode => "v => v".into(),
            Self::Json => "v => Ok(v)".into(),
        }
    }

    fn functions(codecs: &[Self], encode: bool) -> Vec<String> {
        codecs.iter().map(|codec| codec.function(encode)).collect()
    }

    fn name(&self, encode: bool) -> String {
        match (self, encode) {
            (Self::Spice { encode, .. }, true) => format!("Spice.{encode}"),
            (Self::Spice { decode, .. }, false) => format!("Spice.{decode}"),
            (Self::Named { name, .. }, true) => format!("{name}_encode"),
            (Self::Named { name, .. }, false) => format!("{name}_decode"),
            (Self::Json, _) => unreachable!("JSON.t has no codec"),
        }
    }
}

/// The names and punctuation of a ReScript type expression.
struct TypeTokens<'a>(&'a str);

impl<'a> Iterator for TypeTokens<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<&'a str> {
        let rest = self.0.trim_start();
        let first = rest.chars().next()?;
        let len = if first.is_alphanumeric() || first == '_' {
            rest.find(|c: char| !(c.is_alphanumeric() || c == '_' || c == '.' || c == '\''))
                .unwrap_or(rest.len())
        } else {
            first.len_utf8()
        };
        let (token, rest) = rest.split_at(len);

        self.0 = rest;
        Some(token)
    }
}

/// A type's `encode(decode(json))`, for testing a type's Rust codec
/// against the ReScript one ppx-spice generates.
pub type Roundtrip = fn(&Value) -> Result<Value>;

/// A declared type and its [`Roundtrip`].
#[derive(Clone)]
pub struct Codec {
    /// The qualified ReScript type, e.g. `DB__Types.Row.t`.
    pub rescript_type: String,
    pub roundtrip: Roundtrip,
    /// The type's [`Spice::samples`], encoded.
    pub samples: fn() -> Vec<Value>,
}

fn encoded_samples<H: Host, T: Spice>() -> Vec<Value> {
    T::samples().iter().map(T::encode::<H>).collect()
}

fn roundtrip<H: Host, T: Spice>(json: &Value) -> Result<Value> {
    T::decode::<H>(json).map(|value| value.encode::<H>())
}

impl Module<Plain> {
    /// `name` is the file name without `.res`; a nested module's is
    /// `Parent.Child`.
    pub fn new(name: &'static str) -> Self {
        Self::hosted(name)
    }
}

impl<H: Host> Module<H> {
    /// A module whose [`Codec`]s read and write values as `H` sees them.
    pub fn hosted(name: &'static str) -> Self {
        Self {
            name,
            header: String::new(),
            items: vec![],
            host: std::marker::PhantomData,
        }
    }

    /// Text written above the declarations, e.g. a "generated" note.
    pub fn header(mut self, header: impl Into<String>) -> Self {
        self.header = header.into();
        self
    }

    pub fn with<T: Declare + Spice>(mut self) -> Self {
        assert_eq!(
            T::module(),
            self.name,
            "{} is declared in another module",
            std::any::type_name::<T>(),
        );

        self.items.push(Item::Declaration(
            T::declaration(),
            Codec {
                rescript_type: T::rescript_type(),
                roundtrip: roundtrip::<H, T>,
                samples: encoded_samples::<H, T>,
            },
        ));
        self
    }

    /// A function exported from the wasm-bindgen module at `js_module`
    /// (as the generated file imports it, e.g. `./wasm/app.js`), called as
    /// `name(arg, …)`. Where every runtime layout equals serde's it is the
    /// typed external. Otherwise it encodes those arguments, calls the
    /// export and decodes what comes back; a decoded result is a
    /// `result<output, Spice.decodeError>`, any other the value itself. See
    /// `rescript_rs::wasm`.
    pub fn function(mut self, js_module: &str, function: Function) -> Self {
        self.items.push(Item::Function(function.render(js_module)));
        self
    }

    /// A class exported from the wasm-bindgen module at `js_module`, as
    /// `module Name = { type t … }`: `make` constructs an object, each
    /// method takes it first (`clock->Name.outputMs(ms)`), and `free`
    /// releases it. Each member is bound as a function is: an external
    /// where no codec runs, a `result` only where its result is decoded.
    pub fn class(mut self, js_module: &str, class: Class) -> Self {
        let mut scope = Some(self.name);

        while let Some(module) = scope {
            let own = format!("{module}.t");

            for ty in class.types() {
                assert!(
                    !TypeTokens(ty).any(|token| token == own),
                    "{} names {own}, which is `t` inside module {} = {{ … }}: declare it under another name",
                    class.name,
                    class.name,
                );
            }
            scope = module.rsplit_once('.').map(|(parent, _)| parent);
        }

        self.items.push(Item::Function(class.render(js_module)));
        self
    }

    /// `module Child = { … }`, for a module named `Parent.Child`.
    pub fn nested(mut self, module: Module<H>) -> Self {
        assert_eq!(
            module.name.rsplit_once('.').map(|(parent, _)| parent),
            Some(self.name),
            "{} is not a module of {}",
            module.name,
            self.name,
        );

        self.items.push(Item::Module(module));
        self
    }

    pub fn name(&self) -> &'static str {
        self.name
    }

    /// Every type declared in the module and its nested modules.
    pub fn codecs(&self) -> Vec<Codec> {
        self.items
            .iter()
            .flat_map(|item| match item {
                Item::Declaration(_, codec) => vec![codec.clone()],
                Item::Module(module) => module.codecs(),
                Item::Function(_) => vec![],
            })
            .collect()
    }

    pub fn render(&self) -> String {
        let mut out = self.header.clone();

        out.push_str(&self.body(self.name));
        out
    }

    /// The items, unqualified for `root` (the file's module).
    fn body(&self, root: &str) -> String {
        let mut out = String::new();

        for (i, item) in self.items.iter().enumerate() {
            if i > 0 {
                out.push('\n');
            }

            match item {
                Item::Declaration(text, _) => {
                    let depth = self.name.matches('.').count() - root.matches('.').count();

                    out.push_str(&format(&unqualify_all(text, self.name, root), depth));
                    out.push('\n');
                }
                Item::Function(text) => out.push_str(&unqualify_all(text, self.name, root)),
                Item::Module(module) => {
                    let child = module.name.rsplit_once('.').map_or(module.name, |(_, c)| c);
                    let body = module.body(root);

                    out.push_str(&format!("module {child} = {{\n"));

                    for line in body.lines() {
                        if !line.is_empty() {
                            out.push_str("  ");
                        }

                        out.push_str(line);
                        out.push('\n');
                    }

                    out.push_str("}\n");
                }
            }
        }

        out
    }
}

/// Drops the qualifiers that are not needed inside `module`: those of the
/// module itself and of every enclosing module up to `root`.
fn unqualify_all(text: &str, module: &str, root: &str) -> String {
    let mut out = text.to_owned();
    let mut scope = module;

    loop {
        out = unqualify(&out, scope);

        if scope == root {
            return out;
        }

        scope = scope.rsplit_once('.').map_or(root, |(parent, _)| parent);
    }
}

fn unqualify(text: &str, module: &str) -> String {
    let prefix = format!("{module}.");
    let mut out = String::with_capacity(text.len());
    let mut rest = text;

    while let Some(at) = rest.find(&prefix) {
        let before = rest[..at].chars().next_back();
        let inside_name = before.is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '.');

        out.push_str(&rest[..at]);

        if inside_name {
            out.push_str(&prefix);
        }

        rest = &rest[at + prefix.len()..];
    }

    out.push_str(rest);
    out
}

impl Spice for String {
    fn rescript_type() -> String {
        "string".into()
    }

    fn samples() -> Vec<Self> {
        vec![String::new(), "a".into()]
    }

    fn decode<H: Host>(json: &Value) -> Result<Self> {
        match H::view(json) {
            View::String(s) => Ok(s.to_owned()),
            _ => Err(Failure::error("Not a string", json)),
        }
    }

    fn encode<H: Host>(&self) -> Value {
        Value::String(self.clone())
    }

    fn rescript_value(&self) -> Option<String> {
        Some(rescript_string(self))
    }
}

/// `Math.floor(n) | 0`: wraps into the 32-bit range, an infinity is 0.
fn to_int32(value: f64) -> i32 {
    if !value.is_finite() {
        return 0;
    }

    let wrapped = value.floor().rem_euclid(4_294_967_296.0);

    if wrapped >= 2_147_483_648.0 {
        (wrapped - 4_294_967_296.0) as i32
    } else {
        wrapped as i32
    }
}

impl Spice for i32 {
    fn rescript_type() -> String {
        "int".into()
    }

    /// `Spice.intFromJson`: a number equal to its floor (an infinity too).
    fn samples() -> Vec<Self> {
        vec![0, 7, -3]
    }

    fn decode<H: Host>(json: &Value) -> Result<Self> {
        match H::view(json) {
            View::Number(n) if n.floor() == n => Ok(to_int32(n)),
            View::Number(_) => Err(Failure::error("Not an integer", json)),
            _ => Err(Failure::error("Not a number", json)),
        }
    }

    fn encode<H: Host>(&self) -> Value {
        H::number(f64::from(*self))
    }

    fn rescript_value(&self) -> Option<String> {
        Some(self.to_string())
    }
}

/// A ReScript `int` that is never negative, e.g. an index or an order.
impl Spice for u32 {
    fn rescript_type() -> String {
        "int".into()
    }

    fn samples() -> Vec<Self> {
        vec![0, 7]
    }

    fn decode<H: Host>(json: &Value) -> Result<Self> {
        match H::view(json) {
            View::Number(n) if n.floor() == n && (0.0..=f64::from(i32::MAX)).contains(&n) => {
                Ok(n as u32)
            }
            View::Number(_) => Err(Failure::error("Not a non-negative int", json)),
            _ => Err(Failure::error("Not a number", json)),
        }
    }

    fn encode<H: Host>(&self) -> Value {
        H::number(f64::from(*self))
    }

    fn rescript_value(&self) -> Option<String> {
        Some(self.to_string())
    }
}

impl Spice for f64 {
    fn rescript_type() -> String {
        "float".into()
    }

    fn samples() -> Vec<Self> {
        vec![0.0, 1.5, -2.0]
    }

    fn decode<H: Host>(json: &Value) -> Result<Self> {
        match H::view(json) {
            View::Number(n) => Ok(n),
            _ => Err(Failure::error("Not a number", json)),
        }
    }

    fn encode<H: Host>(&self) -> Value {
        H::number(*self)
    }

    fn rescript_value(&self) -> Option<String> {
        rescript_float(*self)
    }
}

/// A JavaScript number read as the nearest `f32`, written as the shortest
/// decimal that reads back as it (`0.1`, not `0.10000000149011612`), as
/// serde_json writes an `f32`.
impl Spice for f32 {
    fn rescript_type() -> String {
        "float".into()
    }

    fn samples() -> Vec<Self> {
        vec![0.0, 0.1, -2.5]
    }

    fn decode<H: Host>(json: &Value) -> Result<Self> {
        f64::decode::<H>(json).map(|n| n as f32)
    }

    fn encode<H: Host>(&self) -> Value {
        H::number(shortest(*self))
    }

    fn rescript_value(&self) -> Option<String> {
        rescript_float(shortest(*self))
    }
}

/// The `f64` the shortest decimal form of `value` reads as: what JSON text
/// of an `f32` reads back as in JavaScript.
pub(crate) fn shortest(value: f32) -> f64 {
    use std::fmt::Write;

    /// Room for any `f32`'s shortest decimal (`-1.1754944e-38` is 14).
    struct Digits {
        bytes: [u8; 48],
        len: usize,
    }

    impl Write for Digits {
        fn write_str(&mut self, text: &str) -> std::fmt::Result {
            let end = self.len + text.len();
            let slot = self.bytes.get_mut(self.len..end).ok_or(std::fmt::Error)?;

            slot.copy_from_slice(text.as_bytes());
            self.len = end;
            Ok(())
        }
    }

    if !value.is_finite() {
        return f64::from(value);
    }

    let mut digits = Digits {
        bytes: [0; 48],
        len: 0,
    };

    match write!(digits, "{value}") {
        Ok(()) => std::str::from_utf8(&digits.bytes[..digits.len])
            .ok()
            .and_then(|text| text.parse().ok())
            .unwrap_or(f64::from(value)),
        Err(_) => f64::from(value),
    }
}

impl Spice for bool {
    fn rescript_type() -> String {
        "bool".into()
    }

    fn samples() -> Vec<Self> {
        vec![true, false]
    }

    fn decode<H: Host>(json: &Value) -> Result<Self> {
        match H::view(json) {
            View::Bool(b) => Ok(b),
            _ => Err(Failure::error("Not a boolean", json)),
        }
    }

    fn encode<H: Host>(&self) -> Value {
        Value::Bool(*self)
    }

    fn rescript_value(&self) -> Option<String> {
        Some(self.to_string())
    }
}

impl Spice for () {
    fn rescript_type() -> String {
        "unit".into()
    }

    fn samples() -> Vec<Self> {
        vec![()]
    }

    fn decode<H: Host>(_: &Value) -> Result<Self> {
        Ok(())
    }

    fn encode<H: Host>(&self) -> Value {
        H::number(0.0)
    }

    fn rescript_value(&self) -> Option<String> {
        Some("()".into())
    }
}

/// `JSON.t`: any value, as is.
impl Spice for Value {
    fn rescript_type() -> String {
        "JSON.t".into()
    }

    fn samples() -> Vec<Self> {
        vec![Value::Null, serde_json::json!({"a": [1, "b"]})]
    }

    fn decode<H: Host>(json: &Value) -> Result<Self> {
        Ok(json.clone())
    }

    fn encode<H: Host>(&self) -> Value {
        self.clone()
    }
}

impl<T: Spice> Spice for Box<T> {
    fn rescript_type() -> String {
        T::rescript_type()
    }

    fn decode<H: Host>(json: &Value) -> Result<Self> {
        T::decode::<H>(json).map(Box::new)
    }

    fn encode<H: Host>(&self) -> Value {
        (**self).encode::<H>()
    }

    fn rescript_value(&self) -> Option<String> {
        (**self).rescript_value()
    }
}

/// Decodes every item, as `Spice.arrayFromJson` and `dictFromJson` do
/// even after one fails (so a later item that throws still throws), and
/// answers the first error, its path prefixed with the item's.
fn decode_all<'a, H: Host, T: Spice>(
    items: impl IntoIterator<Item = (String, &'a Value)>,
) -> Result<Vec<T>> {
    let mut first_error = None;
    let mut decoded = vec![];

    for (prefix, item) in items {
        match T::decode::<H>(item) {
            Ok(value) => decoded.push(value),
            Err(Failure::Throw(throw)) => return Err(Failure::Throw(throw)),
            Err(error) => {
                first_error.get_or_insert_with(|| error.prefixed(&prefix));
            }
        }
    }

    match first_error {
        Some(error) => Err(error),
        None => Ok(decoded),
    }
}

impl<T: Spice> Spice for Vec<T> {
    fn rescript_type() -> String {
        format!("array<{}>", T::rescript_type())
    }

    fn samples() -> Vec<Self> {
        let mut out = vec![vec![]];

        out.extend(T::samples().into_iter().take(2).map(|item| vec![item]));
        out
    }

    fn decode<H: Host>(json: &Value) -> Result<Self> {
        let View::Array(items) = H::view(json) else {
            return Err(Failure::error("Not an array", json));
        };

        decode_all::<H, T>(
            items
                .iter()
                .enumerate()
                .map(|(i, item)| (format!("[{i}]"), item)),
        )
    }

    fn encode<H: Host>(&self) -> Value {
        Value::Array(self.iter().map(T::encode::<H>).collect())
    }

    fn rescript_value(&self) -> Option<String> {
        let items = self
            .iter()
            .map(Spice::rescript_value)
            .collect::<Option<Vec<_>>>()?;

        Some(format!("[{}]", items.join(", ")))
    }
}

/// `option<T>` where it is a value: in an array, tuple, variant payload or
/// result. `None` is `null`. (A record field of type `option<T>` is
/// omittable instead; `#[rescript(codec)]` reads it with [`field`].)
impl<T: Spice> Spice for Option<T> {
    fn rescript_type() -> String {
        format!("option<{}>", T::rescript_type())
    }

    fn samples() -> Vec<Self> {
        let mut out = vec![None];

        out.extend(T::samples().into_iter().take(2).map(Some));
        out
    }

    fn decode<H: Host>(json: &Value) -> Result<Self> {
        match H::view(json) {
            View::Null => Ok(None),
            _ => T::decode::<H>(json).map(Some),
        }
    }

    fn encode<H: Host>(&self) -> Value {
        match self {
            Some(value) => value.encode::<H>(),
            None => Value::Null,
        }
    }

    fn rescript_value(&self) -> Option<String> {
        match self {
            Some(value) => Some(format!("Some({})", value.rescript_value()?)),
            None => Some("None".into()),
        }
    }
}

/// `Null.t<T>`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub enum Null<T> {
    #[default]
    Null,
    Value(T),
}

impl<T: Spice> Spice for Null<T> {
    fn rescript_type() -> String {
        format!("Null.t<{}>", T::rescript_type())
    }

    fn samples() -> Vec<Self> {
        let mut out = vec![Null::Null];

        out.extend(T::samples().into_iter().take(2).map(Null::Value));
        out
    }

    fn decode<H: Host>(json: &Value) -> Result<Self> {
        match H::view(json) {
            View::Null => Ok(Null::Null),
            _ => T::decode::<H>(json).map(Null::Value),
        }
    }

    fn encode<H: Host>(&self) -> Value {
        match self {
            Null::Value(value) => value.encode::<H>(),
            Null::Null => Value::Null,
        }
    }
}

/// `dict<T>`, in JavaScript's key order.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Dict<T>(pub Vec<(String, T)>);

impl<T: Spice> Spice for Dict<T> {
    fn rescript_type() -> String {
        format!("dict<{}>", T::rescript_type())
    }

    fn samples() -> Vec<Self> {
        let mut out = vec![Dict(vec![])];

        out.extend(
            T::samples()
                .into_iter()
                .take(2)
                .map(|item| Dict(vec![("k".into(), item)])),
        );
        out
    }

    fn decode<H: Host>(json: &Value) -> Result<Self> {
        let View::Object(map) = H::view(json) else {
            return Err(Failure::error("Not a dict", json));
        };

        let values = decode_all::<H, T>(map.iter().map(|(key, value)| (format!(".{key}"), value)))?;

        Ok(Dict(map.keys().cloned().zip(values).collect()))
    }

    fn encode<H: Host>(&self) -> Value {
        object(
            self.0
                .iter()
                .map(|(key, value)| (key.clone(), Some(value.encode::<H>()))),
        )
    }
}

/// `JSON.Object(Dict.fromArray(Spice.filterOptional(entries)))`: the
/// entries that are `Some`, a later duplicate key replacing an earlier one's
/// value, in JavaScript's key order.
pub fn object(entries: impl IntoIterator<Item = (String, Option<Value>)>) -> Value {
    let map: Map<String, Value> = entries
        .into_iter()
        .filter_map(|(key, value)| Some((key, value?)))
        .collect();

    Value::Object(js_order(map))
}

impl<T: Spice, E: Spice> Spice for std::result::Result<T, E> {
    fn rescript_type() -> String {
        format!("result<{}, {}>", T::rescript_type(), E::rescript_type())
    }

    fn samples() -> Vec<Self> {
        let mut out: Vec<Self> = T::samples().into_iter().take(1).map(Ok).collect();

        out.extend(E::samples().into_iter().take(1).map(Err));
        out
    }

    fn decode<H: Host>(json: &Value) -> Result<Self> {
        let View::Array(items) = H::view(json) else {
            return Err(Failure::error("Not an array", json));
        };

        let [tag, payload] = &items[..] else {
            return Err(Failure::error("Expected exactly 2 values in array", json));
        };

        match H::view(tag) {
            View::String("Ok") => T::decode::<H>(payload).map(Ok),
            View::String("Error") => E::decode::<H>(payload).map(Err),
            View::String(_) => Err(Failure::error("Expected either \"Ok\" or \"Error\"", tag)),
            _ => Err(Failure::error("Not a string", tag)),
        }
    }

    fn encode<H: Host>(&self) -> Value {
        match self {
            Ok(value) => Value::Array(vec!["Ok".into(), value.encode::<H>()]),
            Err(error) => Value::Array(vec!["Error".into(), error.encode::<H>()]),
        }
    }
}

macro_rules! tuple {
    ($($name:ident $index:tt),+) => {
        impl<$($name: Spice),+> Spice for ($($name,)+) {
            fn rescript_type() -> String {
                let types: Vec<String> = vec![$($name::rescript_type()),+];

                format!("({})", types.join(", "))
            }

            /// Every item's first sample, then one more per other sample.
            fn samples() -> Vec<Self> {
                let all = ($($name::samples(),)+);

                if [$(all.$index.is_empty()),+].contains(&true) {
                    return vec![];
                }

                let first = ($(all.$index[0].clone(),)+);
                let mut out = vec![first.clone()];

                $(
                    for sample in all.$index.iter().skip(1) {
                        let mut value = first.clone();

                        value.$index = sample.clone();
                        out.push(value);
                    }
                )+

                out
            }

            fn decode<HOST: Host>(json: &Value) -> Result<Self> {
                let View::Array(items) = HOST::view(json) else {
                    return Err(Failure::error("Not a tuple", json));
                };

                if items.len() != [$($index),+].len() {
                    return Err(Failure::error("Incorrect cardinality", json));
                }

                let decoded = ($(args::decode::<HOST, $name>(&items[$index])?,)+);

                Ok(($(args::finish(decoded.$index, $index)?,)+))
            }

            fn encode<HOST: Host>(&self) -> Value {
                Value::Array(vec![$(self.$index.encode::<HOST>()),+])
            }

            fn rescript_value(&self) -> Option<String> {
                let items = [$(self.$index.rescript_value()?),+];

                Some(format!("({})", items.join(", ")))
            }
        }
    };
}

/// `tuple!` for every length from the first list's up to both lists'.
macro_rules! tuples {
    ($($name:ident $index:tt),+;) => {
        tuple!($($name $index),+);
    };
    ($($name:ident $index:tt),+; $next:ident $next_index:tt $(, $rest:ident $rest_index:tt)*) => {
        tuple!($($name $index),+);
        tuples!($($name $index),+, $next $next_index; $($rest $rest_index),*);
    };
}

/// Tuples of 2 to 16 values, as serde has; the derive rejects a longer one.
pub const MAX_TUPLE: usize = 16;

/// A ReScript tuple and a serde tuple are both a JavaScript array.
macro_rules! wasm_tuple {
    ($($name:ident),+) => {
        impl<$($name: Wasm),+> Wasm for ($($name,)+) {
            fn input_identity() -> bool {
                true $(&& $name::input_identity())+
            }
            fn output_identity() -> bool {
                true $(&& $name::output_identity())+
            }
        }
    };
}

/// `wasm_tuple!` for every length from the first list's up to both lists'.
macro_rules! wasm_tuples {
    ($($name:ident),+;) => {
        wasm_tuple!($($name),+);
    };
    ($($name:ident),+; $next:ident $(, $rest:ident)*) => {
        wasm_tuple!($($name),+);
        wasm_tuples!($($name),+, $next; $($rest),*);
    };
}

wasm_tuples!(A, B; C, D, E, F, G, H, I, J, K, L, M, N, O, P);

tuples!(A 0, B 1; C 2, D 3, E 4, F 5, G 6, H 7, I 8, J 9, K 10, L 11, M 12, N 13, O 14, P 15);

/// Variant and tuple arguments: spice decodes every argument before it
/// looks at any result, so a throw in a later argument wins over an error
/// in an earlier one, and otherwise the first error wins.
pub mod args {
    use super::*;

    /// Decodes one argument; only a throw stops the others.
    pub fn decode<H: Host, T: Spice>(json: &Value) -> Result<Result<T>> {
        decode_with(json, T::decode::<H>)
    }

    pub fn decode_with<T>(
        json: &Value,
        decode: impl FnOnce(&Value) -> Result<T>,
    ) -> Result<Result<T>> {
        match decode(json) {
            Err(Failure::Throw(throw)) => Err(Failure::Throw(throw)),
            result => Ok(result),
        }
    }

    /// The argument at `index` (a variant counts its constructor name as
    /// 0), its error prefixed with `[index]`.
    pub fn finish<T>(decoded: Result<T>, index: usize) -> Result<T> {
        decoded.map_err(|error| error.prefixed(&format!("[{index}]")))
    }
}

/// Record fields, as ppx-spice's generated decoder reads them: one after
/// the other, stopping at the first error.
pub mod field {
    use super::*;

    /// The record's own fields, or spice's error for a non-object.
    pub fn object<H: Host>(json: &Value) -> Result<Cow<'_, Map<String, Value>>> {
        match H::view(json) {
            View::Object(fields) => Ok(fields),
            _ => Err(Failure::error("Not an object", json)),
        }
    }

    fn at<T>(result: Result<T>, key: &str) -> Result<T> {
        result.map_err(|error| error.prefixed(&format!(".{key}")))
    }

    /// A field without a default: missing is an error.
    pub fn required<T>(
        fields: &Map<String, Value>,
        key: &str,
        record: &Value,
        decode: impl FnOnce(&Value) -> Result<T>,
    ) -> Result<T> {
        match fields.get(key) {
            Some(json) => at(decode(json), key),
            None => at(Err(Failure::error(format!("{key} missing"), record)), key),
        }
    }

    /// A field with `@spice.default(…)`: missing is the default.
    pub fn defaulted<T>(
        fields: &Map<String, Value>,
        key: &str,
        default: impl FnOnce() -> T,
        decode: impl FnOnce(&Value) -> Result<T>,
    ) -> Result<T> {
        match fields.get(key) {
            Some(json) => at(decode(json), key),
            None => Ok(default()),
        }
    }

    /// An omittable field (`option<T>` or `field?: T`) without a default:
    /// missing is `None`.
    pub fn optional<T>(
        fields: &Map<String, Value>,
        key: &str,
        decode: impl FnOnce(&Value) -> Result<Option<T>>,
    ) -> Result<Option<T>> {
        defaulted(fields, key, || None, decode)
    }

    /// `Spice.optionalFieldFromJson`: `null` is whatever `T` makes of it,
    /// or `None` when `T` rejects or throws on it.
    pub fn omittable<H: Host, T>(
        json: &Value,
        decode: impl FnOnce(&Value) -> Result<T>,
    ) -> Result<Option<T>> {
        match H::view(json) {
            View::Null => Ok(decode(json).ok()),
            _ => decode(json).map(Some),
        }
    }
}

/// A variant's JSON: `["Constructor", …arguments]`.
pub mod variant {
    use super::*;

    /// A variant or a polymorphic variant, whose errors spice words
    /// differently.
    #[derive(Clone, Copy)]
    pub enum Kind {
        Variant,
        Poly,
    }

    impl Kind {
        fn pick(self, variant: &'static str, poly: &'static str) -> &'static str {
            match self {
                Kind::Variant => variant,
                Kind::Poly => poly,
            }
        }
    }

    /// The constructor name, if the first item is a string.
    pub fn tag<H: Host>(items: &[Value]) -> Option<&str> {
        match H::view(&items[0]) {
            View::String(name) => Some(name),
            _ => None,
        }
    }

    pub fn arity_of(kind: Kind, items: &[Value], arguments: usize, json: &Value) -> Result<()> {
        if items.len() == arguments + 1 {
            Ok(())
        } else {
            Err(Failure::error(
                kind.pick(
                    "Invalid number of arguments to variant constructor",
                    "Invalid number of arguments to polyvariant constructor",
                ),
                json,
            ))
        }
    }

    pub fn unknown_of(kind: Kind, items: &[Value]) -> Failure {
        Failure::error(
            kind.pick(
                "Invalid variant constructor",
                "Invalid polymorphic variant constructor",
            ),
            &items[0],
        )
    }
}

/// An enum's JSON with `#[rescript(codec)]` (`@spice.serde`), encoded like
/// Rust's serde derive: externally tagged (`"A"`, `{"B": x}`,
/// `{"C": [x, y]}`, `{"D": {…}}`), or with `tag = "type"` internally tagged
/// (`{"type": "A"}`, `{"type": "D", …}`). With `legacy`
/// (`@spice.serde({legacy: true})`) the decoder also reads the default
/// spice array and, internally tagged, a unit constructor's bare name.
///
/// Two constructors can't share a JSON name.
///
/// ```compile_fail
/// #[derive(rescript_rs::ReScript, Clone)]
/// #[rescript(codec)]
/// enum E {
///     A,
///     #[rescript(rename = "A")]
///     B,
/// }
/// ```
///
/// An internally tagged constructor has at most one value.
///
/// ```compile_fail
/// #[derive(rescript_rs::ReScript, Clone)]
/// #[rescript(codec, tag = "type")]
/// enum E {
///     A(i32, i32),
/// }
/// ```
///
/// No payload field is keyed like the tag.
///
/// ```compile_fail
/// #[derive(rescript_rs::ReScript, Clone)]
/// #[rescript(codec, tag = "type")]
/// enum E {
///     A {
///         #[rescript(name = "kind", key = "type")]
///         kind: i32,
///     },
/// }
/// ```
///
/// A JSON name is a string.
///
/// ```compile_fail
/// #[derive(rescript_rs::ReScript, Clone)]
/// #[rescript(codec)]
/// enum E {
///     #[rescript(rename = 1)]
///     A,
/// }
/// ```
///
/// `tag`, `legacy` and `poly` are for enums.
///
/// ```compile_fail
/// #[derive(rescript_rs::ReScript, Clone)]
/// #[rescript(codec, legacy)]
/// struct S {
///     a: i32,
/// }
/// ```
///
/// A polymorphic variant has no constructors with named fields.
///
/// ```compile_fail
/// #[derive(rescript_rs::ReScript, Clone)]
/// #[rescript(codec, poly)]
/// enum E {
///     A { x: i32 },
/// }
/// ```
///
/// A tuple has at most 16 values, also inside another type.
///
/// ```compile_fail
/// #[derive(rescript_rs::ReScript, Clone)]
/// #[rescript(codec)]
/// struct S {
///     a: Vec<(i32, i32, i32, i32, i32, i32, i32, i32, i32, i32, i32, i32, i32, i32, i32, i32, i32)>,
/// }
/// ```
///
/// A declared type's field of a type parameter's type has no default: its
/// ReScript type is a type variable.
///
/// ```compile_fail
/// #[derive(rescript_rs::ReScript, Clone)]
/// #[rescript(codec, module = "M")]
/// struct S<T: Default> {
///     #[serde(default)]
///     a: T,
/// }
/// ```
///
/// A declared type doesn't use a type parameter's associated type, which
/// ReScript has no counterpart of.
///
/// ```compile_fail
/// trait Has {
///     type X;
/// }
///
/// #[derive(rescript_rs::ReScript, Clone)]
/// #[rescript(codec, module = "M")]
/// struct S<T: Has>
/// where
///     T::X: rescript_rs::spice::Spice,
/// {
///     a: T::X,
/// }
/// ```
pub mod serde {
    use super::*;

    /// What the decoder dispatches on.
    pub enum Shape<'a> {
        /// A unit constructor's name.
        Name(&'a str),
        Object(Cow<'a, Map<String, Value>>),
        /// The default spice encoding, `["Constructor", …]`, not empty.
        Array(Cow<'a, [Value]>),
    }

    /// An empty array is an error only with `legacy`; without it the
    /// caller rejects every array ([`found_array`]).
    pub fn shape<H: Host>(json: &Value, legacy: bool) -> Result<Shape<'_>> {
        match H::view(json) {
            View::String(name) => Ok(Shape::Name(name)),
            View::Object(map) => Ok(Shape::Object(map)),
            View::Array(items) if legacy && items.is_empty() => {
                Err(Failure::error("Expected variant, found empty array", json))
            }
            View::Array(items) => Ok(Shape::Array(items)),
            _ => Err(not_a_variant(json)),
        }
    }

    pub fn not_a_variant(json: &Value) -> Failure {
        Failure::error("Not a variant", json)
    }

    /// An array without `legacy`.
    pub fn found_array(json: &Value) -> Failure {
        Failure::error("Expected variant, found array", json)
    }

    /// The name and payload of an externally tagged object.
    pub fn external<'a>(map: &'a Map<String, Value>, json: &Value) -> Result<(&'a str, &'a Value)> {
        let mut entries = map.iter();

        match (entries.next(), entries.next()) {
            (Some((name, payload)), None) => Ok((name, payload)),
            _ => Err(Failure::error("Expected an object with one key", json)),
        }
    }

    /// The name under `tag` in an internally tagged object.
    pub fn tag<'a, H: Host>(
        map: &'a Map<String, Value>,
        tag: &str,
        json: &Value,
    ) -> Result<&'a str> {
        match map.get(tag).map(H::view) {
            Some(View::String(name)) => Ok(name),
            _ => Err(Failure::error(format!("{tag} missing"), json)),
        }
    }

    pub fn unknown(json: &Value) -> Failure {
        Failure::error("Invalid variant constructor", json)
    }

    /// The error under the constructor's name: `.Name…`.
    pub fn at<T>(result: Result<T>, name: &str) -> Result<T> {
        result.map_err(|error| error.prefixed(&format!(".{name}")))
    }

    /// `{"Unit": null}`, which serde also reads.
    pub fn unit<H: Host>(payload: &Value) -> Result<()> {
        match H::view(payload) {
            View::Null => Ok(()),
            _ => Err(Failure::error("Expected null", payload)),
        }
    }

    /// The array of a constructor with several values.
    pub fn args<H: Host>(payload: &Value, count: usize) -> Result<Cow<'_, [Value]>> {
        match H::view(payload) {
            View::Array(items) if items.len() == count => Ok(items),
            View::Array(_) => Err(Failure::error(
                "Invalid number of arguments to variant constructor",
                payload,
            )),
            _ => Err(Failure::error("Not an array", payload)),
        }
    }

    /// `Spice.untagged`: the object without its tag, which an internally
    /// tagged single value decodes from.
    pub fn untagged(map: &Map<String, Value>, tag: &str) -> Value {
        Value::Object(
            map.iter()
                .filter(|(key, _)| *key != tag)
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
        )
    }

    /// `Spice.taggedObject`: an internally tagged single value, the tag
    /// first. Like the ReScript encoder, panics when the value doesn't
    /// encode to an object or already has the tag.
    pub fn tagged_object(tag: &str, name: &str, value: Value) -> Value {
        let Value::Object(map) = value else {
            panic!(
                "Can't encode {name} with tag \"{tag}\": its payload doesn't encode to an object"
            );
        };

        if map.contains_key(tag) {
            panic!(
                "Can't encode {name} with tag \"{tag}\": its payload already has a \"{tag}\" key"
            );
        }

        object(
            std::iter::once((tag.to_owned(), Some(Value::String(name.to_owned()))))
                .chain(map.into_iter().map(|(key, value)| (key, Some(value)))),
        )
    }
}

/// serde's `Serialize` and `Deserialize` through a type's codec, which
/// `#[rescript(codec, serde)]` implements them with: serde reads and writes
/// the JSON ppx-spice does.
pub mod serde_impl {
    pub use ::serde::{de::Error, Deserialize, Deserializer, Serialize, Serializer};

    use super::{Failure, Plain, Spice, Value};

    pub fn serialize<T: Spice, S: Serializer>(value: &T, serializer: S) -> Result<S::Ok, S::Error> {
        value.encode::<Plain>().serialize(serializer)
    }

    pub fn deserialize<'de, T: Spice, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<T, D::Error> {
        let json = Value::deserialize(deserializer)?;

        T::decode::<Plain>(&json).map_err(|failure| match failure {
            Failure::Decode(error) if error.path.is_empty() => D::Error::custom(error.message),
            Failure::Decode(error) => {
                D::Error::custom(format!("{} at `{}`", error.message, error.path))
            }
            Failure::Throw(message) => D::Error::custom(message),
        })
    }
}

#[cfg(test)]
mod value_codec {
    use super::ValueCodec;

    fn codecs(rescript_type: &str) -> (String, String) {
        let codec = ValueCodec::of(rescript_type);

        (codec.encode("x"), codec.decode("json"))
    }

    /// A generic type's codec takes its type arguments' codecs one call at
    /// a time and a `Spice` codec takes them with the value; an optional
    /// dict value is left out, an optional argument is `null`.
    #[test]
    fn a_value_is_read_with_the_codecs_ppx_spice_picks() {
        let cases = [
            (
                "Shapes.pair<int, array<Shapes.t>>",
                "Shapes.pair_encode(Spice.intToJson)(Spice.arrayToJson(Shapes.t_encode, ...))(x)",
                "Shapes.pair_decode(Spice.intFromJson)(Spice.arrayFromJson(Shapes.t_decode, ...))(json)",
            ),
            (
                "array<option<generic<float>>>",
                "Spice.arrayToJson(Spice.optionToNullableJson(generic_encode(Spice.floatToJson), ...), x)",
                "json->Spice.arrayFromJson(Spice.optionFromJson(generic_decode(Spice.floatFromJson), ...), _)",
            ),
            (
                "dict<option<int>>",
                "Spice.dictOptionalToJson(Spice.optionToJson(Spice.intToJson, ...), x)",
                "json->Spice.dictFromJson(Spice.optionFromJson(Spice.intFromJson, ...), _)",
            ),
            (
                "result<string, Null.t<JSON.t>>",
                "Spice.resultToJson(Spice.stringToJson, Spice.nullToJson(v => v, ...), x)",
                "json->Spice.resultFromJson(Spice.stringFromJson, Spice.nullFromJson(v => Ok(v), ...), _)",
            ),
            ("JSON.t", "x", "Ok(json)"),
        ];

        for (rescript_type, encode, decode) in cases {
            assert_eq!(
                codecs(rescript_type),
                (encode.to_owned(), decode.to_owned()),
                "{rescript_type}"
            );
        }
    }

    #[test]
    #[should_panic(expected = "give the type a name")]
    fn a_tuple_has_no_named_codec() {
        ValueCodec::of("array<(int, string)>");
    }
}

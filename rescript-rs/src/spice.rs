//! Rust codecs that read and write JSON exactly as
//! [ppx-spice](https://github.com/jfrolich/ppx_spice) does in ReScript.
//!
//! `#[derive(Spice)]` on a struct or enum gives it a [`Spice`] impl and a
//! ReScript declaration with spice annotations ([`Declare`]). ppx-spice
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

/// A ReScript type declaration generated by `#[derive(Spice)]`.
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
                    out.push_str(&unqualify_all(text, self.name, root));
                    out.push('\n');
                }
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
}

/// `option<T>` where it is a value: in an array, tuple, variant payload or
/// result. `None` is `null`. (A record field of type `option<T>` is
/// omittable instead; `#[derive(Spice)]` reads it with [`field`].)
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

    /// The array, or spice's error for anything else.
    pub fn items<H: Host>(json: &Value) -> Result<Cow<'_, [Value]>> {
        match H::view(json) {
            View::Array(items) if items.is_empty() => {
                Err(Failure::error("Expected variant, found empty array", json))
            }
            View::Array(items) => Ok(items),
            _ => Err(Failure::error("Not a variant", json)),
        }
    }

    /// The constructor name, if the first item is a string.
    pub fn tag<H: Host>(items: &[Value]) -> Option<&str> {
        match H::view(&items[0]) {
            View::String(name) => Some(name),
            _ => None,
        }
    }

    pub fn arity(items: &[Value], arguments: usize, json: &Value) -> Result<()> {
        if items.len() == arguments + 1 {
            Ok(())
        } else {
            Err(Failure::error(
                "Invalid number of arguments to variant constructor",
                json,
            ))
        }
    }

    pub fn unknown(items: &[Value]) -> Failure {
        Failure::error("Invalid variant constructor", &items[0])
    }

    /// The JSON of a variant whose constructors all have `@spice.as`.
    pub enum Alias<'a> {
        String(&'a str),
        Number(f64),
    }

    pub fn alias<H: Host>(json: &Value) -> Result<Alias<'_>> {
        match H::view(json) {
            View::String(s) => Ok(Alias::String(s)),
            View::Number(n) => Ok(Alias::Number(n)),
            _ => Err(Failure::error("Not a JSONString", json)),
        }
    }

    pub fn not_matched(json: &Value) -> Failure {
        Failure::error("Not matched", json)
    }
}

/// A `@spice.serde` variant's JSON, encoded like Rust's serde derive:
/// externally tagged (`"A"`, `{"B": x}`, `{"C": [x, y]}`, `{"D": {…}}`), or
/// with `@tag("type")` internally tagged (`{"type": "A"}`,
/// `{"type": "D", …}`). The decoder also reads the default spice array.
///
/// Two constructors can't share a JSON name.
///
/// ```compile_fail
/// #[derive(rescript_rs::Spice, Clone)]
/// #[spice(serde)]
/// enum E {
///     A,
///     #[spice(alias = "A")]
///     B,
/// }
/// ```
///
/// An internally tagged constructor has at most one value.
///
/// ```compile_fail
/// #[derive(rescript_rs::Spice, Clone)]
/// #[spice(serde, tag = "type")]
/// enum E {
///     A(i32, i32),
/// }
/// ```
///
/// No payload field is keyed like the tag.
///
/// ```compile_fail
/// #[derive(rescript_rs::Spice, Clone)]
/// #[spice(serde, tag = "type")]
/// enum E {
///     A {
///         #[spice(name = "kind", key = "type")]
///         kind: i32,
///     },
/// }
/// ```
///
/// A JSON name is a string.
///
/// ```compile_fail
/// #[derive(rescript_rs::Spice, Clone)]
/// #[spice(serde)]
/// enum E {
///     #[spice(alias = 1)]
///     A,
/// }
/// ```
///
/// `serde` is for enums.
///
/// ```compile_fail
/// #[derive(rescript_rs::Spice, Clone)]
/// #[spice(serde)]
/// struct S {
///     a: i32,
/// }
/// ```
///
/// `tag` needs `serde`.
///
/// ```compile_fail
/// #[derive(rescript_rs::Spice, Clone)]
/// #[spice(tag = "type")]
/// enum E {
///     A,
/// }
/// ```
///
/// `serde` and `unboxed` don't mix.
///
/// ```compile_fail
/// #[derive(rescript_rs::Spice, Clone)]
/// #[spice(serde, unboxed)]
/// enum E {
///     A(i32),
/// }
/// ```
///
/// A tuple has at most 16 values, also inside another type.
///
/// ```compile_fail
/// #[derive(rescript_rs::Spice, Clone)]
/// struct S {
///     a: Vec<(i32, i32, i32, i32, i32, i32, i32, i32, i32, i32, i32, i32, i32, i32, i32, i32, i32)>,
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

    pub fn shape<H: Host>(json: &Value) -> Result<Shape<'_>> {
        match H::view(json) {
            View::String(name) => Ok(Shape::Name(name)),
            View::Object(map) => Ok(Shape::Object(map)),
            View::Array(items) if items.is_empty() => {
                Err(Failure::error("Expected variant, found empty array", json))
            }
            View::Array(items) => Ok(Shape::Array(items)),
            _ => Err(Failure::error("Not a variant", json)),
        }
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

# rescript-rs

<h1 align="center" style="padding-top: 0; margin-top: 0;">
rescript-rs
</h1>
<p align="center">
Generate ReScript type declarations from Rust types
</p>

### Why?
When building a web application in Rust, data structures have to be shared between backend and frontend.
Using this library, you can easily generate ReScript bindings to your Rust structs & enums so that you can keep your
types in one place.

rescript-rs might also come in handy when working with WebAssembly.

### How?
rescript-rs exposes a single trait, `TS`. Using the `ReScript` derive macro, you can implement this interface for your types.
Then, you can use this trait to obtain the ReScript bindings.
We recommend doing this in your tests.
[See the example](https://github.com/jfrolich/rescript-rs/blob/main/example/src/lib.rs) and the docs.

### Get started
```toml
[dependencies]
rescript-rs = { git = "https://github.com/jfrolich/rescript-rs.git" }
```

```rust
#[derive(rescript_rs::ReScript)]
#[rescript(export)]
struct User {
    user_id: i32,
    first_name: String,
    last_name: String,
}
```

When running `cargo test` or `cargo test export_bindings`, the following ReScript type will be exported to `bindings/User.res`:

```rescript
type user = { user_id: int, first_name: string, last_name: string }
```

### Features
- generate type declarations from Rust structs
- generate variant declarations from Rust enums
- works with generic types
- compatible with serde
- generate necessary imports when exporting to multiple files
- precise control over generated types

If there's a type you're dealing with which doesn't implement `TS`, you can use either
`#[rescript(as = "..")]` or `#[rescript(type = "..")]`, enable the appropriate cargo feature, or open a PR.

### Codecs (`spice` feature)
`#[rescript(codec)]` defines a type once in Rust for both languages. Instead of the plain type export, the derive
writes a ReScript declaration with [ppx-spice](https://github.com/jfrolich/ppx_spice) annotations (so ppx-spice
generates the ReScript `t_decode` and `t_encode`), and a Rust codec (`rescript_rs::spice::Spice`) that reads and
writes JSON the way that generated code does: same accepted inputs, same error paths and messages, same output.

```rust
#[derive(rescript_rs::ReScript, Clone)]
#[rescript(codec, module = "DB__Row", name = "t")]
struct Row {
    #[rescript(key = "PK")]
    pk: String,
    #[rescript(name = "userID")]
    user_id: String,
    #[rescript(default = "[]")]
    words: Vec<String>,
    #[rescript(optional)]
    title: Option<String>,
}
```

`rescript_rs::spice::Module::new("DB__Row").with::<Row>().render()` gives `DB__Row.res`:

```rescript
@spice
type t = {
  @spice.key("PK")
  pk: string,
  userID: string,
  @spice.default([])
  words: array<string>,
  title?: string,
}
```

Enums are encoded the way Rust's serde derive does (`@spice.serde`, ppx-spice 0.7). `tag = "type"` adds
`@tag("type")`, like `#[serde(tag = "type")]`:

```rust
#[derive(rescript_rs::ReScript, Clone)]
#[rescript(codec, module = "Shape", name = "t", tag = "type")]
enum Shape {
    Empty,                      // {"type": "Empty"}
    Circle { radius: f64 },     // {"type": "Circle", "radius": 1.5}
    #[rescript(rename = "box")]
    Rect(Size),                 // {"type": "box", "w": 1, "h": 2}, Size being a record
}
```

Without `tag`, `Empty` is `"Empty"`, `Circle { radius }` is `{"Circle": {"radius": 1.5}}` and a constructor with
several values is `{"Pair": [1, "a"]}`. Decoders read only that form; `legacy` (`@spice.serde({legacy: true})`)
lets both decoders also read spice's default `["Constructor", …]` arrays and, with `tag`, a unit constructor's bare
name, so values stored before a type switched keep decoding. `legacy_write` (`@spice.serde({legacyWrite: true})`,
ppx-spice 0.8) reads both forms too but writes spice's default arrays, led by the constructor's Rust name, so JSON
that is already stored keeps its form until every reader understands serde. The derive rejects what ppx-spice rejects (two
constructors with the same JSON name, several values or a field keyed like the tag in a tagged constructor).

Container attributes: `codec`, `module`, `name`, `attrs` (extra ReScript attributes, e.g. `@genType`), `tag`,
`legacy`, `legacy_write`, `poly` (a polymorphic variant, `[#A | #B]`), `decode_only`, `encode_only`, `rename_all = "camelCase"`.
`serde` also implements serde's `Serialize` and `Deserialize` with the codec, so a codec type can be a field of a type
serde derives and serde reads and writes the JSON ppx-spice does. A codec can have type parameters, each bound by `Spice`;
its declaration has a type variable for each (`struct Size<T>` declares `type t<'t> = {width: 't, …}`) and
`Size<f64>` is `Size.t<float>` in the types that use it. `f32` reads as the nearest `f32` and writes its shortest decimal (`0.1`).
Field attributes: `name` (the ReScript field), `key` (`@spice.key`), `default` (`@spice.default`, a ReScript
expression; simple literals and constructors are translated, otherwise add `rust_default`), `optional` (`field?:`; `#[serde(skip_serializing_if = "Option::is_none")]` on an `Option` field implies it),
`codec` + `with` (`@spice.codec` and the Rust module with `decode`/`encode`/`samples`). Variant attribute: `rename`
(ReScript's `@as`, which names the constructor in memory and in JSON; `@spice.as`, the JSON name only, on a
polymorphic variant, which ReScript can't rename). A `Host` says how values JSON cannot hold (`Infinity`, `bigint`, `Set`) stand in a
`serde_json::Value`. `Spice::samples` and `Module::codecs` exist to test a type's Rust codec against the ReScript
one.

### Calling Rust through wasm-bindgen (`wasm-bindgen` feature)

Use native serde derives and `#[rescript(codec, wasm)]` on boundary types:

```rust
#[derive(Clone, serde::Serialize, serde::Deserialize, rescript_rs::ReScript)]
#[rescript(codec, wasm, module = "Geometry")]
#[serde(tag = "type", rename_all = "camelCase")]
enum Fit {
    Unbounded,
    Within { width: f64, height: f64 },
}
```

The declaration uses `@tag("type")`, `@as("unbounded") Unbounded({})` and
`@as("within") Within({width: float, height: float})`. The empty inline record
keeps unit variants as tagged objects, not runtime strings. Renamed record
fields receive both `@as` and `@spice.key`: runtime and JSON keys match.
Payload enums must be internally tagged. A payload-free enum needs no tag: it
is an ordinary variant, the same string on both sides.

```rust
#[derive(Clone, Copy, serde::Serialize, serde::Deserialize, rescript_rs::ReScript)]
#[rescript(codec, wasm, module = "Library")]
enum Format {
    Hardcover,
    Paperback,
    #[serde(rename = "e-book")]
    Ebook,
}
```

That is `Hardcover | Paperback | @as("e-book") Ebook`: serde's names unless
renamed. `poly` (`[#Hardcover | #Paperback | #"e-book"]`) and
`#[serde(rename_all = …)]` are choices, not requirements.

`#[rescript_rs::wasm]` on a function exports it through **direct native serde**
(`serde-wasm-bindgen`), without a `serde_json::Value` bridge. Add its generated
`<function>_rescript()` description to `Module::function`. Where layouts match,
the generated binding is just `Ok(wasm(arg0, arg1))`: no JS Spice conversion or
copying, and no redundant validation of trusted same-build Rust output.

Eligibility is recursive through records, arrays, boxes and tagged variants.
An `Option` field needs `#[serde(skip_serializing_if = "Option::is_none")]`,
which alone makes it an optional ReScript field (`label?: string`): omitted
keys are valid ReScript optional fields. Optional fields whose values can themselves be null
(JSON, unit or another option) are rejected: native serde would lose the
difference between a present null and an absent field. Standalone options, unit values and recursive layouts
retain JS codecs where representations differ, but still use native serde in
Rust. Unknown/custom serde attributes are rejected rather than guessing.
`#[rescript(serde)]` is the legacy Spice-to-serde adapter and cannot be combined
with `wasm`; derive `serde::Serialize` and `serde::Deserialize` instead.

`#[rescript_rs::wasm(codec)]` explicitly retains the original JSON-tree bridge
for legacy/custom codecs. Native serde owns its validation rules and errors;
it does not reproduce JSON normalization for non-finite floats, f32, defaults or
tuples. f32 is deliberately not a built-in `Wasm` type. Never bypass decoders for
arbitrary external JSON. This optimization is confined to the typed WASM boundary.

Enable `wasm-bindgen` in the final WASM crate and `pub use` each library's
`<function>_wasm` export there so the linker retains it. The application function
stays an ordinary typed Rust function. Errors are thrown as JS exceptions;
successful generated calls return `Ok(value)`. `name` and `js_name` rename the
ReScript function and the export, respectively.

`e2e/wasm/run.sh` compiles real generated ReScript and optimized WASM and runs
them together in Node, covering tagged unit/payload variants, renamed keys,
optional fields, arrays and the standalone-option codec fallback.

### Configuration
When using `#[rescript(export)]` on a type, rescript-rs generates a test which writes the bindings for it to disk.\
The following environment variables may be set to configure *how* and *where*:
| Variable                 | Description                                                         | Default      |
|--------------------------|---------------------------------------------------------------------|--------------|
| `TS_RS_EXPORT_DIR`       | Base directory into which bindings will be exported                 | `./bindings` |

We recommend putting this configuration in the project's [config.toml](https://doc.rust-lang.org/cargo/reference/config.html#env) to make it persistent:
```toml
# <project-root>/.cargo/config.toml
[env]
TS_RS_EXPORT_DIR = { value = "bindings", relative = true }
```

To export bindings programmatically without the use of tests, `TS::export_all`, `TS::export`, and `TS::export_to_string` can be used instead.

### Serde Compatibility
With the `serde-compat` feature (enabled by default), serde attributes are parsed for enums and structs.\
Supported serde attributes: `rename`, `rename-all`, `rename-all-fields`, `tag`, `content`, `untagged`, `skip`, `skip_serializing`, `skip_serializing_if`, `flatten`, `default`

**Note**: `skip_serializing` and `skip_serializing_if` only have an effect when used together with
`#[serde(default)]`. This ensures that the generated type is correct for both serialization and deserialization.

**Note**: `skip_deserializing` is ignored. If you wish to exclude a field
from the generated type, but cannot use `#[serde(skip)]`, use `#[rescript(skip)]` instead.

When rescript-rs encounters an unsupported serde attribute, a warning is emitted, unless the feature `no-serde-warnings` is enabled.

### Cargo Features
| **Feature**        | **Description**                                                                                                                                     |
|:-------------------|-----------------------------------------------------------------------------------------------------------------------------------------------------|
| serde-compat       | **Enabled by default** <br/>See the *"serde compatibility"* section above for more information.                                                     |
| no-serde-warnings  | By default, warnings are printed during build if unsupported serde attributes are encountered. <br/>Enabling this feature silences these warnings.  |
| serde-json-impl    | Implement `TS` for types from *serde_json*                                                                                                          |
| chrono-impl        | Implement `TS` for types from *chrono*                                                                                                              |
| bigdecimal-impl    | Implement `TS` for types from *bigdecimal*                                                                                                          |
| url-impl           | Implement `TS` for types from *url*                                                                                                                 |
| uuid-impl          | Implement `TS` for types from *uuid*                                                                                                                |
| bson-uuid-impl     | Implement `TS` for *bson::oid::ObjectId* and *bson::uuid*                                                                                           |
| bytes-impl         | Implement `TS` for types from *bytes*                                                                                                               |
| indexmap-impl      | Implement `TS` for types from *indexmap*                                                                                                            |
| ordered-float-impl | Implement `TS` for types from *ordered_float*                                                                                                       |
| heapless-impl      | Implement `TS` for types from *heapless*                                                                                                            |
| semver-impl        | Implement `TS` for types from *semver*                                                                                                              |
| smol_str-impl      | Implement `TS` for types from *smol_str*                                                                                                            |
| tokio-impl         | Implement `TS` for types from *tokio*                                                                                                               |
| jiff-impl          | Implement `TS` for types from *jiff*                                                                                                                |
| arrayvec-impl      | Implement `TS` for types from *arrayvec*                                                                                                            |
| wasm-bindgen       | The wasm-bindgen exports of `#[rescript_rs::wasm]` functions, and `rescript_rs::wasm`                                                               |

### Contributing
Contributions are always welcome!
Feel free to open an issue, discuss using GitHub discussions or open a PR.
[See CONTRIBUTING.md](https://github.com/jfrolich/rescript-rs/blob/main/CONTRIBUTING.md)

### Credits
rescript-rs is a fork of [ts-rs](https://github.com/Aleph-Alpha/ts-rs) by Aleph Alpha, adapted to generate ReScript types instead of TypeScript.

### MSRV
The Minimum Supported Rust Version for this crate is 1.78.0

License: MIT

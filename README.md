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

### Spice codecs (`spice` feature)
`#[derive(Spice)]` defines a type once in Rust for both languages. It writes a ReScript declaration with
[ppx-spice](https://github.com/jfrolich/ppx_spice) annotations (so ppx-spice generates the ReScript `t_decode` and
`t_encode`), and a Rust codec (`rescript_rs::spice::Spice`) that reads and writes JSON the way that generated code
does: same accepted inputs, same error paths and messages, same output. That keeps JSON that ReScript code already
reads and writes (stored rows, API bodies) compatible.

```rust
#[derive(rescript_rs::Spice, Clone)]
#[spice(module = "DB__Row", name = "t")]
struct Row {
    #[spice(key = "PK")]
    pk: String,
    #[spice(name = "userID")]
    user_id: String,
    #[spice(default = "[]")]
    words: Vec<String>,
    #[spice(optional)]
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

Container attributes: `module`, `name`, `attrs` (extra ReScript attributes, e.g. `@genType`), `unboxed`, `poly` (a
polymorphic variant, `[#A | #B]`; its JSON is a variant's),
`decode_only`, `encode_only`, `rename_all = "camelCase"`. Field attributes: `name` (the ReScript field), `key`
(`@spice.key`), `default` (`@spice.default`, a ReScript expression; simple literals and constructors are translated,
otherwise add `rust_default`), `optional` (`field?:`), `codec` + `with` (`@spice.codec` and the Rust module with
`decode`/`encode`/`samples`). Variant attributes: `name` (the ReScript constructor, which spice also writes in JSON) and `alias` (`@spice.as`). A `Host` says how values JSON cannot hold
(`Infinity`, `bigint`, `Set`) stand in a `serde_json::Value`. `Spice::samples` and `Module::codecs` exist to test a
type's Rust codec against the ReScript one.

`#[spice(serde)]` on an enum writes `@spice.serde` (ppx-spice since jfrolich/ppx_spice#5), which encodes variants the way Rust's serde derive
does instead of spice's `["Constructor", …]` arrays; `tag = "type"` adds `@tag("type")`, like `#[serde(tag = "type")]`:

```rust
#[derive(rescript_rs::Spice, Clone)]
#[spice(module = "Shape", name = "t", serde, tag = "type")]
enum Shape {
    Empty,                      // {"type": "Empty"}
    Circle { radius: f64 },     // {"type": "Circle", "radius": 1.5}
    #[spice(alias = "box")]
    Rect(Size),                 // {"type": "box", "w": 1, "h": 2}, Size being a record
}
```

Without `tag`, `Empty` is `"Empty"`, `Circle { radius }` is `{"Circle": {"radius": 1.5}}` and a constructor with
several values is `{"Pair": [1, "a"]}`. Both decoders still read the old arrays, so stored values keep decoding after
a type switches to `serde`. `alias` sets a constructor's JSON name (on any constructor); the derive rejects what
ppx-spice rejects (two constructors with the same JSON name, several values or a field keyed like the tag in a
tagged constructor).

The `spice` feature turns on serde_json's `preserve_order`, so objects keep JavaScript's key order. Where that is
unwanted for the whole build, `spice-codec` is the same codec without it: objects have sorted keys, the values are
the same.

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

### Contributing
Contributions are always welcome!
Feel free to open an issue, discuss using GitHub discussions or open a PR.
[See CONTRIBUTING.md](https://github.com/jfrolich/rescript-rs/blob/main/CONTRIBUTING.md)

### Credits
rescript-rs is a fork of [ts-rs](https://github.com/Aleph-Alpha/ts-rs) by Aleph Alpha, adapted to generate ReScript types instead of TypeScript.

### MSRV
The Minimum Supported Rust Version for this crate is 1.78.0

License: MIT

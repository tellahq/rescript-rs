# Generated ReScript ↔ direct serde WASM

Run `bash run.sh` with Bun, Node, the wasm32-unknown-unknown Rust target and
wasm-bindgen-cli 0.2.113 installed. Dependencies are installed in this isolated
project; do not symlink a webapp's compiled ppx-spice dependency here.

The fixture is shared with the library's WASM tests. `src/Geometry.res` is
generated from the Rust function signatures. `Exercise.res` constructs real
ReScript values and calls those bindings; `verify.mjs` checks the results.

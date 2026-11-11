#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"
bun install
# Bun hard-links package build metadata; isolate it before ReScript writes it.
copy="$(mktemp -d)"
cp -rL node_modules/@jfrolich/ppx-spice "$copy/ppx-spice"
# A copied compiler cache can still name files in the source checkout.
rm -rf "$copy/ppx-spice/lib"
rm -rf node_modules/@jfrolich/ppx-spice
mv "$copy/ppx-spice" node_modules/@jfrolich/ppx-spice
rmdir "$copy"
cargo run --quiet --bin wasm-bridge-e2e > src/Geometry.res
cargo build --release --lib --target wasm32-unknown-unknown
wasm-bindgen target/wasm32-unknown-unknown/release/wasm_bridge_e2e.wasm --target nodejs --out-dir src/wasm
printf '{"type":"commonjs"}\n' > src/wasm/package.json
bunx rescript build
node verify.mjs

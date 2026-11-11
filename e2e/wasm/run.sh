#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"
bun install
cargo run --quiet --bin wasm-bridge-e2e > src/Geometry.res
cargo build --release --lib --target wasm32-unknown-unknown
wasm-bindgen target/wasm32-unknown-unknown/release/wasm_bridge_e2e.wasm --target nodejs --out-dir src/wasm
printf '{"type":"commonjs"}\n' > src/wasm/package.json
bunx rescript build
node verify.mjs

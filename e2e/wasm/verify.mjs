import assert from "node:assert/strict";
import * as run from "./src/Exercise.bs.js";
import * as geometry from "./src/Geometry.bs.js";

assert.deepEqual(run.fitted, {TAG: "Ok", _0: {width: 100, height: 25, label: "nab"}});
assert.deepEqual(run.empty, {TAG: "Ok", _0: {kind: "empty"}});
assert.deepEqual(run.placed, {TAG: "Ok", _0: {kind: "placed", metadata: {"content-type": "image/png", visible: false}, sizes: [{width: 3, height: 4}]}});
assert.deepEqual(run.unbounded, {TAG: "Ok", _0: {width: 3, height: 4}});
assert.deepEqual(run.absent, {TAG: "Ok", _0: undefined});
assert.deepEqual(run.present, {TAG: "Ok", _0: "value"});
assert.throws(() => geometry.fitted({width: "invalid", height: 1}, {type: "unbounded"}, [], ""));
const cyclic = []; cyclic.push(cyclic);
assert.throws(() => geometry.fitted({width: 1, height: 1}, {type: "unbounded"}, cyclic, ""));
console.log("Generated ReScript → native serde WASM → ReScript: tagged unit/payload variants, @as fields, optional fields, arrays and codec fallback passed");

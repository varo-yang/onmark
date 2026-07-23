// The adapter-facing type subpath must remain free of runtime behavior.

import assert from "node:assert/strict";
import test from "node:test";

import * as contracts from "../src/types.js";

test("keeps the adapter contract entrypoint type-only", () => {
  assert.deepEqual(Object.keys(contracts), []);
});

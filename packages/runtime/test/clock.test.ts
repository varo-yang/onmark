// Exact-frame projection contract for browser-facing time APIs.
// Frame identity remains integral while seconds are derived from the Rust-owned rate.

import assert from "node:assert/strict";
import test from "node:test";

import { runtimeFrameAt } from "../src/index.js";

test("projects exact and NTSC frame boundaries from the rational rate", () => {
  assert.deepEqual(runtimeFrameAt(60, { numerator: 30, denominator: 1 }), {
    index: 60,
    timeSeconds: 2,
  });
  assert.deepEqual(
    runtimeFrameAt(30_000, { numerator: 30_000, denominator: 1_001 }),
    { index: 30_000, timeSeconds: 1_001 },
  );
});

test("returns an immutable value and rejects facts outside the wire domain", () => {
  const frame = runtimeFrameAt(1, {
    numerator: 30_000,
    denominator: 1_001,
  });

  assert.equal(frame.timeSeconds, 1_001 / 30_000);
  assert.equal(Object.isFrozen(frame), true);
  assert.throws(
    () =>
      runtimeFrameAt(Number.MAX_SAFE_INTEGER + 1, {
        numerator: 30,
        denominator: 1,
      }),
    TypeError,
  );
  assert.throws(
    () => runtimeFrameAt(0, { numerator: 0, denominator: 1 }),
    TypeError,
  );
});

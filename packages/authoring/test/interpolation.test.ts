// Piecewise interpolation behavior and explicit extrapolation policy.
// Tests exercise the public authoring facade rather than internal segments.

import assert from "node:assert/strict";
import test from "node:test";

import { easing, interpolate } from "../src/index.js";

test("interpolates piecewise values with explicit edge behavior", () => {
  assert.equal(interpolate(0.5, [0, 1], [10, 20]), 15);
  assert.equal(interpolate(1.5, [0, 1, 2], [0, 10, 30]), 20);
  assert.equal(interpolate(-1, [0, 1], [10, 20]), 10);
  assert.equal(
    interpolate(2, [0, 1], [10, 20], {
      extrapolateRight: "extend",
    }),
    30,
  );
});

test("applies easing within the selected interpolation segment", () => {
  assert.equal(
    interpolate(0.5, [0, 1], [0, 100], {
      easing: easing.inCubic,
    }),
    12.5,
  );
  assert.equal(easing.outCubic(0.5), 0.875);
  assert.equal(easing.inOutCubic(0.25), 0.0625);
  assert.equal(easing.inOutCubic(0.75), 0.9375);
});

test("rejects ambiguous interpolation domains", () => {
  assert.throws(() => interpolate(0, [0], [0]), /at least two/);
  assert.throws(() => interpolate(0, [0, 1], [0]), /same length/);
  assert.throws(() => interpolate(0, [0, 0], [0, 1]), /strictly increasing/);
  assert.throws(
    () => interpolate(0.5, [0, 1], [0, 1], { easing: () => Number.NaN }),
    /finite progress/,
  );
  assert.throws(
    () =>
      Reflect.apply(interpolate, undefined, [
        0.5,
        [0, 1],
        [0, 1],
        { extrapolateLeft: "invalid" },
      ]),
    /must be clamp or extend/,
  );
});
